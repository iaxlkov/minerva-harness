//! SQLite implementation for the session domain.
use crate::sessions::{ContentPart, IntegrationState, SessionInfo, SessionMessage};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::{path::Path, sync::Mutex};

pub struct SessionStore {
    connection: Mutex<Connection>,
    _lock: std::fs::File,
}
impl SessionStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|_| "База диалогов уже открыта другим экземпляром приложения.".to_string())?;
        let mut db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version > 3 {
            return Err("База диалогов создана более новой версией приложения".into());
        }
        if version < 3 {
            let tx = db.transaction().map_err(|e| e.to_string())?;
            if version == 0 {
                tx.execute_batch(" CREATE TABLE sessions(id TEXT PRIMARY KEY,title TEXT NOT NULL,updated_at INTEGER NOT NULL,model TEXT); CREATE TABLE messages(session_id TEXT NOT NULL REFERENCES sessions(id),message_id TEXT NOT NULL,parent_id TEXT,role TEXT NOT NULL,created_at TEXT NOT NULL,content_json TEXT NOT NULL,extras_json TEXT NOT NULL,seq INTEGER NOT NULL,PRIMARY KEY(session_id,message_id)); CREATE INDEX messages_session_seq ON messages(session_id,seq); CREATE TABLE session_integration_state(session_id TEXT NOT NULL REFERENCES sessions(id),namespace TEXT NOT NULL,state_json TEXT NOT NULL,PRIMARY KEY(session_id,namespace)); PRAGMA user_version=2;").map_err(|e|e.to_string())?;
            }
            if version == 1 {
                migrate_v1(&tx)?;
            }
            if version < 3 {
                tx.execute_batch(" CREATE TABLE runs(session_id TEXT NOT NULL REFERENCES sessions(id),id TEXT NOT NULL,parent_id TEXT NOT NULL,request_json TEXT NOT NULL,status TEXT NOT NULL,error TEXT,created_at INTEGER NOT NULL,PRIMARY KEY(session_id,id)); PRAGMA user_version=3;").map_err(|e|e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        db.execute("UPDATE runs SET status='interrupted',error='Приложение завершилось до сохранения результата' WHERE status='running'", []).map_err(|e| e.to_string())?;
        Ok(Self {
            connection: Mutex::new(db),
            _lock: lock,
        })
    }
    pub fn create(&self, id: &str) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "INSERT INTO sessions(id,title,updated_at) VALUES(?1,'Новый диалог',unixepoch())",
                [id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<SessionInfo>, String> {
        let db = self.connection.lock().map_err(|e| e.to_string())?;
        let mut q=db.prepare("SELECT id,title,updated_at,model FROM sessions ORDER BY updated_at DESC,rowid DESC").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([], |r| {
                Ok(SessionInfo {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    updated_at: r.get(2)?,
                    model: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn get(&self, id: &str) -> Result<SessionInfo, String> {
        let db = self.connection.lock().map_err(|e| e.to_string())?;
        db.query_row(
            "SELECT id,title,updated_at,model FROM sessions WHERE id=?1",
            [id],
            |r| {
                Ok(SessionInfo {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    updated_at: r.get(2)?,
                    model: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("Диалог не найден".into())
    }
    pub fn load(&self, id: &str) -> Result<Vec<SessionMessage>, String> {
        let db = self.connection.lock().map_err(|e| e.to_string())?;
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("Диалог не найден".into());
        }
        let mut q=db.prepare("SELECT message_id,parent_id,role,created_at,content_json,extras_json FROM messages WHERE session_id=?1 ORDER BY seq").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (id, parent_id, role, created_at, content, extras) =
                row.map_err(|e| e.to_string())?;
            out.push(SessionMessage {
                id,
                parent_id,
                role,
                created_at,
                parts: serde_json::from_str(&content).map_err(|e| e.to_string())?,
                extensions: serde_json::from_str(&extras).map_err(|e| e.to_string())?,
            });
        }
        Ok(out)
    }
    pub fn save(
        &self,
        id: &str,
        m: &SessionMessage,
        title: Option<&str>,
        state: Option<&IntegrationState>,
        finish_run: bool,
    ) -> Result<(), String> {
        let mut db = self.connection.lock().map_err(|e| e.to_string())?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if let Some(parent) = &m.parent_id {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND message_id=?2)",
                    params![id, parent],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if parent == &m.id || !exists {
                return Err("Родительское сообщение отсутствует в этом диалоге.".into());
            }
        }
        let content = serde_json::to_string(&m.parts).map_err(|e| e.to_string())?;
        let old: Option<(Option<String>,String,String,String)> = tx.query_row("SELECT parent_id,role,created_at,content_json FROM messages WHERE session_id=?1 AND message_id=?2", params![id,m.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
        if let Some((parent, role, date, body)) = old {
            if parent != m.parent_id
                || role != m.role
                || date != m.created_at
                || serde_json::from_str::<Value>(&body).map_err(|e| e.to_string())?
                    != serde_json::to_value(&m.parts).map_err(|e| e.to_string())?
            {
                return Err("Сохранённое сообщение неизменяемо; создайте новую ветку.".into());
            }
        }
        tx.execute("INSERT INTO messages(session_id,message_id,parent_id,role,created_at,content_json,extras_json,seq) VALUES(?1,?2,?3,?4,?5,?6,?7,(SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE session_id=?1)) ON CONFLICT(session_id,message_id) DO UPDATE SET extras_json=excluded.extras_json",params![id,m.id,m.parent_id,m.role,m.created_at,content,serde_json::to_string(&m.extensions).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        if let Some(state) = state {
            tx.execute("INSERT INTO session_integration_state(session_id,namespace,state_json) VALUES(?1,?2,?3) ON CONFLICT(session_id,namespace) DO UPDATE SET state_json=excluded.state_json",params![id,state.namespace,state.value.to_string()]).map_err(|e|e.to_string())?;
        }
        tx.execute("UPDATE sessions SET title=CASE WHEN title='Новый диалог' AND ?2 IS NOT NULL THEN ?2 ELSE title END,updated_at=unixepoch() WHERE id=?1",params![id,title]).map_err(|e|e.to_string())?;
        if finish_run && tx.execute("UPDATE runs SET status='completed' WHERE session_id=?1 AND id=?2 AND status='running'", params![id,m.id]).map_err(|e|e.to_string())? != 1 {
            return Err("Запуск уже завершён или не найден.".into());
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn set_model(&self, id: &str, model: &str) -> Result<(), String> {
        let db = self.connection.lock().map_err(|e| e.to_string())?;
        if db
            .execute(
                "UPDATE sessions SET model=?2 WHERE id=?1",
                params![id, model],
            )
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("Диалог не найден".into());
        }
        Ok(())
    }
    pub fn integration_state(&self, id: &str, namespace: &str) -> Result<Option<Value>, String> {
        let db = self.connection.lock().map_err(|e| e.to_string())?;
        let row:Option<String>=db.query_row("SELECT state_json FROM session_integration_state WHERE session_id=?1 AND namespace=?2",params![id,namespace],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        row.map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn begin_run(
        &self,
        session: &str,
        id: &str,
        parent: &str,
        request: &Value,
        invalidate: Option<&str>,
    ) -> Result<(), String> {
        let mut db = self.connection.lock().map_err(|e| e.to_string())?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let conflict: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND message_id=?2) OR EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND status='running')", params![session,id], |r|r.get(0)).map_err(|e|e.to_string())?;
        if conflict {
            return Err("Ответ уже сохранён или диалог занят.".into());
        }
        tx.execute(
            "INSERT INTO runs VALUES(?1,?2,?3,?4,'running',NULL,unixepoch())",
            params![session, id, parent, request.to_string()],
        )
        .map_err(|e| e.to_string())?;
        if let Some(namespace) = invalidate {
            tx.execute(
                "DELETE FROM session_integration_state WHERE session_id=?1 AND namespace=?2",
                params![session, namespace],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn fail_run(
        &self,
        session: &str,
        id: &str,
        cancelled: bool,
        error: &str,
    ) -> Result<(), String> {
        self.connection.lock().map_err(|e|e.to_string())?.execute("UPDATE runs SET status=?3,error=?4 WHERE session_id=?1 AND id=?2 AND status='running'",params![session,id,if cancelled {"cancelled"} else {"failed"},error]).map_err(|e|e.to_string())?;
        Ok(())
    }
}
fn migrate_v1(tx: &rusqlite::Transaction<'_>) -> Result<(), String> {
    tx.execute_batch("CREATE TABLE session_integration_state(session_id TEXT NOT NULL REFERENCES sessions(id),namespace TEXT NOT NULL,state_json TEXT NOT NULL,PRIMARY KEY(session_id,namespace)); INSERT INTO session_integration_state SELECT id,'codex',json_object('threadId',codex_thread_id,'model',codex_model,'anchorMessageId',(SELECT message_id FROM messages WHERE session_id=sessions.id AND role='assistant' ORDER BY seq DESC LIMIT 1)) FROM sessions WHERE codex_thread_id IS NOT NULL AND codex_model IS NOT NULL;").map_err(|e|e.to_string())?;
    let rows = {
        let mut q=tx.prepare("SELECT session_id,message_id,content_json,extras_json,run_config_json FROM messages").map_err(|e|e.to_string())?;
        let v = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        v.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    for (sid, mid, content, extras, run) in rows {
        let source: Vec<Value> = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let extras: Value = serde_json::from_str(&extras).map_err(|e| e.to_string())?;
        let run: Option<Value> = run
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        let mut part_extras = Vec::new();
        for item in source {
            let mut obj = item
                .as_object()
                .cloned()
                .ok_or("Некорректное содержимое старой истории")?;
            let kind = obj
                .remove("type")
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or("Нет типа содержимого")?;
            if matches!(kind.as_str(), "text" | "reasoning") {
                let text = obj.remove("text").ok_or("Нет текста сообщения")?;
                parts.push(ContentPart {
                    kind,
                    data: json!({"text":text}),
                });
                part_extras.push(Value::Object(obj));
            } else {
                obj.insert("type".into(), Value::String(kind));
                parts.push(ContentPart {
                    kind: "extension".into(),
                    data: Value::Object(obj),
                });
                part_extras.push(json!({}));
            }
        }
        let mut ext = Map::new();
        ext.insert(
            "assistant-ui".into(),
            json!({"message":extras,"parts":part_extras,"runConfig":run}),
        );
        tx.execute("UPDATE messages SET content_json=?3,extras_json=?4 WHERE session_id=?1 AND message_id=?2",params![sid,mid,serde_json::to_string(&parts).map_err(|e|e.to_string())?,serde_json::to_string(&ext).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    }
    tx.execute_batch("ALTER TABLE sessions DROP COLUMN codex_thread_id; ALTER TABLE sessions DROP COLUMN codex_model; ALTER TABLE messages DROP COLUMN run_config_json; PRAGMA user_version=2;").map_err(|e|e.to_string())?;
    Ok(())
}
