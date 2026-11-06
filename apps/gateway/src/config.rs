use base64::{engine::general_purpose::STANDARD, Engine};
use minerva_worker_auth::AssertionKey;
use std::{net::SocketAddr, path::PathBuf};
#[derive(Debug, thiserror::Error)]
#[error("invalid or missing hosted gateway configuration")]
pub struct ConfigError;
/// Secrets deliberately do not implement Debug.
pub struct GatewayConfig {
    pub public_origin: String,
    pub authority: String,
    pub database_url: String,
    pub oidc_issuer: String,
    pub oidc_client_id: String,
    pub oidc_client_secret: Option<String>,
    pub runner_origin: String,
    pub assertion_key: AssertionKey,
    pub web_dist: PathBuf,
    pub bind: SocketAddr,
    pub allow_enrollment: bool,
    pub session_seconds: u32,
}
pub fn https_origin(input: &str) -> Result<String, ConfigError> {
    let u = url::Url::parse(input).map_err(|_| ConfigError)?;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(ConfigError);
    }
    Ok(u.origin().ascii_serialization())
}
impl GatewayConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        fn required(name: &str) -> Result<String, ConfigError> {
            std::env::var(name)
                .ok()
                .filter(|s| !s.is_empty())
                .ok_or(ConfigError)
        }
        let public_origin = https_origin(&required("MINERVA_PUBLIC_ORIGIN")?)?;
        let url = url::Url::parse(&public_origin).map_err(|_| ConfigError)?;
        let authority = url[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
        let runner_origin = https_origin(&required("MINERVA_RUNNER_ORIGIN")?)?;
        let database_url = required("MINERVA_CONTROL_DATABASE_URL")?;
        let db = url::Url::parse(&database_url).map_err(|_| ConfigError)?;
        if !matches!(db.scheme(), "postgres" | "postgresql") {
            return Err(ConfigError);
        }
        let oidc_issuer = required("MINERVA_OIDC_ISSUER")?;
        let issuer = url::Url::parse(&oidc_issuer).map_err(|_| ConfigError)?;
        if issuer.scheme() != "https"
            || issuer.query().is_some()
            || issuer.fragment().is_some()
            || !issuer.username().is_empty()
            || issuer.password().is_some()
        {
            return Err(ConfigError);
        }
        let key = STANDARD
            .decode(required("MINERVA_WORKER_ASSERTION_KEY")?)
            .map_err(|_| ConfigError)?;
        let assertion_key = AssertionKey::new(key).map_err(|_| ConfigError)?;
        let session_seconds = std::env::var("MINERVA_SESSION_SECONDS")
            .unwrap_or_else(|_| "43200".into())
            .parse()
            .map_err(|_| ConfigError)?;
        if session_seconds == 0 || session_seconds > 86400 {
            return Err(ConfigError);
        }
        let allow_enrollment = match std::env::var("MINERVA_ALLOW_ENROLLMENT").as_deref() {
            Ok("true") => true,
            Ok("false") | Err(_) => false,
            _ => return Err(ConfigError),
        };
        Ok(Self {
            public_origin,
            authority,
            database_url,
            oidc_issuer,
            oidc_client_id: required("MINERVA_OIDC_CLIENT_ID")?,
            oidc_client_secret: std::env::var("MINERVA_OIDC_CLIENT_SECRET").ok(),
            runner_origin,
            assertion_key,
            web_dist: std::env::var_os("MINERVA_WEB_DIST")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("dist")),
            bind: std::env::var("MINERVA_GATEWAY_BIND")
                .unwrap_or_else(|_| "127.0.0.1:4320".into())
                .parse()
                .map_err(|_| ConfigError)?,
            allow_enrollment,
            session_seconds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_and_runner_origins_reject_credentials_paths_and_plaintext() {
        for bad in [
            "http://example.test",
            "https://example.test/path",
            "https://user:pass@example.test",
            "https://example.test?x=1",
            "https://example.test/#x",
        ] {
            assert!(https_origin(bad).is_err());
        }
        assert_eq!(
            https_origin("https://example.test/").unwrap(),
            "https://example.test"
        );
    }
}
