//! Host-owned service composition shared by model routes and transports.
use crate::{
    context::ContextService,
    tools::{self, InteractionBroker, ToolRegistry},
};
use serde_json::json;
use std::{path::Path, sync::Arc};

pub struct Harness {
    pub context: Arc<ContextService>,
    pub tools: Arc<ToolRegistry>,
    pub interactions: Arc<InteractionBroker>,
    pub local: bool,
    pub mcp: Arc<crate::mcp::McpManager>,
}

pub struct AgentRunContext {
    pub session_id: String,
    pub run_id: String,
    pub tools: crate::tools::AdmittedTools,
    pub tool_context: crate::tools::ToolContext,
    pub context: Arc<ContextService>,
    pub messages: Vec<crate::sessions::SessionMessage>,
}

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
    pub fn open(path: &Path, local: bool) -> Result<Self, String> {
        let context = Arc::new(ContextService::open(path)?);
        let tools = Arc::new(ToolRegistry::default());
        if local {
            tools::register_native(&tools)?;
            let skills = context.clone();
            tools.register("native.workspace", tools::ToolDefinition {
                name: "load_skill".into(),
                description: "Load the instructions of an available workspace skill by name".into(),
                input_schema: json!({"type":"object","properties":{"name":{"type":"string","minLength":1}},"required":["name"],"additionalProperties":false}),
                output_schema: json!({"type":"object","properties":{"instructions":{"type":"string"}},"required":["instructions"]}),
                side_effect: false,
            }, move |ctx, args| {
                let skills = skills.clone();
                Box::pin(async move {
                    Ok(json!({"instructions":skills.load_skill(&ctx.session_id, args["name"].as_str().ok_or("missing skill name")?)?}))
                })
            })?;
        }
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
