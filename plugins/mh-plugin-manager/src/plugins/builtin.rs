//! Compiled-in plugins. Cordis owns their publications and execution effects.
use super::*;
use crate::{
    context::ContextService,
    mcp::McpManager,
    tools::ToolRegistry,
};
#[cfg(test)]
use crate::tools::InteractionBroker;
#[cfg(test)]
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
        id: "workspaces",
        name: "Workspaces",
        description: "Projects, folders and session membership.",
        local_only: false,
        active: published::<crate::workspaces::WorkspaceService>,
    },
    SystemPlugin {
        id: "sessions",
        name: "Сессии",
        description: "Хранение истории и событий диалога.",
        local_only: false,
        active: published::<crate::sessions::SessionService>,
    },
    SystemPlugin {
        id: "agent-loop",
        name: "Выполнение диалога",
        description: "Запуск, отмена и завершение ходов диалога.",
        local_only: false,
        active: published::<crate::AgentLoopMarker>,
    },
    SystemPlugin {
        id: "message-queue",
        name: "Очередь сообщений",
        description: "Очередь сообщений с вложениями в открытом чате. Не сохраняется при закрытии чата.",
        local_only: false,
        active: published::<crate::sessions::SessionService>,
    },
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
    SystemPlugin { id: "mh-editor", name: "MH Editor", description: "Workspace code editor.", local_only: true, active: published::<crate::editor::EditorService> },
    SystemPlugin {
        id: "mh-lsp", name: "MH LSP", description: "Language servers for project workspaces.",
        local_only: true, active: published::<crate::lsp::LspService>,
    },
    SystemPlugin {
        id: "mh-lsp-rust", name: "MH LSP Rust", description: "Rust language server configuration.",
        local_only: true, active: published::<crate::lsp::RustLanguageService>,
    },
    SystemPlugin {
        id: "mcp",
        name: "Менеджер MCP",
        description: "Подключения MCP, обнаружение и жизненный цикл инструментов серверов.",
        local_only: false,
        active: published::<McpManager>,
    },
];

pub(super) use mh_tool_runtime::ToolRuntimePlugin;
pub(super) use mh_context::ContextPlugin;
pub(super) use mh_mcp::McpPlugin;

pub(super) use mh_workspace_tools::{WorkspaceTools, WorkspaceToolsPlugin};

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
        let without_workspace = tools.snapshot().without_workspace().definitions();
        assert!(without_workspace
            .iter()
            .any(|t| t.name == "read_attachment"));
        assert!(!without_workspace
            .iter()
            .any(|t| t.name == "save_attachment"));
        let reference = context
            .put_attachment("s", "data.bin", "application/octet-stream", "AP+A")
            .unwrap();
        let mut call = crate::tools::ToolContext {
            session_id: "s".into(),
            run_id: "r".into(),
            call_id: "c".into(),
            cwd: Some(dir.path().to_path_buf()),
            policy: crate::tools::PermissionPolicy::ReadOnly,
            cancellation: crate::tools::Cancellation::default(),
            interactions: interactions.clone(),
            sink: None,
        };
        let args = json!({"id":reference.id,"path":"copy.bin"});
        assert!(tools
            .snapshot()
            .execute("save_attachment", args.clone(), call.clone())
            .await
            .is_err());
        call.policy = crate::tools::PermissionPolicy::Allow;
        tools
            .snapshot()
            .execute("save_attachment", args.clone(), call.clone())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("copy.bin")).unwrap(),
            [0, 255, 128]
        );
        assert!(tools
            .snapshot()
            .execute("save_attachment", args.clone(), call.clone())
            .await
            .is_err());
        assert!(tools
            .snapshot()
            .execute(
                "save_attachment",
                json!({"id":reference.id,"path":"../escape"}),
                call.clone()
            )
            .await
            .is_err());
        call.session_id = "other".into();
        assert!(tools
            .snapshot()
            .execute("read_attachment", json!({"id":reference.id}), call)
            .await
            .is_err());
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
