//! Version one wire contracts. Native local plugins are trusted executable code.
pub mod context;
pub mod engine;
pub mod mcp;
pub mod tools;
pub mod transcript;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};
/// Built-in mandatory plugins share their protected namespace with installed packages.
pub const SYSTEM_PLUGIN_IDS: &[&str] = &[
    "plugin-manager",
    "message-queue",
    "providers",
    "shell",
    "tool-runtime",
    "context",
    "workspace-tools",
    "workspaces",
    "mh-editor",
    "mh-lsp",
    "mh-lsp-rust",
    "mcp",
    "sessions",
    "agent-loop",
];
/// Static declaration of a language server contributed by a trusted local plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LanguageServerContribution {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub extensions: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub api_version: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub requires: BTreeMap<String, u32>,
    #[serde(default)]
    pub provides: BTreeMap<String, u32>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub language_servers: Vec<LanguageServerContribution>,
    #[serde(default)]
    pub backend: Option<Backend>,
    #[serde(default)]
    pub ui: Option<Ui>,
    #[serde(default)]
    pub config_schema: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    pub kind: String,
    pub entry: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ui {
    pub entry: String,
    pub sdk_version: u32,
}
pub fn relative_entry(entry: &str) -> Result<(), String> {
    if entry.is_empty()
        || entry.contains('\\')
        || entry.contains(':')
        || !Path::new(entry)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("entry must be a contained relative path".into());
    }
    Ok(())
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        for (name, version) in self.requires.iter().chain(self.provides.iter()) {
            if name.is_empty()
                || name.len() > 100
                || *version == 0
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
            {
                return Err("invalid service name or version".into());
            }
        }
        if self.language_servers.len() > 16
            || self.language_servers.iter().any(|server| {
                server.id.is_empty()
                    || server.id.len() > 64
                    || !server
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                    || server.command.trim().is_empty()
                    || server.command.len() > 1024
                    || server.args.len() > 64
                    || server.args.iter().any(|arg| arg.len() > 4096)
                    || server.extensions.is_empty()
                    || server.extensions.len() > 64
                    || server.extensions.iter().any(|(ext, language)| {
                        ext.len() < 2
                            || ext.len() > 32
                            || !ext.starts_with('.')
                            || !ext[1..]
                                .bytes()
                                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                            || language.is_empty()
                            || language.len() > 64
                    })
            })
        {
            return Err("invalid language server contribution".into());
        }
        if self.language_servers.iter().enumerate().any(|(i, server)| {
            self.language_servers[..i].iter().any(|other| {
                other.id == server.id
                    || server
                        .extensions
                        .keys()
                        .any(|ext| other.extensions.contains_key(ext))
            })
        }) {
            return Err("duplicate language server id or extension".into());
        }
        if !self.language_servers.is_empty() && self.backend.is_none() {
            return Err("language server contribution requires a backend".into());
        }
        if !self.provides.is_empty() && self.backend.is_none() {
            return Err("service provider requires a backend".into());
        }
        if self.api_version != 1 {
            return Err("unsupported plugin apiVersion".into());
        }
        if self.id.is_empty()
            || self.id.len() > 100
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
            || self.id.starts_with('.')
            || self.id.ends_with('.')
        {
            return Err("invalid plugin id".into());
        }
        if self.id == "example" || SYSTEM_PLUGIN_IDS.contains(&self.id.as_str()) {
            return Err("plugin id is reserved for a builtin".into());
        }
        if self.version.is_empty() || self.version.len() > 100 {
            return Err("invalid plugin version".into());
        }
        if let Some(b) = &self.backend {
            if b.kind != "process" {
                return Err("unsupported backend kind".into());
            }
            relative_entry(&b.entry)?;
        }
        if let Some(u) = &self.ui {
            relative_entry(&u.entry)?;
            if u.sdk_version != 2 {
                return Err("unsupported UI SDK".into());
            }
        }
        if self.backend.is_none() && self.ui.is_none() {
            return Err("plugin has no entry".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod lsp_manifest_tests {
    use super::Manifest;
    use serde_json::json;

    fn manifest(servers: serde_json::Value) -> Manifest {
        serde_json::from_value(json!({"id":"example.language", "version":"1", "apiVersion":1, "backend":{"kind":"process","entry":"plugin.exe"}, "languageServers":servers})).unwrap()
    }

    #[test]
    fn language_contributions_validate_extensions_and_conflicts() {
        assert!(
            manifest(json!([{"id":"one", "command":"server", "extensions":{".rs":"rust"}}]))
                .validate()
                .is_ok()
        );
        assert!(
            manifest(json!([{"id":"one", "command":"server", "extensions":{".RS":"rust"}}]))
                .validate()
                .is_err()
        );
        assert!(manifest(json!([{"id":"one", "command":"server", "extensions":{".rs":"rust"}},{"id":"two", "command":"server", "extensions":{".rs":"rust"}}])).validate().is_err());
    }
}
