use uuid::Uuid;
/// Broadcast notifications are an optimization; DB revalidation is mandatory on every delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevocationScope {
    Session(Uuid),
    Membership {
        user_id: Uuid,
        tenant_id: Uuid,
        workspace_id: Uuid,
    },
    User(Uuid),
}
impl RevocationScope {
    pub fn affects(&self, grant: &crate::policy::AuthorizedWorkspace) -> bool {
        match self {
            Self::Session(id) => *id == grant.session_id(),
            Self::User(id) => *id == grant.user_id(),
            Self::Membership {
                user_id,
                tenant_id,
                workspace_id,
            } => {
                *user_id == grant.user_id()
                    && *tenant_id == grant.resource().tenant_id
                    && *workspace_id == grant.resource().workspace_id
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Principal;
    use crate::policy::{authorize, Action, Membership, Resource, Role};
    #[test]
    fn revocation_targets_only_matching_user_workspace_or_session() {
        let p = Principal::authenticated(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let r = Resource {
            tenant_id: p.tenant_id(),
            workspace_id: Uuid::new_v4(),
        };
        let m = Membership {
            user_id: p.user_id(),
            tenant_id: r.tenant_id,
            workspace_id: r.workspace_id,
            role: Role::Viewer,
            revision: 4,
        };
        let grant = authorize(&p, Action::Read, Some(&r), Some(&m)).unwrap();
        assert!(RevocationScope::Session(p.session_id()).affects(&grant));
        assert!(!RevocationScope::Session(Uuid::new_v4()).affects(&grant));
        assert!(RevocationScope::User(p.user_id()).affects(&grant));
        assert!(!RevocationScope::User(Uuid::new_v4()).affects(&grant));
        assert!(RevocationScope::Membership {
            user_id: p.user_id(),
            tenant_id: r.tenant_id,
            workspace_id: r.workspace_id
        }
        .affects(&grant));
        assert!(!RevocationScope::Membership {
            user_id: p.user_id(),
            tenant_id: r.tenant_id,
            workspace_id: Uuid::new_v4()
        }
        .affects(&grant));
    }
}
