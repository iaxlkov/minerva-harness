//! Compiled-in plugins. Cordis owns their publications and execution effects.
use super::*;
use crate::{
    context::ContextService,
    mcp::McpManager,
    tools::{InteractionBroker, ToolRegistry},
};
use serde_json::json;

pub(super) struct SystemPlugin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub local_only: bool,
    pub active: fn(&Context) -> bool,
}
fn published<T: Service>(ctx: &Context) -> bool {
    ctx.try_service::<T>().is_ok()
}
pub(super) const SYSTEM: &[SystemPlugin] = &[
    SystemPlugin {
        id: "plugin-manager",
        name: "Менеджер плагинов",
        description: "Управление встроенными и установленными плагинами.",
        local_only: false,
        active: published::<ManagerService>,
    },
    SystemPlugin {
        id: "providers",
        name: "Провайдеры моделей",
        description: "Конфигурация и подключение моделей.",
        local_only: false,
        active: published::<ProviderService>,
    },
    SystemPlugin {
        id: "shell",
        name: "Командная оболочка",
        description: "Запуск команд и ограничения вывода.",
        local_only: true,
        active: published::<ShellService>,
    },
    SystemPlugin {
        id: "tool-runtime",
        name: "Выполнение инструментов",
        description: "Реестр, разрешения, подтверждения и вопросы.",
        local_only: false,
        active: published::<ToolRegistry>,
    },
    SystemPlugin {
        id: "context",
        name: "Контекст диалога",
        description: "Рабочая папка, инструкции, навыки, сжатие и вложения.",
        local_only: false,
        active: published::<ContextService>,
    },
    SystemPlugin {
        id: "workspace-tools",
        name: "Инструменты рабочей папки",
        description: "Чтение, поиск, изменение файлов, shell и загрузка навыков.",
        local_only: true,
        active: published::<WorkspaceTools>,
    },
    SystemPlugin {
        id: "mcp",
        name: "Менеджер MCP",
        description: "Подключения MCP, обнаружение и жизненный цикл инструментов серверов.",
        local_only: false,
        active: published::<McpManager>,
    },
];

pub(super) struct ToolRuntimePlugin {
    pub tools: Arc<ToolRegistry>,
    pub interactions: Arc<InteractionBroker>,
}
impl Plugin for ToolRuntimePlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = cordis::service::ServicePublishError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let _tools = ctx.provide(self.tools.clone())?;
        let _interactions = ctx.provide(self.interactions.clone())?;
        Ok(())
    }
}

pub(super) struct ContextPlugin(pub Arc<ContextService>);
impl Plugin for ContextPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = cordis::service::ServicePublishError;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let _context = ctx.provide(self.0.clone())?;
        Ok(())
    }
}

struct WorkspaceTools;
impl Service for WorkspaceTools {
    const NAME: &'static str = "studio.workspace-tools";
}
pub(super) struct WorkspaceToolsPlugin(pub bool);
fn apply_error(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
impl Plugin for WorkspaceToolsPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), std::io::Error> {
        if !self.0 {
            return Ok(());
        }
        let tools = ctx.try_service::<ToolRegistry>().map_err(apply_error)?;
        let context = ctx.try_service::<ContextService>().map_err(apply_error)?;
        let owned = tools.clone();
        let _cleanup = ctx
            .effect_sync(move || {
                owned.revoke_owner("native.workspace");
                owned.revoke_owner("native.interaction");
            })
            .map_err(apply_error)?;
        crate::tools::register_native(&tools).map_err(apply_error)?;
        tools.register("native.workspace", crate::tools::ToolDefinition {
            name: "load_skill".into(),
            description: "Load the instructions of an available workspace skill by name".into(),
            input_schema: json!({"type":"object","properties":{"name":{"type":"string","minLength":1}},"required":["name"],"additionalProperties":false}),
            output_schema: json!({"type":"object","properties":{"instructions":{"type":"string"}},"required":["instructions"]}),
            side_effect: false,
        }, move |ctx, args| {
            let context = context.clone();
            Box::pin(async move { Ok(json!({"instructions":context.load_skill(&ctx.session_id, args["name"].as_str().ok_or("missing skill name")?)?})) })
        }).map_err(apply_error)?;
        let _publication = ctx.provide(Arc::new(WorkspaceTools)).map_err(apply_error)?;
        Ok(())
    }
}

pub(super) struct McpPlugin(pub Arc<McpManager>);
impl Plugin for McpPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), std::io::Error> {
        ctx.try_service::<ToolRegistry>().map_err(apply_error)?;
        let manager = self.0.clone();
        let _cleanup = ctx
            .effect(move || async move { manager.set_paused(true).await.map_err(apply_error) })
            .map_err(apply_error)?;
        let _publication = ctx.provide(self.0.clone()).map_err(apply_error)?;
        self.0.set_paused(false).await.map_err(apply_error)?;
        self.0.start().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn services_and_native_effects_are_owned_by_their_fibers() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::new();
        let tools = Arc::new(ToolRegistry::default());
        let interactions = Arc::new(InteractionBroker::default());
        let context = Arc::new(ContextService::open(&dir.path().join("context.sqlite3")).unwrap());
        let core = ctx
            .spawn(PreparedPlugin::from_input(
                ToolRuntimePlugin {
                    tools: tools.clone(),
                    interactions: interactions.clone(),
                },
                (),
            ))
            .await
            .unwrap();
        let storage = ctx
            .spawn(PreparedPlugin::from_input(
                ContextPlugin(context.clone()),
                (),
            ))
            .await
            .unwrap();
        let native = ctx
            .spawn(PreparedPlugin::from_input(WorkspaceToolsPlugin(true), ()))
            .await
            .unwrap();
        assert!(Arc::ptr_eq(
            &ctx.try_service::<ToolRegistry>().unwrap(),
            &tools
        ));
        assert!(Arc::ptr_eq(
            &ctx.try_service::<InteractionBroker>().unwrap(),
            &interactions
        ));
        assert!(Arc::ptr_eq(
            &ctx.try_service::<ContextService>().unwrap(),
            &context
        ));
        assert!(ctx.try_service::<WorkspaceTools>().is_ok());
        assert!(tools
            .snapshot()
            .definitions()
            .iter()
            .any(|t| t.name == "load_skill"));
        native.dispose().await.unwrap();
        assert!(ctx.try_service::<WorkspaceTools>().is_err());
        assert!(tools.snapshot().definitions().is_empty());
        storage.dispose().await.unwrap();
        assert!(ctx.try_service::<ContextService>().is_err());
        core.dispose().await.unwrap();
        assert!(ctx.try_service::<ToolRegistry>().is_err());
        assert!(ctx.try_service::<InteractionBroker>().is_err());
    }

    #[tokio::test]
    async fn failed_workspace_plugin_rolls_back_only_its_own_registrations() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::new();
        let tools = Arc::new(ToolRegistry::default());
        tools
            .register(
                "foreign",
                crate::tools::ToolDefinition {
                    name: "write_file".into(),
                    description: "Reserved by another owner".into(),
                    input_schema: json!({}),
                    output_schema: json!({}),
                    side_effect: true,
                },
                |_, _| Box::pin(async { Ok(json!({})) }),
            )
            .unwrap();
        let core = ctx
            .spawn(PreparedPlugin::from_input(
                ToolRuntimePlugin {
                    tools: tools.clone(),
                    interactions: Arc::new(InteractionBroker::default()),
                },
                (),
            ))
            .await
            .unwrap();
        let storage = ctx
            .spawn(PreparedPlugin::from_input(
                ContextPlugin(Arc::new(
                    ContextService::open(&dir.path().join("context.sqlite3")).unwrap(),
                )),
                (),
            ))
            .await
            .unwrap();
        assert!(ctx
            .spawn(PreparedPlugin::from_input(WorkspaceToolsPlugin(true), ()))
            .await
            .is_err());
        assert!(ctx.try_service::<WorkspaceTools>().is_err());
        let remaining = tools.snapshot().definitions();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "write_file");
        storage.dispose().await.unwrap();
        core.dispose().await.unwrap();
    }

    #[test]
    fn system_catalogue_matches_the_protected_package_namespace() {
        let mut catalogue = SYSTEM.iter().map(|p| p.id).collect::<Vec<_>>();
        let mut protected = minerva_plugin_sdk::SYSTEM_PLUGIN_IDS.to_vec();
        catalogue.sort();
        protected.sort();
        assert_eq!(catalogue, protected);
    }
}
