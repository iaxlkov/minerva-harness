pub mod config;
pub mod conversation;
pub mod credential_store;
pub mod engine;
pub mod events;
pub mod editor;
pub mod network_policy;
pub mod plugins;
pub mod providers;
pub mod rig_history;
pub mod run_supervisor;
pub mod runs;
mod runtime_services;
pub use run_supervisor::{RunHandle, RunMessage, RunSnapshot, StartRunRequest};
pub mod context;
pub mod harness;
mod harness_api;
pub mod mcp;
pub mod lsp;
pub mod session_store;
pub mod sessions;
pub mod shell;
pub mod tools;
pub mod transcript;
pub mod workspaces;
pub use config::RuntimeConfig;
use plugins::PluginManager;
use sessions::SessionService;
use std::sync::Arc;

#[derive(Debug)]
pub enum RuntimeError {
    Configuration(String),
    Bootstrap(String),
    Database(String),
    Lifecycle(String),
}
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, detail) = match self {
            Self::Configuration(e) => ("configuration", e),
            Self::Bootstrap(e) => ("bootstrap", e),
            Self::Database(e) => ("database", e),
            Self::Lifecycle(e) => ("lifecycle", e),
        };
        write!(f, "{kind}: {detail}")
    }
}
impl std::error::Error for RuntimeError {}

pub struct Runtime {
    config: RuntimeConfig,
    pub sessions: Arc<SessionService>,
    pub plugins: Arc<PluginManager>,
    agent_loop: Arc<run_supervisor::AgentLoopService>,
    fibers: Vec<cordis::FiberHandle>,
    shutdown: tokio::sync::Mutex<bool>,
}
impl Runtime {
    pub async fn is_stopped(&self) -> bool {
        self.agent_loop.is_stopped().await
    }
    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }
    pub async fn start(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        if [&config.tenant_id, &config.workspace_id, &config.profile]
            .iter()
            .any(|s| s.trim().is_empty())
            || !config.data_dir.is_absolute()
            || !config.config_dir.is_absolute()
        {
            return Err(RuntimeError::Configuration(
                "nonempty scope/profile and absolute paths are required".into(),
            ));
        }
        std::fs::create_dir_all(&config.config_dir)
            .map_err(|e| RuntimeError::Bootstrap(e.to_string()))?;
        let composition = minerva_plugin_host::config::ConfigStore::open(&config.config_dir)
            .and_then(|store| store.snapshot())
            .map_err(RuntimeError::Configuration)?;
        let plugins = Arc::new(
            PluginManager::start_scoped_at(
                config.config_dir.join("shell.json"),
                matches!(config.profile.as_str(), "local" | "desktop"),
                credential_store::CredentialScope::new(
                    serde_json::to_string(&(&config.tenant_id, &config.workspace_id))
                        .map_err(|e| RuntimeError::Configuration(e.to_string()))?,
                )
                .map_err(RuntimeError::Configuration)?,
                config.data_dir.join("context.sqlite3"),
            )
            .await
            .map_err(RuntimeError::Bootstrap)?,
        );
        let mut fibers = Vec::new();
        let services = async {
            let ctx = plugins.context();
            fibers.push(
                ctx.spawn(cordis::PreparedPlugin::from_input(
                    runtime_services::SessionsPlugin(config.data_dir.join("sessions.sqlite3")),
                    (),
                ))
                .await
                .map_err(|e| RuntimeError::Database(e.to_string()))?,
            );
            fibers.push(
                ctx.spawn(cordis::PreparedPlugin::from_input(
                    workspaces::WorkspacePlugin {
                        root: config.data_dir.join("default-workspace"),
                        local: plugins.harness.local,
                    },
                    (),
                ))
                .await
                .map_err(|e| RuntimeError::Bootstrap(e.to_string()))?,
            );
            fibers.push(ctx.spawn(cordis::PreparedPlugin::from_input(editor::EditorPlugin, ())).await.map_err(|e| RuntimeError::Bootstrap(e.to_string()))?);
            fibers.push(ctx.spawn(cordis::PreparedPlugin::from_input(lsp::LspPlugin(plugins.harness.local), ())).await.map_err(|e| RuntimeError::Bootstrap(e.to_string()))?);
            fibers.push(ctx.spawn(cordis::PreparedPlugin::from_input(lsp::RustLanguagePlugin, ())).await.map_err(|e| RuntimeError::Bootstrap(e.to_string()))?);
            plugins.sync_lsp().await.map_err(RuntimeError::Bootstrap)?;
            fibers.push(
                ctx.spawn(cordis::PreparedPlugin::from_input(
                    runtime_services::AgentLoopPlugin {
                        plugins: Arc::downgrade(&plugins),
                        shutdown_timeout: std::time::Duration::from_millis(
                            composition.services.agent_loop.shutdown_timeout_ms,
                        ),
                    },
                    (),
                ))
                .await
                .map_err(|e| RuntimeError::Bootstrap(e.to_string()))?,
            );
            let sessions = ctx
                .try_service::<SessionService>()
                .map_err(|e| RuntimeError::Bootstrap(e.to_string()))?;
            let agent_loop = ctx
                .try_service::<run_supervisor::AgentLoopService>()
                .map_err(|e| RuntimeError::Bootstrap(e.to_string()))?;
            Ok::<_, RuntimeError>((sessions, agent_loop))
        }
        .await;
        let (sessions, agent_loop) = match services {
            Ok(services) => services,
            Err(error) => {
                for fiber in fibers.iter().rev() {
                    let _ = fiber.dispose().await;
                }
                plugins.shutdown().await;
                return Err(error);
            }
        };
        Ok(Self {
            config,
            sessions,
            plugins,
            agent_loop,
            fibers,
            shutdown: tokio::sync::Mutex::new(false),
        })
    }
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let mut stopped = self.shutdown.lock().await;
        if *stopped {
            return Ok(());
        }
        let mut failure = None;
        for fiber in self.fibers.iter().rev() {
            if let Err(error) = fiber.dispose().await {
                failure.get_or_insert_with(|| RuntimeError::Lifecycle(error.to_string()));
            }
        }
        if let Err(error) = self.sessions.shutdown_result() {
            failure.get_or_insert(RuntimeError::Lifecycle(error));
        }
        self.plugins.shutdown().await;
        *stopped = true;
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod runtime_services_tests {
    use super::*;
    #[tokio::test]
    async fn invalid_composition_is_rejected_before_opening_any_runtime_database() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::write(dir.path().join("minerva.conf.json"), r#"{"version":99}"#).unwrap();
        let result = Runtime::start(RuntimeConfig {
            tenant_id: "test".into(),
            workspace_id: "test".into(),
            profile: "local".into(),
            data_dir: data.clone(),
            config_dir: dir.path().into(),
        })
        .await;
        assert!(
            matches!(result, Err(RuntimeError::Configuration(ref error)) if error.contains("minerva.conf.json"))
        );
        assert!(!data.exists());
    }
    #[tokio::test]
    async fn session_and_execution_services_are_system_plugins_with_owned_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let config = RuntimeConfig {
            tenant_id: "test".into(),
            workspace_id: "test".into(),
            profile: "local".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        };
        let runtime = Arc::new(Runtime::start(config.clone()).await.unwrap());
        assert!(Arc::ptr_eq(
            &runtime.sessions,
            &runtime
                .plugins
                .context()
                .try_service::<SessionService>()
                .unwrap()
        ));
        assert!(Arc::ptr_eq(
            &runtime.agent_loop,
            &runtime
                .plugins
                .context()
                .try_service::<run_supervisor::AgentLoopService>()
                .unwrap()
        ));
        let roster = serde_json::to_value(runtime.plugins.list().await.unwrap()).unwrap();
        for id in ["sessions", "agent-loop", "workspaces"] {
            let row = roster
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == id)
                .unwrap_or_else(|| panic!("Missing system service {id}"));
            assert_eq!(row["enabled"], true);
            assert_eq!(row["canToggle"], false);
            assert!(runtime.plugins.set_enabled(id, false).await.is_err());
        }
        runtime.sessions.create("persisted").unwrap();
        let workspaces = runtime
            .plugins
            .context()
            .try_service::<workspaces::WorkspaceService>()
            .unwrap();
        runtime.shutdown().await.unwrap();
        assert!(workspaces.list().is_err());
        assert!(runtime
            .plugins
            .context()
            .try_service::<SessionService>()
            .is_err());
        assert!(runtime
            .plugins
            .context()
            .try_service::<run_supervisor::AgentLoopService>()
            .is_err());
        assert!(runtime.is_stopped().await);
        assert!(runtime
            .start_run(StartRunRequest {
                session_id: "persisted".into(),
                request_id: "late".into(),
                parent_id: None,
                message: RunMessage {
                    id: "late-message".into(),
                    text: "hello".into()
                },
                model: "unconfigured".into(),
                options: serde_json::json!({}),
            })
            .await
            .unwrap_err()
            .contains("stopped"));
        runtime.shutdown().await.unwrap();
        let reopened = Runtime::start(config).await.unwrap();
        assert_eq!(reopened.sessions.get("persisted").unwrap().id, "persisted");
        reopened.shutdown().await.unwrap();
    }
}
