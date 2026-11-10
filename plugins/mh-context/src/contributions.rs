//! Optional, bounded context providers. Registration guards revoke future snapshots.
use minerva_plugin_sdk::learning::{AccessContext, LearningError, Scope, SourceRef};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Contribution {
    pub provider_id: String,
    pub revision: String,
    pub scope: Scope,
    pub text: String,
    pub sources: Vec<SourceRef>,
}
pub type ContributionFuture =
    Pin<Box<dyn Future<Output = Result<Contribution, LearningError>> + Send>>;
pub trait SectionProvider: Send + Sync {
    /// Implementations must use local prepared state, not perform model/network calls.
    fn prepare(&self, access: AccessContext) -> ContributionFuture;
    fn prepare_for_query(&self, access: AccessContext, _query: String) -> ContributionFuture {
        self.prepare(access)
    }
}
#[derive(Default)]
struct State {
    generation: u64,
    providers: BTreeMap<String, (u64, Arc<dyn SectionProvider>)>,
}
#[derive(Clone, Default)]
pub struct ContributionRegistry(Arc<Mutex<State>>);
pub struct Registration {
    state: Weak<Mutex<State>>,
    id: String,
    generation: u64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            if let Ok(mut state) = state.lock() {
                if state
                    .providers
                    .get(&self.id)
                    .is_some_and(|(g, _)| *g == self.generation)
                {
                    state.providers.remove(&self.id);
                }
            }
        }
    }
}
#[derive(Default)]
pub struct ContributionSnapshot {
    pub sections: Vec<Contribution>,
    pub degraded: Vec<(String, String)>,
}
impl ContributionRegistry {
    pub fn register(
        &self,
        id: &str,
        provider: Arc<dyn SectionProvider>,
    ) -> Result<Registration, LearningError> {
        if id.is_empty() || id.len() > 100 {
            return Err(LearningError::new(
                "INVALID_INPUT",
                "invalid context provider id",
            ));
        }
        let mut state = self
            .0
            .lock()
            .map_err(|_| LearningError::new("STORAGE_ERROR", "context registry unavailable"))?;
        if state.providers.contains_key(id) || state.providers.len() >= 32 {
            return Err(LearningError::new(
                "INVALID_INPUT",
                "context provider duplicate or capacity reached",
            ));
        }
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or_else(|| LearningError::new("STORAGE_ERROR", "context generation exhausted"))?;
        let generation = state.generation;
        state.providers.insert(id.into(), (generation, provider));
        Ok(Registration {
            state: Arc::downgrade(&self.0),
            id: id.into(),
            generation,
        })
    }
    pub async fn snapshot(
        &self,
        access: &AccessContext,
        token_budget: usize,
    ) -> ContributionSnapshot {
        self.snapshot_for_query(access, "", token_budget).await
    }
    pub async fn snapshot_for_query(
        &self,
        access: &AccessContext,
        query: &str,
        token_budget: usize,
    ) -> ContributionSnapshot {
        let providers = match self.0.lock() {
            Ok(state) => state.providers.clone(),
            Err(_) => {
                return ContributionSnapshot {
                    sections: vec![],
                    degraded: vec![("registry".into(), "STORAGE_ERROR".into())],
                }
            }
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut snapshot = ContributionSnapshot::default();
        let mut bytes = 0usize;
        let mut tokens = 0usize;
        for (id, (generation, provider)) in providers {
            let section = match tokio::time::timeout_at(
                deadline,
                provider.prepare_for_query(access.clone(), query.chars().take(2048).collect()),
            )
            .await
            {
                Ok(Ok(section)) => section,
                Ok(Err(error)) => {
                    snapshot.degraded.push((id, error.code));
                    continue;
                }
                Err(_) => {
                    snapshot.degraded.push((id, "DEADLINE_EXCEEDED".into()));
                    continue;
                }
            };
            let still_active = self
                .0
                .lock()
                .is_ok_and(|s| s.providers.get(&id).is_some_and(|(g, _)| *g == generation));
            if !still_active {
                snapshot.degraded.push((id, "PLUGIN_UNAVAILABLE".into()));
                continue;
            }
            let section_tokens = super::estimate_tokens(&section.text);
            if section.provider_id != id
                || section.revision.is_empty()
                || section.text.len() > 32 * 1024
                || section.sources.len() > 128
                || section.sources.iter().any(|s| s.validate().is_err())
                || access.authorize(&section.scope).is_err()
                || bytes.saturating_add(section.text.len()) > 64 * 1024
                || tokens.saturating_add(section_tokens) > token_budget
            {
                snapshot.degraded.push((id, "INVALID_CONTRIBUTION".into()));
                continue;
            }
            bytes += section.text.len();
            tokens += section_tokens;
            if section.text.is_empty() {
                continue;
            }
            snapshot.sections.push(section);
        }
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Provider(Scope);
    impl SectionProvider for Provider {
        fn prepare(&self, _: AccessContext) -> ContributionFuture {
            let scope = self.0.clone();
            Box::pin(async move {
                Ok(Contribution {
                    provider_id: "memory".into(),
                    revision: "1".into(),
                    scope,
                    text: "fact".into(),
                    sources: vec![],
                })
            })
        }
    }
    #[tokio::test]
    async fn snapshot_rejects_other_scope_and_guard_revokes() {
        let registry = ContributionRegistry::default();
        let access = AccessContext {
            actor_id: "u".into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: true,
        };
        let guard = registry
            .register(
                "memory",
                Arc::new(Provider(Scope::Workspace {
                    workspace_id: "other".into(),
                })),
            )
            .unwrap();
        let snapshot = registry.snapshot(&access, 100).await;
        assert!(snapshot.sections.is_empty());
        assert_eq!(snapshot.degraded.len(), 1);
        drop(guard);
        assert!(registry.snapshot(&access, 100).await.degraded.is_empty());
        let _guard = registry
            .register("memory", Arc::new(Provider(Scope::User)))
            .unwrap();
        assert_eq!(registry.snapshot(&access, 100).await.sections.len(), 1);
    }
}
