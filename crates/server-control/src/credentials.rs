//! Control-plane-only encrypted credential service. No browser secret-read API.
//! Master keys must come from the control service secret mount, never worker env.
use crate::policy::{Action, AuthorizedWorkspace, Resource};
use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use std::collections::BTreeMap;
use tokio_postgres::Client;
#[derive(Debug, thiserror::Error)]
#[error("credential operation unavailable or denied")]
pub struct CredentialError;
/// No Debug/Serialize implementation: keys must never enter logs or wire payloads.
pub struct CredentialCipher {
    active: i32,
    keys: BTreeMap<i32, LessSafeKey>,
}
impl CredentialCipher {
    pub fn new(
        active: i32,
        keys: impl IntoIterator<Item = (i32, [u8; 32])>,
    ) -> Result<Self, CredentialError> {
        let mut keyring = BTreeMap::new();
        for (version, mut bytes) in keys {
            if version <= 0 || keyring.contains_key(&version) {
                bytes.fill(0);
                return Err(CredentialError);
            }
            let key = UnboundKey::new(&aead::AES_256_GCM, &bytes).map_err(|_| CredentialError);
            bytes.fill(0);
            keyring.insert(version, LessSafeKey::new(key?));
        }
        if !keyring.contains_key(&active) {
            return Err(CredentialError);
        }
        Ok(Self {
            active,
            keys: keyring,
        })
    }
    fn aad(
        scope: Resource,
        reference: &str,
        version: i64,
        key_version: i32,
    ) -> Result<Vec<u8>, CredentialError> {
        serde_json::to_vec(&(
            "minerva-credential-v1",
            scope.tenant_id.to_string(),
            scope.workspace_id.to_string(),
            reference,
            version,
            key_version,
        ))
        .map_err(|_| CredentialError)
    }
    fn seal(
        &self,
        scope: Resource,
        reference: &str,
        version: i64,
        secret: &str,
    ) -> Result<(i32, Vec<u8>, Vec<u8>), CredentialError> {
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| CredentialError)?;
        let mut ciphertext = secret.as_bytes().to_vec();
        self.keys
            .get(&self.active)
            .ok_or(CredentialError)?
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(Self::aad(scope, reference, version, self.active)?),
                &mut ciphertext,
            )
            .map_err(|_| CredentialError)?;
        Ok((self.active, nonce.to_vec(), ciphertext))
    }
    fn open(
        &self,
        scope: Resource,
        reference: &str,
        version: i64,
        key_version: i32,
        nonce: &[u8],
        mut ciphertext: Vec<u8>,
    ) -> Result<String, CredentialError> {
        let nonce: [u8; 12] = nonce.try_into().map_err(|_| CredentialError)?;
        let plain = self
            .keys
            .get(&key_version)
            .ok_or(CredentialError)?
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(Self::aad(scope, reference, version, key_version)?),
                &mut ciphertext,
            )
            .map_err(|_| CredentialError)?;
        String::from_utf8(plain.to_vec()).map_err(|_| CredentialError)
    }
}
pub struct CredentialService {
    client: Client,
    cipher: CredentialCipher,
}
impl CredentialService {
    pub fn new(client: Client, cipher: CredentialCipher) -> Self {
        Self { client, cipher }
    }
    fn scope(
        grant: &AuthorizedWorkspace,
        actions: &[Action],
        reference: &str,
    ) -> Result<Resource, CredentialError> {
        if !actions.contains(&grant.action())
            || reference.is_empty()
            || reference.len() > 256
            || reference.chars().any(char::is_control)
        {
            return Err(CredentialError);
        }
        Ok(grant.resource())
    }
    /// Caller obtains a fresh authorization grant from ControlStore for each operation.
    pub async fn put(
        &mut self,
        grant: &AuthorizedWorkspace,
        reference: &str,
        secret: &str,
    ) -> Result<(), CredentialError> {
        let scope = Self::scope(grant, &[Action::Configure], reference)?;
        if secret.is_empty() || secret.len() > 65536 {
            return Err(CredentialError);
        }
        let tx = self
            .client
            .transaction()
            .await
            .map_err(|_| CredentialError)?;
        // Serialize absent rows and updates alike so versions never race.
        let lock = serde_json::to_string(&(
            scope.tenant_id.to_string(),
            scope.workspace_id.to_string(),
            reference,
        ))
        .map_err(|_| CredentialError)?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&lock],
        )
        .await
        .map_err(|_| CredentialError)?;
        let old=tx.query_opt("SELECT version FROM workspace_credentials WHERE tenant_id=$1 AND workspace_id=$2 AND reference=$3",&[&scope.tenant_id,&scope.workspace_id,&reference]).await.map_err(|_|CredentialError)?;
        let version = old
            .map(|r| r.get::<_, i64>(0))
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(CredentialError)?;
        let (key_version, nonce, ciphertext) =
            self.cipher.seal(scope, reference, version, secret)?;
        tx.execute("INSERT INTO workspace_credentials(tenant_id,workspace_id,reference,version,key_version,nonce,ciphertext) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(tenant_id,workspace_id,reference) DO UPDATE SET version=$4,key_version=$5,nonce=$6,ciphertext=$7",&[&scope.tenant_id,&scope.workspace_id,&reference,&version,&key_version,&nonce,&ciphertext]).await.map_err(|_|CredentialError)?;
        tx.commit().await.map_err(|_| CredentialError)?;
        Ok(())
    }
    pub async fn has(
        &self,
        grant: &AuthorizedWorkspace,
        reference: &str,
    ) -> Result<bool, CredentialError> {
        let scope = Self::scope(
            grant,
            &[
                Action::Read,
                Action::Configure,
                Action::ReadSecrets,
                Action::Start,
            ],
            reference,
        )?;
        Ok(self.client.query_opt("SELECT 1 FROM workspace_credentials WHERE tenant_id=$1 AND workspace_id=$2 AND reference=$3",&[&scope.tenant_id,&scope.workspace_id,&reference]).await.map_err(|_|CredentialError)?.is_some())
    }
    pub async fn delete(
        &self,
        grant: &AuthorizedWorkspace,
        reference: &str,
    ) -> Result<(), CredentialError> {
        let scope = Self::scope(grant, &[Action::Configure], reference)?;
        self.client.execute("DELETE FROM workspace_credentials WHERE tenant_id=$1 AND workspace_id=$2 AND reference=$3",&[&scope.tenant_id,&scope.workspace_id,&reference]).await.map_err(|_|CredentialError)?;
        Ok(())
    }
    /// Internal adapter/broker use only. Never register a browser endpoint for resolve.
    pub async fn resolve(
        &self,
        grant: &AuthorizedWorkspace,
        reference: &str,
    ) -> Result<Option<String>, CredentialError> {
        let scope = Self::scope(grant, &[Action::ReadSecrets, Action::Start], reference)?;
        let row=self.client.query_opt("SELECT version,key_version,nonce,ciphertext FROM workspace_credentials WHERE tenant_id=$1 AND workspace_id=$2 AND reference=$3",&[&scope.tenant_id,&scope.workspace_id,&reference]).await.map_err(|_|CredentialError)?;
        row.map(|r| {
            self.cipher.open(
                scope,
                reference,
                r.get(0),
                r.get(1),
                r.get::<_, Vec<u8>>(2).as_slice(),
                r.get(3),
            )
        })
        .transpose()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aad_binds_scope_reference_version_and_key_version() {
        let cipher = CredentialCipher::new(1, [(1, [42; 32])]).unwrap();
        let scope = Resource {
            tenant_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
        };
        let (k, n, c) = cipher.seal(scope, "openai", 1, "secret").unwrap();
        assert_eq!(
            cipher.open(scope, "openai", 1, k, &n, c.clone()).unwrap(),
            "secret"
        );
        assert!(cipher
            .open(
                Resource {
                    workspace_id: uuid::Uuid::new_v4(),
                    ..scope
                },
                "openai",
                1,
                k,
                &n,
                c.clone()
            )
            .is_err());
        assert!(cipher
            .open(
                Resource {
                    tenant_id: uuid::Uuid::new_v4(),
                    ..scope
                },
                "openai",
                1,
                k,
                &n,
                c.clone()
            )
            .is_err());
        assert!(cipher.open(scope, "other", 1, k, &n, c.clone()).is_err());
        assert!(cipher.open(scope, "openai", 2, k, &n, c.clone()).is_err());
        let restored = CredentialCipher::new(2, [(1, [42; 32]), (2, [43; 32])]).unwrap();
        assert_eq!(
            restored.open(scope, "openai", 1, k, &n, c.clone()).unwrap(),
            "secret"
        );
        assert!(CredentialCipher::new(2, [(2, [43; 32])])
            .unwrap()
            .open(scope, "openai", 1, k, &n, c)
            .is_err());
    }
    #[tokio::test]
    #[ignore = "requires disposable PostgreSQL via MINERVA_CREDENTIAL_TEST_DATABASE_URL"]
    async fn postgres_two_scopes_crud_and_reference_substitution() {
        use crate::{
            identity::Principal,
            policy::{authorize, Membership, Role},
        };
        let (client, connection) = tokio_postgres::connect(
            &std::env::var("MINERVA_CREDENTIAL_TEST_DATABASE_URL")
                .expect("disposable database URL"),
            tokio_postgres::NoTls,
        )
        .await
        .unwrap();
        tokio::spawn(async move {
            connection.await.unwrap();
        });
        let schema = format!("credentials_test_{}", uuid::Uuid::new_v4().simple());
        client
            .batch_execute(&format!(
                "CREATE SCHEMA {schema}; SET search_path TO {schema}"
            ))
            .await
            .unwrap();
        client
            .batch_execute(include_str!("../migrations/001_identity.sql"))
            .await
            .unwrap();
        client
            .batch_execute(include_str!("../migrations/002_credentials.sql"))
            .await
            .unwrap();
        let tenant = uuid::Uuid::new_v4();
        let wa = uuid::Uuid::new_v4();
        let wb = uuid::Uuid::new_v4();
        let user = uuid::Uuid::new_v4();
        client
            .execute("INSERT INTO tenants(id) VALUES($1)", &[&tenant])
            .await
            .unwrap();
        for w in [wa, wb] {
            client
                .execute(
                    "INSERT INTO workspaces(id,tenant_id) VALUES($1,$2)",
                    &[&w, &tenant],
                )
                .await
                .unwrap();
        }
        let principal = Principal::authenticated(user, tenant, uuid::Uuid::new_v4());
        let grant = |workspace, action| {
            let membership = Membership {
                user_id: user,
                tenant_id: tenant,
                workspace_id: workspace,
                role: Role::Owner,
                revision: 1,
            };
            authorize(
                &principal,
                action,
                Some(&Resource {
                    tenant_id: tenant,
                    workspace_id: workspace,
                }),
                Some(&membership),
            )
            .unwrap()
        };
        let mut service =
            CredentialService::new(client, CredentialCipher::new(1, [(1, [7; 32])]).unwrap());
        service
            .put(&grant(wa, Action::Configure), "openai", "alice")
            .await
            .unwrap();
        service
            .put(&grant(wb, Action::Configure), "openai", "bob")
            .await
            .unwrap();
        assert_eq!(
            service
                .resolve(&grant(wa, Action::Start), "openai")
                .await
                .unwrap()
                .as_deref(),
            Some("alice")
        );
        assert_eq!(
            service
                .resolve(&grant(wb, Action::Start), "openai")
                .await
                .unwrap()
                .as_deref(),
            Some("bob")
        );
        service
            .put(&grant(wa, Action::Configure), "alice-only", "hidden")
            .await
            .unwrap();
        assert!(service
            .resolve(&grant(wb, Action::Start), "alice-only")
            .await
            .unwrap()
            .is_none());
        assert!(service
            .resolve(&grant(wa, Action::Read), "openai")
            .await
            .is_err());
        service
            .delete(&grant(wa, Action::Configure), "openai")
            .await
            .unwrap();
        assert!(!service
            .has(&grant(wa, Action::Read), "openai")
            .await
            .unwrap());
        assert!(service
            .has(&grant(wb, Action::Read), "openai")
            .await
            .unwrap());
        service
            .client
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
    }
}
