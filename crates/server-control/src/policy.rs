#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn foreign_and_missing_resources_are_indistinguishable() {
        let user = Uuid::new_v4();
        let tenant = Uuid::new_v4();
        let workspace = Uuid::new_v4();
        let principal = Principal::authenticated(user, tenant, Uuid::new_v4());
        let membership = Membership {
            user_id: user,
            tenant_id: tenant,
            workspace_id: workspace,
            role: Role::Owner,
            revision: 1,
        };
        let foreign = Resource {
            tenant_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
        };
        assert_eq!(
            authorize(&principal, Action::Read, Some(&foreign), Some(&membership)),
            Err(Denied)
        );
        assert_eq!(
            authorize(&principal, Action::Read, None, Some(&membership)),
            Err(Denied)
        );
    }

    #[test]
    fn role_matrix_is_explicit_and_lists_are_scoped() {
        let principal = Principal::authenticated(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let resource = Resource {
            tenant_id: principal.tenant_id(),
            workspace_id: Uuid::new_v4(),
        };
        for role in [Role::Owner, Role::Editor, Role::Viewer] {
            let m = Membership {
                user_id: principal.user_id(),
                tenant_id: resource.tenant_id,
                workspace_id: resource.workspace_id,
                role,
                revision: 1,
            };
            for action in [
                Action::List,
                Action::Read,
                Action::Start,
                Action::Cancel,
                Action::Configure,
                Action::ManagePlugins,
                Action::WriteFiles,
                Action::ManageMembers,
                Action::ReadSecrets,
            ] {
                let expected = match action {
                    Action::List | Action::Read => true,
                    Action::Start | Action::Cancel | Action::WriteFiles => role != Role::Viewer,
                    _ => role == Role::Owner,
                };
                assert_eq!(
                    authorize(&principal, action, Some(&resource), Some(&m)).is_ok(),
                    expected,
                    "{role:?} {action:?}"
                );
            }
            let other = Resource {
                tenant_id: resource.tenant_id,
                workspace_id: Uuid::new_v4(),
            };
            assert_eq!(
                filter_list(&principal, &[m], vec![resource, other]),
                vec![resource]
            );
        }
    }
}
use crate::identity::Principal;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}
impl Role {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "editor" => Some(Self::Editor),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    List,
    Read,
    Start,
    Cancel,
    Configure,
    ManagePlugins,
    WriteFiles,
    ManageMembers,
    ReadSecrets,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource {
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
}
/// Trusted database membership, never populated from a browser DTO.
#[derive(Clone, Copy, Debug)]
pub struct Membership {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
    pub role: Role,
    pub revision: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("resource not found")]
pub struct Denied;
/// Proof of authorization for exactly one action and workspace at a membership revision.
/// Revalidate before dispatch and each event delivery; this is not a permanent capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedWorkspace {
    resource: Resource,
    user_id: Uuid,
    session_id: Uuid,
    role: Role,
    revision: i64,
    action: Action,
}
impl AuthorizedWorkspace {
    pub fn resource(&self) -> Resource {
        self.resource
    }
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }
    pub fn membership_revision(&self) -> i64 {
        self.revision
    }
    pub fn action(&self) -> Action {
        self.action
    }
    pub fn role(&self) -> Role {
        self.role
    }
}
pub fn authorize(
    principal: &Principal,
    action: Action,
    resource: Option<&Resource>,
    membership: Option<&Membership>,
) -> Result<AuthorizedWorkspace, Denied> {
    let r = resource.ok_or(Denied)?;
    let m = membership.ok_or(Denied)?;
    if principal.user_id() != m.user_id
        || principal.tenant_id() != r.tenant_id
        || m.tenant_id != r.tenant_id
        || m.workspace_id != r.workspace_id
    {
        return Err(Denied);
    }
    let allowed = match action {
        Action::List | Action::Read => true,
        Action::Start | Action::Cancel | Action::WriteFiles => m.role != Role::Viewer,
        Action::Configure | Action::ManagePlugins | Action::ManageMembers | Action::ReadSecrets => {
            m.role == Role::Owner
        }
    };
    if !allowed {
        return Err(Denied);
    }
    Ok(AuthorizedWorkspace {
        resource: *r,
        user_id: m.user_id,
        session_id: principal.session_id(),
        role: m.role,
        revision: m.revision,
        action,
    })
}
pub fn filter_list(
    principal: &Principal,
    memberships: &[Membership],
    resources: Vec<Resource>,
) -> Vec<Resource> {
    resources
        .into_iter()
        .filter(|r| {
            memberships
                .iter()
                .any(|m| authorize(principal, Action::List, Some(r), Some(m)).is_ok())
        })
        .collect()
}
