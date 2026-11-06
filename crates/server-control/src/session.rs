#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_cookie_and_debug_never_expose_token() {
        let token = SessionToken::generate();
        let cookie = token.cookie(3600).unwrap();
        assert!(cookie.starts_with("__Host-minerva_session="));
        for flag in [
            "HttpOnly",
            "Secure",
            "SameSite=Lax",
            "Path=/",
            "Max-Age=3600",
        ] {
            assert!(cookie.contains(flag));
        }
        assert!(!format!("{token:?}").contains(token.expose()));
        assert!(token.cookie(0).is_err());
        assert!(token.cookie(86401).is_err());
    }
    #[test]
    fn unsafe_redirects_and_cross_site_mutations_are_rejected() {
        for path in [
            "https://evil.invalid",
            "//evil.invalid",
            "/\\evil",
            "/%2fevil",
            "/\r\nevil",
        ] {
            assert!(safe_return_path(path).is_err(), "{path:?}");
        }
        assert!(safe_return_path("/workspaces/one").is_ok());
        assert!(verify_csrf(
            "https://minerva.test",
            Some("https://evil.test"),
            "secret",
            "secret"
        )
        .is_err());
        assert!(verify_csrf(
            "https://minerva.test",
            Some("https://minerva.test"),
            "secret",
            "wrong"
        )
        .is_err());
        assert!(verify_csrf(
            "https://minerva.test",
            Some("https://minerva.test"),
            "secret",
            "secret"
        )
        .is_ok());
    }
}
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;
#[derive(Debug, thiserror::Error)]
#[error("invalid session request")]
pub struct SessionError;
/// An opaque bearer value. Debug is always redacted; only the cookie serializer exposes it.
pub struct SessionToken(String);
impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken([REDACTED])")
    }
}
impl SessionToken {
    pub fn generate() -> Self {
        Self(format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        ))
    }
    pub fn parse(value: &str) -> Result<Self, SessionError> {
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(SessionError);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub(crate) fn hash(&self) -> Vec<u8> {
        Sha256::digest(self.0.as_bytes()).to_vec()
    }
    pub fn cookie(&self, seconds: u32) -> Result<String, SessionError> {
        if seconds == 0 || seconds > 86400 {
            return Err(SessionError);
        }
        Ok(format!(
            "__Host-minerva_session={}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={seconds}",
            self.0
        ))
    }
    pub fn login_cookie(&self) -> String {
        format!(
            "__Host-minerva_login={}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=600",
            self.0
        )
    }
}
pub const CLEAR_SESSION_COOKIE: &str =
    "__Host-minerva_session=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0";
pub const CLEAR_LOGIN_COOKIE: &str =
    "__Host-minerva_login=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0";
pub fn safe_return_path(path: &str) -> Result<&str, SessionError> {
    // Intentionally narrow: percent encoding and backslashes can change interpretation in proxies.
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['\\', '%'])
        || path.chars().any(char::is_control)
    {
        return Err(SessionError);
    }
    Ok(path)
}
/// Require an exact configured Origin and a session-bound synchronizer token on mutations.
pub fn verify_csrf(
    expected_origin: &str,
    origin: Option<&str>,
    expected: &str,
    received: &str,
) -> Result<(), SessionError> {
    if origin != Some(expected_origin)
        || expected.is_empty()
        || !bool::from(expected.as_bytes().ct_eq(received.as_bytes()))
    {
        return Err(SessionError);
    }
    Ok(())
}
