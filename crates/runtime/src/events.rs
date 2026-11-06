//! Durable projection events. Triggers share the mutation's SQLite transaction.
use crate::sessions::{SessionBaseline, SessionEvent, SessionInfo, SessionMessage};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub(crate) fn baseline(db: &Connection, id: &str) -> Result<SessionBaseline, String> {
    let session = db
        .query_row(
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
        .ok_or("session not found")?;
    let mut q = db.prepare("SELECT m.message_id,m.parent_id,m.role,m.created_at,m.content_json,CASE WHEN r.id IS NULL THEN m.extras_json ELSE json_set(m.extras_json,'$.execution',json_object('status',r.status,'error',r.error)) END FROM messages m LEFT JOIN runs r ON r.session_id=m.session_id AND r.id=m.message_id WHERE m.session_id=?1 ORDER BY m.seq").map_err(|e|e.to_string())?;
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
    let mut messages = Vec::new();
    for row in rows {
        let (id, parent_id, role, created_at, parts, extensions) =
            row.map_err(|e| e.to_string())?;
        messages.push(SessionMessage {
            id,
            parent_id,
            role,
            created_at,
            parts: serde_json::from_str(&parts).map_err(|e| e.to_string())?,
            extensions: serde_json::from_str(&extensions).map_err(|e| e.to_string())?,
        });
    }
    let active:Option<(String,String,Option<String>,Option<String>)>=db.query_row("SELECT r.id,r.status,r.error,m.content_json FROM runs r LEFT JOIN messages m ON m.session_id=r.session_id AND m.message_id=r.id WHERE r.session_id=?1 AND r.status='running' ORDER BY r.created_at DESC LIMIT 1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
    let active_run = active
        .map(|(run_id, status, error, parts)| {
            Ok::<_, String>(crate::RunSnapshot {
                session_id: id.into(),
                run_id,
                status,
                error,
                parts: parts
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default(),
            })
        })
        .transpose()?;
    let cursor = db
        .query_row(
            "SELECT COALESCE(MAX(seq),0) FROM session_events WHERE session_id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(SessionBaseline {
        session,
        messages,
        cursor,
        active_run,
    })
}
pub(crate) fn tail(
    db: &Connection,
    id: &str,
    cursor: i64,
    limit: usize,
) -> Result<Vec<SessionEvent>, String> {
    if cursor < 0 || limit == 0 || limit > 1000 {
        return Err("cursor must be nonnegative and limit must be 1..=1000".into());
    }
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("session not found".into());
    }
    let mut q=db.prepare("SELECT seq,kind,data_json FROM session_events WHERE session_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![id, cursor, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.map(|row| {
        let (seq, kind, data) = row.map_err(|e| e.to_string())?;
        Ok(SessionEvent {
            session_id: id.into(),
            seq,
            kind,
            data: serde_json::from_str::<Value>(&data).map_err(|e| e.to_string())?,
        })
    })
    .collect()
}
pub(crate) fn migrate(db: &mut Connection) -> Result<(), String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS session_events(session_id TEXT NOT NULL REFERENCES sessions(id),seq INTEGER NOT NULL,kind TEXT NOT NULL,data_json TEXT NOT NULL,PRIMARY KEY(session_id,seq));").map_err(|e|e.to_string())?;
    let ids = {
        let mut q=tx.prepare("SELECT id FROM sessions WHERE NOT EXISTS(SELECT 1 FROM session_events WHERE session_id=sessions.id)").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    for id in ids {
        let mut snapshot = baseline(&tx, &id)?;
        snapshot.cursor = 1;
        tx.execute(
            "INSERT INTO session_events VALUES(?1,1,'history.snapshot',?2)",
            params![
                id,
                serde_json::to_string(&snapshot).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.execute_batch(r#"CREATE TRIGGER IF NOT EXISTS events_session_insert AFTER INSERT ON sessions  BEGIN INSERT INTO session_events(session_id,seq,kind,data_json) VALUES(NEW.id,(SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=NEW.id),'session.updated',json_object('id',NEW.id,'title',NEW.title,'updatedAt',NEW.updated_at,'model',NEW.model)); END;CREATE TRIGGER IF NOT EXISTS events_session_update AFTER UPDATE ON sessions WHEN OLD.title IS NOT NEW.title OR OLD.model IS NOT NEW.model OR OLD.updated_at IS NOT NEW.updated_at BEGIN INSERT INTO session_events(session_id,seq,kind,data_json) VALUES(NEW.id,(SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=NEW.id),'session.updated',json_object('id',NEW.id,'title',NEW.title,'updatedAt',NEW.updated_at,'model',NEW.model)); END;CREATE TRIGGER IF NOT EXISTS events_message_insert AFTER INSERT ON messages  BEGIN INSERT INTO session_events(session_id,seq,kind,data_json) VALUES(NEW.session_id,(SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=NEW.session_id),'message.upsert',json_object('id',NEW.message_id,'parentId',NEW.parent_id,'role',NEW.role,'createdAt',NEW.created_at,'parts',json(NEW.content_json),'extensions',json(NEW.extras_json))); END;CREATE TRIGGER IF NOT EXISTS events_message_update AFTER UPDATE ON messages WHEN OLD.content_json IS NOT NEW.content_json OR OLD.extras_json IS NOT NEW.extras_json BEGIN INSERT INTO session_events(session_id,seq,kind,data_json) VALUES(NEW.session_id,(SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=NEW.session_id),'message.upsert',json_object('id',NEW.message_id,'parentId',NEW.parent_id,'role',NEW.role,'createdAt',NEW.created_at,'parts',json(NEW.content_json),'extensions',json(NEW.extras_json))); END;CREATE TRIGGER IF NOT EXISTS events_run_insert AFTER INSERT ON runs  BEGIN INSERT INTO session_events(session_id,seq,kind,data_json) VALUES(NEW.session_id,(SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=NEW.session_id),'run.updated',json_object('sessionId',NEW.session_id,'runId',NEW.id,'status',NEW.status,'error',NEW.error,'parts',json(COALESCE((SELECT content_json FROM messages WHERE session_id=NEW.session_id AND message_id=NEW.id),'[]')))); END;CREATE TRIGGER IF NOT EXISTS events_run_update AFTER UPDATE ON runs WHEN OLD.status IS NOT NEW.status OR OLD.error IS NOT NEW.error BEGIN INSERT INTO session_events(session_id,seq,kind,data_json) VALUES(NEW.session_id,(SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=NEW.session_id),'run.updated',json_object('sessionId',NEW.session_id,'runId',NEW.id,'status',NEW.status,'error',NEW.error,'parts',json(COALESCE((SELECT content_json FROM messages WHERE session_id=NEW.session_id AND message_id=NEW.id),'[]')))); END;PRAGMA user_version=5;"#).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
