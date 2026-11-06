use axum::{
    http::{header, HeaderMap},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use minerva_server_control::session::SessionToken;
use serde_json::{json, Value};
use uuid::Uuid;
pub const SESSION_COOKIE: &str = "__Host-minerva_session";
pub const CSRF_COOKIE: &str = "__Host-minerva_csrf";
pub fn failure(status: StatusCode, code: &str) -> Response {
    let id = Uuid::new_v4().to_string();
    let mut response=(status,Json(json!({"ok":false,"error":{"code":code,"message":match status{StatusCode::UNAUTHORIZED=>"Sign in to this Minerva host",StatusCode::NOT_FOUND=>"Resource not found",StatusCode::SERVICE_UNAVAILABLE=>"No healthy isolated worker is available",_=>"The request could not be completed"},"requestId":id}}))).into_response();
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}
pub fn success(value: Value) -> Response {
    Json(json!({"ok":true,"value":value})).into_response()
}
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<SessionToken> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for pair in header.to_str().ok()?.split(';') {
            let Some((key, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if key == name {
                if found.is_some() {
                    return None;
                }
                found = Some(SessionToken::parse(value).ok()?);
            }
        }
    }
    found
}
pub fn origin_allowed(headers: &HeaderMap, origin: &str, required: bool) -> bool {
    let mut values = headers.get_all(header::ORIGIN).iter();
    match values.next() {
        Some(v) => v.to_str().ok() == Some(origin) && values.next().is_none(),
        None => !required,
    }
}
pub fn csrf_cookie(token: &SessionToken, lifetime: u32) -> String {
    format!(
        "{CSRF_COOKIE}={}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={lifetime}",
        token.expose()
    )
}
pub const CLEAR_CSRF_COOKIE: &str =
    "__Host-minerva_csrf=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0";
pub fn append_cookie(response: &mut Response, value: &str) {
    if let Ok(header) = HeaderValue::from_str(value) {
        response.headers_mut().append(header::SET_COOKIE, header);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookies_are_unambiguous_and_origins_are_exact() {
        let t = SessionToken::generate();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={}", t.expose())).unwrap(),
        );
        assert!(cookie(&headers, SESSION_COOKIE).is_some());
        headers.append(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={}", t.expose())).unwrap(),
        );
        assert!(cookie(&headers, SESSION_COOKIE).is_none());
        assert!(!origin_allowed(&headers, "https://gateway.test", true));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://gateway.test"),
        );
        assert!(origin_allowed(&headers, "https://gateway.test", true));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://gateway.test.evil"),
        );
        assert!(!origin_allowed(&headers, "https://gateway.test", false));
    }
}
