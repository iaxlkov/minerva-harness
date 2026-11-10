//! Derived index of host-sanitized, visible history. The host owns the source.
use minerva_plugin_sdk::learning::{AccessContext, LearningError, Scope, SourceRef};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Mutex};

type Result<T> = std::result::Result<T, LearningError>;
fn err(code: &str, message: &str) -> LearningError {
    LearningError::new(code, message)
}
fn storage(_: impl std::fmt::Display) -> LearningError {
    err("STORAGE_ERROR", "history index unavailable")
}
fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub actor: String,
    pub workspace: Option<String>,
    pub source: SourceRef,
    pub text: String,
    pub surface: Surface,
    pub revision: String,
    /// Host marks inherited fork entries so they are not indexed twice.
    pub inherited: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Surface {
    Current,
    Superseded,
    LogOnly,
}
impl Surface {
    fn wire(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Superseded => "superseded",
            Self::LogOnly => "logOnly",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        match s {
            "current" => Ok(Self::Current),
            "superseded" => Ok(Self::Superseded),
            "logOnly" => Ok(Self::LogOnly),
            _ => Err(storage("surface")),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub generation: i64,
    pub offset: usize,
    pub binding: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub hits: Vec<Event>,
    pub next: Option<Cursor>,
}

#[derive(Clone, Debug)]
pub struct Alias {
    pub actor: String,
    pub fork_session: String,
    pub source: SourceRef,
}

/// Implementations must authorize and redact source data before returning it.
pub trait HistorySource {
    fn read(&self, access: &AccessContext, source: &SourceRef) -> Result<Event>;
    fn trace(
        &self,
        access: &AccessContext,
        session: &str,
        event: Option<i64>,
    ) -> Result<Vec<SourceRef>>;
}

pub struct HistoryService {
    db: Mutex<Connection>,
}
impl cordis::Service for HistoryService {
    const NAME: &'static str = "minerva.history";
}
impl HistoryService {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path).map_err(storage)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage)?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(storage)?;
        if version > 2 {
            return Err(storage("newer schema"));
        }
        if version < 2 {
            db.execute_batch("BEGIN; DROP TABLE IF EXISTS event_fts; DROP TABLE IF EXISTS events; DROP TABLE IF EXISTS checkpoint; COMMIT;").map_err(storage)?;
        }
        db.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS events(actor TEXT NOT NULL,workspace TEXT,session TEXT NOT NULL,seq INTEGER NOT NULL,message TEXT,run TEXT,digest TEXT NOT NULL,text TEXT NOT NULL,surface TEXT NOT NULL,revision TEXT NOT NULL,msgkey TEXT NOT NULL,PRIMARY KEY(actor,session,seq,msgkey)); CREATE VIRTUAL TABLE IF NOT EXISTS event_fts USING fts5(text,content='events',content_rowid='rowid'); CREATE TABLE IF NOT EXISTS aliases(actor TEXT NOT NULL,fork TEXT NOT NULL,session TEXT NOT NULL,seq INTEGER NOT NULL,message TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(actor,fork,session,seq,message)); CREATE TABLE IF NOT EXISTS checkpoint(id INTEGER PRIMARY KEY CHECK(id=1),watermark INTEGER NOT NULL,generation INTEGER NOT NULL); INSERT OR IGNORE INTO checkpoint VALUES(1,0,0); PRAGMA user_version=2; COMMIT;").map_err(storage)?;
        Ok(Self { db: Mutex::new(db) })
    }
    fn apply(
        &self,
        watermark: i64,
        events: &[Event],
        deleted: &[(String, String)],
        rebuild: bool,
        aliases: &[Alias],
    ) -> Result<()> {
        if deleted.len() > 1000 || watermark < 0 {
            return Err(err("INVALID_INPUT", "invalid batch"));
        }
        for e in events {
            e.source.validate()?;
            if e.actor.is_empty()
                || e.source.event_seq.is_none()
                || e.text.len() > 1_048_576
                || e.revision.is_empty()
                || digest(&e.text) != e.source.content_digest
            {
                return Err(err("INVALID_INPUT", "invalid projection"));
            }
        }
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        let previous: i64 = tx
            .query_row("SELECT watermark FROM checkpoint WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(storage)?;
        if !rebuild && watermark < previous {
            return Err(err("INVALID_INPUT", "watermark regression"));
        }
        if rebuild {
            tx.execute("DELETE FROM aliases", []).map_err(storage)?;
            tx.execute("DELETE FROM events", []).map_err(storage)?;
        }
        for (actor, session) in deleted {
            tx.execute(
                "DELETE FROM events WHERE actor=?1 AND session=?2",
                params![actor, session],
            )
            .map_err(storage)?;
        }
        for e in events.iter().filter(|e| !e.inherited) {
            tx.execute("INSERT INTO events(actor,workspace,session,seq,message,run,digest,text,surface,revision,msgkey) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,COALESCE(?5,'')) ON CONFLICT(actor,session,seq,msgkey) DO UPDATE SET workspace=excluded.workspace,message=excluded.message,run=excluded.run,digest=excluded.digest,text=excluded.text,surface=excluded.surface,revision=excluded.revision", params![e.actor,e.workspace,e.source.session_id,e.source.event_seq,e.source.message_id,e.source.run_id,e.source.content_digest,e.text,e.surface.wire(),e.revision]).map_err(storage)?;
        }
        for alias in aliases {
            alias.source.validate()?;
            let seq = alias
                .source
                .event_seq
                .ok_or_else(|| err("INVALID_INPUT", "alias requires event sequence"))?;
            let message = alias
                .source
                .message_id
                .as_deref()
                .ok_or_else(|| err("INVALID_INPUT", "alias requires message"))?;
            if alias.actor.is_empty() || alias.fork_session.is_empty() {
                return Err(err("INVALID_INPUT", "invalid alias"));
            }
            tx.execute(
                "INSERT INTO aliases VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    alias.actor,
                    alias.fork_session,
                    alias.source.session_id,
                    seq,
                    message,
                    alias.source.content_digest
                ],
            )
            .map_err(storage)?;
        }
        tx.execute("INSERT INTO event_fts(event_fts) VALUES('rebuild')", [])
            .map_err(storage)?;
        tx.execute(
            "UPDATE checkpoint SET watermark=?1,generation=generation+1 WHERE id=1",
            [watermark],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)
    }
    pub fn replay(
        &self,
        watermark: i64,
        events: &[Event],
        deleted: &[(String, String)],
    ) -> Result<()> {
        self.apply(watermark, events, deleted, false, &[])
    }
    /// Rebuild is atomic and leaves the authoritative history untouched.
    pub fn rebuild(&self, watermark: i64, events: &[Event]) -> Result<()> {
        self.apply(watermark, events, &[], true, &[])
    }
    pub fn rebuild_with_aliases(
        &self,
        watermark: i64,
        events: &[Event],
        aliases: &[Alias],
    ) -> Result<()> {
        self.apply(watermark, events, &[], true, aliases)
    }
    pub fn watermark(&self) -> Result<i64> {
        self.db
            .lock()
            .map_err(storage)?
            .query_row("SELECT watermark FROM checkpoint WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(storage)
    }
    pub fn search(
        &self,
        access: &AccessContext,
        scope: &Scope,
        session: Option<&str>,
        query: &str,
        limit: usize,
        cursor: Option<&Cursor>,
    ) -> Result<Page> {
        access.authorize(scope)?;
        if matches!(scope, Scope::User)
            || query.trim().is_empty()
            || query.len() > 2048
            || !(1..=100).contains(&limit)
        {
            return Err(err("INVALID_INPUT", "invalid history query"));
        }
        if let Scope::Session { session_id } = scope {
            if session.is_some_and(|s| s != session_id) {
                return Err(err("NOT_FOUND_OR_FORBIDDEN", "session unavailable"));
            }
        }
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        let binding = digest(&format!(
            "{:?}\0{}\0{:?}\0{}\0{}",
            scope, access.actor_id, session, query, limit
        ));
        let db = self.db.lock().map_err(storage)?;
        let generation: i64 = db
            .query_row("SELECT generation FROM checkpoint WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(storage)?;
        let offset = if let Some(c) = cursor {
            if c.generation != generation || c.binding != binding || c.offset > 100_000 {
                return Err(err("INVALID_INPUT", "invalid cursor"));
            }
            c.offset
        } else {
            0
        };
        let (workspace, session_filter) = match scope {
            Scope::Workspace { workspace_id } => (Some(workspace_id.as_str()), session),
            Scope::Session { session_id } => (None, Some(session_id.as_str())),
            Scope::User => unreachable!(),
        };
        let mut stmt = db.prepare("SELECT e.session,e.workspace,e.seq,e.message,e.run,e.digest,e.text,e.surface,e.revision FROM event_fts JOIN events e ON e.rowid=event_fts.rowid WHERE event_fts MATCH ?1 AND e.actor=?2 AND (?3 IS NULL OR e.workspace=?3) AND (?4 IS NULL OR e.session=?4 OR (?3 IS NOT NULL AND EXISTS(SELECT 1 FROM aliases a WHERE a.actor=e.actor AND a.fork=?4 AND a.session=e.session AND a.seq=e.seq AND a.message=e.msgkey AND a.digest=e.digest))) ORDER BY e.session,e.seq,e.msgkey LIMIT ?5 OFFSET ?6").map_err(storage)?;
        let rows = stmt
            .query_map(
                params![
                    phrase,
                    access.actor_id,
                    workspace,
                    session_filter,
                    (limit + 1) as i64,
                    offset as i64
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                    ))
                },
            )
            .map_err(storage)?;
        let mut hits = Vec::new();
        for row in rows {
            let (session, workspace, seq, message, run, digest, text, surface, revision) =
                row.map_err(storage)?;
            hits.push(Event {
                actor: access.actor_id.clone(),
                workspace,
                source: SourceRef {
                    session_id: session,
                    message_id: message,
                    event_seq: Some(seq),
                    run_id: run,
                    content_digest: digest,
                },
                text,
                surface: Surface::parse(&surface)?,
                revision,
                inherited: false,
            });
        }
        let next = if hits.len() > limit {
            hits.pop();
            Some(Cursor {
                generation,
                offset: offset + limit,
                binding,
            })
        } else {
            None
        };
        Ok(Page { hits, next })
    }
    pub fn read_event(
        &self,
        host: &dyn HistorySource,
        access: &AccessContext,
        scope: &Scope,
        source: &SourceRef,
    ) -> Result<Event> {
        access.authorize(scope)?;
        source.validate()?;
        let db = self.db.lock().map_err(storage)?;
        let permitted: Option<String> = match scope {
            Scope::Session { session_id } if session_id == &source.session_id => db.query_row("SELECT digest FROM events WHERE actor=?1 AND session=?2 AND seq=?3 AND message IS ?4", params![access.actor_id,source.session_id,source.event_seq,source.message_id], |r| r.get(0)).optional().map_err(storage)?,
            Scope::Workspace { workspace_id } => db.query_row("SELECT digest FROM events WHERE actor=?1 AND workspace=?2 AND session=?3 AND seq=?4 AND message IS ?5", params![access.actor_id,workspace_id,source.session_id,source.event_seq,source.message_id], |r| r.get(0)).optional().map_err(storage)?,
            _ => None,
        };
        drop(db);
        let indexed =
            permitted.ok_or_else(|| err("NOT_FOUND_OR_FORBIDDEN", "source unavailable"))?;
        if indexed != source.content_digest {
            return Err(err("SOURCE_CHANGED", "source changed"));
        }
        let event = host.read(access, source)?;
        if event.source != *source
            || event.actor != access.actor_id
            || digest(&event.text) != source.content_digest
        {
            return Err(err("SOURCE_CHANGED", "source changed"));
        }
        Ok(event)
    }
    pub fn trace(
        &self,
        host: &dyn HistorySource,
        access: &AccessContext,
        scope: &Scope,
        source: &SourceRef,
    ) -> Result<Vec<SourceRef>> {
        self.read_event(host, access, scope, source)?;
        let mut refs = host.trace(access, &source.session_id, source.event_seq)?;
        if let Scope::Session { session_id } = scope { refs.retain(|r| &r.session_id == session_id); }
        if refs.len() > 100 {
            return Err(err("INVALID_INPUT", "trace exceeds limit"));
        }
        for r in &refs {
            r.validate()?;
        }
        Ok(refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host(Vec<Event>);
    impl HistorySource for Host {
        fn read(&self, _: &AccessContext, source: &SourceRef) -> Result<Event> {
            self.0
                .iter()
                .find(|e| {
                    e.source.session_id == source.session_id
                        && e.source.event_seq == source.event_seq
                })
                .cloned()
                .ok_or_else(|| err("SOURCE_UNAVAILABLE", "missing"))
        }
        fn trace(
            &self,
            _: &AccessContext,
            session: &str,
            _: Option<i64>,
        ) -> Result<Vec<SourceRef>> {
            Ok(self
                .0
                .iter()
                .filter(|e| e.source.session_id == session)
                .map(|e| e.source.clone())
                .collect())
        }
    }
    fn event(session: &str, workspace: &str, seq: i64, text: &str) -> Event {
        Event {
            actor: "actor".into(),
            workspace: Some(workspace.into()),
            source: SourceRef {
                session_id: session.into(),
                message_id: None,
                event_seq: Some(seq),
                run_id: None,
                content_digest: digest(text),
            },
            text: text.into(),
            surface: Surface::Current,
            revision: "1".into(),
            inherited: false,
        }
    }
    fn access() -> AccessContext {
        AccessContext {
            actor_id: "actor".into(),
            session_id: "new".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        }
    }
    #[test]
    fn isolation_paging_rebuild_delete_and_sources() {
        let db = HistoryService::open(Path::new(":memory:")).unwrap();
        let a = event("old", "w", 1, "hello world");
        let b = event("old", "w", 2, "hello again");
        let other = event("private", "other", 1, "hello secret");
        let mut inherited = a.clone();
        inherited.source.session_id = "fork".into();
        inherited.inherited = true;
        db.replay(1, &[a.clone(), b.clone(), other, inherited], &[])
            .unwrap();
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let first = db
            .search(&access(), &scope, None, "hello", 1, None)
            .unwrap();
        assert_eq!(first.hits, vec![a.clone()]);
        let second = db
            .search(&access(), &scope, None, "hello", 1, first.next.as_ref())
            .unwrap();
        assert_eq!(second.hits, vec![b.clone()]);
        assert!(db
            .search(
                &access(),
                &Scope::Workspace {
                    workspace_id: "other".into()
                },
                None,
                "hello",
                10,
                None
            )
            .is_err());
        let host = Host(vec![a.clone(), b.clone()]);
        assert_eq!(
            db.read_event(&host, &access(), &scope, &a.source).unwrap(),
            a
        );
        assert_eq!(
            db.trace(&host, &access(), &scope, &b.source).unwrap().len(),
            2
        );
        let changed = Host(vec![event("old", "w", 1, "changed")]);
        assert_eq!(
            db.read_event(&changed, &access(), &scope, &a.source)
                .unwrap_err()
                .code,
            "SOURCE_CHANGED"
        );
        db.replay(2, &[], &[("actor".into(), "old".into())])
            .unwrap();
        assert!(db
            .search(&access(), &scope, None, "hello", 1, first.next.as_ref())
            .is_err());
        assert!(db
            .search(&access(), &scope, None, "hello", 10, None)
            .unwrap()
            .hits
            .is_empty());
        db.rebuild(3, std::slice::from_ref(&b)).unwrap();
        assert_eq!(
            db.search(&access(), &scope, None, "hello", 10, None)
                .unwrap()
                .hits,
            vec![b]
        );
        assert_eq!(db.watermark().unwrap(), 3);
        assert!(db.replay(2, &[], &[]).is_err());
    }
    #[test]
    fn failed_batch_does_not_advance_checkpoint() {
        let db = HistoryService::open(Path::new(":memory:")).unwrap();
        let mut bad = event("s", "w", 1, "hello");
        bad.text = "changed".into();
        assert!(db.replay(4, &[bad], &[]).is_err());
        assert_eq!(db.watermark().unwrap(), 0);
        assert!(db
            .search(
                &access(),
                &Scope::Workspace {
                    workspace_id: "w".into()
                },
                None,
                "hello\" OR secret",
                10,
                None
            )
            .unwrap()
            .hits
            .is_empty());
    }
}
