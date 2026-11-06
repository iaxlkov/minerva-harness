use crate::{
    audit::{AuditEvent, AuditKind},
    identity::{PersonalWorkspace, Principal, VerifiedIdentity},
    membership::RevocationScope,
    policy::{authorize, Action, AuthorizedWorkspace, Denied, Membership, Resource, Role},
    session::{SessionError, SessionToken},
};
use sha2::{Digest, Sha256};
use tokio_postgres::Client;
use uuid::Uuid;
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("control database unavailable")]
    Database(#[from] tokio_postgres::Error),
    #[error("access denied")]
    Denied,
    #[error("invalid request")]
    Invalid,
}
impl From<SessionError> for ControlError {
    fn from(_: SessionError) -> Self {
        Self::Invalid
    }
}
/// Caller owns connection TLS and pooling. This store never opens a connection or logs SQL values.
pub struct ControlStore {
    client: Client,
    revocations: tokio::sync::broadcast::Sender<RevocationScope>,
}
impl ControlStore {
    pub fn new(client: Client) -> Self {
        let (revocations, _) = tokio::sync::broadcast::channel(256);
        Self {
            client,
            revocations,
        }
    }
    pub fn into_client(self) -> Client {
        self.client
    }
    pub fn subscribe_revocations(&self) -> tokio::sync::broadcast::Receiver<RevocationScope> {
        self.revocations.subscribe()
    }
    pub async fn migrate(&mut self) -> Result<(), ControlError> {
        let tx = self.client.transaction().await?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(739418991); CREATE TABLE IF NOT EXISTS minerva_control_migrations(version integer PRIMARY KEY)").await?;
        if tx
            .query_opt(
                "SELECT version FROM minerva_control_migrations WHERE version=1",
                &[],
            )
            .await?
            .is_none()
        {
            tx.batch_execute(include_str!("../migrations/001_identity.sql"))
                .await?;
            tx.execute(
                "INSERT INTO minerva_control_migrations(version) VALUES(1)",
                &[],
            )
            .await?;
        }
        if tx
            .query_opt(
                "SELECT version FROM minerva_control_migrations WHERE version=2",
                &[],
            )
            .await?
            .is_none()
        {
            tx.batch_execute(include_str!("../migrations/002_credentials.sql"))
                .await?;
            tx.execute(
                "INSERT INTO minerva_control_migrations(version) VALUES(2)",
                &[],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    /// Existing identities may log in even when enrollment is disabled. Identity must be OIDC-verified.
    pub async fn provision_personal(
        &mut self,
        identity: &VerifiedIdentity,
        allow_new: bool,
    ) -> Result<PersonalWorkspace, ControlError> {
        let tx = self.client.transaction().await?;
        // Serialize provisioning for this identity, including the absent-user case.
        let lock = serde_json::to_string(&(&identity.issuer, &identity.subject))
            .map_err(|_| ControlError::Invalid)?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&lock],
        )
        .await?;
        let existing = tx
            .query_opt(
                "SELECT id FROM users WHERE issuer=$1 AND subject=$2 FOR UPDATE",
                &[&identity.issuer, &identity.subject],
            )
            .await?;
        let user = match existing {
            Some(row) => row.get(0),
            None => {
                if !allow_new {
                    return Err(ControlError::Denied);
                }
                let user = Uuid::new_v4();
                tx.execute(
                    "INSERT INTO users(id,issuer,subject) VALUES($1,$2,$3)",
                    &[&user, &identity.issuer, &identity.subject],
                )
                .await?;
                user
            }
        };
        if let Some(row) = tx
            .query_opt(
                "SELECT tenant_id,workspace_id FROM personal_workspaces WHERE user_id=$1",
                &[&user],
            )
            .await?
        {
            let result = PersonalWorkspace {
                user_id: user,
                tenant_id: row.get(0),
                workspace_id: row.get(1),
            };
            tx.commit().await?;
            return Ok(result);
        }
        let tenant = Uuid::new_v4();
        let workspace = Uuid::new_v4();
        tx.execute("INSERT INTO tenants(id) VALUES($1)", &[&tenant])
            .await?;
        tx.execute(
            "INSERT INTO workspaces(id,tenant_id) VALUES($1,$2)",
            &[&workspace, &tenant],
        )
        .await?;
        tx.execute("INSERT INTO workspace_memberships(user_id,tenant_id,workspace_id,role) VALUES($1,$2,$3,'owner')", &[&user,&tenant,&workspace]).await?;
        tx.execute(
            "INSERT INTO personal_workspaces(user_id,tenant_id,workspace_id) VALUES($1,$2,$3)",
            &[&user, &tenant, &workspace],
        )
        .await?;
        tx.commit().await?;
        Ok(PersonalWorkspace {
            user_id: user,
            tenant_id: tenant,
            workspace_id: workspace,
        })
    }
    pub async fn create_session(
        &mut self,
        personal: &PersonalWorkspace,
        lifetime_seconds: u32,
    ) -> Result<(SessionToken, SessionToken), ControlError> {
        if lifetime_seconds == 0 || lifetime_seconds > 86400 {
            return Err(ControlError::Invalid);
        }
        let token = SessionToken::generate();
        let csrf = SessionToken::generate();
        let id = Uuid::new_v4();
        let inserted = self.client.execute("INSERT INTO auth_sessions(token_hash,id,user_id,tenant_id,csrf_hash,expires_at) SELECT $1,$2,$3,$4,$5,now()+($6::double precision * interval '1 second') WHERE EXISTS (SELECT 1 FROM workspace_memberships WHERE user_id=$3 AND tenant_id=$4 AND workspace_id=$7)", &[&token.hash(),&id,&personal.user_id,&personal.tenant_id,&csrf.hash(),&(lifetime_seconds as f64),&personal.workspace_id]).await?;
        if inserted != 1 {
            return Err(ControlError::Denied);
        }
        Ok((token, csrf))
    }
    pub async fn authenticate(&self, token: &SessionToken) -> Result<Principal, ControlError> {
        let row=self.client.query_opt("SELECT user_id,tenant_id,id FROM auth_sessions WHERE token_hash=$1 AND revoked_at IS NULL AND expires_at>now()", &[&token.hash()]).await?.ok_or(ControlError::Denied)?;
        Ok(Principal::authenticated(row.get(0), row.get(1), row.get(2)))
    }
    pub async fn check_csrf(
        &self,
        principal: &Principal,
        expected_origin: &str,
        origin: Option<&str>,
        token: &SessionToken,
    ) -> Result<(), ControlError> {
        if origin != Some(expected_origin) {
            return Err(ControlError::Denied);
        }
        let row=self.client.query_opt("SELECT 1 FROM auth_sessions WHERE id=$1 AND user_id=$2 AND tenant_id=$3 AND csrf_hash=$4 AND revoked_at IS NULL AND expires_at>now()", &[&principal.session_id(),&principal.user_id(),&principal.tenant_id(),&token.hash()]).await?;
        if row.is_none() {
            return Err(ControlError::Denied);
        }
        Ok(())
    }
    pub async fn authorize(
        &self,
        principal: &Principal,
        action: Action,
        resource: Option<Resource>,
    ) -> Result<AuthorizedWorkspace, ControlError> {
        let resource = resource.ok_or(ControlError::Denied)?;
        let row=self.client.query_opt("SELECT m.role,m.revision FROM workspace_memberships m JOIN auth_sessions s ON s.user_id=m.user_id AND s.tenant_id=m.tenant_id WHERE s.id=$1 AND s.revoked_at IS NULL AND s.expires_at>now() AND m.user_id=$2 AND m.tenant_id=$3 AND m.workspace_id=$4", &[&principal.session_id(),&principal.user_id(),&resource.tenant_id,&resource.workspace_id]).await?.ok_or(ControlError::Denied)?;
        let role: String = row.get(0);
        let m = Membership {
            user_id: principal.user_id(),
            tenant_id: resource.tenant_id,
            workspace_id: resource.workspace_id,
            role: Role::parse(&role).ok_or(ControlError::Denied)?,
            revision: row.get(1),
        };
        authorize(principal, action, Some(&resource), Some(&m))
            .map_err(|Denied| ControlError::Denied)
    }
    pub async fn revalidate(&self, grant: &AuthorizedWorkspace) -> Result<(), ControlError> {
        let principal = Principal::authenticated(
            grant.user_id(),
            grant.resource().tenant_id,
            grant.session_id(),
        );
        let fresh = self
            .authorize(&principal, grant.action(), Some(grant.resource()))
            .await?;
        if fresh.membership_revision() != grant.membership_revision() {
            return Err(ControlError::Denied);
        }
        Ok(())
    }
    pub async fn list_workspaces(
        &self,
        principal: &Principal,
    ) -> Result<Vec<Resource>, ControlError> {
        let rows=self.client.query("SELECT m.tenant_id,m.workspace_id FROM workspace_memberships m JOIN auth_sessions s ON s.user_id=m.user_id AND s.tenant_id=m.tenant_id WHERE s.id=$1 AND m.user_id=$2 AND m.tenant_id=$3 AND s.revoked_at IS NULL AND s.expires_at>now()", &[&principal.session_id(),&principal.user_id(),&principal.tenant_id()]).await?;
        Ok(rows
            .into_iter()
            .map(|r| Resource {
                tenant_id: r.get(0),
                workspace_id: r.get(1),
            })
            .collect())
    }
    pub async fn revoke_session(&mut self, principal: &Principal) -> Result<(), ControlError> {
        self.client
            .execute(
                "UPDATE auth_sessions SET revoked_at=now() WHERE id=$1 AND user_id=$2",
                &[&principal.session_id(), &principal.user_id()],
            )
            .await?;
        let _ = self
            .revocations
            .send(RevocationScope::Session(principal.session_id()));
        self.audit(&AuditEvent::new(
            AuditKind::SessionRevoked,
            Some(principal.user_id()),
            None,
            serde_json::json!({}),
        ))
        .await
    }
    /// Owner-only operation. Never accepts an unverified browser-supplied authorization proof.
    pub async fn revoke_membership(
        &mut self,
        owner: &AuthorizedWorkspace,
        user: Uuid,
    ) -> Result<(), ControlError> {
        if owner.action() != Action::ManageMembers {
            return Err(ControlError::Denied);
        }
        let r = owner.resource();
        let tx = self.client.transaction().await?;
        // Hold both membership and session locks until the mutation and its audit are committed.
        let row=tx.query_opt("SELECT m.revision FROM workspace_memberships m JOIN auth_sessions s ON s.user_id=m.user_id AND s.tenant_id=m.tenant_id WHERE s.id=$1 AND s.revoked_at IS NULL AND s.expires_at>now() AND m.user_id=$2 AND m.tenant_id=$3 AND m.workspace_id=$4 AND m.role='owner' FOR UPDATE OF m,s", &[&owner.session_id(),&owner.user_id(),&r.tenant_id,&r.workspace_id]).await?.ok_or(ControlError::Denied)?;
        if row.get::<_, i64>(0) != owner.membership_revision() {
            return Err(ControlError::Denied);
        }
        tx.execute("DELETE FROM workspace_memberships WHERE user_id=$1 AND tenant_id=$2 AND workspace_id=$3", &[&user,&r.tenant_id,&r.workspace_id]).await?;
        let event = serde_json::to_value(AuditEvent::new(
            AuditKind::MembershipChanged,
            Some(owner.user_id()),
            Some(r.workspace_id),
            serde_json::json!({"action":"memberships.update"}),
        ))
        .map_err(|_| ControlError::Invalid)?;
        tx.execute("INSERT INTO control_audit(event) VALUES($1)", &[&event])
            .await?;
        tx.commit().await?;
        let _ = self.revocations.send(RevocationScope::Membership {
            user_id: user,
            tenant_id: r.tenant_id,
            workspace_id: r.workspace_id,
        });
        Ok(())
    }
    pub async fn audit(&self, event: &AuditEvent) -> Result<(), ControlError> {
        let event = serde_json::to_value(event).map_err(|_| ControlError::Invalid)?;
        self.client
            .execute("INSERT INTO control_audit(event) VALUES($1)", &[&event])
            .await?;
        Ok(())
    }
    pub(crate) async fn save_pending(
        &self,
        state: &str,
        browser: &SessionToken,
        issuer: &str,
        nonce: &str,
        verifier: &str,
        path: &str,
    ) -> Result<(), ControlError> {
        let state_hash = Sha256::digest(state.as_bytes()).to_vec();
        self.client.execute("INSERT INTO oidc_pending(state_hash,browser_hash,issuer,nonce,pkce_verifier,return_path,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+interval '10 minutes')", &[&state_hash,&browser.hash(),&issuer,&nonce,&verifier,&path]).await?;
        Ok(())
    }
    pub(crate) async fn consume_pending(
        &self,
        state: &str,
        browser: &SessionToken,
        issuer: &str,
    ) -> Result<(String, String, String), ControlError> {
        let state_hash = Sha256::digest(state.as_bytes()).to_vec();
        let row=self.client.query_opt("DELETE FROM oidc_pending WHERE state_hash=$1 AND browser_hash=$2 AND issuer=$3 AND expires_at>now() RETURNING nonce,pkce_verifier,return_path", &[&state_hash,&browser.hash(),&issuer]).await?.ok_or(ControlError::Denied)?;
        Ok((row.get(0), row.get(1), row.get(2)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires MINERVA_TEST_POSTGRES_URL pointing to a disposable PostgreSQL database"]
    async fn persistent_identity_sessions_replay_and_revocation() {
        let (client, connection) = tokio_postgres::connect(
            &std::env::var("MINERVA_TEST_POSTGRES_URL").unwrap(),
            tokio_postgres::NoTls,
        )
        .await
        .unwrap();
        tokio::spawn(async move {
            connection.await.unwrap();
        });
        let mut store = ControlStore::new(client);
        store.migrate().await.unwrap();
        let suffix = Uuid::new_v4().to_string();
        let alice = VerifiedIdentity {
            issuer: "https://fixture.test".into(),
            subject: format!("alice-{suffix}"),
        };
        let bob = VerifiedIdentity {
            issuer: "https://fixture.test".into(),
            subject: format!("bob-{suffix}"),
        };
        let a = store.provision_personal(&alice, true).await.unwrap();
        assert_eq!(a, store.provision_personal(&alice, false).await.unwrap());
        let b = store.provision_personal(&bob, true).await.unwrap();
        assert_ne!(a.tenant_id, b.tenant_id);
        let (token, csrf) = store.create_session(&a, 300).await.unwrap();
        let p = store.authenticate(&token).await.unwrap();
        let foreign = Resource {
            tenant_id: b.tenant_id,
            workspace_id: b.workspace_id,
        };
        assert!(store
            .authorize(&p, Action::Read, Some(foreign))
            .await
            .is_err());
        assert_eq!(
            store.list_workspaces(&p).await.unwrap(),
            vec![Resource {
                tenant_id: a.tenant_id,
                workspace_id: a.workspace_id
            }]
        );
        assert!(store
            .check_csrf(
                &p,
                "https://gateway.test",
                Some("https://gateway.test"),
                &csrf
            )
            .await
            .is_ok());
        assert!(store
            .check_csrf(&p, "https://gateway.test", Some("https://evil.test"), &csrf)
            .await
            .is_err());
        let resource = Resource {
            tenant_id: a.tenant_id,
            workspace_id: a.workspace_id,
        };
        let grant = store
            .authorize(&p, Action::Read, Some(resource))
            .await
            .unwrap();
        let owner = store
            .authorize(&p, Action::ManageMembers, Some(resource))
            .await
            .unwrap();
        let browser = SessionToken::generate();
        let state = Uuid::new_v4().to_string();
        store
            .save_pending(
                &state,
                &browser,
                "https://fixture.test",
                "nonce",
                "verifier",
                "/",
            )
            .await
            .unwrap();
        assert!(store
            .consume_pending(&state, &SessionToken::generate(), "https://fixture.test")
            .await
            .is_err());
        assert!(store
            .consume_pending(&state, &browser, "https://fixture.test")
            .await
            .is_ok());
        assert!(store
            .consume_pending(&state, &browser, "https://fixture.test")
            .await
            .is_err());
        let mut notifications = store.subscribe_revocations();
        store.revoke_membership(&owner, a.user_id).await.unwrap();
        assert!(notifications.recv().await.unwrap().affects(&grant));
        assert!(store.revalidate(&grant).await.is_err());
        store.revoke_session(&p).await.unwrap();
        assert!(store.authenticate(&token).await.is_err());
    }
}
