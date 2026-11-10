//! Prepared service references; built-in Cordis plugins own activation and cleanup.
use crate::{
    context::ContextService,
    tools::{InteractionBroker, ToolRegistry},
};
use std::{path::Path, sync::Arc};

pub struct Harness {
    pub context: Arc<ContextService>,
    pub tools: Arc<ToolRegistry>,
    pub interactions: Arc<InteractionBroker>,
    pub local: bool,
    pub mcp: Arc<crate::mcp::McpManager>,
}

pub use mh_providers::AgentRunContext;

pub struct RunCleanup {
    pub cancellation: crate::tools::Cancellation,
    pub interactions: Arc<InteractionBroker>,
    pub session: String,
    pub run: String,
}
impl Drop for RunCleanup {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.interactions.cancel_run(&self.session, &self.run);
    }
}

impl Harness {
    pub(crate) fn prepare(path: &Path, local: bool) -> Result<Self, String> {
        let context = Arc::new(ContextService::open(path)?);
        let tools = Arc::new(ToolRegistry::default());
        use sha2::{Digest, Sha256};
        let scope = crate::credential_store::CredentialScope::new(format!(
            "mcp-{:x}",
            Sha256::digest(path.to_string_lossy().as_bytes())
        ))?;
        let mcp = crate::mcp::McpManager::new(
            path.with_file_name("mcp.json"),
            scope,
            if local {
                Arc::new(crate::credential_store::LocalKeyring)
            } else {
                Arc::new(crate::credential_store::UnavailableCredentialStore)
            },
            tools.clone(),
            if local {
                crate::network_policy::EndpointPolicy::Local
            } else {
                crate::network_policy::EndpointPolicy::Hosted
            },
        )?;
        Ok(Self {
            context,
            tools,
            interactions: Arc::new(InteractionBroker::default()),
            local,
            mcp,
        })
    }
}
