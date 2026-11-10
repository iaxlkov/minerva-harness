//! Session domain and validation. This contract is independent of the UI runtime.
use crate::session_store::SessionStore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

/// A consistent projection and its durable event cursor.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct SessionBaseline {
    pub session: SessionInfo,
    pub messages: Vec<SessionMessage>,
    #[ts(type = "number")]
    pub cursor: i64,
    pub active_run: Option<crate::RunSnapshot>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct SessionEvent {
    pub session_id: String,
    #[ts(type = "number")]
    pub seq: i64,
    pub kind: String,
    #[ts(type = "JsonValue")]
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub has_messages: Option<bool>,
}
pub use crate::transcript::ContentPart;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct SessionMessage {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub created_at: String,
    pub parts: Vec<ContentPart>,
    #[serde(default)]
    #[ts(type = "Record<string, JsonValue>")]
    pub extensions: Map<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct IntegrationState {
    pub namespace: String,
    #[ts(type = "JsonValue")]
    pub value: Value,
}

pub use minerva_plugin_sdk::engine::Turn;

impl cordis::Service for SessionService {
    const NAME: &'static str = "studio.sessions";
}

#[derive(Clone)]
pub struct SessionService {
    store: std::sync::Arc<SessionStore>,
    shutdown_result: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>,
}
impl SessionService {
    pub fn rename(&self, id: &str, title: &str) -> Result<(), String> {
        let title=title.trim();
        if title.is_empty() || title.chars().count()>128 || title.chars().any(char::is_control) { return Err("Invalid session title".into()); }
        self.store.rename(id,title)
    }
    pub fn fork(&self, source: &str, id: &str) -> Result<(), String> { self.store.fork(source,id) }
    pub fn baseline(&self, session: &str) -> Result<SessionBaseline, String> {
        self.store.baseline(session)
    }
    pub fn events_after(
        &self,
        session: &str,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<SessionEvent>, String> {
        self.store.events_after(session, cursor, limit)
    }

    pub fn server_timestamp(&self) -> Result<String, String> {
        self.store.server_timestamp()
    }
    pub fn accept_run(
        &self,
        request: &crate::StartRunRequest,
        created_at: &str,
    ) -> Result<bool, String> {
        if !valid_id(&request.session_id)
            || !valid_id(&request.request_id)
            || !valid_id(&request.message.id)
            || request.message.id == request.request_id
            || (request.message.text.trim().is_empty()
                && !request.options["attachments"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty()))
            || request.parent_id.as_deref().is_some_and(|p| !valid_id(p))
        {
            return Err("invalid run request".into());
        }
        self.store.accept_run(request, created_at)
    }
    pub fn invalidate_integration(
        &self,
        session: &str,
        namespace: &str,
    ) -> Result<(), String> {
        self.store.invalidate_integration(session, namespace)
    }
    pub fn run_snapshot(&self, session: &str, id: &str) -> Result<crate::RunSnapshot, String> {
        self.store.run_snapshot(session, id)
    }
    pub fn shutdown(&self) -> Result<(), String> {
        let mut outcome = self.shutdown_result.lock().map_err(|e| e.to_string())?;
        let result = self.store.shutdown();
        *outcome = Some(result.clone());
        result
    }
    pub fn shutdown_result(&self) -> Result<(), String> {
        self.shutdown_result
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .unwrap_or_else(|| Err("Session service cleanup did not complete".into()))
    }
    pub fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            store: std::sync::Arc::new(SessionStore::open(path)?),
            shutdown_result: Default::default(),
        })
    }
    pub fn create(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err("Некорректный ID диалога".into());
        }
        self.store.create(id)
    }
    pub fn list(&self) -> Result<Vec<SessionInfo>, String> {
        self.store.list()
    }
    pub fn get(&self, id: &str) -> Result<SessionInfo, String> {
        self.store.get(id)
    }
    pub fn load(&self, id: &str) -> Result<Vec<SessionMessage>, String> {
        self.store.load(id)
    }
    pub fn save(
        &self,
        id: &str,
        message: SessionMessage,
        state: Option<IntegrationState>,
    ) -> Result<(), String> {
        if !valid_id(&message.id)
            || !matches!(message.role.as_str(), "user" | "assistant" | "system")
            || message.created_at.is_empty()
        {
            return Err("Некорректный формат сообщения".into());
        }
        crate::transcript::Transcript {
            parts: message.parts.clone(),
            replay: Map::new(),
        }
        .validate()?;
        if message.role != "assistant"
            && (message.parts.iter().any(|p| p.kind.starts_with("tool-"))
                || message.extensions.contains_key("replay"))
        {
            return Err("Инструменты принадлежат ответу ассистента.".into());
        }
        if state.is_some() && message.role != "assistant" {
            return Err("Состояние интеграции сохраняется вместе с ответом".into());
        }
        if let Some(parent) = message.parent_id.as_deref() {
            if !valid_id(parent) {
                return Err("Некорректный ID родительского сообщения".into());
            }
        }
        if let Some(value) = state.as_ref() {
            if !valid_namespace(&value.namespace) {
                return Err("Некорректный ID интеграции".into());
            }
        }
        let title = if message.role == "user" {
            message
                .parts
                .iter()
                .find_map(|part| {
                    if part.kind == "text" {
                        part.data.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .map(|text| text.chars().take(80).collect::<String>())
        } else {
            None
        };
        self.store
            .save(id, &message, title.as_deref(), state.as_ref(), false)
    }
    pub fn begin_run(
        &self,
        session: &str,
        id: &str,
        parent: &str,
        request: &Value,
        invalidate: Option<&str>,
    ) -> Result<(), String> {
        if !valid_id(id) {
            return Err("Некорректный ID запуска".into());
        }
        self.store
            .begin_run(session, id, parent, request, invalidate)
    }
    pub fn commit_reply(
        &self,
        session: &str,
        message: &SessionMessage,
        integration: Option<&IntegrationState>,
    ) -> Result<(), String> {
        crate::transcript::Transcript {
            parts: message.parts.clone(),
            replay: Map::new(),
        }
        .validate_complete()?;
        self.store.save(session, message, None, integration, true)
    }
    pub fn checkpoint(&self, session: &str, message: &SessionMessage) -> Result<(), String> {
        crate::transcript::Transcript {
            parts: message.parts.clone(),
            replay: Map::new(),
        }
        .validate()?;
        self.store.checkpoint(session, message)
    }
    pub fn fail_run(
        &self,
        session: &str,
        id: &str,
        cancelled: bool,
        error: &str,
    ) -> Result<(), String> {
        self.store.fail_run(session, id, cancelled, error)
    }
    pub fn branch(&self, id: &str, last_id: &str) -> Result<Vec<SessionMessage>, String> {
        let messages = self.load(id)?;
        let by_id: std::collections::HashMap<_, _> =
            messages.iter().map(|m| (m.id.as_str(), m)).collect();
        let mut chain = Vec::new();
        let mut current = Some(last_id.to_owned());
        let mut remaining = messages.len() + 1;
        while let Some(mid) = current {
            if remaining == 0 {
                return Err("В истории обнаружен цикл сообщений".into());
            }
            remaining -= 1;
            let item = by_id
                .get(mid.as_str())
                .ok_or("Сообщение истории не найдено")?;
            current = item.parent_id.clone();
            if item
                .extensions
                .get("assistant-ui")
                .and_then(|v| v.get("message"))
                .and_then(|v| v.get("attachments"))
                .and_then(Value::as_array)
                .is_some_and(|v| !v.is_empty())
            {
                return Err("Вложения пока нельзя передать модели.".into());
            }
            if item.role == "system" {
                return Err("Передача сохранённых system-сообщений пока не поддерживается.".into());
            }
            if matches!(item.role.as_str(), "user" | "assistant") {
                if item.parts.iter().any(|p| {
                    (item.role == "user" && p.kind == "extension")
                        || !matches!(
                            p.kind.as_str(),
                            "text"
                                | "reasoning"
                                | "tool-call"
                                | "tool-result"
                                | "extension"
                                | "image"
                                | "file"
                        )
                }) {
                    return Err("Этот тип содержимого пока нельзя передать модели.".into());
                }
                chain.push((*item).clone());
            }
        }
        chain.reverse();
        Ok(chain)
    }
    pub fn history(&self, id: &str, last_id: &str) -> Result<(Vec<Turn>, Option<String>), String> {
        let messages = self.branch(id, last_id)?;
        let parent = messages.last().and_then(|m| m.parent_id.clone());
        let turns = messages
            .into_iter()
            .map(|message| Turn {
                role: message.role,
                parts: message.parts,
                replay: message
                    .extensions
                    .get("replay")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect();
        Ok((turns, parent))
    }
    pub fn set_model(&self, id: &str, model: &str) -> Result<(), String> {
        self.store.set_model(id, model)
    }
    pub fn integration_state(&self, id: &str, namespace: &str) -> Result<Option<Value>, String> {
        if !valid_namespace(namespace) {
            return Err("Некорректный ID интеграции".into());
        }
        self.store.integration_state(id, namespace)
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
fn valid_namespace(namespace: &str) -> bool {
    !namespace.is_empty()
        && namespace.len() <= 128
        && namespace
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_summaries_identify_empty_drafts_without_changing_storage() {
        let dir = tempfile::tempdir().unwrap();
        let service = SessionService::open(&dir.path().join("history.sqlite3")).unwrap();
        service.create("draft").unwrap();
        service.create("used").unwrap();
        service.save("used", message("u1", None, "user", "hello"), None).unwrap();
        assert_eq!(service.get("draft").unwrap().has_messages, Some(false));
        assert_eq!(service.get("used").unwrap().has_messages, Some(true));
        assert_eq!(service.baseline("used").unwrap().session.has_messages, Some(true));
        let sessions = service.list().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.iter().find(|s| s.id == "draft").unwrap().has_messages, Some(false));
    }
    #[test]
    fn rejects_broken_links_and_rewriting_history() {
        let dir = tempfile::tempdir().unwrap();
        let service = SessionService::open(&dir.path().join("history.sqlite")).unwrap();
        service.create("one").unwrap();
        service.create("two").unwrap();
        service
            .save("one", message("u1", None, "user", "first"), None)
            .unwrap();
        assert!(service
            .save(
                "two",
                message("u2", Some("u1"), "user", "cross-session"),
                None
            )
            .is_err());
        assert!(service
            .save(
                "one",
                message("u2", Some("missing"), "user", "orphan"),
                None
            )
            .is_err());
        assert!(service
            .save("one", message("u1", Some("u1"), "user", "cycle"), None)
            .is_err());
        assert!(service
            .save("one", message("u1", None, "user", "changed"), None)
            .is_err());
        service
            .save("one", message("u1", None, "user", "first"), None)
            .unwrap();
        assert_eq!(service.load("one").unwrap().len(), 1);
    }
    #[test]
    fn migrates_v2_and_recovers_interrupted_runs_under_exclusive_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let service = SessionService::open(&path).unwrap();
        service.create("one").unwrap();
        service
            .save("one", message("u1", None, "user", "first"), None)
            .unwrap();
        assert!(SessionService::open(&path).is_err());
        drop(service);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("DROP TABLE runs; PRAGMA user_version=2;")
            .unwrap();
        drop(db);
        let service = SessionService::open(&path).unwrap();
        service
            .begin_run("one", "a1", "u1", &serde_json::json!({}), None)
            .unwrap();
        drop(service);
        let service = SessionService::open(&path).unwrap();
        assert_eq!(service.load("one").unwrap().len(), 1);
        let db = rusqlite::Connection::open(&path).unwrap();
        let status: String = db
            .query_row("SELECT status FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "interrupted");
    }
    use serde_json::json;
    fn message(id: &str, parent: Option<&str>, role: &str, text: &str) -> SessionMessage {
        SessionMessage {
            id: id.into(),
            parent_id: parent.map(str::to_owned),
            role: role.into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            parts: vec![ContentPart {
                kind: "text".into(),
                data: json!({"text":text}),
            }],
            extensions: Map::new(),
        }
    }
    #[test]
    fn persists_history_and_integration_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.sqlite3");
        let service = SessionService::open(&path).unwrap();
        service.create("one").unwrap();
        service.create("two").unwrap();
        service
            .save("one", message("u1", None, "user", "hello"), None)
            .unwrap();
        service
            .save(
                "one",
                message("a1", Some("u1"), "assistant", "answer"),
                Some(IntegrationState {
                    namespace: "plugin:fixture.engine".into(),
                    value: json!({"threadId":"t1","model":"fixture:m","anchorMessageId":"a1"}),
                }),
            )
            .unwrap();
        service
            .save("two", message("u1", None, "user", "other"), None)
            .unwrap();
        drop(service);
        let service = SessionService::open(&path).unwrap();
        assert_eq!(service.load("one").unwrap().len(), 2);
        assert_eq!(
            service.load("two").unwrap()[0].parts[0].data["text"],
            "other"
        );
        assert_eq!(
            service
                .integration_state("one", "plugin:fixture.engine")
                .unwrap()
                .unwrap()["threadId"],
            "t1"
        );
        let (history, parent) = service.history("one", "a1").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(parent.as_deref(), Some("u1"));
        assert!(service
            .save("missing", message("u2", None, "user", "x"), None)
            .is_err());
    }
    #[test]
    fn failure_to_save_integration_rolls_back_assistant_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atomic.sqlite3");
        let service = SessionService::open(&path).unwrap();
        service.create("one").unwrap();
        service
            .save("one", message("u1", None, "user", "hello"), None)
            .unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER fail_state BEFORE INSERT ON session_integration_state BEGIN SELECT RAISE(ABORT,'forced failure'); END;").unwrap();
        drop(db);
        let result = service.save(
            "one",
            message("a1", Some("u1"), "assistant", "answer"),
            Some(IntegrationState {
                namespace: "plugin:fixture.engine".into(),
                value: json!({"threadId":"t1"}),
            }),
        );
        assert!(result.is_err());
        assert_eq!(service.load("one").unwrap().len(), 1);
        assert!(service
            .integration_state("one", "plugin:fixture.engine")
            .unwrap()
            .is_none());
    }
    #[test]
    fn migration_preserves_v1_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.sqlite3");
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY,title TEXT NOT NULL,updated_at INTEGER NOT NULL,model TEXT,codex_thread_id TEXT,codex_model TEXT);CREATE TABLE messages(session_id TEXT NOT NULL REFERENCES sessions(id),message_id TEXT NOT NULL,parent_id TEXT,role TEXT NOT NULL,created_at TEXT NOT NULL,content_json TEXT NOT NULL,extras_json TEXT NOT NULL,run_config_json TEXT,seq INTEGER NOT NULL,PRIMARY KEY(session_id,message_id));CREATE INDEX messages_session_seq ON messages(session_id,seq);INSERT INTO sessions VALUES('one','title',1,'codex:m','thread','codex:m');INSERT INTO messages VALUES('one','a1',NULL,'assistant','time','[{\"type\":\"text\",\"text\":\"hello\",\"id\":\"p1\"},{\"type\":\"custom\",\"data\":42}]','{\"metadata\":{\"custom\":{}}}','{\"x\":1}',1);PRAGMA user_version=1;").unwrap();
        }
        let service = SessionService::open(&path).unwrap();
        let m = service.load("one").unwrap();
        assert_eq!(m[0].parts[0].data["text"], "hello");
        assert_eq!(m[0].parts[1].data["data"], 42);
        assert_eq!(m[0].extensions["assistant-ui"]["parts"][0]["id"], "p1");
        assert_eq!(m[0].extensions["assistant-ui"]["runConfig"]["x"], 1);
        assert!(service.integration_state("one", "codex").unwrap().is_none());
        drop(service);
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            5
        );
    }
    #[test]
    fn migration_failure_does_not_change_v1() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.sqlite3");
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY,title TEXT,updated_at INTEGER,model TEXT,codex_thread_id TEXT,codex_model TEXT);CREATE TABLE messages(session_id TEXT,message_id TEXT,parent_id TEXT,role TEXT,created_at TEXT,content_json TEXT,extras_json TEXT,run_config_json TEXT,seq INTEGER);INSERT INTO sessions VALUES('s','title',1,NULL,NULL,NULL);INSERT INTO messages VALUES('s','m',NULL,'user','time','not-json','{}',NULL,1);PRAGMA user_version=1;").unwrap();
        }
        assert!(SessionService::open(&path).is_err());
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT content_json FROM messages", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "not-json"
        );
    }
    #[test]
    fn refuses_future_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.sqlite3");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", 9).unwrap();
        drop(db);
        assert!(SessionService::open(&path).is_err());
    }
    #[test]
    #[ignore = "reads a disposable copy of the user's v1 database"]
    fn migrate_backup_copy() {
        let source = std::env::var("SESSION_V1_COPY").expect("copy path");
        let service = SessionService::open(Path::new(&source)).expect("migrate copy");
        let sessions = service.list().unwrap();
        assert_eq!(sessions.len(), 2);
        let count: usize = sessions
            .iter()
            .map(|s| service.load(&s.id).unwrap().len())
            .sum();
        assert_eq!(count, 4);
    }
}
