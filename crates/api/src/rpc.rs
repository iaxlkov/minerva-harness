use super::{auth, failure, success, ApiState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    Json,
};
use minerva_runtime::{
    conversation, providers::ProviderInput, sessions::SessionMessage, shell::ShellSettings, Runtime,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RpcRequest {
    pub protocol_version: u32,
    pub method: String,
    pub params: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateSession {
    id: String,
    #[serde(rename = "workspaceId")]
    workspace_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    id: String,
    model: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    id: String,
    item: SessionMessage,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    id: String,
    request: conversation::RunRequest,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cancel {
    id: String,
    run_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provider {
    provider: ProviderInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Toggle {
    id: String,
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionalPlugins {
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    settings: ShellSettings,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    command: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunRef {
    session_id: String,
    run_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginCall {
    id: String,
    method: String,
    params: Value,
}

fn args<T: DeserializeOwned>(value: Value) -> Result<T, DispatchError> {
    serde_json::from_value(value).map_err(|_| DispatchError::Invalid)
}
fn encoded<T: Serialize>(value: T) -> Result<Value, DispatchError> {
    serde_json::to_value(value).map_err(|_| DispatchError::Internal)
}
pub(crate) enum DispatchError {
    Invalid,
    NotFound,
    Service(String),
    Internal,
}
impl From<String> for DispatchError {
    fn from(value: String) -> Self {
        Self::Service(value)
    }
}

pub(crate) async fn handle(
    State(state): State<ApiState>,
    headers: HeaderMap,
    input: Result<Json<RpcRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if let Err(r) = auth::authorize(&state, &headers, true) {
        return *r;
    }
    let input = match input {
        Ok(Json(v)) => v,
        Err(_) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "request/invalid",
                "Invalid RPC request",
            )
        }
    };
    if input.protocol_version != 1 {
        return failure(
            StatusCode::BAD_REQUEST,
            "protocol/unsupported",
            "Unsupported protocol version",
        );
    }
    dispatch_response(state.runtime, &input.method, input.params).await
}
pub(crate) async fn dispatch_response(
    runtime: Arc<Runtime>,
    method: &str,
    params: Value,
) -> Response {
    if matches!(method, "session_save" | "session_run") {
        return failure(
            StatusCode::GONE,
            "method/retired",
            "Use runs.start for atomic supervised execution",
        );
    }
    match dispatch(runtime, method, params).await {
        Ok(v) => success(v),
        Err(DispatchError::Invalid) => failure(
            StatusCode::BAD_REQUEST,
            "request/invalid",
            "Invalid method arguments",
        ),
        Err(DispatchError::NotFound) => failure(
            StatusCode::NOT_FOUND,
            "method/not-found",
            "Method is not registered",
        ),
        Err(DispatchError::Service(message)) => {
            failure(StatusCode::BAD_REQUEST, "service/failed", &message)
        }
        Err(DispatchError::Internal) => failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "service/internal",
            "The operation could not complete",
        ),
    }
}

pub(crate) async fn dispatch(
    runtime: Arc<Runtime>,
    method: &str,
    params: Value,
) -> Result<Value, DispatchError> {
    match method {
        "session.snapshot"
        | "sessions.rename" | "sessions.state" | "sessions.fork"
        | "workspaces.list"
        | "workspaces.rename"
        | "workspaces.remove"
        | "workspaces.create"
        | "mh-editor.list" | "mh-editor.read" | "mh-editor.save" | "mh-editor.search"
        | "mh-terminal.open" | "mh-terminal.write" | "mh-terminal.read" | "mh-terminal.resize" | "mh-terminal.close"
        | "mh-lsp.start" | "mh-lsp.status" | "mh-lsp.stop" | "mh-lsp.open" | "mh-lsp.change" | "mh-lsp.close" | "mh-lsp.query" | "mh-lsp.diagnostics"
        | "workspace.get"
        | "workspace.save"
        | "tools.list"
        | "interactions.list"
        | "interactions.answer"
        | "context.status"
        | "context.compact"
        | "attachments.put"
        | "attachments.get"
        | "mcp.list"
        | "mcp.save"
        | "mcp.delete" => runtime
            .harness_call(method, params)
            .await
            .map_err(Into::into),
        "plugins.models" => {
            let _: Empty = args(params)?;
            encoded(runtime.plugins.models().await?)
        }
        "plugins.ui" => {
            let a: Id = args(params)?;
            encoded(runtime.plugins.ui_document(&a.id).await?)
        }
        "plugins.call" => {
            let a: PluginCall = args(params)?;
            Ok(runtime
                .plugins
                .plugin_call(&a.id, &a.method, a.params)
                .await?)
        }
        "runs.start" => {
            let a: minerva_runtime::StartRunRequest = args(params)?;
            encoded(runtime.start_run(a).await?)
        }
        "runs.get" => {
            let a: RunRef = args(params)?;
            encoded(runtime.get_run(&a.session_id, &a.run_id)?)
        }
        "runs.cancel" => {
            let a: RunRef = args(params)?;
            encoded(runtime.cancel_run(&a.session_id, &a.run_id)?)
        }
        "session_list" => {
            let _: Empty = args(params)?;
            encoded(runtime.sessions.list()?)
        }
        "session_create" => {
            let a: CreateSession = args(params)?;
            runtime.create_session(&a.id, a.workspace_id.as_deref())?;
            Ok(Value::Null)
        }
        "session_get" => {
            let a: Id = args(params)?;
            encoded(runtime.sessions.get(&a.id)?)
        }
        "session_load" => {
            let a: Id = args(params)?;
            encoded(runtime.sessions.load(&a.id)?)
        }
        "session_save" => {
            let a: Save = args(params)?;
            if a.item.role != "user" {
                return Err(DispatchError::Invalid);
            }
            runtime.sessions.save(&a.id, a.item, None)?;
            Ok(Value::Null)
        }
        "session_set_model" => {
            let a: Model = args(params)?;
            runtime.sessions.set_model(&a.id, &a.model)?;
            Ok(Value::Null)
        }
        "session_run" => {
            let a: Run = args(params)?;
            encoded(runtime.run_legacy(a.id, a.request).await?)
        }
        "session_cancel" => {
            let a: Cancel = args(params)?;
            encoded(runtime.cancel_run(&a.id, &a.run_id)?)
        }
        "model_providers" => {
            let _: Empty = args(params)?;
            encoded(runtime.plugins.providers()?.list().await)
        }
        "save_model_provider" => {
            let a: Provider = args(params)?;
            encoded(runtime.plugins.providers()?.save(a.provider).await?)
        }
        "discover_models" => {
            let a: Provider = args(params)?;
            encoded(runtime.plugins.providers()?.discover(a.provider).await?)
        }
        "delete_model_provider" => {
            let a: Id = args(params)?;
            encoded(runtime.plugins.providers()?.delete(&a.id).await?)
        }
        "list_plugins" => {
            let _: Empty = args(params)?;
            encoded(runtime.plugins.list().await?)
        }
        "set_plugin_enabled" => {
            let a: Toggle = args(params)?;
            encoded(runtime.plugins.set_enabled(&a.id, a.enabled).await?)
        }
        "set_optional_plugins_enabled" => {
            let a: OptionalPlugins = args(params)?;
            encoded(runtime.plugins.set_optional_enabled(a.enabled).await?)
        }
        "shell_settings" => {
            let _: Empty = args(params)?;
            encoded(runtime.plugins.shell()?.settings().await)
        }
        "save_shell_settings" => {
            let a: Settings = args(params)?;
            encoded(runtime.plugins.shell()?.save(a.settings).await?)
        }
        "run_shell_command" => {
            let a: Command = args(params)?;
            encoded(runtime.plugins.shell()?.run(a.command).await?)
        }
        _ => Err(DispatchError::NotFound),
    }
}

pub(crate) fn session_snapshot(runtime: &Runtime, id: &str) -> Result<Value, DispatchError> {
    encoded(runtime.sessions.baseline(id)?)
}
