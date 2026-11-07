//! Private per-workspace API. Only a scoped, short-lived gateway assertion admits a call.
use super::{failure, rpc, streams};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use minerva_runtime::Runtime;
use minerva_worker_auth::{AssertionKey, ExpectedRequest, ReplayCache, HEADER};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct WorkerApiConfig {
    pub assignment_id: Uuid,
    pub generation: u64,
    pub worker_id: Uuid,
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
    /// Derived assignment key only. Never the platform's master assertion key.
    pub assertion_key: AssertionKey,
}
#[derive(Clone)]
struct WorkerState {
    runtime: Arc<Runtime>,
    config: WorkerApiConfig,
    replay: Arc<ReplayCache>,
}
pub fn worker_router(runtime: Arc<Runtime>, config: WorkerApiConfig) -> Result<Router, String> {
    if config.generation == 0
        || runtime.config().tenant_id != config.tenant_id.to_string()
        || runtime.config().workspace_id != config.workspace_id.to_string()
    {
        return Err("worker scope does not match its runtime".into());
    }
    Ok(Router::new()
        .route("/api/v1/rpc", post(invoke))
        .route("/api/v1/streams", get(follow))
        .route("/health/ready", get(ready))
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .with_state(WorkerState {
            runtime,
            config,
            replay: Arc::new(ReplayCache::default()),
        }))
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |v| v.as_secs())
}
fn verify(
    state: &WorkerState,
    headers: &HeaderMap,
    action: &str,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<u64, Box<Response>> {
    let header = headers
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            super::rejection(
                StatusCode::UNAUTHORIZED,
                "worker/unauthorized",
                "Gateway assertion required",
            )
        })?;
    let c = &state.config;
    let verified = c
        .assertion_key
        .verify(
            header,
            &ExpectedRequest {
                assignment_id: c.assignment_id,
                generation: c.generation,
                worker_id: c.worker_id,
                tenant_id: c.tenant_id,
                workspace_id: c.workspace_id,
                action,
                http_method: method,
                path,
                body,
                now: now(),
            },
        )
        .map_err(|_| {
            super::rejection(
                StatusCode::UNAUTHORIZED,
                "worker/unauthorized",
                "Gateway assertion rejected",
            )
        })?;
    state.replay.admit(&verified, now()).map_err(|_| {
        super::rejection(
            StatusCode::UNAUTHORIZED,
            "worker/replay",
            "Gateway assertion already consumed",
        )
    })?;
    Ok(verified.claims().expires_at)
}
async fn ready(State(state): State<WorkerState>) -> Response {
    if state.runtime.is_stopped().await {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    Json(serde_json::json!({"ready":true,"assignmentId":state.config.assignment_id,"generation":state.config.generation})).into_response()
}
async fn invoke(State(state): State<WorkerState>, headers: HeaderMap, body: Bytes) -> Response {
    if state.runtime.is_stopped().await {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "worker/stopped",
            "Workspace Host is stopping",
        );
    }
    let input = match serde_json::from_slice::<rpc::RpcRequest>(&body) {
        Ok(v) => v,
        Err(_) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "request/invalid",
                "Invalid RPC envelope",
            )
        }
    };
    if let Err(r) = verify(
        &state,
        &headers,
        &input.method,
        "POST",
        "/api/v1/rpc",
        &body,
    ) {
        return *r;
    }
    if input.protocol_version != 1 {
        return failure(
            StatusCode::BAD_REQUEST,
            "protocol/unsupported",
            "Unsupported protocol version",
        );
    }
    if input.method == "run_shell_command" {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "capability/unavailable",
            "Hosted execution adapter is not configured",
        );
    }
    rpc::dispatch_response(state.runtime, &input.method, input.params).await
}
async fn follow(
    State(state): State<WorkerState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let expires = match verify(
        &state,
        &headers,
        "streams.connect",
        "GET",
        "/api/v1/streams",
        &[],
    ) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    ws.max_message_size(65536).on_upgrade(move |socket| {
        streams::serve_workspace(socket, state.runtime, move || now() < expires)
    })
}
