//! Runtime service integration tests.
pub(crate) use mh_sessions::SessionsPlugin;
pub(crate) use mh_agent_loop::AgentLoopPlugin;
#[cfg(test)]
use crate::sessions::SessionService;
#[cfg(test)]
use cordis::Context;
#[cfg(test)]
use std::{sync::Weak, time::Duration};

#[cfg(test)]
mod tests {
    use super::*;
    use cordis::PreparedPlugin;
    #[tokio::test]
    async fn later_plugin_failure_can_rollback_storage_even_with_retained_service_handles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.sqlite3");
        let ctx = Context::new();
        let fiber = ctx
            .spawn(PreparedPlugin::from_input(SessionsPlugin(path.clone()), ()))
            .await
            .unwrap();
        let retained = ctx.try_service::<SessionService>().unwrap();
        assert!(ctx
            .spawn(PreparedPlugin::from_input(
                AgentLoopPlugin {
                    plugins: Weak::new(),
                    shutdown_timeout: Duration::from_millis(100)
                },
                ()
            ))
            .await
            .is_err());
        fiber.dispose().await.unwrap();
        assert!(retained.shutdown_result().is_ok());
        assert!(retained.list().is_err());
        let reopened = SessionService::open(&path).unwrap();
        reopened.shutdown().unwrap();
    }
}
