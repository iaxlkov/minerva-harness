pub use mh_agent_loop::run_supervisor::*;
use crate::{conversation::RunRequest, Runtime};
use std::sync::Arc;
impl Runtime {
    pub async fn start_run(
        self: &Arc<Self>,
        request: StartRunRequest,
    ) -> Result<RunHandle, String> {
        self.agent_loop.start_run(request).await
    }
    pub fn get_run(&self, session: &str, run_id: &str) -> Result<RunSnapshot, String> {
        self.agent_loop.get_run(session, run_id)
    }
    pub fn cancel_run(&self, session: &str, run_id: &str) -> Result<bool, String> {
        self.agent_loop.cancel_run(session, run_id)
    }
    pub async fn run_legacy(
        &self,
        session: String,
        request: RunRequest,
    ) -> Result<crate::conversation::SessionRunResult, String> {
        self.agent_loop.run_legacy(session, request).await
    }
}
