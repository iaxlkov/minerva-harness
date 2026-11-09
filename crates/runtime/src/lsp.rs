//! Workspace-scoped LSP process lifecycle.
use cordis::{Context, Plugin, Service};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    convert::Infallible,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_lsp_client::{
    Client,
    client::{NotificationParameters, OutgoingRequest, RequestParameters, ServerMessage},
    types::{InitializeParams, WorkspaceFolder},
};

#[derive(Clone, Debug)]
pub struct LanguageServer {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    /// Lowercase file extension (including the dot) to LSP language ID.
    pub extensions: HashMap<String, String>,
}

#[derive(Default)]
struct State {
    providers: HashMap<String, LanguageServer>,
    servers: HashMap<(String, String), Running>,
}

struct Running {
    client: Client,
    events: tokio::task::JoinHandle<()>,
    diagnostics: Arc<Mutex<HashMap<String, serde_json::Value>>>,
}

enum JsonResponse {}
impl OutgoingRequest for JsonResponse {
    type Params = serde_json::Value;
    type Result = serde_json::Value;
    const METHOD: &'static str = "mh/response";
}
#[derive(Default)]
pub struct LspService {
    state: Mutex<State>,
    stopped: AtomicBool,
}
impl Service for LspService {
    const NAME: &'static str = "mh.lsp";
}
impl LspService {
    /// Registers one provider atomically; duplicate IDs or extensions are rejected.
    pub async fn register(&self, provider: LanguageServer) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return Err("LSP service is stopped".into());
        }
        if provider.id.trim().is_empty()
            || provider.command.trim().is_empty()
            || provider.extensions.is_empty()
            || provider.extensions.iter().any(|(ext, lang)| {
                !ext.starts_with('.')
                    || ext.len() < 2
                    || ext != &ext.to_lowercase()
                    || lang.trim().is_empty()
            })
        {
            return Err("Invalid LSP provider".into());
        }
        if state.providers.contains_key(&provider.id)
            || state.providers.values().any(|other| {
                other
                    .extensions
                    .keys()
                    .any(|ext| provider.extensions.contains_key(ext))
            })
        {
            return Err("LSP provider or extension conflict".into());
        }
        state.providers.insert(provider.id.clone(), provider);
        Ok(())
    }

    pub async fn provider_for_file(&self, path: &Path) -> Option<LanguageServer> {
        let extension = format!(".{}", path.extension()?.to_str()?.to_lowercase());
        self.state
            .lock()
            .await
            .providers
            .values()
            .find(|provider| provider.extensions.contains_key(&extension))
            .cloned()
    }

    pub async fn unregister(&self, id: &str) {
        let mut state = self.state.lock().await;
        state.providers.remove(id);
        let keys: Vec<_> = state
            .servers
            .keys()
            .filter(|(_, server)| server == id)
            .cloned()
            .collect();
        let servers: Vec<_> = keys
            .into_iter()
            .filter_map(|key| state.servers.remove(&key))
            .collect();
        drop(state);
        for server in servers {
            stop_server(server).await;
        }
    }

    pub async fn start(&self, workspace: &str, root: &Path, id: &str) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return Err("LSP service is stopped".into());
        }
        let provider = state
            .providers
            .get(id)
            .cloned()
            .ok_or("LSP provider not found")?;
        state
            .servers
            .retain(|_, server| !server.events.is_finished());
        let key = (workspace.to_owned(), id.to_owned());
        if state.servers.contains_key(&key) {
            return Ok(());
        }
        let uri = url::Url::from_directory_path(root)
            .map_err(|_| "Invalid workspace root")?
            .to_string();
        let folder = WorkspaceFolder {
            uri: uri.parse().map_err(|_| "Invalid workspace URI")?,
            name: workspace.to_owned(),
        };
        let mut cmd = tokio::process::Command::new(&provider.command);
        cmd.args(&provider.args).current_dir(root);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let (mut client, mut incoming) =
            Client::from_command(&mut cmd).map_err(|e| e.to_string())?;
        let diagnostics = Arc::new(Mutex::new(HashMap::new()));
        let observed = diagnostics.clone();
        let responder = client.clone();
        let workspace_folder = serde_json::to_value(&folder).map_err(|e| e.to_string())?;
        let events = tokio::spawn(async move {
            while let Some(message) = incoming.next().await {
                match message {
                    ServerMessage::Notification {
                        parameters: NotificationParameters::PublishDiagnostics(params),
                    } => {
                        let uri = params.uri.to_string();
                        if let Ok(value) = serde_json::to_value(params) {
                            let mut entries = observed.lock().await;
                            if entries.len() < 1000 || entries.contains_key(&uri) {
                                entries.insert(uri, value);
                            }
                        }
                    }
                    ServerMessage::Request { id, parameters } => {
                        let result = match parameters {
                            RequestParameters::WorkspaceConfiguration(params) => {
                                serde_json::Value::Array(vec![
                                    serde_json::Value::Null;
                                    params.items.len()
                                ])
                            }
                            RequestParameters::WorkspaceFoldersRequest(_) => {
                                serde_json::json!([workspace_folder])
                            }
                            RequestParameters::WorkDoneProgressCreate(_) => serde_json::Value::Null,
                            _ => {
                                let error = serde_json::from_value(serde_json::json!({"code": -32601, "message": "Method not supported"})).expect("valid LSP error");
                                let _ = responder.send_error(id, error).await;
                                continue;
                            }
                        };
                        let _ = responder.send_result::<JsonResponse>(id, result).await;
                    }
                    _ => {}
                }
            }
        });
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            client
                .initialize(InitializeParams {
                    workspace_folders: Some(vec![folder]),
                    ..Default::default()
                })
                .await
                .map_err(|e| e.to_string())?;
            client.initialized().await.map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "LSP initialization timed out".to_owned())
        .and_then(|x| x);
        if let Err(error) = result {
            events.abort();
            return Err(error);
        }
        state.servers.insert(
            key,
            Running {
                client,
                events,
                diagnostics,
            },
        );
        Ok(())
    }
    pub async fn stop(&self, workspace: &str, id: &str) -> Result<(), String> {
        let server = self
            .state
            .lock()
            .await
            .servers
            .remove(&(workspace.to_owned(), id.to_owned()));
        if let Some(server) = server {
            stop_server(server).await;
        }
        Ok(())
    }
    pub async fn stop_workspace(&self, workspace: &str) {
        let mut state = self.state.lock().await;
        let keys: Vec<_> = state
            .servers
            .keys()
            .filter(|(w, _)| w == workspace)
            .cloned()
            .collect();
        let servers: Vec<_> = keys
            .into_iter()
            .filter_map(|key| state.servers.remove(&key))
            .collect();
        drop(state);
        for server in servers {
            stop_server(server).await;
        }
    }
    pub async fn diagnostics(&self, workspace: &str, id: &str) -> Vec<serde_json::Value> {
        let diagnostics = self
            .state
            .lock()
            .await
            .servers
            .get(&(workspace.to_owned(), id.to_owned()))
            .map(|server| server.diagnostics.clone());
        match diagnostics {
            Some(entries) => entries.lock().await.values().cloned().collect(),
            None => vec![],
        }
    }
    pub async fn running(&self, workspace: &str, id: &str) -> bool {
        self.state
            .lock()
            .await
            .servers
            .get(&(workspace.to_owned(), id.to_owned()))
            .is_some_and(|server| !server.events.is_finished())
    }
    async fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        let servers: Vec<_> = self
            .state
            .lock()
            .await
            .servers
            .drain()
            .map(|(_, server)| server)
            .collect();
        for server in servers {
            stop_server(server).await;
        }
    }
}

async fn stop_server(mut server: Running) {
    let _ = tokio::time::timeout(Duration::from_secs(2), server.client.shutdown()).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), server.client.exit()).await;
    server.events.abort();
}

pub(crate) struct LspPlugin;
impl Plugin for LspPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, _: ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        ctx.try_service::<crate::workspaces::WorkspaceService>()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let service = Arc::new(LspService::default());
        let owned = service.clone();
        let _cleanup = ctx
            .effect(move || async move {
                owned.shutdown().await;
                Ok::<_, std::io::Error>(())
            })
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let _publication = ctx
            .provide(service)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }
}

/// Built-in Rust language contribution. Other native plugins can register their own providers.
pub struct RustLanguageService;
impl Service for RustLanguageService {
    const NAME: &'static str = "mh.lsp.rust";
}
pub(crate) struct RustLanguagePlugin;
impl Plugin for RustLanguagePlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, _: ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let lsp = ctx
            .try_service::<LspService>()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        lsp.register(LanguageServer {
            id: "rust".into(),
            command: "rust-analyzer".into(),
            args: vec![],
            extensions: HashMap::from([(".rs".into(), "rust".into())]),
        })
        .await
        .map_err(std::io::Error::other)?;
        let owned = lsp.clone();
        if let Err(error) = ctx.effect(move || async move {
            owned.unregister("rust").await;
            Ok::<_, std::io::Error>(())
        }) {
            lsp.unregister("rust").await;
            return Err(std::io::Error::other(error.to_string()));
        }
        let _publication = ctx
            .provide(Arc::new(RustLanguageService))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn provider(id: &str, extension: &str) -> LanguageServer {
        LanguageServer {
            id: id.into(),
            command: "server".into(),
            args: vec![],
            extensions: HashMap::from([(extension.into(), id.into())]),
        }
    }

    #[tokio::test]
    async fn providers_are_selected_and_conflicts_are_atomic() {
        let service = LspService::default();
        service.register(provider("rust", ".rs")).await.unwrap();
        assert_eq!(
            service
                .provider_for_file(Path::new("main.RS"))
                .await
                .unwrap()
                .id,
            "rust"
        );
        assert!(service.register(provider("other", ".rs")).await.is_err());
        assert!(service.register(provider("rust", ".py")).await.is_err());
        assert!(
            service
                .provider_for_file(Path::new("main.py"))
                .await
                .is_none()
        );
        service.unregister("rust").await;
        assert!(
            service
                .provider_for_file(Path::new("main.rs"))
                .await
                .is_none()
        );
        service.register(provider("other", ".rs")).await.unwrap();
    }

    #[tokio::test]
    async fn rust_plugin_registration_follows_its_fiber() {
        let ctx = Context::new();
        let lsp = Arc::new(LspService::default());
        let _publication = ctx.provide(lsp.clone()).unwrap();
        let fiber = ctx
            .spawn(cordis::PreparedPlugin::from_input(RustLanguagePlugin, ()))
            .await
            .unwrap();
        assert_eq!(
            lsp.provider_for_file(Path::new("lib.rs"))
                .await
                .unwrap()
                .command,
            "rust-analyzer"
        );
        assert!(ctx.try_service::<RustLanguageService>().is_ok());
        fiber.dispose().await.unwrap();
        assert!(lsp.provider_for_file(Path::new("lib.rs")).await.is_none());
        assert!(ctx.try_service::<RustLanguageService>().is_err());
    }

    #[tokio::test]
    #[ignore = "requires Python 3 for the fixture language server"]
    async fn isolates_workspace_processes_and_stops_them() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mh_lsp_server.py");
        let path = fixture.to_str().unwrap();
        let service = LspService::default();
        service
            .register(LanguageServer {
                id: "test".into(),
                command: "python".into(),
                args: vec![path.into()],
                extensions: HashMap::from([(".test".into(), "test".into())]),
            })
            .await
            .unwrap();
        service
            .register(LanguageServer {
                id: "other".into(),
                command: "python".into(),
                args: vec![path.into()],
                extensions: HashMap::from([(".other".into(), "other".into())]),
            })
            .await
            .unwrap();
        service.start("a", &a, "test").await.unwrap();
        service.start("a", &a, "other").await.unwrap();
        service.start("b", &b, "test").await.unwrap();
        assert!(service.running("a", "test").await);
        assert!(service.running("b", "test").await);
        tokio::time::timeout(Duration::from_secs(3), async {
            while service.diagnostics("a", "test").await.is_empty()
                || service.diagnostics("b", "test").await.is_empty()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("server configuration response and diagnostics");
        assert!(service.running("a", "other").await);
        service.unregister("other").await;
        assert!(!service.running("a", "other").await);
        assert!(service.running("a", "test").await);
        service.stop("a", "test").await.unwrap();
        assert!(!service.running("a", "test").await);
        assert!(service.running("b", "test").await);
        service.shutdown().await;
        assert!(!service.running("b", "test").await);
        assert!(service.start("a", &a, "test").await.is_err());
    }
}
