use crate::{
    conversation::{run_inner, RunRequest},
    plugins::PluginManager,
    runs::Runs,
    sessions::SessionService,
};
use std::sync::{Arc, Weak};
use tokio::sync::Mutex;

pub struct AgentLoopService {
    sessions: Arc<SessionService>,
    plugins: Weak<PluginManager>,
    runs: Runs,
    stopped: Mutex<bool>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    shutdown_timeout: std::time::Duration,
}
impl cordis::Service for AgentLoopService {
    const NAME: &'static str = "studio.agent-loop";
}
impl AgentLoopService {
    pub(crate) fn new(
        sessions: Arc<SessionService>,
        plugins: Weak<PluginManager>,
        shutdown_timeout: std::time::Duration,
    ) -> Self {
        Self {
            sessions,
            plugins,
            runs: Runs::default(),
            stopped: Mutex::new(false),
            tasks: Mutex::new(Vec::new()),
            shutdown_timeout,
        }
    }
    pub async fn is_stopped(&self) -> bool {
        *self.stopped.lock().await
    }
    pub(crate) async fn shutdown(&self) {
        let mut stopped = self.stopped.lock().await;
        if *stopped {
            return;
        }
        *stopped = true;
        self.runs.cancel_all();
        let mut tasks = self.tasks.lock().await;
        let deadline = tokio::time::Instant::now() + self.shutdown_timeout;
        for mut task in tasks.drain(..) {
            if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
    }
}
pub use mh_sessions::{RunHandle, RunMessage, RunSnapshot, StartRunRequest};
impl AgentLoopService {
    pub async fn start_run(&self, mut request: StartRunRequest) -> Result<RunHandle, String> {
        let stopped = self.stopped.lock().await;
        if *stopped {
            return Err("runtime is stopped".into());
        }
        let plugins = self
            .plugins
            .upgrade()
            .ok_or("plugin services are stopped")?;
        self.sessions.get(&request.session_id)?;
        if let Some(value) = request.options.get("attachments") {
            let supplied: Vec<minerva_plugin_sdk::context::AttachmentRef> =
                serde_json::from_value(value.clone())
                    .map_err(|_| "Invalid attachment references")?;
            if supplied.len() > 8 {
                return Err("At most eight attachments can be sent in one message".into());
            }
            let mut total = 0usize;
            let mut canonical = Vec::new();
            for reference in supplied {
                let stored = plugins
                    .harness
                    .context
                    .get_attachment(&request.session_id, &reference.id)?;
                total = total
                    .checked_add(stored.bytes)
                    .ok_or("Attachment size overflow")?;
                if total > 20 * 1024 * 1024 {
                    return Err("Combined attachment size exceeds 20 MiB".into());
                }
                canonical.push(minerva_plugin_sdk::context::AttachmentRef {
                    id: stored.id,
                    name: stored.name,
                    mime_type: stored.mime_type,
                    bytes: stored.bytes,
                });
            }
            request.options["attachments"] =
                serde_json::to_value(canonical).map_err(|e| e.to_string())?;
        }
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|task| !task.is_finished());
        let created_at = self.sessions.server_timestamp()?;
        let accepted = self.sessions.accept_run(&request, &created_at)?;
        let handle = RunHandle {
            session_id: request.session_id.clone(),
            run_id: request.request_id.clone(),
        };
        if !accepted {
            return Ok(handle);
        }
        let mut guard = match self.runs.acquire(&request.session_id, &request.request_id) {
            Ok(g) => g,
            Err(e) => {
                self.sessions
                    .fail_run(&request.session_id, &request.request_id, false, &e)?;
                return Err(e);
            }
        };
        let sessions = self.sessions.clone();
        let settlement = OwnedSettlement {
            store: sessions.clone(),
            session: request.session_id.clone(),
            id: request.request_id.clone(),
        };
        tasks.push(tokio::spawn(async move {
            let _settlement = settlement;
            let session = request.session_id;
            let run_id = request.request_id;
            let execution = RunRequest {
                model: request.model,
                last_message_id: request.message.id,
                response_id: run_id.clone(),
                created_at,
                options: request.options,
            };
            let result =
                run_inner(&sessions, &plugins, &session, execution, &mut guard, true).await;
            if let Err(error) = result {
                let _ = sessions.fail_run(&session, &run_id, *guard.cancel.borrow(), &error);
            }
        }));
        Ok(handle)
    }
    pub fn get_run(&self, session: &str, run_id: &str) -> Result<RunSnapshot, String> {
        self.sessions.run_snapshot(session, run_id)
    }
    pub fn cancel_run(&self, session: &str, run_id: &str) -> Result<bool, String> {
        self.runs.cancel(session, run_id)
    }
    pub async fn run_legacy(
        &self,
        session: String,
        request: RunRequest,
    ) -> Result<crate::conversation::SessionRunResult, String> {
        let stopped = self.stopped.lock().await;
        if *stopped {
            return Err("runtime is stopped".into());
        }
        let plugins = self
            .plugins
            .upgrade()
            .ok_or("plugin services are stopped")?;
        let sessions = self.sessions.clone();
        let mut guard = self.runs.acquire(&session, &request.response_id)?;
        let (send, receive) = tokio::sync::oneshot::channel();
        let mut tasks = self.tasks.lock().await;
        tasks.retain(|task| !task.is_finished());
        tasks.push(tokio::spawn(async move {
            let result = run_inner(&sessions, &plugins, &session, request, &mut guard, false).await;
            let _ = send.send(result);
        }));
        drop(tasks);
        drop(stopped);
        receive
            .await
            .map_err(|_| "run task interrupted before settlement".to_string())?
    }
}

struct OwnedSettlement {
    store: Arc<crate::sessions::SessionService>,
    session: String,
    id: String,
}
impl Drop for OwnedSettlement {
    fn drop(&mut self) {
        let _ = self.store.fail_run(
            &self.session,
            &self.id,
            false,
            "run task interrupted before settlement",
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_aborts_and_joins_noncooperative_tasks_after_configured_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let sessions =
            Arc::new(SessionService::open(&dir.path().join("sessions.sqlite3")).unwrap());
        let service = AgentLoopService::new(
            sessions.clone(),
            Weak::new(),
            std::time::Duration::from_millis(100),
        );
        let guard = service.runs.acquire("session", "run").unwrap();
        let (send, receive) = tokio::sync::oneshot::channel();
        service.tasks.lock().await.push(tokio::spawn(async move {
            let _guard = guard;
            let _ = send.send(());
            std::future::pending::<()>().await;
        }));
        receive.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), service.shutdown())
            .await
            .unwrap();
        assert!(service.is_stopped().await);
        assert!(service.tasks.lock().await.is_empty());
        assert!(
            service.runs.acquire("session", "next").is_ok(),
            "aborted task retained its run guard"
        );
        service.shutdown().await;
        sessions.shutdown().unwrap();
    }
}
