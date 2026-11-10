use crate::learning::{ArtifactRef, Scope, SourceRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComponentKind {
    SystemPrompt,
    Guardrails,
    Memory,
    Skills,
    ToolGuidance,
    Scripts,
    Subagents,
    MiddlewarePlugin,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleComponent {
    pub kind: ComponentKind,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub adapter_version: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub digest: String,
    pub parent_digest: Option<String>,
    pub components: Vec<BundleComponent>,
    pub scope: Scope,
    pub source_refs: Vec<SourceRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExperienceStatus {
    Provisional,
    Validated,
    Retired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishExperience {
    pub id: String,
    pub experiment_id: String,
    pub domain: String,
    pub scope: Scope,
    pub cue: String,
    pub lesson: String,
    pub evidence_refs: Vec<ArtifactRef>,
    pub bundle_digest: String,
    pub adoption_reason: String,
    pub limitations: String,
    pub confidence: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Experience {
    #[serde(flatten)]
    pub publication: PublishExperience,
    pub status: ExperienceStatus,
    pub revision: u64,
}
