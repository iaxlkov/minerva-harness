//! Workspace-scoped LSP process lifecycle.
use cordis::Service;
use futures_util::StreamExt;
use std::{
    collections::HashMap,
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
    client::{
        NotificationParameters, OutgoingNotification, OutgoingRequest, RequestParameters,
        ServerMessage,
    },
    types::{InitializeParams, WorkspaceFolder},
};

#[derive(Clone, Debug, PartialEq, Eq)]
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
    documents: HashMap<String, (String, i64)>,
}

enum JsonResponse {}
impl OutgoingRequest for JsonResponse {
    type Params = serde_json::Value;
    type Result = serde_json::Value;
    const METHOD: &'static str = "mh/response";
}
macro_rules! notification {
    ($name:ident, $method:literal) => {
        enum $name {}
        impl OutgoingNotification for $name {
            type Params = serde_json::Value;
            const METHOD: &'static str = $method;
        }
    };
}
notification!(DidOpen, "textDocument/didOpen");
notification!(DidChange, "textDocument/didChange");
notification!(DidClose, "textDocument/didClose");
macro_rules! request {
    ($name:ident, $method:literal) => {
        enum $name {}
        impl OutgoingRequest for $name {
            type Params = serde_json::Value;
            type Result = serde_json::Value;
            const METHOD: &'static str = $method;
        }
    };
}
request!(Completion, "textDocument/completion");
request!(Hover, "textDocument/hover");
request!(Definition, "textDocument/definition");
request!(References, "textDocument/references");

pub fn document_uri(root: &Path, path: &str) -> Result<String, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("Invalid workspace file path".into());
    }
    let joined = root.join(path);
    let file = match joined.canonicalize() {
        Ok(file) if file.is_file() => file,
        Ok(_) => return Err("Path is not a file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(&joined).is_ok() {
                return Err("Unresolvable workspace file".into());
            }
            let parent = joined
                .parent()
                .ok_or("Invalid file path")?
                .canonicalize()
                .map_err(|e| e.to_string())?;
            parent.join(joined.file_name().ok_or("Invalid file path")?)
        }
        Err(error) => return Err(error.to_string()),
    };
    if !file.starts_with(&root) {
        return Err("File is outside workspace".into());
    }
    url::Url::from_file_path(file)
        .map(|url| url.to_string())
        .map_err(|_| "Invalid file URI".into())
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

    pub async fn external_provider_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .await
            .providers
            .keys()
            .filter(|id| id.starts_with("plugin:"))
            .cloned()
            .collect()
    }

    pub async fn provider(&self, id: &str) -> Option<LanguageServer> {
        self.state.lock().await.providers.get(id).cloned()
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
                documents: HashMap::new(),
            },
        );
        Ok(())
    }
    pub async fn document_owner(
        &self,
        workspace: &str,
        root: &Path,
        path: &str,
    ) -> Result<Option<String>, String> {
        let uri = document_uri(root, path)?;
        let provider = self
            .provider_for_file(Path::new(path))
            .await
            .ok_or("No language server for file")?;
        Ok(self
            .state
            .lock()
            .await
            .servers
            .get(&(workspace.into(), provider.id))
            .and_then(|server| server.documents.get(&uri).map(|(owner, _)| owner.clone())))
    }

    pub async fn open_document(
        &self,
        workspace: &str,
        root: &Path,
        session: &str,
        path: &str,
        version: i64,
        text: &str,
    ) -> Result<String, String> {
        if !(0..=i32::MAX as i64).contains(&version) || text.len() > 4_000_000 {
            return Err("Invalid document version or size".into());
        }
        let uri = document_uri(root, path)?;
        let provider = self
            .provider_for_file(Path::new(path))
            .await
            .ok_or("No language server for file")?;
        let ext = format!(
            ".{}",
            Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .ok_or("File has no extension")?
                .to_lowercase()
        );
        let language = provider.extensions.get(&ext).ok_or("Unknown language")?;
        self.start(workspace, root, &provider.id).await?;
        let mut state = self.state.lock().await;
        let server = state
            .servers
            .get_mut(&(workspace.into(), provider.id.clone()))
            .ok_or("LSP server stopped")?;
        if server.documents.contains_key(&uri) {
            return Err("Document is already open in this workspace".into());
        }
        server.client.send_notification::<DidOpen>(serde_json::json!({"textDocument":{"uri":uri,"languageId":language,"version":version,"text":text}})).await.map_err(|e| e.to_string())?;
        server.documents.insert(uri, (session.into(), version));
        Ok(provider.id)
    }

    pub async fn update_document(
        &self,
        workspace: &str,
        root: &Path,
        session: &str,
        path: &str,
        version: i64,
        text: Option<&str>,
    ) -> Result<(), String> {
        if text.is_some_and(|text| text.len() > 4_000_000) {
            return Err("Document too large".into());
        }
        let uri = document_uri(root, path)?;
        let provider = self
            .provider_for_file(Path::new(path))
            .await
            .ok_or("No language server for file")?;
        let mut state = self.state.lock().await;
        let server = state
            .servers
            .get_mut(&(workspace.into(), provider.id))
            .ok_or("LSP server stopped")?;
        let (owner, previous) = server.documents.get(&uri).ok_or("Document is not open")?;
        if owner != session {
            return Err("Document is open in another session".into());
        }
        if let Some(text) = text {
            if version <= *previous || version > i32::MAX as i64 {
                return Err("Document version must increase".into());
            }
            server.client.send_notification::<DidChange>(serde_json::json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":text}]})).await.map_err(|e| e.to_string())?;
            server.documents.insert(uri, (session.into(), version));
        } else {
            server
                .client
                .send_notification::<DidClose>(serde_json::json!({"textDocument":{"uri":uri}}))
                .await
                .map_err(|e| e.to_string())?;
            server.documents.remove(&uri);
            server.diagnostics.lock().await.remove(&uri);
        }
        Ok(())
    }

    pub async fn query_document(
        &self,
        workspace: &str,
        root: &Path,
        session: &str,
        path: &str,
        method: &str,
        line: u32,
        character: u32,
    ) -> Result<serde_json::Value, String> {
        let uri = document_uri(root, path)?;
        let provider = self
            .provider_for_file(Path::new(path))
            .await
            .ok_or("No language server for file")?;
        let state = self.state.lock().await;
        let server = state
            .servers
            .get(&(workspace.into(), provider.id))
            .ok_or("LSP server stopped")?;
        if server
            .documents
            .get(&uri)
            .is_none_or(|(owner, _)| owner != session)
        {
            return Err("Document is not open in this session".into());
        }
        let client = server.client.clone();
        drop(state);
        let mut params = serde_json::json!({"textDocument":{"uri":uri}, "position":{"line":line,"character":character}});
        if method == "references" {
            params["context"] = serde_json::json!({"includeDeclaration":true});
        }
        macro_rules! call {
            ($t:ty) => {{
                let mut response = client
                    .send_request::<$t>(params)
                    .await
                    .map_err(|e| e.to_string())?;
                tokio::time::timeout(Duration::from_secs(15), response.next())
                    .await
                    .map_err(|_| "LSP request timed out".to_owned())?
                    .ok_or("LSP response stream closed")?
                    .map_err(|e| format!("{}: {}", e.code, e.message))
            }};
        }
        match method {
            "completion" => call!(Completion),
            "hover" => call!(Hover),
            "definition" => call!(Definition),
            "references" => call!(References),
            _ => Err("Unsupported LSP request".into()),
        }
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
    pub async fn document_diagnostics(
        &self,
        workspace: &str,
        root: &Path,
        session: &str,
        path: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let uri = document_uri(root, path)?;
        let provider = self
            .provider_for_file(Path::new(path))
            .await
            .ok_or("No language server for file")?;
        let state = self.state.lock().await;
        let server = state
            .servers
            .get(&(workspace.into(), provider.id))
            .ok_or("LSP server stopped")?;
        if server
            .documents
            .get(&uri)
            .is_none_or(|(owner, _)| owner != session)
        {
            return Err("Document is not open in this session".into());
        }
        let diagnostics = server.diagnostics.clone();
        drop(state);
        let value = diagnostics.lock().await.get(&uri).cloned();
        Ok(value.into_iter().collect())
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
    pub async fn shutdown(&self) {
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
    #[ignore = "requires rust-analyzer"]
    async fn rust_analyzer_returns_hover_for_workspace_document() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"mh-lsp-proof\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let source = "fn greeting() -> i32 { 1 }\nfn main() { let _ = greeting(); }\n";
        std::fs::write(temp.path().join("src/main.rs"), source).unwrap();
        let service = LspService::default();
        service
            .register(LanguageServer {
                id: "rust".into(),
                command: "rust-analyzer".into(),
                args: vec![],
                extensions: HashMap::from([(".rs".into(), "rust".into())]),
            })
            .await
            .unwrap();
        service
            .open_document("rust-project", temp.path(), "s", "src/main.rs", 1, source)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                if let Ok(result) = service
                    .query_document(
                        "rust-project",
                        temp.path(),
                        "s",
                        "src/main.rs",
                        "hover",
                        1,
                        22,
                    )
                    .await
                {
                    if !result.is_null() && result.to_string().contains("greeting") {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .expect("rust-analyzer hover");
        service.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "requires Python 3 for the fixture language server"]
    async fn documents_are_versioned_and_scoped_to_session_and_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mh_lsp_server.py");
        let service = LspService::default();
        service
            .register(LanguageServer {
                id: "test".into(),
                command: "python".into(),
                args: vec![fixture.to_str().unwrap().into()],
                extensions: HashMap::from([(".test".into(), "test".into())]),
            })
            .await
            .unwrap();
        service
            .open_document("a", &a, "session-a", "new.test", 1, "before")
            .await
            .unwrap();
        service
            .open_document("b", &b, "session-b", "new.test", 1, "other")
            .await
            .unwrap();
        assert!(
            service
                .open_document("a", &a, "other", "new.test", 1, "bad")
                .await
                .is_err()
        );
        assert!(
            service
                .update_document("a", &a, "session-a", "new.test", 1, Some("bad"))
                .await
                .is_err()
        );
        assert!(
            service
                .update_document("a", &a, "other", "new.test", 2, Some("bad"))
                .await
                .is_err()
        );
        assert!(
            service
                .open_document("a", &a, "session-a", "../escape.test", 1, "bad")
                .await
                .is_err()
        );
        service
            .update_document("a", &a, "session-a", "new.test", 2, Some("after"))
            .await
            .unwrap();
        assert_eq!(
            service
                .query_document("a", &a, "session-a", "new.test", "hover", 0, 0)
                .await
                .unwrap()["contents"],
            "after"
        );
        assert_eq!(
            service
                .query_document("b", &b, "session-b", "new.test", "hover", 0, 0)
                .await
                .unwrap()["contents"],
            "other"
        );
        service
            .update_document("a", &a, "session-a", "new.test", 0, None)
            .await
            .unwrap();
        assert!(
            service
                .query_document("a", &a, "session-a", "new.test", "hover", 0, 0)
                .await
                .is_err()
        );
        service.shutdown().await;
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

pub struct LspPlugin(pub bool);
impl cordis::Plugin for LspPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = std::convert::Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, _: ()) -> Result<(), std::convert::Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: cordis::Context, _: &()) -> Result<(), Self::ApplyError> {
        let workspaces = ctx
            .try_service::<mh_workspaces::WorkspaceService>()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let service = Arc::new(LspService::default());
        if self.0 {
            let tools = ctx
                .try_service::<mh_tool_runtime::ToolRegistry>()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            let owned = tools.clone();
            let _revoke = ctx
                .effect_sync(move || owned.revoke_owner("native.lsp"))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            let lsp = service.clone();
            let serial = Arc::new(Mutex::new(()));
            tools.register("native.lsp", mh_tool_runtime::ToolDefinition {
                name:"lsp".into(), description:"Navigate source code using the workspace language server. Positions are zero-based UTF-16.".into(),
                input_schema:serde_json::json!({"type":"object","properties":{"path":{"type":"string","minLength":1},"operation":{"type":"string","enum":["hover","definition","references"]},"line":{"type":"integer","minimum":0,"maximum":4294967295u64},"character":{"type":"integer","minimum":0,"maximum":4294967295u64}},"required":["path","operation","line","character"],"additionalProperties":false}),
                output_schema:serde_json::json!({"type":"object","properties":{"result":{}},"required":["result"]}), side_effect:false,
            }, move |context,args| {
                let lsp=lsp.clone();let workspaces=workspaces.clone();let serial=serial.clone();
                Box::pin(async move { tokio::spawn(async move {
                    let _guard=serial.lock().await;
                    let project=workspaces.list()?.into_iter().find(|project|project.session_ids.contains(&context.session_id)).ok_or("Workspace not found")?;
                    let root=std::path::PathBuf::from(project.path.ok_or("Workspace directory unavailable")?);
                    let path=args["path"].as_str().ok_or("Missing path")?;
                    let operation=args["operation"].as_str().ok_or("Missing operation")?;
                    let owner=lsp.document_owner(&project.id,&root,path).await?;
                    let opened=match owner {
                        Some(owner) if owner==context.session_id => false,
                        Some(_) => return Err("Document is open in another session".into()),
                        None => {
                            let uri=document_uri(&root,path)?;
                            let file=url::Url::parse(&uri).map_err(|e|e.to_string())?.to_file_path().map_err(|_|"Invalid file URI")?;
                            let metadata=tokio::fs::metadata(&file).await.map_err(|e|e.to_string())?;
                            if metadata.len()>4_000_000 { return Err("Document too large".into()); }
                            let source=tokio::fs::read_to_string(file).await.map_err(|e|e.to_string())?;
                            lsp.open_document(&project.id,&root,&context.session_id,path,1,&source).await?;
                            true
                        }
                    };
                    let query=lsp.query_document(&project.id,&root,&context.session_id,path,operation,args["line"].as_u64().ok_or("Missing line")? as u32,args["character"].as_u64().ok_or("Missing character")? as u32);
                    let result=tokio::select! { result=query => result, _=context.cancellation.cancelled()=>Err("LSP operation cancelled".into()) };
                    if opened { let _=lsp.update_document(&project.id,&root,&context.session_id,path,0,None).await; }
                    result.map(|result|serde_json::json!({"result":result}))
                }).await.map_err(|error| error.to_string())? })
            }).map_err(std::io::Error::other)?;
        }
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
