//! Built-in Rust language server contribution.
use cordis::{Context, Plugin, Service};
use mh_lsp::{LanguageServer, LspService};
use std::{collections::HashMap, convert::Infallible, sync::Arc};

/// Built-in Rust language contribution. Other native plugins can register their own providers.
pub struct RustLanguageService;
impl Service for RustLanguageService {
    const NAME: &'static str = "mh.lsp.rust";
}
pub struct RustLanguagePlugin;
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
    use std::path::Path;
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

}
