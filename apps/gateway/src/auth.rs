use crate::{http::*, GatewayState};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use minerva_server_control::{
    identity::Principal,
    policy::{Action, AuthorizedWorkspace, Role},
    session::{SessionToken, CLEAR_LOGIN_COOKIE, CLEAR_SESSION_COOKIE},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginQuery {
    #[serde(default)]
    return_to: Option<String>,
}
pub async fn login(State(state): State<GatewayState>, Query(query): Query<LoginQuery>) -> Response {
    let path = query.return_to.as_deref().unwrap_or("/");
    let result = state
        .oidc
        .begin(&*state.login_control.lock().await, path)
        .await;
    match result {
        Ok(start) => {
            let mut response = StatusCode::SEE_OTHER.into_response();
            let Ok(location) = start.redirect_url.parse() else {
                return failure(StatusCode::INTERNAL_SERVER_ERROR, "auth/failed");
            };
            response.headers_mut().insert(header::LOCATION, location);
            append_cookie(&mut response, &start.login_cookie);
            response
        }
        Err(_) => failure(StatusCode::BAD_REQUEST, "auth/failed"),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Callback {
    code: Option<String>,
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    session_state: Option<String>,
}
pub async fn callback(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Response {
    let mut response = callback_inner(&state, &headers, query).await;
    append_cookie(&mut response, CLEAR_LOGIN_COOKIE);
    response
}
async fn callback_inner(state: &GatewayState, headers: &HeaderMap, query: Callback) -> Response {
    let _ = (&query.iss, &query.session_state);
    if query.error.is_some() {
        return failure(StatusCode::UNAUTHORIZED, "auth/failed");
    }
    let (Some(code), Some(nonce_state), Some(browser)) = (
        query.code,
        query.state,
        cookie(headers, "__Host-minerva_login"),
    ) else {
        return failure(StatusCode::UNAUTHORIZED, "auth/failed");
    };
    let mut store = state.login_control.lock().await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        state.oidc.complete(&store, &nonce_state, &code, &browser),
    )
    .await;
    let Ok(Ok((identity, path))) = result else {
        return failure(StatusCode::UNAUTHORIZED, "auth/failed");
    };
    let Ok(personal) = store
        .provision_personal(&identity, state.config.allow_enrollment)
        .await
    else {
        return failure(StatusCode::FORBIDDEN, "auth/enrollment-denied");
    };
    let Ok((session, csrf)) = store
        .create_session(&personal, state.config.session_seconds)
        .await
    else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "auth/unavailable");
    };
    let Ok(cookie) = session.cookie(state.config.session_seconds) else {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, "auth/failed");
    };
    let mut response = StatusCode::SEE_OTHER.into_response();
    let Ok(location) = path.parse() else {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, "auth/failed");
    };
    response.headers_mut().insert(header::LOCATION, location);
    append_cookie(&mut response, &cookie);
    append_cookie(
        &mut response,
        &csrf_cookie(&csrf, state.config.session_seconds),
    );
    response
}
#[allow(clippy::result_large_err)] // Final denial responses are shared verbatim across transports.
pub(crate) async fn authenticate(
    state: &GatewayState,
    headers: &HeaderMap,
    mutation: bool,
) -> Result<Principal, Response> {
    let token = preflight(headers, &state.config.public_origin, mutation)?;
    let store = state.control.lock().await;
    let principal = store
        .authenticate(&token)
        .await
        .map_err(|error| match error {
            minerva_server_control::store::ControlError::Database(_) => {
                failure(StatusCode::SERVICE_UNAVAILABLE, "control/unavailable")
            }
            _ => failure(StatusCode::UNAUTHORIZED, "auth/required"),
        })?;
    if mutation {
        let csrf = mutation_csrf(headers, &state.config.public_origin)?;
        store
            .check_csrf(
                &principal,
                &state.config.public_origin,
                headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
                &csrf,
            )
            .await
            .map_err(|_| failure(StatusCode::FORBIDDEN, "auth/csrf"))?;
    }
    Ok(principal)
}
#[allow(clippy::result_large_err)] // Keep missing and foreign-resource denials identical.
pub(crate) async fn workspace_grant(
    state: &GatewayState,
    p: &Principal,
    id: Uuid,
    action: Action,
) -> Result<AuthorizedWorkspace, Response> {
    let store = state.control.lock().await;
    let resources = store
        .list_workspaces(p)
        .await
        .map_err(|_| failure(StatusCode::SERVICE_UNAVAILABLE, "control/unavailable"))?;
    let resource = resources
        .into_iter()
        .find(|r| r.workspace_id == id)
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "resource/not-found"))?;
    store
        .authorize(p, action, Some(resource))
        .await
        .map_err(|_| failure(StatusCode::NOT_FOUND, "resource/not-found"))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapQuery {
    workspace_id: Option<Uuid>,
}
pub async fn bootstrap(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Query(query): Query<BootstrapQuery>,
) -> Response {
    let p = match authenticate(&state, &headers, false).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(csrf) = cookie(&headers, CSRF_COOKIE) else {
        return failure(StatusCode::UNAUTHORIZED, "auth/required");
    };
    if state
        .control
        .lock()
        .await
        .check_csrf(
            &p,
            &state.config.public_origin,
            Some(&state.config.public_origin),
            &csrf,
        )
        .await
        .is_err()
    {
        return failure(StatusCode::UNAUTHORIZED, "auth/required");
    }
    let id = match query.workspace_id {
        Some(id) => id,
        None => {
            let row=match state.metadata.query_opt("SELECT workspace_id FROM personal_workspaces WHERE user_id=$1 AND tenant_id=$2",&[&p.user_id(),&p.tenant_id()]).await{Ok(Some(row))=>row,_=>return failure(StatusCode::NOT_FOUND,"resource/not-found")};
            row.get(0)
        }
    };
    let grant = match workspace_grant(&state, &p, id, Action::Read).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    let ready = state.directory.resolve(grant.resource()).await.is_ok();
    let capabilities: Vec<&str> = if ready {
        vec!["sessions", "runs", "providers", "plugins", "shell"]
    } else {
        vec![]
    };
    Json(json!({"protocolVersion":1,"csrfToken":csrf.expose(),"principal":{"id":p.user_id(),"tenantId":p.tenant_id()},"workspace":{"id":id},"capabilities":capabilities,"permissions":{"read":true,"write":grant.role()!=Role::Viewer,"configure":grant.role()==Role::Owner,"managePlugins":grant.role()==Role::Owner},"mode":"hosted","hostedReady":ready,"unavailableReason":if ready{None}else{Some("No healthy runsc workspace worker is assigned")}})).into_response()
}
pub async fn logout(State(state): State<GatewayState>, headers: HeaderMap) -> Response {
    let p = match authenticate(&state, &headers, true).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if state.control.lock().await.revoke_session(&p).await.is_err() {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "control/unavailable");
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    for cookie in [CLEAR_SESSION_COOKIE, CLEAR_CSRF_COOKIE, CLEAR_LOGIN_COOKIE] {
        append_cookie(&mut response, cookie);
    }
    response
}

#[allow(clippy::result_large_err)] // Same denial shape as the authenticated database checks.
fn preflight(headers: &HeaderMap, origin: &str, mutation: bool) -> Result<SessionToken, Response> {
    if !origin_allowed(headers, origin, mutation) {
        return Err(failure(StatusCode::FORBIDDEN, "auth/origin"));
    }
    cookie(headers, SESSION_COOKIE)
        .ok_or_else(|| failure(StatusCode::UNAUTHORIZED, "auth/required"))
}

#[allow(clippy::result_large_err)] // A single final HTTP denial shape for all CSRF failures.
fn mutation_csrf(headers: &HeaderMap, origin: &str) -> Result<SessionToken, Response> {
    let denied = || failure(StatusCode::FORBIDDEN, "auth/csrf");
    let cookie = cookie(headers, CSRF_COOKIE).ok_or_else(denied)?;
    let mut values = headers.get_all("x-minerva-csrf").iter();
    let header = values
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| SessionToken::parse(v).ok())
        .ok_or_else(denied)?;
    if values.next().is_some() {
        return Err(denied());
    }
    minerva_server_control::session::verify_csrf(
        origin,
        headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
        cookie.expose(),
        header.expose(),
    )
    .map_err(|_| denied())?;
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_authentication_fails_before_storage_on_cross_origin_missing_or_duplicate_credentials(
    ) {
        let mut headers = HeaderMap::new();
        assert_eq!(
            preflight(&headers, "https://gateway.test", true)
                .unwrap_err()
                .status(),
            StatusCode::FORBIDDEN
        );
        headers.insert(header::ORIGIN, "https://evil.test".parse().unwrap());
        assert_eq!(
            preflight(&headers, "https://gateway.test", true)
                .unwrap_err()
                .status(),
            StatusCode::FORBIDDEN
        );
        headers.insert(header::ORIGIN, "https://gateway.test".parse().unwrap());
        assert_eq!(
            preflight(&headers, "https://gateway.test", true)
                .unwrap_err()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let token = SessionToken::generate();
        headers.insert(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.expose())
                .parse()
                .unwrap(),
        );
        assert!(preflight(&headers, "https://gateway.test", true).is_ok());
        headers.append(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.expose())
                .parse()
                .unwrap(),
        );
        assert_eq!(
            preflight(&headers, "https://gateway.test", true)
                .unwrap_err()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[cfg(test)]
mod csrf_cookie_tests {
    use super::*;
    #[test]
    fn mutation_header_must_match_the_bound_cookie_and_be_unambiguous() {
        let token = SessionToken::generate();
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://gateway.test".parse().unwrap());
        headers.insert("x-minerva-csrf", token.expose().parse().unwrap());
        assert!(mutation_csrf(&headers, "https://gateway.test").is_err());
        headers.insert(
            header::COOKIE,
            format!("{CSRF_COOKIE}={}", SessionToken::generate().expose())
                .parse()
                .unwrap(),
        );
        assert!(mutation_csrf(&headers, "https://gateway.test").is_err());
        headers.insert(
            header::COOKIE,
            format!("{CSRF_COOKIE}={}", token.expose()).parse().unwrap(),
        );
        assert!(mutation_csrf(&headers, "https://gateway.test").is_ok());
        headers.append("x-minerva-csrf", token.expose().parse().unwrap());
        assert!(mutation_csrf(&headers, "https://gateway.test").is_err());
    }
}
