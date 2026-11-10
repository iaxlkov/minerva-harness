//! Contracts for the optional skills service.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub provider_id: String,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction_path: Option<String>,
    pub model_invocable: bool,
    pub user_invocable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillCatalog {
    pub items: Vec<SkillInfo>,
    pub incomplete: bool,
    pub revision: String,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadSkill {
    pub name: String,
    #[serde(default)]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invocation {
    Model,
    User,
    Selected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadedSkill {
    pub info: SkillInfo,
    pub body: String,
}
