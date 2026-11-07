use super::{failure, ApiState};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[derive(Clone)]
pub struct LocalApiConfig {
    pub origin: String,
    pub launch_token: String,
    pub web_dist: PathBuf,
    pub(crate) authority: String,
    allowed_origins: Vec<String>,
    cookie_name: String,
}
impl LocalApiConfig {
    pub fn new(origin: &str, web_dist: PathBuf) -> Result<Self, String> {
        let url = url::Url::parse(origin).map_err(|e| e.to_string())?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Local API requires an HTTP loopback origin".into());
        }
        let authority = url[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
        let origin = url.origin().ascii_serialization();
        Ok(Self {
            allowed_origins: vec![origin.clone()],
            origin,
            authority,
            launch_token: secret(),
            web_dist,
            cookie_name: format!(
                "minerva_session_{}",
                url.port_or_known_default().unwrap_or(80)
            ),
        })
    }
    pub fn allow_dev_origin(&mut self, origin: &str) -> Result<(), String> {
        let validated = Self::new(origin, PathBuf::new())?;
        self.allowed_origins.push(validated.origin);
        Ok(())
    }
    fn origin_allowed(&self, headers: &HeaderMap) -> bool {
        headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| self.allowed_origins.iter().any(|origin| origin == v))
    }
}

#[derive(Clone)]
pub(crate) struct Session {
    pub csrf: String,
    expires: Instant,
}
pub(crate) struct LocalAuth {
    launch: Mutex<Option<(String, Instant)>>,
    sessions: Mutex<HashMap<String, Session>>,
}
impl LocalAuth {
    pub fn new(config: &LocalApiConfig) -> Self {
        Self {
            launch: Mutex::new(Some((
                config.launch_token.clone(),
                Instant::now() + Duration::from_secs(300),
            ))),
            sessions: Mutex::new(HashMap::new()),
        }
    }
    fn authenticate(&self, headers: &HeaderMap, config: &LocalApiConfig) -> Option<Session> {
        let cookie = cookie(headers, &config.cookie_name)?;
        let mut sessions = self.sessions.lock().ok()?;
        sessions.retain(|_, s| s.expires > Instant::now());
        sessions.get(cookie).cloned()
    }
}
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

pub(crate) fn authorize(
    state: &ApiState,
    headers: &HeaderMap,
    mutation: bool,
) -> Result<Session, Box<Response>> {
    let session = state
        .auth
        .authenticate(headers, &state.config)
        .ok_or_else(|| {
            super::rejection(
                StatusCode::UNAUTHORIZED,
                "auth/required",
                "Sign in to this Minerva host",
            )
        })?;
    if mutation {
        if !state.config.origin_allowed(headers) {
            return Err(super::rejection(
                StatusCode::FORBIDDEN,
                "auth/origin",
                "Origin is not allowed",
            ));
        }
        let csrf = headers
            .get("x-minerva-csrf")
            .map_or(&[][..], |s| s.as_bytes());
        if !bool::from(csrf.ct_eq(session.csrf.as_bytes())) {
            return Err(super::rejection(
                StatusCode::FORBIDDEN,
                "auth/csrf",
                "Invalid CSRF token",
            ));
        }
    } else if headers.contains_key(header::ORIGIN) && !state.config.origin_allowed(headers) {
        return Err(super::rejection(
            StatusCode::FORBIDDEN,
            "auth/origin",
            "Origin is not allowed",
        ));
    }
    Ok(session)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Exchange {
    token: String,
}
pub(crate) async fn exchange(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Exchange>,
) -> Response {
    if !state.config.origin_allowed(&headers) {
        return failure(
            StatusCode::FORBIDDEN,
            "auth/origin",
            "Origin is not allowed",
        );
    }
    let accepted = {
        let mut pending = state.auth.launch.lock().expect("local launch lock");
        let valid = pending.as_ref().is_some_and(|(token, expires)| {
            *expires > Instant::now() && bool::from(input.token.as_bytes().ct_eq(token.as_bytes()))
        });
        if valid {
            pending.take();
        }
        valid
    };
    if !accepted {
        return failure(
            StatusCode::UNAUTHORIZED,
            "auth/invalid-launch",
            "Launch credential is invalid or expired",
        );
    }
    let id = secret();
    state
        .auth
        .sessions
        .lock()
        .expect("local sessions lock")
        .insert(
            id.clone(),
            Session {
                csrf: secret(),
                expires: Instant::now() + Duration::from_secs(43200),
            },
        );
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{}={id}; Path=/api/v1; HttpOnly; SameSite=Strict; Max-Age=43200",
            state.config.cookie_name
        ))
        .expect("generated cookie"),
    );
    response
}
pub(crate) async fn bootstrap(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let session = match authorize(&state, &headers, false) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    Json(json!({"protocolVersion":1,"csrfToken":session.csrf,"principal":{"id":"local"},"workspace":{"id":state.runtime.config().workspace_id},"capabilities":["sessions","runs","providers","plugins","shell"]})).into_response()
}
pub(crate) async fn logout(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers, true) {
        return *r;
    }
    if let Some(id) = cookie(&headers, &state.config.cookie_name) {
        state
            .auth
            .sessions
            .lock()
            .expect("local sessions lock")
            .remove(id);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{}=; Path=/api/v1; HttpOnly; SameSite=Strict; Max-Age=0",
            state.config.cookie_name
        ))
        .expect("generated cookie"),
    );
    response
}
