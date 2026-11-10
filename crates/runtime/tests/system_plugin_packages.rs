use cordis::{Context, PreparedPlugin};
use std::{path::Path, sync::Arc};

#[tokio::test]
async fn agent_loop_package_owns_its_service_and_marker() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(mh_plugin_manager::plugins::PluginManager::start(dir.path().join("shell.json")).await.unwrap());
    let ctx = manager.context();
    let sessions = ctx.spawn(cordis::PreparedPlugin::from_input(mh_sessions::SessionsPlugin(dir.path().join("sessions.sqlite3")), ())).await.unwrap();
    let fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_agent_loop::AgentLoopPlugin { plugins: Arc::downgrade(&manager), shutdown_timeout: std::time::Duration::from_secs(1) }, ())).await.unwrap();
    assert!(ctx.try_service::<mh_agent_loop::AgentLoopService>().is_ok());
    assert!(ctx.try_service::<mh_plugin_manager::AgentLoopMarker>().is_ok());
    fiber.dispose().await.unwrap();
    assert!(ctx.try_service::<mh_agent_loop::AgentLoopService>().is_err());
    assert!(ctx.try_service::<mh_plugin_manager::AgentLoopMarker>().is_err());
    sessions.dispose().await.unwrap();
    manager.shutdown().await;
}

#[test]
fn context_package_initializes_shared_workspace_tables() {
    let dir = tempfile::tempdir().unwrap();
    let context = mh_context::ContextService::open(&dir.path().join("context.sqlite3")).unwrap();
    let _: &minerva_runtime::context::ContextService = &context;
    let db = context.db().unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM projects", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[tokio::test]
async fn rust_provider_package_registers_and_unregisters_with_its_plugin() {
    let ctx = Context::new();
    let service = Arc::new(mh_lsp::LspService::default());
    let _publication = ctx.provide(service.clone()).unwrap();
    let fiber = ctx.spawn(PreparedPlugin::from_input(mh_lsp_rust::RustLanguagePlugin, ())).await.unwrap();
    assert_eq!(service.provider_for_file(Path::new("main.rs")).await.unwrap().command, "rust-analyzer");
    fiber.dispose().await.unwrap();
    assert!(service.provider_for_file(Path::new("main.rs")).await.is_none());
}
#[tokio::test]
async fn lsp_package_plugin_publishes_service_from_shared_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = cordis::Context::new();
    let context = Arc::new(minerva_runtime::context::ContextService::open(&dir.path().join("context.sqlite3")).unwrap());
    let sessions = Arc::new(minerva_runtime::sessions::SessionService::open(&dir.path().join("sessions.sqlite3")).unwrap());
    let _context = ctx.provide(context).unwrap();
    let _sessions = ctx.provide(sessions).unwrap();
    let workspace = ctx.spawn(PreparedPlugin::from_input(mh_workspaces::WorkspacePlugin { root: dir.path().join("root"), local: true }, ())).await.unwrap();
    let fiber = ctx.spawn(PreparedPlugin::from_input(mh_lsp::LspPlugin(false), ())).await.unwrap();
    assert!(ctx.try_service::<mh_lsp::LspService>().is_ok());
    fiber.dispose().await.unwrap();
    workspace.dispose().await.unwrap();
    assert!(ctx.try_service::<mh_lsp::LspService>().is_err());
}

#[tokio::test]
async fn mcp_package_shares_registry_and_policy_with_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(minerva_runtime::tools::ToolRegistry::default());
    let manager = mh_mcp::McpManager::new(
        dir.path().join("mcp.json"),
        minerva_runtime::credential_store::CredentialScope::new("test").unwrap(),
        Arc::new(minerva_runtime::credential_store::MemoryCredentialStore::default()),
        registry,
        minerva_runtime::network_policy::EndpointPolicy::Local,
    ).unwrap();
    let _: &minerva_runtime::mcp::McpManager = &manager;
    assert!(manager.list().is_empty());
}

#[tokio::test]
async fn manager_package_bootstraps_and_shuts_down_its_services() {
    let dir = tempfile::tempdir().unwrap();
    let manager = mh_plugin_manager::plugins::PluginManager::start(dir.path().join("shell.json")).await.unwrap();
    let _: &minerva_runtime::plugins::PluginManager = &manager;
    assert!(manager.providers().is_ok());
    manager.shutdown().await;
}

#[tokio::test]
async fn provider_package_publishes_the_runtime_service() {
    let dir = tempfile::tempdir().unwrap();
    let service = Arc::new(mh_providers::ProviderService::new(dir.path().join("providers.json")).unwrap());
    let ctx = cordis::Context::new();
    let fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_providers::ProviderPlugin(service), ())).await.unwrap();
    assert!(ctx.try_service::<minerva_runtime::providers::ProviderService>().is_ok());
    fiber.dispose().await.unwrap();
    assert!(ctx.try_service::<minerva_runtime::providers::ProviderService>().is_err());
}

#[tokio::test]
async fn package_plugins_publish_and_remove_their_services() {
    let ctx = cordis::Context::new();
    let tools = Arc::new(mh_tool_runtime::ToolRegistry::default());
    let interactions = Arc::new(mh_tool_runtime::InteractionBroker::default());
    let fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_tool_runtime::ToolRuntimePlugin { tools, interactions }, ())).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let context = Arc::new(mh_context::ContextService::open(&dir.path().join("context.sqlite3")).unwrap());
    let context_fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_context::ContextPlugin(context), ())).await.unwrap();
    assert!(ctx.try_service::<minerva_runtime::context::ContextService>().is_ok());
    context_fiber.dispose().await.unwrap();
    fiber.dispose().await.unwrap();
    assert!(ctx.try_service::<minerva_runtime::tools::ToolRegistry>().is_err());
    assert!(ctx.try_service::<minerva_runtime::context::ContextService>().is_err());
}
#[tokio::test]
async fn workspace_tools_package_unregisters_tools_when_disposed() {
    let ctx = cordis::Context::new();
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(mh_tool_runtime::ToolRegistry::default());
    let _registry = ctx.provide(registry.clone()).unwrap();
    let _context = ctx.provide(Arc::new(mh_context::ContextService::open(&dir.path().join("context.sqlite3")).unwrap())).unwrap();
    let fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_workspace_tools::WorkspaceToolsPlugin(true), ())).await.unwrap();
    assert!(registry.snapshot().definitions().iter().any(|item| item.name == "read_file"));
    fiber.dispose().await.unwrap();
    assert!(!registry.snapshot().definitions().iter().any(|item| item.name == "read_file"));
}

#[test]
fn session_package_and_runtime_share_persistent_session_service() {
    let dir = tempfile::tempdir().unwrap();
    let service = mh_sessions::sessions::SessionService::open(&dir.path().join("history.sqlite3")).unwrap();
    let _: &minerva_runtime::sessions::SessionService = &service;
    service.create("package-session").unwrap();
    assert_eq!(service.get("package-session").unwrap().id, "package-session");
}
#[tokio::test]
async fn session_package_plugin_owns_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = cordis::Context::new();
    let fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_sessions::SessionsPlugin(dir.path().join("sessions.sqlite3")), ())).await.unwrap();
    let service = ctx.try_service::<minerva_runtime::sessions::SessionService>().unwrap();
    service.create("one").unwrap();
    fiber.dispose().await.unwrap();
    assert!(service.list().is_err());
}

#[tokio::test]
async fn shell_package_shares_settings_with_runtime_composition() {
    let dir = tempfile::tempdir().unwrap();
    let service = Arc::new(mh_shell::ShellService::new(dir.path().join("shell.json")).unwrap());
    let ctx = cordis::Context::new();
    let fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_shell::ShellPlugin(service.clone()), ())).await.unwrap();
    service.save(mh_shell::ShellSettings { timeout_ms: 1500, max_output_bytes: 1024 }).await.unwrap();
    assert_eq!(ctx.try_service::<minerva_runtime::shell::ShellService>().unwrap().settings().await.timeout_ms, 1500);
    fiber.dispose().await.unwrap();
    assert!(ctx.try_service::<minerva_runtime::shell::ShellService>().is_err());
}

#[test]
fn tool_package_and_runtime_use_the_same_registry() {
    let registry = mh_tool_runtime::ToolRegistry::default();
    let _: &minerva_runtime::tools::ToolRegistry = &registry;
    mh_workspace_tools::register_native(&registry).unwrap();
    assert!(!registry.snapshot().definitions().is_empty());
    registry.revoke_owner("native.workspace");
    assert!(registry.snapshot().definitions().iter().all(|tool| tool.name != "read_file"));
}

#[tokio::test]
async fn workspace_package_uses_shared_context_and_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = cordis::Context::new();
    let context = Arc::new(minerva_runtime::context::ContextService::open(&dir.path().join("context.sqlite3")).unwrap());
    let sessions = Arc::new(minerva_runtime::sessions::SessionService::open(&dir.path().join("sessions.sqlite3")).unwrap());
    let _context = ctx.provide(context).unwrap();
    let _sessions = ctx.provide(sessions).unwrap();
    let fiber = ctx.spawn(cordis::PreparedPlugin::from_input(mh_workspaces::WorkspacePlugin { root: dir.path().join("root"), local: true }, ())).await.unwrap();
    let service = ctx.try_service::<minerva_runtime::workspaces::WorkspaceService>().unwrap();
    assert!(!service.list().unwrap().is_empty());
    fiber.dispose().await.unwrap();
    assert!(service.list().is_err());
}
