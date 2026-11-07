//! SQLite implementation for the session domain.
use crate::sessions::{ContentPart, IntegrationState, SessionInfo, SessionMessage};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::{path::Path, sync::Mutex};

// Keep a live connection guard for existing transaction code, while permitting
// explicit shutdown even when hosts retain service handles.
struct Database(Mutex<Option<Connection>>);
struct DatabaseGuard<'a>(std::sync::MutexGuard<'a, Option<Connection>>);
impl Database {
    fn lock(&self) -> Result<DatabaseGuard<'_>, String> {
        let guard = self.0.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            return Err("Runtime database is closed".into());
        }
        Ok(DatabaseGuard(guard))
    }
}
impl std::ops::Deref for DatabaseGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0.as_ref().expect("checked live connection")
    }
}
impl std::ops::DerefMut for DatabaseGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.0.as_mut().expect("checked live connection")
    }
}

pub struct SessionStore {
    connection: Database,
    _lock: std::fs::File,
}
impl SessionStore {
    pub fn baseline(&self, session: &str) -> Result<crate::sessions::SessionBaseline, String> {
        let mut db = self.connection.lock()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let snapshot = crate::events::baseline(&tx, session)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(snapshot)
    }
    pub fn events_after(
        &self,
        session: &str,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<crate::sessions::SessionEvent>, String> {
        let db = self.connection.lock()?;
        crate::events::tail(&db, session, cursor, limit)
    }

    pub fn shutdown(&self) -> Result<(), String> {
        let mut connection = self.connection.0.lock().map_err(|e| e.to_string())?;
        if let Some(db) = connection.take() {
            if let Err((db, error)) = db.close() {
                *connection = Some(db);
                return Err(error.to_string());
            }
            self._lock.unlock().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

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
        if version > 5 {
            return Err("База диалогов создана более новой версией приложения".into());
        }
        if (1..5).contains(&version) {
            // SQLite's backup API includes committed WAL data and leaves the source untouched.
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let name = path
                .file_name()
                .ok_or("database filename required")?
                .to_string_lossy();
            let backup = path.with_file_name(format!("{name}.pre-v5-{stamp}.sqlite3"));
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
                .map_err(|e| e.to_string())?;
            db.backup(rusqlite::MAIN_DB, &backup, None)
                .map_err(|e| format!("Не удалось создать резервную копию перед миграцией: {e}"))?;
        }
        if version < 4 {
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
            tx.execute_batch("PRAGMA user_version=4;")
                .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        if version < 5 {
            crate::events::migrate(&mut db)?;
        }
        db.execute("UPDATE runs SET status='interrupted',error='Приложение завершилось до сохранения результата' WHERE status='running'", []).map_err(|e| e.to_string())?;
        Ok(Self {
            connection: Database(Mutex::new(Some(db))),
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
        let mut q=db.prepare("SELECT m.message_id,m.parent_id,m.role,m.created_at,m.content_json,CASE WHEN r.id IS NULL THEN m.extras_json ELSE json_set(m.extras_json,'$.execution',json_object('status',r.status,'error',r.error)) END FROM messages m LEFT JOIN runs r ON r.session_id=m.session_id AND r.id=m.message_id WHERE m.session_id=?1 ORDER BY m.seq").map_err(|e|e.to_string())?;
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
        let draft: bool = finish_run && tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND id=?2 AND status='running')", params![id,m.id], |r|r.get(0)).map_err(|e|e.to_string())?;
        if let Some((parent, role, date, body)) = old {
            if parent != m.parent_id
                || role != m.role
                || date != m.created_at
                || (!draft
                    && serde_json::from_str::<Value>(&body).map_err(|e| e.to_string())?
                        != serde_json::to_value(&m.parts).map_err(|e| e.to_string())?)
            {
                return Err("Сохранённое сообщение неизменяемо; создайте новую ветку.".into());
            }
        }
        tx.execute("INSERT INTO messages(session_id,message_id,parent_id,role,created_at,content_json,extras_json,seq) VALUES(?1,?2,?3,?4,?5,?6,?7,(SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE session_id=?1)) ON CONFLICT(session_id,message_id) DO UPDATE SET extras_json=excluded.extras_json,content_json=excluded.content_json",params![id,m.id,m.parent_id,m.role,m.created_at,content,serde_json::to_string(&m.extensions).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
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
    // Only the owner of a running attempt may replace its provisional snapshot.
    // Completed messages still go through the immutable save contract.
    pub fn checkpoint(&self, session: &str, m: &SessionMessage) -> Result<(), String> {
        if m.parts.is_empty() {
            return Ok(());
        }
        let mut db = self.connection.lock().map_err(|e| e.to_string())?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM runs r JOIN messages p ON p.session_id=r.session_id AND p.message_id=r.parent_id WHERE r.session_id=?1 AND r.id=?2 AND r.parent_id=?3 AND r.status='running')", params![session,m.id,m.parent_id], |r|r.get(0)).map_err(|e|e.to_string())?;
        if !valid || m.role != "assistant" {
            return Err("Запуск не найден или уже завершён.".into());
        }
        tx.execute("INSERT INTO messages(session_id,message_id,parent_id,role,created_at,content_json,extras_json,seq) VALUES(?1,?2,?3,'assistant',?4,?5,?6,(SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE session_id=?1)) ON CONFLICT(session_id,message_id) DO UPDATE SET content_json=excluded.content_json,extras_json=excluded.extras_json",params![session,m.id,m.parent_id,m.created_at,serde_json::to_string(&m.parts).map_err(|e|e.to_string())?,serde_json::to_string(&m.extensions).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
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
    pub fn server_timestamp(&self) -> Result<String, String> {
        self.connection
            .lock()
            .map_err(|e| e.to_string())?
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())
    }
    pub fn invalidate_integration(&self, session: &str, namespace: &str) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "DELETE FROM session_integration_state WHERE session_id=?1 AND namespace=?2",
                params![session, namespace],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn accept_run(
        &self,
        request: &crate::StartRunRequest,
        created_at: &str,
    ) -> Result<bool, String> {
        let mut db = self.connection.lock().map_err(|e| e.to_string())?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let payload = serde_json::to_value(request).map_err(|e| e.to_string())?;
        let old: Option<String> = tx
            .query_row(
                "SELECT request_json FROM runs WHERE session_id=?1 AND id=?2",
                params![request.session_id, request.request_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(old) = old {
            if serde_json::from_str::<Value>(&old).map_err(|e| e.to_string())? == payload {
                return Ok(false);
            }
            return Err("conflict: requestId already has a different payload".into());
        }
        let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND status='running') OR EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND message_id IN (?2,?3))",params![request.session_id,request.request_id,request.message.id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if busy {
            return Err("conflict: session busy or message ID already exists".into());
        }
        if let Some(parent) = &request.parent_id {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND message_id=?2)",
                    params![request.session_id, parent],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exists {
                return Err("parent is not in this session".into());
            }
        }
        let mut input = Vec::new();
        if !request.message.text.is_empty() {
            input.push(ContentPart::text("text", &request.message.text));
        }
        if let Some(images) = request.options["attachments"].as_array() {
            input.extend(images.iter().cloned().map(|data| ContentPart {
                kind: "image".into(),
                data,
            }));
        }
        crate::transcript::Transcript {
            parts: input.clone(),
            replay: Default::default(),
        }
        .validate()?;
        tx.execute("INSERT INTO messages(session_id,message_id,parent_id,role,created_at,content_json,extras_json,seq) VALUES(?1,?2,?3,'user',?4,?5,'{}',(SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE session_id=?1))",params![request.session_id,request.message.id,request.parent_id,created_at,serde_json::to_string(&input).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        tx.execute(
            "INSERT INTO runs VALUES(?1,?2,?3,?4,'running',NULL,unixepoch())",
            params![
                request.session_id,
                request.request_id,
                request.message.id,
                payload.to_string()
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE sessions SET title=CASE WHEN title='Новый диалог' THEN ?3 ELSE title END,updated_at=unixepoch(),model=?2 WHERE id=?1",
            params![request.session_id, request.model,request.message.text.chars().take(80).collect::<String>()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }
    pub fn run_snapshot(&self, session: &str, id: &str) -> Result<crate::RunSnapshot, String> {
        let db = self.connection.lock().map_err(|e| e.to_string())?;
        let (status,error,parts): (String,Option<String>,Option<String>) = db.query_row("SELECT r.status,r.error,m.content_json FROM runs r LEFT JOIN messages m ON m.session_id=r.session_id AND m.message_id=r.id WHERE r.session_id=?1 AND r.id=?2",params![session,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?.ok_or("run not found")?;
        Ok(crate::RunSnapshot {
            session_id: session.into(),
            run_id: id.into(),
            status,
            error,
            parts: parts
                .map(|v| serde_json::from_str(&v))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default(),
        })
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
    tx.execute_batch("CREATE TABLE session_integration_state(session_id TEXT NOT NULL REFERENCES sessions(id),namespace TEXT NOT NULL,state_json TEXT NOT NULL,PRIMARY KEY(session_id,namespace));").map_err(|e|e.to_string())?;
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
