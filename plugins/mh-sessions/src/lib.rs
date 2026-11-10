pub mod sessions;
pub mod session_store;
pub mod events;
mod run_types;
pub mod transcript { pub use minerva_plugin_sdk::transcript::*; }
pub use run_types::{RunHandle, RunMessage, RunSnapshot, StartRunRequest};

use cordis::{Context, Plugin};
use std::{convert::Infallible, path::PathBuf, sync::Arc};
use sessions::SessionService;
fn error(value: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(value.to_string())
}
pub struct SessionsPlugin(pub PathBuf);
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
