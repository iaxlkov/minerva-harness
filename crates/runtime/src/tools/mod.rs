//! Scoped admission, policy, cancellation and canonical tool validation.
//! DSH contracts/pipeline/executor mapped to owned Rust snapshots and futures;
//! approval/questions use per-call oneshots rather than Cordis waterfalls.
pub(crate) mod files;
mod interaction;
mod native;
mod process_tree;
pub use interaction::InteractionBroker;
pub use minerva_plugin_sdk::tools::*;
pub use native::register_native;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::watch;
pub type ToolFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
pub type ToolHandler = dyn Fn(ToolContext, Value) -> ToolFuture + Send + Sync;
#[derive(Clone)]
pub struct Cancellation {
    local: watch::Sender<bool>,
    parent: Option<Arc<Cancellation>>,
}
impl Default for Cancellation {
    fn default() -> Self {
        Self {
            local: watch::channel(false).0,
            parent: None,
        }
    }
}
impl Cancellation {
    fn child(parent: Cancellation) -> Self {
        Self {
            parent: Some(Arc::new(parent)),
            ..Self::default()
        }
    }
    pub fn cancel(&self) {
        self.local.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.local.borrow() || self.parent.as_ref().is_some_and(|p| p.is_cancelled())
    }
    pub fn cancelled(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let mut rx = self.local.subscribe();
            if self.is_cancelled() {
                return;
            }
            let own = async move {
                while rx.changed().await.is_ok() {
                    if *rx.borrow() {
                        return;
                    }
                }
            };
            let parent = async {
                match &self.parent {
                    Some(parent) => parent.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {_=own=>{},_=parent=>{}}
        })
    }
}
pub trait ToolEventSink: Send + Sync {
    fn emit(
        &self,
        context: &ToolContext,
        name: &str,
        args: &Value,
        status: &str,
        result: Option<&Result<Value, String>>,
    ) -> Result<(), String>;
}
#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub run_id: String,
    pub call_id: String,
    pub cwd: Option<PathBuf>,
    pub policy: PermissionPolicy,
    pub cancellation: Cancellation,
    pub interactions: Arc<InteractionBroker>,
    pub sink: Option<Arc<dyn ToolEventSink>>,
}
struct Registration {
    owner: String,
    definition: ToolDefinition,
    handler: Arc<ToolHandler>,
    active: AtomicBool,
    input: jsonschema::Validator,
    output: jsonschema::Validator,
    timeout: Duration,
}
#[derive(Default)]
pub struct ToolRegistry(Mutex<BTreeMap<String, Arc<Registration>>>);
#[derive(Clone, Default)]
pub struct AdmittedTools(BTreeMap<String, Arc<Registration>>, IdentityHook);
impl ToolRegistry {
    pub fn register<F>(
        &self,
        owner: &str,
        definition: ToolDefinition,
        handler: F,
    ) -> Result<(), String>
    where
        F: Fn(ToolContext, Value) -> ToolFuture + Send + Sync + 'static,
    {
        self.register_with_timeout(owner, definition, Duration::from_secs(120), handler)
    }
    pub fn register_with_timeout<F>(
        &self,
        owner: &str,
        definition: ToolDefinition,
        timeout: Duration,
        handler: F,
    ) -> Result<(), String>
    where
        F: Fn(ToolContext, Value) -> ToolFuture + Send + Sync + 'static,
    {
        if owner.is_empty()
            || definition.name.is_empty()
            || definition.name.len() > 128
            || !definition
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
        {
            return Err("invalid tool name or owner".into());
        }
        let input =
            jsonschema::validator_for(&definition.input_schema).map_err(|e| e.to_string())?;
        let output =
            jsonschema::validator_for(&definition.output_schema).map_err(|e| e.to_string())?;
        let mut tools = self.0.lock().map_err(|e| e.to_string())?;
        if tools.contains_key(&definition.name) {
            return Err("tool already registered".into());
        }
        tools.insert(
            definition.name.clone(),
            Arc::new(Registration {
                owner: owner.into(),
                definition,
                handler: Arc::new(handler),
                active: AtomicBool::new(true),
                input,
                output,
                timeout,
            }),
        );
        Ok(())
    }
    pub fn revoke_owner(&self, owner: &str) {
        if let Ok(mut tools) = self.0.lock() {
            tools.retain(|_, tool| {
                if tool.owner == owner {
                    tool.active.store(false, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            })
        }
    }
    pub fn snapshot(&self) -> AdmittedTools {
        {
            let tools = self.0.lock().expect("tool registry poisoned").clone();
            let hook = IdentityHook {
                names: tools.keys().cloned().collect(),
                pending: Arc::default(),
            };
            AdmittedTools(tools, hook)
        }
    }
}
impl AdmittedTools {
    pub fn identity_hook(&self) -> IdentityHook {
        self.1.clone()
    }
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.0.values().map(|r| r.definition.clone()).collect()
    }
    pub fn without_workspace(mut self) -> Self {
        self.0.retain(|_, t| t.owner != "native.workspace");
        self
    }
    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        mut ctx: ToolContext,
    ) -> Result<Value, String> {
        let caller_cancellation = ctx.cancellation.clone();
        ctx.cancellation = Cancellation::child(caller_cancellation.clone());
        let tool = self
            .0
            .get(name)
            .ok_or("tool was not admitted for this step")?;
        let emit = |status: &str, result: Option<&Result<Value, String>>| -> Result<(), String> {
            if let Some(sink) = &ctx.sink {
                sink.emit(&ctx, name, &args, status, result)?;
            }
            Ok(())
        };
        emit("pending", None)?;
        let outcome = async {
            if ctx.cancellation.is_cancelled() || caller_cancellation.is_cancelled() {
                return Err("tool cancelled".into());
            }
            if !tool.active.load(Ordering::SeqCst) {
                return Err("tool owner was revoked".into());
            }
            tool.input
                .validate(&args)
                .map_err(|e| format!("invalid tool input: {e}"))?;
            if tool.definition.side_effect {
                match ctx.policy {
                    PermissionPolicy::ReadOnly => {
                        return Err("read-only policy denies side effects".into())
                    }
                    PermissionPolicy::Ask => {
                        emit("requires-approval", None)?;
                        let answer = ctx
                            .interactions
                            .request(
                                &ctx,
                                "approval",
                                name,
                                &format!("Allow {name} for this call?"),
                                vec![],
                            )
                            .await?;
                        if answer != "allow" {
                            return Err("tool approval denied".into());
                        }
                    }
                    PermissionPolicy::Allow => {}
                }
            }
            if ctx.cancellation.is_cancelled() {
                return Err("tool cancelled".into());
            }
            if !tool.active.load(Ordering::SeqCst) {
                return Err("tool owner was revoked".into());
            }
            emit("running", None)?;
            let output = (tool.handler)(ctx.clone(), args.clone()).await?;
            tool.output
                .validate(&output)
                .map_err(|e| format!("invalid tool output: {e}"))?;
            if serde_json::to_vec(&output)
                .map_err(|e| e.to_string())?
                .len()
                > 1024 * 1024
            {
                return Err("tool output exceeds 1 MiB".into());
            }
            Ok(output)
        };
        // Native handlers settle after killing owned processes. In-process handlers
        // must cooperate; a broken handler cannot hold the session forever.
        tokio::pin!(outcome);
        let result = tokio::select! {
            result=&mut outcome=>result,
            _=caller_cancellation.cancelled()=>{
                ctx.cancellation.cancel();ctx.interactions.cancel_call(&ctx);
                let _=tokio::time::timeout(Duration::from_secs(5),&mut outcome).await;
                Err("tool cancelled".into())
            },
            _=tokio::time::sleep(tool.timeout)=>{
                ctx.cancellation.cancel();ctx.interactions.cancel_call(&ctx);
                let _=tokio::time::timeout(Duration::from_secs(5),&mut outcome).await;
                Err("tool timed out".into())
            }
        };
        emit(
            if ctx.cancellation.is_cancelled() {
                "cancelled"
            } else if result.is_ok() {
                "completed"
            } else {
                "failed"
            },
            Some(&result),
        )?;
        result
    }
    pub fn rig_tools(&self, ctx: ToolContext) -> Vec<rig_core::tool::DynamicTool> {
        self.0
            .values()
            .map(|tool| {
                let admitted = self.clone();
                let ctx = ctx.clone();
                let name = tool.definition.name.clone();
                rig_core::tool::DynamicTool::new_with_context(
                    rig_core::message::ToolName::new(name.clone()).expect("validated name"),
                    tool.definition.description.clone(),
                    tool.definition.input_schema.clone(),
                    move |rig_ctx: &mut rig_core::tool::ToolContext, args| {
                        let admitted = admitted.clone();
                        let name = name.clone();
                        let mut ctx = ctx.clone();
                        let identity = rig_ctx.get::<RigCallIdentity>();
                        let envelope = args
                            .get("__minerva_dispatch")
                            .and_then(Value::as_str)
                            .map(String::from);
                        Box::pin(async move {
                            let args = if let Some(token) = envelope {
                                let dispatch = admitted
                                    .1
                                    .pending
                                    .lock()
                                    .map_err(|_| {
                                        rig_core::tool::ToolExecutionError::other(
                                            "identity lock poisoned",
                                        )
                                    })?
                                    .remove(&token)
                                    .ok_or_else(|| {
                                        rig_core::tool::ToolExecutionError::invalid_args(
                                            "invalid host dispatch token",
                                        )
                                    })?;
                                if dispatch.name != name {
                                    return Err(rig_core::tool::ToolExecutionError::invalid_args(
                                        "dispatch tool mismatch",
                                    ));
                                }
                                ctx.call_id = dispatch.call_id;
                                dispatch.args
                            } else {
                                args
                            };
                            if let Some(identity) = identity.map_err(|e| {
                                rig_core::tool::ToolExecutionError::invalid_args(e.to_string())
                            })? {
                                ctx.call_id = identity.call_id;
                            }
                            if ctx.call_id.is_empty() {
                                return Err(rig_core::tool::ToolExecutionError::invalid_args(
                                    "missing exact tool call identity",
                                ));
                            }
                            admitted
                                .execute(&name, args, ctx)
                                .await
                                .map(rig_core::tool::ToolOutput::json)
                                .map_err(rig_core::tool::ToolExecutionError::other)
                        })
                    },
                )
            })
            .collect()
    }
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct RigCallIdentity {
    pub call_id: String,
}
impl rig_core::tool::ContextValue for RigCallIdentity {
    const KEY: &'static str = "minerva.call-identity";
}

#[derive(Clone, Default)]
pub struct IdentityHook {
    names: std::collections::BTreeSet<String>,
    pending: Arc<Mutex<BTreeMap<String, IdentityDispatch>>>,
}
struct IdentityDispatch {
    name: String,
    call_id: String,
    args: Value,
}
impl rig_agent::agent::hook::AgentHook for IdentityHook {
    async fn on_dispatch(
        &self,
        _: &rig_agent::agent::hook::HookContext,
        event: rig_agent::agent::hook::DispatchEvent<'_>,
    ) -> rig_agent::agent::hook::DispatchAction {
        use rig_agent::agent::hook::DispatchAction;
        let (Some(name), Some(args), Some(call)) =
            (event.tool_name(), event.tool_args(), event.call_id)
        else {
            return DispatchAction::proceed();
        };
        if !self.names.contains(name) {
            return DispatchAction::proceed();
        }
        let Ok(args) = serde_json::from_str(args) else {
            return DispatchAction::skip("invalid tool arguments");
        };
        let token = event.id.to_string();
        let Ok(mut pending) = self.pending.lock() else {
            return DispatchAction::stop("identity registry poisoned");
        };
        pending.insert(
            token.clone(),
            IdentityDispatch {
                name: name.into(),
                call_id: call.to_string(),
                args,
            },
        );
        DispatchAction::rewrite_tool_args(event.kind, json!({"__minerva_dispatch":token}))
    }
    async fn on_outcome(
        &self,
        _: &rig_agent::agent::hook::HookContext,
        event: rig_agent::agent::hook::OutcomeEvent<'_>,
    ) -> rig_agent::agent::hook::OutcomeAction {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&event.id.to_string());
        }
        rig_agent::agent::hook::OutcomeAction::proceed()
    }
}
