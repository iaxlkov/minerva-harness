//! Hosted gateway; has no dependency on the local Runtime.
pub mod config;
pub mod directory;
pub mod registry;
pub(crate) async fn bounded_body(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, ()> {
    use futures_util::StreamExt;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(());
    }
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if body.len() + chunk.len() > limit {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
mod rpc;
mod streams;
use axum::{
    extract::{DefaultBodyLimit, OriginalUri, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use config::GatewayConfig;
use directory::WorkerDirectory;
use minerva_server_control::{oidc::OidcAdapter, store::ControlStore};
use std::{sync::Arc, time::Duration};
use tower_http::services::ServeDir;
mod auth;
mod http;
#[derive(Clone)]
pub struct GatewayState {
    pub config: Arc<GatewayConfig>,
    pub control: Arc<tokio::sync::Mutex<ControlStore>>,
    pub login_control: Arc<tokio::sync::Mutex<ControlStore>>,
    pub metadata: Arc<tokio_postgres::Client>,
    pub oidc: Arc<OidcAdapter>,
    pub directory: Arc<dyn WorkerDirectory>,
    pub http: reqwest::Client,
}
pub fn gateway_router(state: GatewayState) -> Router {
    Router::new()
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/logout", post(auth::logout))
        .route("/api/v1/bootstrap", get(auth::bootstrap))
        .route("/api/v1/logout", post(auth::logout))
        .route("/api/v1/rpc", post(rpc::handle))
        .route("/api/v1/streams", get(streams::upgrade))
        .route(
            "/health/live",
            get(|| async { Json(serde_json::json!({"status":"live"})) }),
        )
        .route("/health/ready", get(ready))
        .route("/api/{*rest}", get(not_found).post(not_found))
        .route("/plugins/{*rest}", get(not_found))
        .nest_service(
            "/assets",
            ServeDir::new(state.config.web_dist.join("assets")),
        )
        .fallback_service(
            ServeDir::new(&state.config.web_dist)
                .fallback(get(spa_fallback).with_state(state.clone())),
        )
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}
async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
async fn ready(State(state): State<GatewayState>) -> Response {
    match state.directory.ready().await {
        Ok(true) => Json(serde_json::json!({"ready":true,"sandbox":"runsc"})).into_response(),
        _ => http::failure(StatusCode::SERVICE_UNAVAILABLE, "hosted/not-ready"),
    }
}
async fn spa_fallback(
    State(state): State<GatewayState>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    if uri
        .path()
        .rsplit('/')
        .next()
        .is_some_and(|s| s.contains('.'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(state.config.web_dist.join("index.html")).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn guard(State(state): State<GatewayState>, request: Request, next: Next) -> Response {
    if !valid_host(request.headers(), &state.config.authority) {
        return http::failure(StatusCode::FORBIDDEN, "auth/host");
    }
    let path = request.uri().path();
    if request
        .headers()
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        == Some("cross-site")
        && path.starts_with("/api/")
    {
        return http::failure(StatusCode::FORBIDDEN, "auth/origin");
    }
    let document = document_request(request.method(), path);
    if document {
        if !http::origin_allowed(request.headers(), &state.config.public_origin, false) {
            return http::failure(StatusCode::FORBIDDEN, "auth/origin");
        }
        let authenticated = match (
            http::cookie(request.headers(), http::SESSION_COOKIE),
            http::cookie(request.headers(), http::CSRF_COOKIE),
        ) {
            (Some(token), Some(csrf)) => {
                let store = state.control.lock().await;
                match store.authenticate(&token).await {
                    Ok(p) => store
                        .check_csrf(
                            &p,
                            &state.config.public_origin,
                            Some(&state.config.public_origin),
                            &csrf,
                        )
                        .await
                        .is_ok(),
                    Err(minerva_server_control::store::ControlError::Database(_)) => {
                        return secured(http::failure(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "control/unavailable",
                        ))
                    }
                    Err(_) => false,
                }
            }
            _ => false,
        };
        if !authenticated {
            let return_path = request
                .uri()
                .path_and_query()
                .map(|p| p.as_str())
                .unwrap_or("/");
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("returnTo", return_path)
                .finish();
            let mut response = StatusCode::SEE_OTHER.into_response();
            if let Ok(location) = format!("/auth/login?{query}").parse() {
                response.headers_mut().insert(header::LOCATION, location);
            }
            return secured(response);
        }
    }
    secured(next.run(request).await)
}
fn document_request(method: &axum::http::Method, path: &str) -> bool {
    method == axum::http::Method::GET
        && (path == "/index.html"
            || (!matches!(
                path.split('/').nth(1),
                Some("api" | "auth" | "health" | "assets" | "plugins")
            ) && !path
                .rsplit('/')
                .next()
                .is_some_and(|part| part.contains('.'))))
}
fn valid_host(headers: &axum::http::HeaderMap, authority: &str) -> bool {
    let mut values = headers.get_all(header::HOST).iter();
    matches!(values.next(),Some(v) if v.to_str().ok()==Some(authority)) && values.next().is_none()
}
fn secured(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(header::CONTENT_SECURITY_POLICY,HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'"));
    headers.insert(
        "strict-transport-security",
        HeaderValue::from_static("max-age=31536000"),
    );
    response
}
/// Connect only using verified TLS; deployment trust roots are supplied by the operating system.
pub async fn connect_database(url: &str) -> Result<tokio_postgres::Client, StartupError> {
    let mut config: url::Url = url.parse().map_err(|_| StartupError)?;
    // tokio-postgres parses the original DSN; URL parsing here prevents accidental non-PostgreSQL schemes.
    if !matches!(config.scheme(), "postgres" | "postgresql") {
        return Err(StartupError);
    }
    config.set_fragment(None);
    let mut database: tokio_postgres::Config = url.parse().map_err(|_| StartupError)?;
    database.ssl_mode(tokio_postgres::config::SslMode::Require);
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|_| StartupError)?;
    let (client, connection) = database
        .connect(postgres_native_tls::MakeTlsConnector::new(tls))
        .await
        .map_err(|_| StartupError)?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute("SET statement_timeout='5s'; SET lock_timeout='3s'")
        .await
        .map_err(|_| StartupError)?;
    Ok(client)
}
#[derive(Debug, thiserror::Error)]
#[error("gateway startup failed; verify administrator configuration and service availability")]
pub struct StartupError;
pub async fn start(config: GatewayConfig) -> Result<(), StartupError> {
    let bind = config.bind;
    let config = Arc::new(config);
    let mut control = ControlStore::new(connect_database(&config.database_url).await?);
    control.migrate().await.map_err(|_| StartupError)?;
    let metadata = Arc::new(connect_database(&config.database_url).await?);
    let directory = directory::PostgresWorkerDirectory::new(
        connect_database(&config.database_url).await?,
        config.runner_origin.clone(),
    )
    .await
    .map_err(|_| StartupError)?;
    let oidc = OidcAdapter::discover(
        &config.oidc_issuer,
        &config.oidc_client_id,
        config.oidc_client_secret.as_deref(),
        &format!("{}/auth/callback", config.public_origin),
    )
    .await
    .map_err(|_| StartupError)?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| StartupError)?;
    let login_control = ControlStore::new(connect_database(&config.database_url).await?);
    let state = GatewayState {
        config,
        control: Arc::new(tokio::sync::Mutex::new(control)),
        login_control: Arc::new(tokio::sync::Mutex::new(login_control)),
        metadata,
        oidc: Arc::new(oidc),
        directory: Arc::new(directory),
        http,
    };
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|_| StartupError)?;
    axum::serve(listener, gateway_router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| StartupError)
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn api_routes_never_take_the_document_login_redirect_path() {
        for path in [
            "/api",
            "/api/v1/bootstrap",
            "/api/v1/rpc",
            "/api/v1/streams",
            "/auth/login",
            "/health/ready",
            "/assets/main.js",
            "/plugins/test.js",
        ] {
            assert!(!document_request(&axum::http::Method::GET, path), "{path}");
        }
        for path in ["/", "/workspace/one", "/index.html"] {
            assert!(document_request(&axum::http::Method::GET, path));
        }
        assert!(!document_request(&axum::http::Method::POST, "/index.html"));
    }
    #[test]
    fn duplicate_missing_and_foreign_host_headers_are_rejected() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!valid_host(&headers, "gateway.test"));
        headers.insert(header::HOST, "evil.test".parse().unwrap());
        assert!(!valid_host(&headers, "gateway.test"));
        headers.insert(header::HOST, "gateway.test".parse().unwrap());
        assert!(valid_host(&headers, "gateway.test"));
        headers.append(header::HOST, "gateway.test".parse().unwrap());
        assert!(!valid_host(&headers, "gateway.test"));
    }
    #[tokio::test]
    async fn oversized_worker_responses_are_rejected_without_buffering_whole_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut input = [0; 1024];
            let _ = socket.read(&mut input).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 10000000\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let response = reqwest::get(format!("http://{addr}")).await.unwrap();
        assert!(bounded_body(response, 4096).await.is_err());
        server.await.unwrap();
    }
}
