use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionPolicy {
    #[default]
    ReadOnly,
    Ask,
    Allow,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    #[serde(default)]
    pub side_effect: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PendingInteraction {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub call_id: String,
    pub kind: String,
    pub title: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub created_at: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionAnswer {
    pub session_id: String,
    pub id: String,
    pub decision: String,
    pub answer: Option<String>,
}
