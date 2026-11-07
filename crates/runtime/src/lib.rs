pub mod config;
pub mod conversation;
pub mod credential_store;
pub mod engine;
pub mod events;
pub mod network_policy;
pub mod plugins;
pub mod providers;
pub mod rig_history;
pub mod run_supervisor;
pub mod runs;
pub use run_supervisor::{RunHandle, RunMessage, RunSnapshot, StartRunRequest};
pub mod context;
pub mod harness;
mod harness_api;
pub mod mcp;
pub mod session_store;
pub mod sessions;
pub mod shell;
pub mod tools;
pub mod transcript;
pub use config::RuntimeConfig;
use plugins::PluginManager;
use runs::Runs;
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
    pub runs: Arc<Runs>,
    tasks: tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    stopped: tokio::sync::Mutex<bool>,
}
impl Runtime {
    pub async fn is_stopped(&self) -> bool {
        *self.stopped.lock().await
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
        let sessions = Arc::new(
            SessionService::open(&config.data_dir.join("sessions.sqlite3"))
                .map_err(RuntimeError::Database)?,
        );
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
        Ok(Self {
            config,
            sessions,
            plugins,
            runs: Arc::new(Runs::default()),
            tasks: tokio::sync::Mutex::new(Vec::new()),
            stopped: tokio::sync::Mutex::new(false),
        })
    }
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let mut stopped = self.stopped.lock().await;
        if *stopped {
            return Ok(());
        }
        *stopped = true;
        self.runs.cancel_all();
        let mut tasks = self.tasks.lock().await;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        for mut task in tasks.drain(..) {
            if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        self.plugins.shutdown().await;
        self.sessions.shutdown().map_err(RuntimeError::Lifecycle)?;
        *stopped = true;
        Ok(())
    }
}
#[cfg(test)]
mod history_tests;
