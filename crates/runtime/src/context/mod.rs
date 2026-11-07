//! Session-scoped prompt projection. Durable history is never edited here.
mod files;
mod images;
use crate::sessions::{SessionMessage, Turn};
pub use minerva_plugin_sdk::context::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

pub struct ContextService {
    db: Mutex<Connection>,
}

pub struct PreparedContext {
    pub system_prompt: String,
    pub turns: Vec<Turn>,
    pub budget: ContextBudget,
    pub skills: Vec<SkillInfo>,
}

impl ContextService {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS workspace (session TEXT PRIMARY KEY, settings TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS selected_skills (session TEXT NOT NULL, name TEXT NOT NULL, PRIMARY KEY(session,name));
            CREATE TABLE IF NOT EXISTS summaries (session TEXT PRIMARY KEY, prefix TEXT NOT NULL, summary TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS attachments (id TEXT PRIMARY KEY, session TEXT NOT NULL, name TEXT NOT NULL, mime TEXT NOT NULL, data BLOB NOT NULL);").map_err(|e|e.to_string())?;
        Ok(Self { db: Mutex::new(db) })
    }
    fn db(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.db
            .lock()
            .map_err(|_| "context storage lock poisoned".into())
    }

    pub fn workspace_get(&self, session: &str) -> Result<WorkspaceSettings, String> {
        valid_id(session)?;
        let value: Option<String> = self
            .db()?
            .query_row(
                "SELECT settings FROM workspace WHERE session=?1",
                [session],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        value
            .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            .unwrap_or_else(|| Ok(WorkspaceSettings::default()))
    }
    pub fn workspace_save(
        &self,
        session: &str,
        mut settings: WorkspaceSettings,
    ) -> Result<WorkspaceSettings, String> {
        valid_id(session)?;
        if !(1024..=2_097_152).contains(&settings.context_window) {
            return Err("contextWindow must be between 1024 and 2097152".into());
        }
        if let Some(cwd) = settings.cwd.as_ref() {
            let path = Path::new(cwd);
            if !path.is_absolute() {
                return Err("workspace cwd must be absolute".into());
            }
            let path = path
                .canonicalize()
                .map_err(|e| format!("workspace cwd: {e}"))?;
            if !path.is_dir() {
                return Err("workspace cwd must be a directory".into());
            }
            settings.cwd = Some(
                path.to_str()
                    .ok_or("workspace path must be UTF-8")?
                    .to_owned(),
            );
        }
        let value = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
        self.db()?.execute("INSERT INTO workspace VALUES (?1,?2) ON CONFLICT(session) DO UPDATE SET settings=excluded.settings",params![session,value]).map_err(|e|e.to_string())?;
        Ok(settings)
    }
    pub fn load_skill(&self, session: &str, name: &str) -> Result<String, String> {
        valid_id(session)?;
        if !files::valid_skill_name(name) {
            return Err("invalid skill name".into());
        }
        let settings = self.workspace_get(session)?;
        let cwd = settings
            .cwd
            .as_deref()
            .ok_or("select a workspace to load skills")?;
        let skills = files::discover_skills(Path::new(cwd))?;
        let skill = skills
            .iter()
            .find(|s| s.info.name == name)
            .ok_or("skill not found")?;
        let text = files::read_scoped(&skill.path, &skill.root, files::MAX_SOURCE)?;
        self.db()?
            .execute(
                "INSERT OR IGNORE INTO selected_skills VALUES (?1,?2)",
                params![session, name],
            )
            .map_err(|e| e.to_string())?;
        Ok(text)
    }
    pub fn prepare(
        &self,
        session: &str,
        messages: &[SessionMessage],
    ) -> Result<PreparedContext, String> {
        let settings = self.workspace_get(session)?;
        let mut system_prompt="You are Minerva, a helpful assistant. Follow the user request and use the available tools when needed.".to_owned();
        let mut skills = Vec::new();
        if let Some(cwd) = settings.cwd.as_deref() {
            system_prompt.push_str(&format!(
                "\nWorkspace cwd: {cwd}\nPermission policy: {:?}\n",
                settings.permission_policy
            ));
            system_prompt.push_str(&files::instructions(Path::new(cwd))?);
            let discovered = files::discover_skills(Path::new(cwd))?;
            let selected: Vec<String> = {
                let db = self.db()?;
                let mut stmt = db
                    .prepare("SELECT name FROM selected_skills WHERE session=?1 ORDER BY name")
                    .map_err(|e| e.to_string())?;
                let names = stmt
                    .query_map([session], |r| r.get(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                names
            };
            let mut remaining = files::MAX_TOTAL;
            for skill in discovered {
                system_prompt.push_str(&format!(
                    "\nAvailable skill {}: {}\n",
                    skill.info.name, skill.info.description
                ));
                if selected.contains(&skill.info.name) {
                    let body = files::read_scoped(&skill.path, &skill.root, files::MAX_SOURCE)?;
                    if body.len() > remaining {
                        return Err("loaded skills exceed context byte limit".into());
                    }
                    remaining -= body.len();
                    system_prompt
                        .push_str(&format!("\nLoaded skill {}:\n{body}\n", skill.info.name));
                }
                skills.push(skill.info);
            }
        }
        let mut start = 0;
        let saved: Option<(String, String)> = self
            .db()?
            .query_row(
                "SELECT prefix, summary FROM summaries WHERE session=?1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((prefix, summary)) = saved {
            let anchored: Vec<SessionMessage> =
                serde_json::from_str(&prefix).map_err(|e| e.to_string())?;
            if anchored.len() <= messages.len()
                && serde_json::to_string(&messages[..anchored.len()]).map_err(|e| e.to_string())?
                    == prefix
            {
                let summary: ContextSummary =
                    serde_json::from_str(&summary).map_err(|e| e.to_string())?;
                system_prompt.push_str(&format!(
                    "\nSummary of earlier conversation:\n{}\n",
                    summary.text
                ));
                start = anchored.len();
            }
        }
        let mut turns = Vec::new();
        for message in &messages[start..] {
            let mut turn = Turn {
                role: message.role.clone(),
                parts: message.parts.clone(),
                replay: message
                    .extensions
                    .get("replay")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            };
            for part in &mut turn.parts {
                if part.kind == "tool-result" {
                    if let Some(output) = part.data.get_mut("output") {
                        prune_output(output);
                    }
                }
            }
            if let Some(replay) = turn.replay.get_mut("rig-v1") {
                prune_replay(replay);
            }
            turns.push(turn);
        }
        let estimated_tokens =
            estimate_tokens(&system_prompt) + turns.iter().map(estimate_turn).sum::<usize>();
        let budget = ContextBudget {
            context_window: settings.context_window,
            estimated_tokens,
            available_tokens: settings.context_window.saturating_sub(estimated_tokens),
            should_compact: settings.auto_compact
                && estimated_tokens > settings.context_window.saturating_mul(4) / 5,
            usage: latest_usage(messages),
        };
        Ok(PreparedContext {
            system_prompt,
            turns,
            budget,
            skills,
        })
    }
    pub fn save_summary(
        &self,
        session: &str,
        prefix: &[SessionMessage],
        text: &str,
    ) -> Result<ContextSummary, String> {
        valid_id(session)?;
        if prefix.is_empty()
            || text.trim().is_empty()
            || text.len() > files::MAX_TOTAL
            || !complete_prefix(prefix)
        {
            return Err("summary requires bounded text and complete conversation turns".into());
        }
        let summary = ContextSummary {
            text: text.into(),
            prefix_message_ids: prefix.iter().map(|m| m.id.clone()).collect(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis()
                .to_string(),
        };
        self.db()?.execute("INSERT INTO summaries VALUES (?1,?2,?3) ON CONFLICT(session) DO UPDATE SET prefix=excluded.prefix, summary=excluded.summary",params![session,serde_json::to_string(prefix).map_err(|e|e.to_string())?,serde_json::to_string(&summary).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        Ok(summary)
    }
}

fn latest_usage(messages: &[SessionMessage]) -> Option<TokenUsage> {
    let values = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")?
        .extensions
        .get("replay")?
        .get("usage")?
        .as_array()?;
    let sum = |key: &str| {
        let mut total = None;
        for value in values {
            if let Some(value) = value.get(key).and_then(Value::as_u64) {
                total = Some(total.unwrap_or(0u64).saturating_add(value));
            }
        }
        total
    };
    Some(TokenUsage {
        input_tokens: sum("input_tokens"),
        output_tokens: sum("output_tokens"),
        total_tokens: sum("total_tokens"),
    })
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    #[test]
    fn latest_assistant_without_usage_does_not_inherit_previous_usage() {
        let mut api = SessionMessage {
            id: "api".into(),
            parent_id: None,
            role: "assistant".into(),
            created_at: "now".into(),
            parts: vec![],
            extensions: serde_json::from_value(
                serde_json::json!({"replay":{"usage":[{"input_tokens":42}]}}),
            )
            .unwrap(),
        };
        let mut external = api.clone();
        external.id = "external".into();
        external.extensions.clear();
        assert!(latest_usage(&[api.clone(), external]).is_none());
        let assistant = api.clone();
        api.role = "user".into();
        api.extensions =
            serde_json::from_value(serde_json::json!({"replay":{"usage":[{"input_tokens":999}]}}))
                .unwrap();
        assert_eq!(
            latest_usage(&[assistant, api.clone()])
                .unwrap()
                .input_tokens,
            Some(42)
        );
        assert!(latest_usage(&[api]).is_none());
    }
    #[test]
    fn cjk_saved_tool_output_projection_shortens_without_changing_source() {
        let original = Value::String("\u{754c}".repeat(3000));
        let mut projected = original.clone();
        prune_output(&mut projected);
        assert!(projected.as_str().unwrap().len() < 8192);
        assert!(projected.as_str().unwrap().len() < original.as_str().unwrap().len());
        assert_eq!(original.as_str().unwrap().len(), 9000);
    }
    #[test]
    fn measured_usage_is_separate_from_context_estimates_and_unreported_counters() {
        let message = |id: &str, usage: Value| SessionMessage {
            id: id.into(),
            parent_id: None,
            role: "assistant".into(),
            created_at: "now".into(),
            parts: vec![],
            extensions: serde_json::from_value(serde_json::json!({"replay":{"usage":usage}}))
                .unwrap(),
        };
        let usage = latest_usage(&[
            message("old", serde_json::json!([{"input_tokens":500}])),
            message(
                "new",
                serde_json::json!([{"input_tokens":4,"output_tokens":2},{"input_tokens":6}]),
            ),
        ])
        .unwrap();
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(2));
        assert_eq!(usage.total_tokens, None);
        assert!(latest_usage(&[]).is_none());
    }
}

fn valid_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        Err("invalid session or attachment id".into())
    } else {
        Ok(())
    }
}

pub fn compact_prefix(messages: &[SessionMessage], keep_recent: usize) -> Vec<SessionMessage> {
    let limit = messages.len().saturating_sub(keep_recent.max(2));
    for end in (1..=limit).rev() {
        if messages.get(end).is_some_and(|m| m.role == "user") && complete_prefix(&messages[..end])
        {
            return messages[..end].to_vec();
        }
    }
    Vec::new()
}

fn complete_prefix(messages: &[SessionMessage]) -> bool {
    if !messages.last().is_some_and(|m| m.role == "assistant") {
        return false;
    }
    let mut pending = std::collections::HashSet::new();
    for message in messages {
        if message
            .extensions
            .get("execution")
            .and_then(|e| e.get("status"))
            .and_then(Value::as_str)
            .is_some_and(|s| s != "completed")
        {
            return false;
        }
        if message
            .extensions
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|s| matches!(s, "running" | "pending"))
        {
            return false;
        }
        for part in &message.parts {
            if part.kind == "tool-call" {
                if let Some(id) = part.data["callId"].as_str() {
                    pending.insert(id.to_owned());
                }
            }
            if part.kind == "tool-result" {
                if let Some(id) = part.data["callId"].as_str() {
                    pending.remove(id);
                }
            }
        }
    }
    pending.is_empty()
}

// shortcut: provider-independent estimates use UTF-8 bytes / 3; use a routed tokenizer when available.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(3)
}
fn estimate_turn(turn: &Turn) -> usize {
    let parts = turn
        .parts
        .iter()
        .map(|p| {
            if p.kind == "image" {
                1024
            } else {
                estimate_tokens(&p.data.to_string())
            }
        })
        .sum::<usize>();
    let replay = turn
        .replay
        .get("rig-v1")
        .map(|v| estimate_tokens(&v.to_string()))
        .unwrap_or(0);
    8 + parts.max(replay)
}

fn prune_output(value: &mut Value) {
    let serialized = match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    };
    if let Some(pruned) = prune_text(&serialized) {
        *value = Value::String(pruned);
    }
}

pub(crate) fn prune_text(text: &str) -> Option<String> {
    if text.len() <= 8192 {
        return None;
    }
    let mut head_end = 4096;
    while !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len() - 2048;
    while !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    Some(format!(
        "{}\n[tool output pruned for model context; full result retained in session history]\n{}",
        &text[..head_end],
        &text[tail_start..]
    ))
}

fn prune_replay(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                prune_replay(item);
            }
        }
        Value::Object(fields) => {
            let is_result = fields.get("type").and_then(Value::as_str) == Some("toolresult");
            if is_result {
                if let Some(content) = fields.get_mut("content") {
                    if content.to_string().len() > 8192 {
                        *content = serde_json::json!([{"type":"text","text":"[tool output pruned for model context; full result retained in session history]"}]);
                    }
                }
            }
            for item in fields.values_mut() {
                prune_replay(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
