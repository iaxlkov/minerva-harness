use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    StreamableHttp {
        url: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerConfig {
    pub id: String,
    pub transport: McpTransport,
    #[serde(default)]
    pub enabled: bool,
}
/// Secret values are accepted only on writes and never echoed in views or Debug.
#[derive(Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCredentials {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}
impl std::fmt::Debug for McpCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpCredentials([REDACTED])")
    }
}
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerInput {
    #[serde(flatten)]
    pub config: McpServerConfig,
    /// Omission preserves existing credentials; an empty object clears them.
    pub credentials: Option<McpCredentials>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    #[serde(flatten)]
    pub config: McpServerConfig,
    pub state: String,
    pub tool_count: usize,
    pub has_credentials: bool,
}
