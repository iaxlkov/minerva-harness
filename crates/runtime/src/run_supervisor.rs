use crate::{
    conversation::{run_inner, RunRequest},
    sessions::ContentPart,
    Runtime,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct RunMessage {
    pub id: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct StartRunRequest {
    pub session_id: String,
    pub request_id: String,
    pub parent_id: Option<String>,
    pub message: RunMessage,
    pub model: String,
    #[serde(default)]
    #[ts(type = "JsonValue")]
    pub options: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct RunHandle {
    pub session_id: String,
    pub run_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct RunSnapshot {
    pub session_id: String,
    pub run_id: String,
    pub status: String,
    pub parts: Vec<ContentPart>,
    pub error: Option<String>,
}
impl Runtime {
    pub async fn start_run(
        self: &Arc<Self>,
        request: StartRunRequest,
    ) -> Result<RunHandle, String> {
        let stopped = self.stopped.lock().await;
        if *stopped {
            return Err("runtime is stopped".into());
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
        let runtime = Arc::clone(self);
        let settlement = OwnedSettlement {
            store: runtime.sessions.clone(),
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
            let result = run_inner(
                &runtime.sessions,
                &runtime.plugins,
                &session,
                execution,
                &mut guard,
                true,
            )
            .await;
            if let Err(error) = result {
                let _ =
                    runtime
                        .sessions
                        .fail_run(&session, &run_id, *guard.cancel.borrow(), &error);
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
