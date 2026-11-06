#![windows_subsystem = "windows"]

mod plugins;
mod shell;
mod providers;
mod codex;
mod sessions;
mod session_store;
mod conversation;

use serde::Deserialize;
use tauri::Manager;
use plugins::{PluginInfo, PluginManager};
use shell::{ShellResult, ShellSettings};
use providers::{ProviderInput, ProviderView, Turn};
use sessions::{SessionService,SessionInfo,SessionMessage,IntegrationState};
use codex::{CodexStatus, LoginStatus, CodexModel, CodexReply, CodexOptions};

#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct ChatRequest { provider_id:String, model:String, messages:Vec<Turn> }
#[tauri::command]
fn session_list(store:tauri::State<'_,SessionService>)->Result<Vec<SessionInfo>,String>{store.list()}
#[tauri::command]
fn session_create(store:tauri::State<'_,SessionService>,id:String)->Result<(),String>{store.create(&id)}
#[tauri::command]
fn session_get(store:tauri::State<'_,SessionService>,id:String)->Result<SessionInfo,String>{store.get(&id)}
#[tauri::command]
fn session_load(store:tauri::State<'_,SessionService>,id:String)->Result<Vec<SessionMessage>,String>{store.load(&id)}
#[tauri::command]
fn session_save(store:tauri::State<'_,SessionService>,id:String,item:SessionMessage,integration:Option<IntegrationState>)->Result<(),String>{store.save(&id,item,integration)}
#[tauri::command]
fn session_set_model(store:tauri::State<'_,SessionService>,id:String,model:String)->Result<(),String>{store.set_model(&id,&model)}
#[tauri::command]
async fn session_run(store:tauri::State<'_,SessionService>,manager:tauri::State<'_,PluginManager>,id:String,model:String,last_message_id:String,options:serde_json::Value)->Result<conversation::SessionRunResult,String>{conversation::run(&store,&manager,&id,&model,&last_message_id,options).await}
#[tauri::command]
async fn chat(manager:tauri::State<'_,PluginManager>, request:ChatRequest)->Result<String,String>{manager.providers()?.chat(&request.provider_id,&request.model,request.messages).await}
#[tauri::command]
async fn model_providers(manager:tauri::State<'_,PluginManager>)->Result<Vec<ProviderView>,String>{Ok(manager.providers()?.list().await)}
#[tauri::command]
async fn save_model_provider(manager:tauri::State<'_,PluginManager>,provider:ProviderInput)->Result<Vec<ProviderView>,String>{manager.providers()?.save(provider).await}
#[tauri::command]
async fn discover_models(manager:tauri::State<'_,PluginManager>,provider:ProviderInput)->Result<Vec<String>,String>{manager.providers()?.discover(provider).await}
#[tauri::command]
async fn delete_model_provider(manager:tauri::State<'_,PluginManager>,id:String)->Result<Vec<ProviderView>,String>{manager.providers()?.delete(&id).await}
#[tauri::command]
async fn codex_models(manager:tauri::State<'_,PluginManager>)->Result<Vec<CodexModel>,String>{manager.codex()?.models().await}
#[tauri::command]
async fn codex_chat(manager:tauri::State<'_,PluginManager>,model:String,thread_id:Option<String>,messages:Vec<Turn>,options:CodexOptions)->Result<CodexReply,String>{manager.codex()?.chat(model,thread_id,messages,options).await}
#[tauri::command]
async fn codex_status(manager:tauri::State<'_,PluginManager>)->Result<CodexStatus,String>{manager.codex()?.status().await}

#[tauri::command]
async fn codex_quota(manager:tauri::State<'_,PluginManager>)->Result<CodexStatus,String>{manager.codex()?.quota().await}
#[tauri::command]
async fn codex_login(manager:tauri::State<'_,PluginManager>)->Result<LoginStatus,String>{manager.codex()?.login().await}
#[tauri::command]
async fn codex_login_status(manager:tauri::State<'_,PluginManager>)->Result<Option<LoginStatus>,String>{Ok(manager.codex()?.login_status().await)}
#[tauri::command]
async fn list_plugins(manager: tauri::State<'_, PluginManager>) -> Result<Vec<PluginInfo>, String> {
    manager.list().await
}

#[tauri::command]
async fn set_plugin_enabled(manager: tauri::State<'_, PluginManager>, id: String, enabled: bool) -> Result<Vec<PluginInfo>, String> {
    manager.set_enabled(&id, enabled).await
}

#[tauri::command]
async fn shell_settings(manager: tauri::State<'_, PluginManager>) -> Result<ShellSettings, String> { Ok(manager.shell()?.settings().await) }
#[tauri::command]
async fn save_shell_settings(manager: tauri::State<'_, PluginManager>, settings: ShellSettings) -> Result<ShellSettings, String> { manager.shell()?.save(settings).await }
#[tauri::command]
async fn run_shell_command(manager: tauri::State<'_, PluginManager>, command: String) -> Result<ShellResult, String> { manager.shell()?.run(command).await }

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("shell.json");
            let sessions = SessionService::open(&app.path().app_data_dir()?.join("sessions.sqlite3")).map_err(std::io::Error::other)?;
            app.manage(sessions);
            let manager = tauri::async_runtime::block_on(PluginManager::start(path))
                .map_err(std::io::Error::other)?;
            app.manage(manager);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![session_list,session_create,session_get,session_load,session_save,session_set_model,session_run,chat, model_providers, save_model_provider, discover_models, delete_model_provider, codex_models, codex_chat, codex_status, codex_quota, codex_login, codex_login_status, list_plugins, set_plugin_enabled, shell_settings, save_shell_settings, run_shell_command])
        .build(tauri::generate_context!())
        .expect("Could not start OE Rig prototype");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            tauri::async_runtime::block_on(handle.state::<PluginManager>().shutdown());
        }
    });
}
