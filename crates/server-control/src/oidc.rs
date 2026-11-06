#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use openidconnect::core::{CoreHmacKey, CoreIdTokenClaims, CoreJwsSigningAlgorithm};
    use openidconnect::{Audience, EmptyAdditionalClaims, StandardClaims, SubjectIdentifier};
    fn adapter() -> OidcAdapter {
        let metadata: CoreProviderMetadata = serde_json::from_value(serde_json::json!({
            "issuer":"https://issuer.test", "authorization_endpoint":"https://issuer.test/auth", "token_endpoint":"https://issuer.test/token", "jwks_uri":"https://issuer.test/jwks", "response_types_supported":["code"], "subject_types_supported":["public"], "id_token_signing_alg_values_supported":["HS256"]
        })).unwrap();
        OidcAdapter::from_metadata(
            metadata,
            "client",
            Some("fixture-secret-at-least-32-bytes-long"),
            "https://gateway.test/auth/callback",
        )
        .unwrap()
    }
    fn token(issuer: &str, audience: &str, nonce: &str, seconds: i64, secret: &str) -> CoreIdToken {
        CoreIdToken::new(
            CoreIdTokenClaims::new(
                IssuerUrl::new(issuer.into()).unwrap(),
                vec![Audience::new(audience.into())],
                Utc::now() + Duration::seconds(seconds),
                Utc::now(),
                StandardClaims::new(SubjectIdentifier::new("alice".into())),
                EmptyAdditionalClaims {},
            )
            .set_nonce(Some(Nonce::new(nonce.into()))),
            &CoreHmacKey::new(secret),
            CoreJwsSigningAlgorithm::HmacSha256,
            None,
            None,
        )
        .unwrap()
    }
    #[test]
    fn signed_fixture_rejects_wrong_issuer_audience_nonce_expiry_and_signature() {
        let adapter = adapter();
        let secret = "fixture-secret-at-least-32-bytes-long";
        let valid = token("https://issuer.test", "client", "nonce", 300, secret);
        assert!(adapter.verify_id_token(&valid, "nonce", None).is_ok());
        for invalid in [
            token("https://evil.test", "client", "nonce", 300, secret),
            token("https://issuer.test", "other", "nonce", 300, secret),
            token("https://issuer.test", "client", "other", 300, secret),
            token("https://issuer.test", "client", "nonce", -300, secret),
            token(
                "https://issuer.test",
                "client",
                "nonce",
                300,
                "different-secret-at-least-32-bytes-long",
            ),
        ] {
            assert!(adapter.verify_id_token(&invalid, "nonce", None).is_err());
        }
    }
    #[test]
    fn redirect_uri_must_use_https_without_fragment() {
        assert!(validate_callback("http://gateway.test/callback").is_err());
        assert!(validate_callback("https://gateway.test/callback#bad").is_err());
        assert!(validate_callback("https://gateway.test/callback").is_ok());
    }
}
use crate::{
    identity::VerifiedIdentity,
    session::{safe_return_path, SessionToken},
    store::{ControlError, ControlStore},
};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreIdToken, CoreProviderMetadata},
    AccessToken, AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl,
    Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
};
#[derive(Debug, thiserror::Error)]
#[error("OIDC authentication failed")]
pub struct OidcError;
impl From<ControlError> for OidcError {
    fn from(_: ControlError) -> Self {
        Self
    }
}
/// No Debug: contains confidential-client credentials. Construct only from administrator config.
pub struct OidcAdapter {
    client: CoreClient,
    issuer: String,
}
/// Send redirect_url in Location and login_cookie in Set-Cookie. Never log either value.
pub struct LoginStart {
    pub redirect_url: String,
    pub login_cookie: String,
}
fn validate_callback(callback: &str) -> Result<RedirectUrl, OidcError> {
    let url = RedirectUrl::new(callback.into()).map_err(|_| OidcError)?;
    if url.url().scheme() != "https"
        || url.url().fragment().is_some()
        || !url.url().username().is_empty()
        || url.url().password().is_some()
    {
        return Err(OidcError);
    }
    Ok(url)
}
impl OidcAdapter {
    pub async fn discover(
        issuer: &str,
        client_id: &str,
        secret: Option<&str>,
        callback: &str,
    ) -> Result<Self, OidcError> {
        validate_callback(callback)?;
        let issuer = IssuerUrl::new(issuer.into()).map_err(|_| OidcError)?;
        if issuer.url().scheme() != "https"
            || issuer.url().fragment().is_some()
            || !issuer.url().username().is_empty()
            || issuer.url().password().is_some()
        {
            return Err(OidcError);
        }
        // Audited library client disables redirects and verifies discovery issuer equality.
        let metadata =
            CoreProviderMetadata::discover_async(issuer, openidconnect::reqwest::async_http_client)
                .await
                .map_err(|_| OidcError)?;
        Self::from_metadata(metadata, client_id, secret, callback)
    }
    fn from_metadata(
        metadata: CoreProviderMetadata,
        client_id: &str,
        secret: Option<&str>,
        callback: &str,
    ) -> Result<Self, OidcError> {
        if client_id.is_empty()
            || metadata.authorization_endpoint().url().scheme() != "https"
            || metadata.token_endpoint().map(|u| u.url().scheme()) != Some("https")
            || metadata.jwks_uri().url().scheme() != "https"
        {
            return Err(OidcError);
        }
        let issuer = metadata.issuer().as_str().to_owned();
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(client_id.into()),
            secret.map(|s| ClientSecret::new(s.into())),
        )
        .set_redirect_uri(validate_callback(callback)?);
        Ok(Self { client, issuer })
    }
    pub async fn begin(
        &self,
        store: &ControlStore,
        return_path: &str,
    ) -> Result<LoginStart, OidcError> {
        safe_return_path(return_path).map_err(|_| OidcError)?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = self
            .client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge)
            .url();
        let browser = SessionToken::generate();
        store
            .save_pending(
                state.secret(),
                &browser,
                &self.issuer,
                nonce.secret(),
                verifier.secret(),
                return_path,
            )
            .await?;
        Ok(LoginStart {
            redirect_url: url.to_string(),
            login_cookie: browser.login_cookie(),
        })
    }
    /// Atomically consumes the browser-bound state BEFORE exchanging a code. A replay always fails.
    /// Returns only verified identity and a constrained relative path, never provider tokens.
    pub async fn complete(
        &self,
        store: &ControlStore,
        state: &str,
        code: &str,
        login_cookie: &SessionToken,
    ) -> Result<(VerifiedIdentity, String), OidcError> {
        let (nonce, verifier, path) = store
            .consume_pending(state, login_cookie, &self.issuer)
            .await?;
        let response = self
            .client
            .exchange_code(AuthorizationCode::new(code.into()))
            .set_pkce_verifier(PkceCodeVerifier::new(verifier))
            .request_async(openidconnect::reqwest::async_http_client)
            .await
            .map_err(|_| OidcError)?;
        let token = response.id_token().ok_or(OidcError)?;
        let identity = self.verify_id_token(token, &nonce, Some(response.access_token()))?;
        safe_return_path(&path).map_err(|_| OidcError)?;
        Ok((identity, path))
    }
    fn verify_id_token(
        &self,
        token: &CoreIdToken,
        nonce: &str,
        access_token: Option<&AccessToken>,
    ) -> Result<VerifiedIdentity, OidcError> {
        // Library performs signature, issuer, audience, expiry and nonce validation. No custom JWT parser.
        let claims = token
            .claims(&self.client.id_token_verifier(), &Nonce::new(nonce.into()))
            .map_err(|_| OidcError)?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                access_token.ok_or(OidcError)?,
                &token.signing_alg().map_err(|_| OidcError)?,
            )
            .map_err(|_| OidcError)?;
            if actual != *expected {
                return Err(OidcError);
            }
        }
        Ok(VerifiedIdentity {
            issuer: self.issuer.clone(),
            subject: claims.subject().as_str().into(),
        })
    }
}
