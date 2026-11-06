//! HTTP boundary for a standalone Minerva workspace Host.
mod auth;
mod rpc;
mod streams;
mod worker;
pub use worker::{worker_router, WorkerApiConfig};

pub use auth::LocalApiConfig;
use axum::{
    extract::{DefaultBodyLimit, OriginalUri, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use minerva_runtime::Runtime;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tower_http::services::ServeDir;

#[derive(Clone)]
struct ApiState {
    runtime: Arc<Runtime>,
    auth: Arc<auth::LocalAuth>,
    config: LocalApiConfig,
}

pub fn local_router(runtime: Arc<Runtime>, config: LocalApiConfig) -> Router {
    let state = ApiState {
        runtime,
        auth: Arc::new(auth::LocalAuth::new(&config)),
        config,
    };
    let public = Router::new()
        .route("/api/v1/auth/exchange", post(auth::exchange))
        .route("/api/v1/bootstrap", get(auth::bootstrap))
        .route("/api/v1/logout", post(auth::logout))
        .route("/api/v1/rpc", post(rpc::handle))
        .route("/api/v1/streams", get(streams::upgrade))
        .route(
            "/health/live",
            get(|| async { Json(json!({"status":"live"})) }),
        )
        .route(
            "/health/ready",
            get(|| async { Json(json!({"status":"ready"})) }),
        )
        .route("/api/{*rest}", get(not_found).post(not_found))
        .route("/plugins/{*rest}", get(not_found))
        .nest_service(
            "/assets",
            ServeDir::new(state.config.web_dist.join("assets")),
        )
        .fallback_service(
            ServeDir::new(&state.config.web_dist)
                .fallback(get(spa_fallback).with_state(state.clone())),
        );
    public
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), request_guard))
        .with_state(state)
}

async fn not_found() -> impl IntoResponse {
    StatusCode::NOT_FOUND
}

async fn spa_fallback(State(state): State<ApiState>, OriginalUri(uri): OriginalUri) -> Response {
    if uri
        .path()
        .rsplit('/')
        .next()
        .is_some_and(|part| part.contains('.'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(state.config.web_dist.join("index.html")).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn request_guard(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    if state.runtime.is_stopped().await {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "host/stopped",
            "Host is shutting down",
        );
    }
    if request
        .headers()
        .get(header::HOST)
        .and_then(|s| s.to_str().ok())
        != Some(state.config.authority.as_str())
    {
        return failure(StatusCode::FORBIDDEN, "auth/host", "Host is not allowed");
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(header::CONTENT_SECURITY_POLICY,HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws://127.0.0.1:* ws://localhost:*; frame-ancestors 'none'; base-uri 'none'; object-src 'none'"));
    response
}

fn failure(status: StatusCode, code: &str, message: &str) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut response = (
        status,
        Json(json!({"ok":false,"error":{"code":code,"message":message,"requestId":request_id}})),
    )
        .into_response();
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).expect("UUID is a header value"),
    );
    response
}

fn rejection(status: StatusCode, code: &str, message: &str) -> Box<Response> {
    Box::new(failure(status, code, message))
}

fn success(value: Value) -> Response {
    Json(json!({"ok":true,"value":value})).into_response()
}

/// Resolve user-supplied relative CLI paths once, outside runtime services.
pub fn absolute_path(path: PathBuf) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
