//! Provider-neutral workspace, context and image attachment contracts.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSettings {
    pub cwd: Option<String>,
    pub permission_policy: crate::tools::PermissionPolicy,
    pub context_window: usize,
    pub auto_compact: bool,
}
impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            cwd: None,
            permission_policy: crate::tools::PermissionPolicy::ReadOnly,
            context_window: 32768,
            auto_compact: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudget {
    pub context_window: usize,
    pub estimated_tokens: usize,
    pub available_tokens: usize,
    pub should_compact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub usage: Option<TokenUsage>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ContextSummary {
    pub text: String,
    pub prefix_message_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRef {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub bytes: usize,
}

#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub bytes: usize,
    pub data: String,
}
// Never include image payloads in diagnostic logs.
impl std::fmt::Debug for AttachmentData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentData")
            .field("id", &self.id)
            .field("mime_type", &self.mime_type)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}
