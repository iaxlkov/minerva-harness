//! Credential adapters injected by the trusted runtime bootstrap.
use std::{collections::HashMap, sync::Mutex};
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CredentialScope(String);
impl CredentialScope {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err("Invalid credential scope".into());
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Implementations must never log secrets. This interface is internal, never browser-facing.
pub trait CredentialStore: Send + Sync {
    fn get(&self, scope: &CredentialScope, name: &str) -> Result<Option<String>, String>;
    fn put(&self, scope: &CredentialScope, name: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, scope: &CredentialScope, name: &str) -> Result<(), String>;
    fn local_environment_fallback(&self) -> bool {
        false
    }
}
pub struct LocalKeyring;
impl LocalKeyring {
    fn entry(name: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new("OE Rig model providers", name)
            .map_err(|_| "Credential storage unavailable".into())
    }
}
impl CredentialStore for LocalKeyring {
    fn get(&self, _: &CredentialScope, name: &str) -> Result<Option<String>, String> {
        match Self::entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("Credential storage unavailable".into()),
        }
    }
    fn put(&self, _: &CredentialScope, name: &str, secret: &str) -> Result<(), String> {
        Self::entry(name)?
            .set_password(secret)
            .map_err(|_| "Credential storage unavailable".into())
    }
    fn delete(&self, _: &CredentialScope, name: &str) -> Result<(), String> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Credential storage unavailable".into()),
        }
    }
    fn local_environment_fallback(&self) -> bool {
        true
    }
}
/// Ephemeral scoped broker useful for tests and explicit session-only credentials.
#[derive(Default)]
pub struct MemoryCredentialStore(Mutex<HashMap<(CredentialScope, String), String>>);
impl CredentialStore for MemoryCredentialStore {
    fn get(&self, scope: &CredentialScope, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "Credential storage unavailable")?
            .get(&(scope.clone(), name.into()))
            .cloned())
    }
    fn put(&self, scope: &CredentialScope, name: &str, secret: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "Credential storage unavailable")?
            .insert((scope.clone(), name.into()), secret.into());
        Ok(())
    }
    fn delete(&self, scope: &CredentialScope, name: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "Credential storage unavailable")?
            .remove(&(scope.clone(), name.into()));
        Ok(())
    }
}
/// Production fail-closed adapter when no authenticated credential broker exists.
pub struct UnavailableCredentialStore;
impl CredentialStore for UnavailableCredentialStore {
    fn get(&self, _: &CredentialScope, _: &str) -> Result<Option<String>, String> {
        Err("Hosted credential broker unavailable".into())
    }
    fn put(&self, _: &CredentialScope, _: &str, _: &str) -> Result<(), String> {
        Err("Hosted credential broker unavailable".into())
    }
    fn delete(&self, _: &CredentialScope, _: &str) -> Result<(), String> {
        Err("Hosted credential broker unavailable".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_provider_scopes_are_independent() {
        let s = MemoryCredentialStore::default();
        let a = CredentialScope::new("tenant-a/workspace").unwrap();
        let b = CredentialScope::new("tenant-b/workspace").unwrap();
        s.put(&a, "openai", "alice").unwrap();
        s.put(&b, "openai", "bob").unwrap();
        assert_eq!(s.get(&a, "openai").unwrap().as_deref(), Some("alice"));
        s.delete(&a, "openai").unwrap();
        assert!(s.get(&a, "openai").unwrap().is_none());
        assert_eq!(s.get(&b, "openai").unwrap().as_deref(), Some("bob"));
    }
}
