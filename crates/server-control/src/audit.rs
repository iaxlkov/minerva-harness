#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_redacts_nested_secrets_and_prompts_with_allowlisted_output() {
        let event = AuditEvent::new(
            AuditKind::Denied,
            None,
            None,
            serde_json::json!({"action":"runs.start", "reason":"denied", "prompt":"private prompt", "token":"secret", "nested":{"password":"secret"}}),
        );
        let text = serde_json::to_string(&event).unwrap();
        assert!(text.contains("runs.start"));
        for secret in ["private prompt", "secret", "password", "nested"] {
            assert!(!text.contains(secret));
        }
    }
}
use serde::Serialize;
use uuid::Uuid;
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    MembershipChanged,
    PluginInstalled,
    PluginActivated,
    SecretAccessed,
    Denied,
    SessionRevoked,
}
/// Audit metadata is constrained to enum values. Unstructured reason/prompt/token values are dropped.
#[derive(Debug, Serialize)]
pub struct AuditEvent {
    kind: AuditKind,
    user_id: Option<String>,
    workspace_id: Option<String>,
    metadata: serde_json::Value,
}
impl AuditEvent {
    pub fn new(
        kind: AuditKind,
        user_id: Option<Uuid>,
        workspace_id: Option<Uuid>,
        metadata: serde_json::Value,
    ) -> Self {
        let mut safe = serde_json::Map::new();
        if let Some(action) = metadata.get("action").and_then(|v| v.as_str()) {
            const ACTIONS: &[&str] = &[
                "runs.start",
                "runs.cancel",
                "resources.read",
                "resources.list",
                "config.update",
                "plugins.install",
                "plugins.activate",
                "memberships.update",
                "secrets.resolve",
                "files.write",
            ];
            if ACTIONS.contains(&action) {
                safe.insert("action".into(), action.into());
            }
        }
        Self {
            kind,
            user_id: user_id.map(|v| v.to_string()),
            workspace_id: workspace_id.map(|v| v.to_string()),
            metadata: safe.into(),
        }
    }
}
