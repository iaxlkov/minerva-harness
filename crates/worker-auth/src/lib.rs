use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
pub const HEADER: &str = "x-minerva-assertion";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssertionClaims {
    pub version: u32,
    pub assignment_id: Uuid,
    pub generation: u64,
    pub worker_id: Uuid,
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
    pub action: String,
    pub http_method: String,
    pub path: String,
    pub body_sha256: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub request_id: Uuid,
}
#[derive(Debug, thiserror::Error)]
#[error("worker assertion rejected")]
pub struct AssertionError;
#[derive(Clone)]
pub struct AssertionKey(Vec<u8>);
impl std::fmt::Debug for AssertionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssertionKey([REDACTED])")
    }
}
pub struct ExpectedRequest<'a> {
    pub assignment_id: Uuid,
    pub generation: u64,
    pub worker_id: Uuid,
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
    pub action: &'a str,
    pub http_method: &'a str,
    pub path: &'a str,
    pub body: &'a [u8],
    pub now: u64,
}
#[derive(Debug)]
pub struct VerifiedAssertion(AssertionClaims);
impl VerifiedAssertion {
    pub fn claims(&self) -> &AssertionClaims {
        &self.0
    }
}
pub fn body_digest(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
impl AssertionKey {
    pub fn new(key: Vec<u8>) -> Result<Self, AssertionError> {
        if key.len() < 32 {
            return Err(AssertionError);
        }
        Ok(Self(key))
    }
    pub fn sign(&self, claims: &AssertionClaims) -> Result<String, AssertionError> {
        validate_claims(claims)?;
        let payload = serde_json::to_vec(claims).map_err(|_| AssertionError)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).map_err(|_| AssertionError)?;
        mac.update(&payload);
        Ok(format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&payload),
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        ))
    }
    pub fn verify(
        &self,
        header: &str,
        expected: &ExpectedRequest<'_>,
    ) -> Result<VerifiedAssertion, AssertionError> {
        if header.len() > 8192 {
            return Err(AssertionError);
        }
        let (payload, signature) = header.split_once('.').ok_or(AssertionError)?;
        let payload = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| AssertionError)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| AssertionError)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).map_err(|_| AssertionError)?;
        mac.update(&payload);
        mac.verify_slice(&signature).map_err(|_| AssertionError)?;
        let claims: AssertionClaims =
            serde_json::from_slice(&payload).map_err(|_| AssertionError)?;
        validate_claims(&claims)?;
        if claims.assignment_id != expected.assignment_id
            || claims.generation != expected.generation
            || claims.worker_id != expected.worker_id
            || claims.tenant_id != expected.tenant_id
            || claims.workspace_id != expected.workspace_id
            || claims.action != expected.action
            || claims.http_method != expected.http_method
            || claims.path != expected.path
            || claims.body_sha256 != body_digest(expected.body)
            || expected.now < claims.issued_at
            || expected.now >= claims.expires_at
        {
            return Err(AssertionError);
        }
        Ok(VerifiedAssertion(claims))
    }
}
fn validate_claims(c: &AssertionClaims) -> Result<(), AssertionError> {
    if c.version != 1
        || c.generation == 0
        || c.expires_at <= c.issued_at
        || c.expires_at - c.issued_at > 60
        || c.action.is_empty()
        || c.action.len() > 128
        || !matches!(c.http_method.as_str(), "POST" | "GET")
        || !matches!(c.path.as_str(), "/api/v1/rpc" | "/api/v1/streams")
        || c.body_sha256.len() != 64
        || !c.body_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(AssertionError);
    }
    Ok(())
}
impl AssertionKey {
    /// Derive one assignment's credential. Only gateway and runner hold the master key.
    pub fn derive_worker(
        &self,
        assignment_id: Uuid,
        generation: u64,
        worker_id: Uuid,
        tenant_id: Uuid,
        workspace_id: Uuid,
    ) -> Result<Self, AssertionError> {
        if generation == 0 {
            return Err(AssertionError);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).map_err(|_| AssertionError)?;
        mac.update(b"minerva.worker.assignment-key.v1\0");
        mac.update(assignment_id.as_bytes());
        mac.update(&generation.to_be_bytes());
        mac.update(worker_id.as_bytes());
        mac.update(tenant_id.as_bytes());
        mac.update(workspace_id.as_bytes());
        Self::new(mac.finalize().into_bytes().to_vec())
    }
    /// Trusted runner secret mount only. Never include this in browser DTOs or logs.
    pub fn expose_secret(&self) -> &[u8] {
        &self.0
    }
}
/// Worker-owned, bounded replay protection. Keep one cache for the assignment lifetime.
/// Live entries are never evicted; exhaustion denies requests until entries expire.
#[derive(Default)]
pub struct ReplayCache {
    entries: std::sync::Mutex<std::collections::HashMap<Uuid, u64>>,
}
impl ReplayCache {
    pub fn admit(&self, verified: &VerifiedAssertion, now: u64) -> Result<(), AssertionError> {
        let claims = verified.claims();
        if now < claims.issued_at || now >= claims.expires_at {
            return Err(AssertionError);
        }
        let mut entries = self.entries.lock().map_err(|_| AssertionError)?;
        entries.retain(|_, expiry| *expiry > now);
        if entries.contains_key(&claims.request_id) || entries.len() >= 4096 {
            return Err(AssertionError);
        }
        entries.insert(claims.request_id, claims.expires_at);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn claims() -> AssertionClaims {
        AssertionClaims {
            version: 1,
            assignment_id: Uuid::new_v4(),
            generation: 3,
            worker_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            action: "runs.start".into(),
            http_method: "POST".into(),
            path: "/api/v1/rpc".into(),
            body_sha256: body_digest(b"request"),
            issued_at: 100,
            expires_at: 130,
            request_id: Uuid::new_v4(),
        }
    }
    #[test]
    fn signature_is_bound_to_assignment_scope_method_body_and_lifetime() {
        let signer = AssertionKey::new(vec![7; 32]).unwrap();
        let c = claims();
        let header = signer.sign(&c).unwrap();
        let expected = ExpectedRequest {
            assignment_id: c.assignment_id,
            generation: c.generation,
            worker_id: c.worker_id,
            tenant_id: c.tenant_id,
            workspace_id: c.workspace_id,
            action: "runs.start",
            http_method: "POST",
            path: "/api/v1/rpc",
            body: b"request",
            now: 110,
        };
        assert_eq!(
            signer.verify(&header, &expected).unwrap().claims().user_id,
            c.user_id
        );
        assert!(signer
            .verify(
                &header,
                &ExpectedRequest {
                    body: b"changed",
                    ..expected
                }
            )
            .is_err());
        assert!(signer
            .verify(
                &header,
                &ExpectedRequest {
                    generation: 4,
                    ..expected
                }
            )
            .is_err());
        assert!(signer
            .verify(
                &header,
                &ExpectedRequest {
                    workspace_id: Uuid::new_v4(),
                    ..expected
                }
            )
            .is_err());
        assert!(signer
            .verify(
                &header,
                &ExpectedRequest {
                    action: "runs.cancel",
                    ..expected
                }
            )
            .is_err());
        assert!(signer
            .verify(
                &header,
                &ExpectedRequest {
                    now: 131,
                    ..expected
                }
            )
            .is_err());
        assert!(signer
            .verify(
                &header,
                &ExpectedRequest {
                    now: 90,
                    ..expected
                }
            )
            .is_err());
        assert!(AssertionKey::new(vec![8; 32])
            .unwrap()
            .verify(&header, &expected)
            .is_err());
        let mut too_long = c.clone();
        too_long.expires_at = 200;
        assert!(signer.sign(&too_long).is_err());
        assert!(!format!("{signer:?}").contains("7, 7"));
    }
    #[test]
    fn malformed_headers_and_short_keys_are_rejected() {
        assert!(AssertionKey::new(vec![1; 31]).is_err());
        let signer = AssertionKey::new(vec![7; 32]).unwrap();
        let c = claims();
        let expected = ExpectedRequest {
            assignment_id: c.assignment_id,
            generation: c.generation,
            worker_id: c.worker_id,
            tenant_id: c.tenant_id,
            workspace_id: c.workspace_id,
            action: "runs.start",
            http_method: "POST",
            path: "/api/v1/rpc",
            body: b"request",
            now: 110,
        };
        for value in ["", ".", "a.b.c", "e30.fake"] {
            assert!(signer.verify(value, &expected).is_err());
        }
    }
}

#[cfg(test)]
mod derivation_tests {
    use super::*;
    #[test]
    fn compromised_assignment_key_cannot_authenticate_another_workspace() {
        let master = AssertionKey::new(vec![3; 32]).unwrap();
        let a = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let b = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let ak = master.derive_worker(a.0, 1, a.1, a.2, a.3).unwrap();
        let bk = master.derive_worker(b.0, 1, b.1, b.2, b.3).unwrap();
        let claims = AssertionClaims {
            version: 1,
            assignment_id: b.0,
            generation: 1,
            worker_id: b.1,
            user_id: Uuid::new_v4(),
            tenant_id: b.2,
            workspace_id: b.3,
            action: "session_list".into(),
            http_method: "POST".into(),
            path: "/api/v1/rpc".into(),
            body_sha256: body_digest(b"{}"),
            issued_at: 10,
            expires_at: 40,
            request_id: Uuid::new_v4(),
        };
        let expected = ExpectedRequest {
            assignment_id: b.0,
            generation: 1,
            worker_id: b.1,
            tenant_id: b.2,
            workspace_id: b.3,
            action: "session_list",
            http_method: "POST",
            path: "/api/v1/rpc",
            body: b"{}",
            now: 20,
        };
        assert!(bk.verify(&ak.sign(&claims).unwrap(), &expected).is_err());
        assert!(bk.verify(&bk.sign(&claims).unwrap(), &expected).is_ok());
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    fn proof(id: Uuid, expires: u64) -> VerifiedAssertion {
        VerifiedAssertion(AssertionClaims {
            version: 1,
            assignment_id: Uuid::new_v4(),
            generation: 1,
            worker_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            action: "session_list".into(),
            http_method: "POST".into(),
            path: "/api/v1/rpc".into(),
            body_sha256: body_digest(b"{}"),
            issued_at: 10,
            expires_at: expires,
            request_id: id,
        })
    }
    #[test]
    fn replay_cache_rejects_duplicate_expired_and_future_requests() {
        let cache = ReplayCache::default();
        let p = proof(Uuid::new_v4(), 30);
        assert!(cache.admit(&p, 9).is_err());
        assert!(cache.admit(&p, 10).is_ok());
        assert!(cache.admit(&p, 11).is_err());
        assert!(cache.admit(&proof(Uuid::new_v4(), 30), 30).is_err());
    }
    #[test]
    fn replay_cache_never_evicts_live_requests_to_make_room() {
        let cache = ReplayCache::default();
        for _ in 0..4096 {
            assert!(cache.admit(&proof(Uuid::new_v4(), 30), 10).is_ok());
        }
        assert!(cache.admit(&proof(Uuid::new_v4(), 40), 10).is_err());
        assert!(cache.admit(&proof(Uuid::new_v4(), 40), 30).is_ok());
    }
}
