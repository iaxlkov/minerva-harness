//! System plugins own session storage and supervised execution until fiber disposal.
use crate::{plugins::PluginManager, run_supervisor::AgentLoopService, sessions::SessionService};
use cordis::{Context, Plugin, Service};
use std::{
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

impl Service for SessionService {
    const NAME: &'static str = "studio.sessions";
}

pub(crate) struct SessionsPlugin(pub PathBuf);
fn error(value: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(value.to_string())
}
impl Plugin for SessionsPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, _: ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let service = Arc::new(SessionService::open(&self.0).map_err(error)?);
        let owned = service.clone();
        let _cleanup = ctx
            .effect(move || async move { owned.shutdown().map_err(error) })
            .map_err(error)?;
        let _publication = ctx.provide(service).map_err(error)?;
        Ok(())
    }
}
pub(crate) struct AgentLoopPlugin {
    pub plugins: Weak<PluginManager>,
    pub shutdown_timeout: Duration,
}
impl Plugin for AgentLoopPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, _: ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let sessions = ctx.try_service::<SessionService>().map_err(error)?;
        ctx.try_service::<crate::tools::ToolRegistry>()
            .map_err(error)?;
        ctx.try_service::<crate::providers::ProviderService>()
            .map_err(error)?;
        let service = Arc::new(AgentLoopService::new(
            sessions,
            self.plugins.clone(),
            self.shutdown_timeout,
        ));
        let owned = service.clone();
        let _cleanup = ctx
            .effect(move || async move {
                owned.shutdown().await;
                Ok::<_, std::io::Error>(())
            })
            .map_err(error)?;
        let _publication = ctx.provide(service).map_err(error)?;
        Ok(())
    }
}

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
