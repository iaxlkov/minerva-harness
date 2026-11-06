use uuid::Uuid;
/// Only OIDC verification can construct an external identity.
/// Deliberately has neither Deserialize nor Debug (subjects can be personal data).
pub struct VerifiedIdentity {
    pub(crate) issuer: String,
    pub(crate) subject: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    user_id: Uuid,
    tenant_id: Uuid,
    session_id: Uuid,
}
impl Principal {
    pub(crate) fn authenticated(user_id: Uuid, tenant_id: Uuid, session_id: Uuid) -> Self {
        Self {
            user_id,
            tenant_id,
            session_id,
        }
    }
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }
    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonalWorkspace {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
}
