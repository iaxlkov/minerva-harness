use crate::transcript::{ContentPart, Transcript};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct Turn {
    pub role: String,
    pub parts: Vec<ContentPart>,
    #[serde(default)]
    #[ts(type = "Record<string, JsonValue>")]
    pub replay: Map<String, Value>,
}
impl Turn {
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter(|p| p.kind == "text")
            .filter_map(|p| p.data["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn plain(role: &str, text: &str) -> Self {
        Self {
            role: role.into(),
            parts: vec![ContentPart::text("text", text)],
            replay: Map::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineModel {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineCatalog {
    pub models: Vec<EngineModel>,
    pub loading: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineSnapshot {
    pub status: String,
    pub transcript: Transcript,
    pub state: Option<Value>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<EngineInteraction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineInteraction {
    pub id: String,
    pub call_id: String,
    pub kind: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineStartContext {
    pub system_prompt: String,
    pub cwd: Option<String>,
    pub permission_policy: crate::tools::PermissionPolicy,
    pub tools: Vec<crate::tools::ToolDefinition>,
}
