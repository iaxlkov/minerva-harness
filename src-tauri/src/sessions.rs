//! Session domain and validation. This contract is independent of the UI runtime.
use crate::session_store::SessionStore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub model: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentPart {
    pub kind: String,
    pub data: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub created_at: String,
    pub parts: Vec<ContentPart>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationState {
    pub namespace: String,
    pub value: Value,
}

pub struct SessionService {
    store: SessionStore,
}
impl SessionService {
    pub fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            store: SessionStore::open(path)?,
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
        if message.parts.iter().any(|part| match part.kind.as_str() {
            "text" | "reasoning" => !part.data.get("text").is_some_and(Value::is_string),
            "extension" => !part.data.is_object(),
            _ => true,
        }) {
            return Err("Некорректное содержимое сообщения".into());
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
            if !valid_id(&value.namespace) {
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
            .save(id, &message, title.as_deref(), state.as_ref())
    }
    pub fn history(
        &self,
        id: &str,
        last_id: &str,
    ) -> Result<(Vec<crate::providers::Turn>, Option<String>), String> {
        let messages = self.load(id)?;
        let parent = messages
            .iter()
            .find(|m| m.id == last_id)
            .ok_or("Сообщение истории не найдено")?
            .parent_id
            .clone();
        let mut chain = Vec::new();
        let mut current = Some(last_id.to_owned());
        let mut remaining = messages.len() + 1;
        while let Some(mid) = current {
            if remaining == 0 {
                return Err("В истории обнаружен цикл сообщений".into());
            }
            remaining -= 1;
            let item = messages
                .iter()
                .find(|m| m.id == mid)
                .ok_or("Сообщение истории не найдено")?;
            current = item.parent_id.clone();
            if matches!(item.role.as_str(), "user" | "assistant") {
                if item
                    .parts
                    .iter()
                    .any(|p| !matches!(p.kind.as_str(), "text" | "reasoning"))
                {
                    return Err("Этот диалог содержит вложения или инструменты, которые пока нельзя передать модели.".into());
                }
                let text = item
                    .parts
                    .iter()
                    .filter(|p| p.kind == "text")
                    .filter_map(|p| p.data.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    chain.push(crate::providers::Turn {
                        role: item.role.clone(),
                        text,
                    });
                }
            }
        }
        chain.reverse();
        Ok((chain, parent))
    }
    pub fn set_model(&self, id: &str, model: &str) -> Result<(), String> {
        self.store.set_model(id, model)
    }
    pub fn integration_state(&self, id: &str, namespace: &str) -> Result<Option<Value>, String> {
        if !valid_id(namespace) {
            return Err("Некорректный ID интеграции".into());
        }
        self.store.integration_state(id, namespace)
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
#[cfg(test)]
mod tests {
    use super::*;
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
                    namespace: "codex".into(),
                    value: json!({"threadId":"t1","model":"codex:m","anchorMessageId":"a1"}),
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
            service.integration_state("one", "codex").unwrap().unwrap()["threadId"],
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
                namespace: "codex".into(),
                value: json!({"threadId":"t1"}),
            }),
        );
        assert!(result.is_err());
        assert_eq!(service.load("one").unwrap().len(), 1);
        assert!(service.integration_state("one", "codex").unwrap().is_none());
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
        let state = service.integration_state("one", "codex").unwrap().unwrap();
        assert_eq!(state["threadId"], "thread");
        assert_eq!(state["anchorMessageId"], "a1");
        drop(service);
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
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
        assert_eq!(
            sessions
                .iter()
                .filter(|s| service.integration_state(&s.id, "codex").unwrap().is_some())
                .count(),
            1
        );
    }
}
