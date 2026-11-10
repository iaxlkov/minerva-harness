//! Shared learning contracts. AccessContext is supplied by the host, never deserialized.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Scope {
    Session { session_id: String },
    Workspace { workspace_id: String },
    User,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRef {
    pub session_id: String,
    pub message_id: Option<String>,
    pub event_seq: Option<i64>,
    pub run_id: Option<String>,
    pub content_digest: String,
}
impl SourceRef {
    pub fn validate(&self) -> Result<(), LearningError> {
        if self.session_id.is_empty()
            || (self.message_id.as_deref().is_none_or(str::is_empty) && self.event_seq.is_none())
            || self.event_seq.is_some_and(|seq| seq < 0)
            || !valid_digest(&self.content_digest)
        {
            return Err(LearningError::new(
                "INVALID_INPUT",
                "invalid source reference",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub digest: String,
    pub media_type: String,
    pub bytes: u64,
}

/// A trusted host capability; do not accept these fields from model or browser input.
#[derive(Clone, Debug)]
pub struct AccessContext {
    pub actor_id: String,
    pub session_id: String,
    pub workspace_id: Option<String>,
    pub allow_user_scope: bool,
}
impl AccessContext {
    pub fn authorize(&self, scope: &Scope) -> Result<(), LearningError> {
        let allowed = !self.actor_id.is_empty()
            && !self.session_id.is_empty()
            && match scope {
                Scope::Session { session_id } => session_id == &self.session_id,
                Scope::Workspace { workspace_id } => {
                    self.workspace_id.as_ref() == Some(workspace_id)
                }
                Scope::User => self.allow_user_scope,
            };
        if allowed {
            Ok(())
        } else {
            Err(LearningError::new(
                "NOT_FOUND_OR_FORBIDDEN",
                "scope unavailable",
            ))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LearningError {
    pub code: String,
    pub message: String,
}
impl LearningError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for LearningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for LearningError {}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authority_is_scoped_and_source_requires_an_exact_anchor() {
        let host = AccessContext {
            actor_id: "u".into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        assert!(host
            .authorize(&Scope::Workspace {
                workspace_id: "w".into()
            })
            .is_ok());
        assert!(host
            .authorize(&Scope::Workspace {
                workspace_id: "other".into()
            })
            .is_err());
        assert!(host.authorize(&Scope::User).is_err());
        let mut source = SourceRef {
            session_id: "s".into(),
            message_id: None,
            event_seq: None,
            run_id: None,
            content_digest: "a".repeat(64),
        };
        assert!(source.validate().is_err());
        source.event_seq = Some(1);
        assert!(source.validate().is_ok());
    }
}
