pub mod plugins;
pub mod optional;
pub mod harness;
pub mod engine;
pub use mh_runtime_support::{credential_store, network_policy};
pub use mh_sessions::{sessions, transcript};
pub use mh_context as context;
pub use mh_mcp as mcp;
pub use mh_providers as providers;
pub use mh_shell as shell;
pub use mh_workspaces as workspaces;
pub mod lsp { pub use mh_lsp::*; pub use mh_lsp_rust::*; }
pub mod tools { pub use mh_tool_runtime::*; pub use mh_workspace_tools::{register_native, save_attachment}; }
pub struct AgentLoopMarker;
impl cordis::Service for AgentLoopMarker { const NAME: &'static str = "studio.agent-loop.marker"; }

pub use mh_editor as editor;
