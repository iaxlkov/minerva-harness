#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_registration_requires_resolver_and_unknown_actions_are_denied() {
        let mut registry = DispatchRegistry::default();
        assert_eq!(
            registry.register("runs.cancel", Action::Cancel, None),
            Err(RegistrationError)
        );
        assert!(registry
            .resolve("unregistered", &serde_json::json!({}))
            .is_err());
    }
    #[test]
    fn all_resource_ids_must_resolve_to_same_workspace() {
        let a = Resource {
            tenant_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
        };
        let b = Resource {
            workspace_id: uuid::Uuid::new_v4(),
            ..a
        };
        assert_eq!(consistent_scope(&[Some(a), Some(b)]), Err(Denied));
        assert_eq!(consistent_scope(&[Some(a), None]), Err(Denied));
        assert_eq!(consistent_scope(&[]), Err(Denied));
        assert_eq!(consistent_scope(&[Some(a), Some(a)]), Ok(a));
    }
}
use crate::policy::{Action, Denied, Resource};
use std::{collections::HashMap, sync::Arc};
pub type ResourceResolver =
    Arc<dyn Fn(&serde_json::Value) -> Result<Resource, Denied> + Send + Sync>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("resource endpoint requires a unique resolver")]
pub struct RegistrationError;
#[derive(Default)]
pub struct DispatchRegistry {
    entries: HashMap<String, (Action, ResourceResolver)>,
}
impl DispatchRegistry {
    pub fn register(
        &mut self,
        method: &str,
        action: Action,
        resolver: Option<ResourceResolver>,
    ) -> Result<(), RegistrationError> {
        let resolver = resolver.ok_or(RegistrationError)?;
        if method.is_empty() || self.entries.contains_key(method) {
            return Err(RegistrationError);
        }
        self.entries.insert(method.into(), (action, resolver));
        Ok(())
    }
    /// The resolver must use trusted ownership lookup for EVERY object identifier in the request.
    pub fn resolve(
        &self,
        method: &str,
        request: &serde_json::Value,
    ) -> Result<(Action, Resource), Denied> {
        let (action, resolver) = self.entries.get(method).ok_or(Denied)?;
        Ok((*action, resolver(request)?))
    }
}
pub fn consistent_scope(resources: &[Option<Resource>]) -> Result<Resource, Denied> {
    let first = resources.first().and_then(|r| *r).ok_or(Denied)?;
    if resources.iter().any(|r| *r != Some(first)) {
        return Err(Denied);
    }
    Ok(first)
}
