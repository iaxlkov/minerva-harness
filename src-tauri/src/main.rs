#![windows_subsystem = "windows"]

mod keep_awake;

use minerva_runtime::conversation;
use minerva_runtime::plugins;
use minerva_runtime::providers;
use minerva_runtime::sessions;
use minerva_runtime::shell;

use minerva_runtime::{Runtime, RuntimeConfig};
use plugins::{PluginInfo, PluginManager};
use providers::{ProviderInput, ProviderView};
use sessions::{SessionInfo, SessionMessage, SessionService};
use shell::{ShellResult, ShellSettings};
use std::sync::Arc;
use tauri::Manager;
#[tauri::command]
async fn plugin_models(
    manager: tauri::State<'_, Arc<PluginManager>>,
) -> Result<Vec<plugins::PluginModel>, String> {
    manager.models().await
}
#[tauri::command]
async fn plugin_ui(
    manager: tauri::State<'_, Arc<PluginManager>>,
    id: String,
) -> Result<String, String> {
    manager.ui_document(&id).await
}
#[tauri::command]
async fn plugin_call(
    manager: tauri::State<'_, Arc<PluginManager>>,
    id: String,
    method: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    manager.plugin_call(&id, &method, params).await
}

#[tauri::command]
async fn run_start(
    runtime: tauri::State<'_, Arc<Runtime>>,
    request: minerva_runtime::StartRunRequest,
) -> Result<minerva_runtime::RunHandle, String> {
    runtime.inner().start_run(request).await
}
#[tauri::command]
fn run_get(
    runtime: tauri::State<'_, Arc<Runtime>>,
    session_id: String,
    run_id: String,
) -> Result<minerva_runtime::RunSnapshot, String> {
    runtime.get_run(&session_id, &run_id)
}

#[tauri::command]
async fn harness_rpc(
    runtime: tauri::State<'_, Arc<Runtime>>,
    method: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    runtime.inner().harness_call(&method, params).await
}
#[tauri::command]
fn run_cancel(
    runtime: tauri::State<'_, Arc<Runtime>>,
    session_id: String,
    run_id: String,
) -> Result<bool, String> {
    runtime.cancel_run(&session_id, &run_id)
}

#[tauri::command]
fn session_list(store: tauri::State<'_, Arc<SessionService>>) -> Result<Vec<SessionInfo>, String> {
    store.list()
}
#[tauri::command]
fn session_create(
    runtime: tauri::State<'_, Arc<Runtime>>,
    id: String,
    workspace_id: Option<String>,
) -> Result<(), String> {
    runtime.create_session(&id, workspace_id.as_deref())
}
#[tauri::command]
fn session_get(
    store: tauri::State<'_, Arc<SessionService>>,
    id: String,
) -> Result<SessionInfo, String> {
    store.get(&id)
}
#[tauri::command]
fn session_load(
    store: tauri::State<'_, Arc<SessionService>>,
    id: String,
) -> Result<Vec<SessionMessage>, String> {
    store.load(&id)
}
#[tauri::command]
fn session_save(
    store: tauri::State<'_, Arc<SessionService>>,
    id: String,
    item: SessionMessage,
) -> Result<(), String> {
    if item.role != "user" {
        return Err("Ответы сохраняет ядро.".into());
    }
    store.save(&id, item, None)
}
#[tauri::command]
fn session_set_model(
    store: tauri::State<'_, Arc<SessionService>>,
    id: String,
    model: String,
) -> Result<(), String> {
    store.set_model(&id, &model)
}
#[tauri::command]
async fn session_run(
    runtime: tauri::State<'_, Arc<Runtime>>,
    id: String,
    request: conversation::RunRequest,
) -> Result<conversation::SessionRunResult, String> {
    runtime.run_legacy(id, request).await
}
#[tauri::command]
fn session_cancel(
    runtime: tauri::State<'_, Arc<Runtime>>,
    id: String,
    run_id: String,
) -> Result<bool, String> {
    runtime.cancel_run(&id, &run_id)
}
#[tauri::command]
async fn model_providers(
    manager: tauri::State<'_, Arc<PluginManager>>,
) -> Result<Vec<ProviderView>, String> {
    Ok(manager.providers()?.list().await)
}
#[tauri::command]
async fn save_model_provider(
    manager: tauri::State<'_, Arc<PluginManager>>,
    provider: ProviderInput,
) -> Result<Vec<ProviderView>, String> {
    manager.providers()?.save(provider).await
}
#[tauri::command]
async fn discover_models(
    manager: tauri::State<'_, Arc<PluginManager>>,
    provider: ProviderInput,
) -> Result<Vec<String>, String> {
    manager.providers()?.discover(provider).await
}
#[tauri::command]
async fn delete_model_provider(
    manager: tauri::State<'_, Arc<PluginManager>>,
    id: String,
) -> Result<Vec<ProviderView>, String> {
    manager.providers()?.delete(&id).await
}
#[tauri::command]
async fn list_plugins(
    manager: tauri::State<'_, Arc<PluginManager>>,
) -> Result<Vec<PluginInfo>, String> {
    manager.list().await
}

#[tauri::command]
async fn set_plugin_enabled(
    manager: tauri::State<'_, Arc<PluginManager>>,
    id: String,
    enabled: bool,
) -> Result<Vec<PluginInfo>, String> {
    manager.set_enabled(&id, enabled).await
}

#[tauri::command]
async fn shell_settings(
    manager: tauri::State<'_, Arc<PluginManager>>,
) -> Result<ShellSettings, String> {
    Ok(manager.shell()?.settings().await)
}

#[tauri::command]
async fn set_optional_plugins_enabled(
    manager: tauri::State<'_, Arc<PluginManager>>,
    enabled: bool,
) -> Result<Vec<PluginInfo>, String> {
    manager.set_optional_enabled(enabled).await
}
#[tauri::command]
async fn save_shell_settings(
    manager: tauri::State<'_, Arc<PluginManager>>,
    settings: ShellSettings,
) -> Result<ShellSettings, String> {
    manager.shell()?.save(settings).await
}
#[tauri::command]
async fn run_shell_command(
    manager: tauri::State<'_, Arc<PluginManager>>,
    command: String,
) -> Result<ShellResult, String> {
    manager.shell()?.run(command).await
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(keep_awake::KeepAwake::default())
        .setup(|app| {
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or_else(|| std::io::Error::other("Missing Minerva icon"))?;
            if let Some(window) = app.get_webview_window("main") {
                window.set_icon(icon)?;
            }
            let show =
                tauri::menu::MenuItem::with_id(app, "show", "Открыть Minerva", true, None::<&str>)?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Выйти", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            tauri::tray::TrayIconBuilder::with_id("minerva")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/32x32.png"
                ))?)
                .icon_as_template(false)
                .tooltip("Minerva Harness")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            let runtime = Arc::new(tauri::async_runtime::block_on(Runtime::start(
                RuntimeConfig {
                    tenant_id: "local".into(),
                    workspace_id: "default".into(),
                    data_dir: app.path().app_data_dir()?,
                    config_dir: app.path().app_config_dir()?,
                    profile: "desktop".into(),
                },
            ))?);
            app.manage(runtime.sessions.clone());
            app.manage(runtime.plugins.clone());
            app.manage(runtime);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            plugin_call,
            plugin_models,
            plugin_ui,
            run_start,
            run_get,
            run_cancel,
            session_list,
            session_create,
            session_get,
            session_load,
            session_save,
            session_set_model,
            session_run,
            session_cancel,
            model_providers,
            save_model_provider,
            discover_models,
            delete_model_provider,
            list_plugins,
            set_plugin_enabled,
            set_optional_plugins_enabled,
            harness_rpc,
            shell_settings,
            save_shell_settings,
            run_shell_command,
            keep_awake::set_keep_awake
        ])
        .build(tauri::generate_context!())
        .expect("Could not start OE Rig prototype");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            let _ = handle.state::<keep_awake::KeepAwake>().set(false);
            let _ = tauri::async_runtime::block_on(handle.state::<Arc<Runtime>>().shutdown());
        }
    });
}
