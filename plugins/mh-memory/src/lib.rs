//! Scoped, durable memory. The host supplies the access capability.
use minerva_plugin_sdk::learning::{AccessContext, LearningError, Scope, SourceRef};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Mutex};

type Result<T> = std::result::Result<T, LearningError>;
fn error(code: &str) -> LearningError {
    LearningError::new(code, code)
}
fn storage(_: impl std::fmt::Display) -> LearningError {
    error("STORAGE_ERROR")
}
fn key(scope: &Scope) -> Result<String> {
    serde_json::to_string(scope).map_err(storage)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub revision: i64,
    pub scope: Scope,
    pub kind: String,
    pub text: String,
    pub cue: Option<String>,
    pub source_refs: Vec<SourceRef>,
    pub origin: String,
    pub status: String,
    pub confidence: Option<f64>,
    pub expires_at: Option<i64>,
    pub supersedes: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Write {
    pub record: Record,
    pub expected_revision: Option<i64>,
}

pub struct MemoryService {
    db: Mutex<Connection>,
}
impl cordis::Service for MemoryService {
    const NAME: &'static str = "minerva.memory";
}
impl MemoryService {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path).map_err(storage)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage)?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(storage)?;
        if version > 1 {
            return Err(error("UNSUPPORTED_SCHEMA"));
        }
        db.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS records(actor TEXT NOT NULL, scope TEXT NOT NULL, id TEXT NOT NULL, revision INTEGER NOT NULL, status TEXT NOT NULL, digest TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(actor,scope,id)); CREATE TABLE IF NOT EXISTS revisions(actor TEXT NOT NULL,scope TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,body TEXT NOT NULL,PRIMARY KEY(actor,scope,id,revision)); CREATE TABLE IF NOT EXISTS requests(actor TEXT NOT NULL,request TEXT NOT NULL,digest TEXT NOT NULL,result TEXT NOT NULL,PRIMARY KEY(actor,request)); CREATE TABLE IF NOT EXISTS audit(actor TEXT NOT NULL,scope TEXT NOT NULL,action TEXT NOT NULL,ids TEXT NOT NULL); CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(actor UNINDEXED,scope UNINDEXED,id UNINDEXED,text); CREATE TABLE IF NOT EXISTS unavailable_sources(actor TEXT NOT NULL,scope TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(actor,scope,source)); CREATE TABLE IF NOT EXISTS proposals(actor TEXT NOT NULL,id TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(actor,id)); CREATE TABLE IF NOT EXISTS consumer_cursor(name TEXT PRIMARY KEY,position TEXT NOT NULL); PRAGMA user_version=1; COMMIT;").map_err(storage)?;
        Ok(Self { db: Mutex::new(db) })
    }
    pub fn upsert(
        &self,
        access: &AccessContext,
        request_id: &str,
        writes: &[Write],
    ) -> Result<Vec<Record>> {
        if request_id.is_empty()
            || request_id.len() > 256
            || writes.is_empty()
            || writes.len() > 100
        {
            return Err(error("INVALID_INPUT"));
        }
        for w in writes {
            access.authorize(&w.record.scope)?;
            let r = &w.record;
            if r.id.is_empty()
                || r.id.len() > 256
                || r.text.trim().is_empty()
                || r.text.len() > 16384
                || !["fact", "preference", "lesson", "procedure"].contains(&r.kind.as_str())
                || !["userExplicit", "userCorrection", "inferred"].contains(&r.origin.as_str())
                || r.source_refs.len() > 100
                || (r.origin == "inferred" && r.source_refs.is_empty())
                || r.confidence
                    .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            {
                return Err(error("INVALID_INPUT"));
            }
            for source in &r.source_refs {
                source.validate()?;
            }
        }
        let payload = serde_json::to_string(writes).map_err(storage)?;
        let digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        if let Some((old, result)) = tx
            .query_row(
                "SELECT digest,result FROM requests WHERE actor=?1 AND request=?2",
                params![access.actor_id, request_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(storage)?
        {
            if old != digest {
                return Err(error("IDEMPOTENCY_CONFLICT"));
            }
            return serde_json::from_str(&result).map_err(storage);
        }
        let mut result = Vec::new();
        for w in writes {
            let mut record = w.record.clone();
            let scope = key(&record.scope)?;
            for source in &record.source_refs {
                let encoded = serde_json::to_string(source).map_err(storage)?;
                let unavailable:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM unavailable_sources WHERE actor=?1 AND scope=?2 AND source=?3)",params![access.actor_id,scope,encoded],|r|r.get(0)).map_err(storage)?;
                if unavailable {
                    return Err(error("SOURCE_UNAVAILABLE"));
                }
            }
            let current: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM records WHERE actor=?1 AND scope=?2 AND id=?3",
                    params![access.actor_id, scope, record.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if current != w.expected_revision {
                return Err(error("REVISION_CONFLICT"));
            }
            record.revision = current.unwrap_or(0) + 1;
            record.status = if record.origin == "inferred" {
                "pending"
            } else {
                "active"
            }
            .into();
            let normalized = record
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let text_digest = format!("{:x}", Sha256::digest(normalized.as_bytes()));
            let duplicate: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM records WHERE actor=?1 AND scope=?2 AND digest=?3 AND id<>?4 AND status IN ('active','pending'))",params![access.actor_id,scope,text_digest,record.id],|r|r.get(0)).map_err(storage)?;
            if duplicate {
                return Err(error("DUPLICATE"));
            }
            if let Some(cue) = record.cue.as_deref().filter(|v| !v.trim().is_empty()) {
                let bodies = {
                    let mut stmt=tx.prepare("SELECT body FROM records WHERE actor=?1 AND scope=?2 AND id<>?3 AND status='active'").map_err(storage)?;
                    let rows = stmt
                        .query_map(params![access.actor_id, scope, record.id], |r| {
                            r.get::<_, String>(0)
                        })
                        .map_err(storage)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(storage)?
                };
                for old in bodies {
                    let mut other: Record = serde_json::from_str(&old).map_err(storage)?;
                    if other
                        .cue
                        .as_deref()
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case(cue.trim()))
                        && other.text != record.text
                    {
                        record.status = "pending".into();
                        other.status = "pending".into();
                        other.revision += 1;
                        let body = serde_json::to_string(&other).map_err(storage)?;
                        tx.execute("UPDATE records SET status='pending',revision=?4,body=?5 WHERE actor=?1 AND scope=?2 AND id=?3",params![access.actor_id,scope,other.id,other.revision,body]).map_err(storage)?;
                        tx.execute(
                            "INSERT INTO revisions VALUES(?1,?2,?3,?4,?5)",
                            params![access.actor_id, scope, other.id, other.revision, body],
                        )
                        .map_err(storage)?;
                        tx.execute(
                            "DELETE FROM memory_fts WHERE actor=?1 AND scope=?2 AND id=?3",
                            params![access.actor_id, scope, other.id],
                        )
                        .map_err(storage)?;
                    }
                }
            }
            let body = serde_json::to_string(&record).map_err(storage)?;
            tx.execute("INSERT INTO records VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(actor,scope,id) DO UPDATE SET revision=excluded.revision,status=excluded.status,digest=excluded.digest,body=excluded.body",params![access.actor_id,scope,record.id,record.revision,record.status,text_digest,body]).map_err(storage)?;
            tx.execute(
                "INSERT INTO revisions VALUES(?1,?2,?3,?4,?5)",
                params![access.actor_id, scope, record.id, record.revision, body],
            )
            .map_err(storage)?;
            tx.execute(
                "DELETE FROM memory_fts WHERE actor=?1 AND scope=?2 AND id=?3",
                params![access.actor_id, scope, record.id],
            )
            .map_err(storage)?;
            if record.status == "active" {
                tx.execute(
                    "INSERT INTO memory_fts VALUES(?1,?2,?3,?4)",
                    params![access.actor_id, scope, record.id, record.text],
                )
                .map_err(storage)?;
            }
            tx.execute(
                "INSERT INTO audit VALUES(?1,?2,'upsert',?3)",
                params![access.actor_id, scope, record.id],
            )
            .map_err(storage)?;
            result.push(record);
        }
        tx.execute(
            "INSERT INTO requests VALUES(?1,?2,?3,?4)",
            params![
                access.actor_id,
                request_id,
                digest,
                serde_json::to_string(&result).map_err(storage)?
            ],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    pub fn read(&self, access: &AccessContext, scope: &Scope, id: &str) -> Result<Record> {
        access.authorize(scope)?;
        let db = self.db.lock().map_err(storage)?;
        let body: String=db.query_row("SELECT body FROM records WHERE actor=?1 AND scope=?2 AND id=?3 AND status<>'retracted'",params![access.actor_id,key(scope)?,id],|r|r.get(0)).optional().map_err(storage)?.ok_or_else(||error("NOT_FOUND_OR_FORBIDDEN"))?;
        serde_json::from_str(&body).map_err(storage)
    }
    pub fn recall(
        &self,
        access: &AccessContext,
        scope: &Scope,
        query: &str,
        now: i64,
    ) -> Result<Vec<Record>> {
        access.authorize(scope)?;
        if query.len() > 4096 {
            return Err(error("INVALID_INPUT"));
        }
        let db = self.db.lock().map_err(storage)?;
        let terms: Vec<String> = query
            .split_whitespace()
            .take(32)
            .filter(|t| t.chars().any(char::is_alphanumeric))
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect();
        let expression = terms.join(" OR ");
        let sql = if expression.is_empty() {
            "SELECT body FROM records WHERE actor=?1 AND scope=?2 AND status='active' AND ?3='' ORDER BY id LIMIT 256"
        } else {
            "SELECT r.body FROM memory_fts f JOIN records r ON r.actor=f.actor AND r.scope=f.scope AND r.id=f.id WHERE f.actor=?1 AND f.scope=?2 AND memory_fts MATCH ?3 AND r.status='active' ORDER BY r.id LIMIT 256"
        };
        let mut stmt = db.prepare(sql).map_err(storage)?;
        let rows = stmt
            .query_map(params![access.actor_id, key(scope)?, expression], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?;
        let mut records = Vec::new();
        let mut budget = 0;
        for row in rows {
            let r: Record = serde_json::from_str(&row.map_err(storage)?).map_err(storage)?;
            if r.expires_at.is_some_and(|t| t <= now) {
                continue;
            }
            let size = r.text.chars().count().div_ceil(4);
            if budget + size > 1200 {
                continue;
            }
            budget += size;
            records.push(r);
            if records.len() == 8 {
                break;
            }
        }
        db.execute(
            "INSERT INTO audit VALUES(?1,?2,'recall',?3)",
            params![
                access.actor_id,
                key(scope)?,
                serde_json::to_string(&records.iter().map(|r| &r.id).collect::<Vec<_>>())
                    .map_err(storage)?
            ],
        )
        .map_err(storage)?;
        Ok(records)
    }
}
impl MemoryService {
    /// Retract immediately, or physically erase content and retain a minimal tombstone.
    pub fn forget(
        &self,
        access: &AccessContext,
        scope: &Scope,
        id: &str,
        purge: bool,
    ) -> Result<()> {
        access.authorize(scope)?;
        let scope = key(scope)?;
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        let body: String = tx
            .query_row(
                "SELECT body FROM records WHERE actor=?1 AND scope=?2 AND id=?3",
                params![access.actor_id, scope, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| error("NOT_FOUND_OR_FORBIDDEN"))?;
        if purge {
            tx.execute(
                "DELETE FROM revisions WHERE actor=?1 AND scope=?2 AND id=?3",
                params![access.actor_id, scope, id],
            )
            .map_err(storage)?;
            tx.execute(
                "DELETE FROM requests WHERE actor=?1",
                params![access.actor_id],
            )
            .map_err(storage)?;
            tx.execute(
                "DELETE FROM proposals WHERE actor=?1",
                params![access.actor_id],
            )
            .map_err(storage)?;
            tx.execute("UPDATE records SET body='',digest='',status='retracted' WHERE actor=?1 AND scope=?2 AND id=?3",params![access.actor_id,scope,id]).map_err(storage)?;
        } else {
            let mut record: Record = serde_json::from_str(&body).map_err(storage)?;
            record.status = "retracted".into();
            record.revision += 1;
            let updated = serde_json::to_string(&record).map_err(storage)?;
            tx.execute("UPDATE records SET body=?4,status='retracted',revision=?5 WHERE actor=?1 AND scope=?2 AND id=?3",params![access.actor_id,scope,id,updated,record.revision]).map_err(storage)?;
            tx.execute(
                "INSERT INTO revisions VALUES(?1,?2,?3,?4,?5)",
                params![access.actor_id, scope, id, record.revision, updated],
            )
            .map_err(storage)?;
        }
        tx.execute(
            "DELETE FROM memory_fts WHERE actor=?1 AND scope=?2 AND id=?3",
            params![access.actor_id, scope, id],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO audit VALUES(?1,?2,?3,?4)",
            params![
                access.actor_id,
                scope,
                if purge { "purge" } else { "forget" },
                id
            ],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)
    }
    pub fn export(&self, access: &AccessContext, scope: &Scope) -> Result<Vec<Record>> {
        access.authorize(scope)?;
        let db = self.db.lock().map_err(storage)?;
        let mut stmt=db.prepare("SELECT body FROM records WHERE actor=?1 AND scope=?2 AND status<>'retracted' ORDER BY id LIMIT 1000").map_err(storage)?;
        let rows = stmt
            .query_map(params![access.actor_id, key(scope)?], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?;
        rows.map(|row| serde_json::from_str(&row.map_err(storage)?).map_err(storage))
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_cas_recall_and_forget() {
        let db = MemoryService::open(Path::new(":memory:")).unwrap();
        let ctx = AccessContext {
            actor_id: "a".into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let record = Record {
            id: "1".into(),
            revision: 0,
            scope: scope.clone(),
            kind: "fact".into(),
            text: "test fact".into(),
            cue: None,
            source_refs: vec![],
            origin: "userExplicit".into(),
            status: "pending".into(),
            confidence: None,
            expires_at: None,
            supersedes: None,
        };
        let writes = vec![Write {
            record,
            expected_revision: None,
        }];
        assert_eq!(
            db.upsert(&ctx, "r", &writes).unwrap(),
            db.upsert(&ctx, "r", &writes).unwrap()
        );
        assert_eq!(
            db.upsert(&ctx, "other", &writes).unwrap_err().code,
            "REVISION_CONFLICT"
        );
        assert_eq!(db.recall(&ctx, &scope, "fact", 0).unwrap().len(), 1);
        assert!(db
            .recall(
                &ctx,
                &Scope::Workspace {
                    workspace_id: "other".into()
                },
                "",
                0
            )
            .is_err());
        db.forget(&ctx, &scope, "1", true).unwrap();
        assert!(db.recall(&ctx, &scope, "", 0).unwrap().is_empty());
        assert!(db.export(&ctx, &scope).unwrap().is_empty());
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsolidationProposal {
    pub writes: Vec<Write>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub scope: String,
    pub action: String,
    pub ids: String,
}
impl MemoryService {
    pub fn propose_consolidation(
        &self,
        access: &AccessContext,
        writes: Vec<Write>,
    ) -> Result<ConsolidationProposal> {
        if writes.is_empty() || writes.len() > 100 {
            return Err(error("INVALID_INPUT"));
        }
        for w in &writes {
            access.authorize(&w.record.scope)?;
            let old = self.read(access, &w.record.scope, &w.record.id)?;
            if Some(old.revision) != w.expected_revision {
                return Err(error("REVISION_CONFLICT"));
            }
        }
        let proposal = ConsolidationProposal { writes };
        let body = serde_json::to_string(&proposal).map_err(storage)?;
        let id = format!("{:x}", Sha256::digest(body.as_bytes()));
        let db = self.db.lock().map_err(storage)?;
        db.execute(
            "INSERT OR IGNORE INTO proposals VALUES(?1,?2,?3)",
            params![access.actor_id, id, body],
        )
        .map_err(storage)?;
        Ok(proposal)
    }
    pub fn apply_consolidation(
        &self,
        access: &AccessContext,
        request_id: &str,
        proposal: &ConsolidationProposal,
    ) -> Result<Vec<Record>> {
        let body = serde_json::to_string(proposal).map_err(storage)?;
        let id = format!("{:x}", Sha256::digest(body.as_bytes()));
        {
            let db = self.db.lock().map_err(storage)?;
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM proposals WHERE actor=?1 AND id=?2 AND body=?3)",
                    params![access.actor_id, id, body],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if !exists {
                return Err(error("NOT_FOUND_OR_FORBIDDEN"));
            }
        }
        self.upsert(access, request_id, &proposal.writes)
    }
    pub fn audit(&self, access: &AccessContext, scope: &Scope) -> Result<Vec<AuditEntry>> {
        access.authorize(scope)?;
        let db = self.db.lock().map_err(storage)?;
        let mut stmt=db.prepare("SELECT scope,action,ids FROM audit WHERE actor=?1 AND scope=?2 ORDER BY rowid DESC LIMIT 1000").map_err(storage)?;
        let rows = stmt
            .query_map(params![access.actor_id, key(scope)?], |r| {
                Ok(AuditEntry {
                    scope: r.get(0)?,
                    action: r.get(1)?,
                    ids: r.get(2)?,
                })
            })
            .map_err(storage)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)
    }
    /// Host calls this when an exact source becomes unavailable.
    pub fn invalidate_source(
        &self,
        access: &AccessContext,
        scope: &Scope,
        source: &SourceRef,
    ) -> Result<usize> {
        access.authorize(scope)?;
        source.validate()?;
        let scope = key(scope)?;
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS unavailable_sources(actor TEXT NOT NULL,scope TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(actor,scope,source))").map_err(storage)?;
        let encoded = serde_json::to_string(source).map_err(storage)?;
        tx.execute(
            "INSERT OR IGNORE INTO unavailable_sources VALUES(?1,?2,?3)",
            params![access.actor_id, scope, encoded],
        )
        .map_err(storage)?;
        let bodies = {
            let mut stmt = tx
                .prepare(
                    "SELECT body FROM records WHERE actor=?1 AND scope=?2 AND status<>'retracted'",
                )
                .map_err(storage)?;
            let rows = stmt
                .query_map(params![access.actor_id, scope], |r| r.get::<_, String>(0))
                .map_err(storage)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(storage)?
        };
        let mut count = 0;
        for body in bodies {
            let mut record: Record = serde_json::from_str(&body).map_err(storage)?;
            if record.origin != "inferred" || !record.source_refs.contains(source) {
                continue;
            }
            let mut all = true;
            for s in &record.source_refs {
                let encoded = serde_json::to_string(s).map_err(storage)?;
                let unavailable:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM unavailable_sources WHERE actor=?1 AND scope=?2 AND source=?3)",params![access.actor_id,scope,encoded],|r|r.get(0)).map_err(storage)?;
                all &= unavailable;
            }
            if !all {
                continue;
            }
            record.status = "retracted".into();
            record.revision += 1;
            let body = serde_json::to_string(&record).map_err(storage)?;
            tx.execute("UPDATE records SET body=?4,status='retracted',revision=?5 WHERE actor=?1 AND scope=?2 AND id=?3",params![access.actor_id,scope,record.id,body,record.revision]).map_err(storage)?;
            tx.execute(
                "INSERT INTO revisions VALUES(?1,?2,?3,?4,?5)",
                params![access.actor_id, scope, record.id, record.revision, body],
            )
            .map_err(storage)?;
            tx.execute(
                "DELETE FROM memory_fts WHERE actor=?1 AND scope=?2 AND id=?3",
                params![access.actor_id, scope, record.id],
            )
            .map_err(storage)?;
            tx.execute(
                "INSERT INTO audit VALUES(?1,?2,'sourceInvalidated',?3)",
                params![access.actor_id, scope, record.id],
            )
            .map_err(storage)?;
            count += 1;
        }
        tx.commit().map_err(storage)?;
        Ok(count)
    }
}
#[cfg(test)]
mod additional_tests {
    use super::*;
    #[test]
    fn pending_invalidation_isolation_and_stale_proposal() {
        let db = MemoryService::open(Path::new(":memory:")).unwrap();
        let a = AccessContext {
            actor_id: "a".into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        let mut b = a.clone();
        b.actor_id = "b".into();
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let source = SourceRef {
            session_id: "s".into(),
            message_id: Some("m".into()),
            event_seq: None,
            run_id: None,
            content_digest: "a".repeat(64),
        };
        let r = Record {
            id: "x".into(),
            revision: 0,
            scope: scope.clone(),
            kind: "fact".into(),
            text: "inferred".into(),
            cue: None,
            source_refs: vec![source.clone()],
            origin: "inferred".into(),
            status: "active".into(),
            confidence: None,
            expires_at: None,
            supersedes: None,
        };
        let saved = db
            .upsert(
                &a,
                "one",
                &[Write {
                    record: r,
                    expected_revision: None,
                }],
            )
            .unwrap()
            .remove(0);
        assert_eq!(saved.status, "pending");
        assert!(db.recall(&a, &scope, "", 0).unwrap().is_empty());
        assert!(db.read(&b, &scope, "x").is_err());
        let proposal = db
            .propose_consolidation(
                &a,
                vec![Write {
                    record: saved.clone(),
                    expected_revision: Some(1),
                }],
            )
            .unwrap();
        db.upsert(
            &a,
            "two",
            &[Write {
                record: saved,
                expected_revision: Some(1),
            }],
        )
        .unwrap();
        assert_eq!(
            db.apply_consolidation(&a, "three", &proposal)
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        assert_eq!(db.invalidate_source(&a, &scope, &source).unwrap(), 1);
        assert!(db.read(&a, &scope, "x").is_err());
        assert_eq!(db.invalidate_source(&a, &scope, &source).unwrap(), 0);
        let mut replay = proposal.writes[0].clone();
        replay.record.id = "new".into();
        replay.expected_revision = None;
        assert_eq!(
            db.upsert(&a, "replay", &[replay]).unwrap_err().code,
            "SOURCE_UNAVAILABLE"
        );
        assert!(db
            .audit(&a, &scope)
            .unwrap()
            .iter()
            .any(|e| e.action == "sourceInvalidated"));
    }
}
impl MemoryService {
    /// Trusted host decision on a pending record, protected by its revision.
    pub fn decide(
        &self,
        access: &AccessContext,
        scope: &Scope,
        id: &str,
        expected_revision: i64,
        accept: bool,
    ) -> Result<Record> {
        access.authorize(scope)?;
        let scope = key(scope)?;
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        let body:String=tx.query_row("SELECT body FROM records WHERE actor=?1 AND scope=?2 AND id=?3 AND status='pending'",params![access.actor_id,scope,id],|r|r.get(0)).optional().map_err(storage)?.ok_or_else(||error("NOT_FOUND_OR_FORBIDDEN"))?;
        let mut record: Record = serde_json::from_str(&body).map_err(storage)?;
        if record.revision != expected_revision {
            return Err(error("REVISION_CONFLICT"));
        }
        for source in &record.source_refs {
            let encoded = serde_json::to_string(source).map_err(storage)?;
            let unavailable:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM unavailable_sources WHERE actor=?1 AND scope=?2 AND source=?3)",params![access.actor_id,scope,encoded],|r|r.get(0)).map_err(storage)?;
            if accept && unavailable {
                return Err(error("SOURCE_UNAVAILABLE"));
            }
        }
        if accept {
            if let Some(cue) = record.cue.as_deref().filter(|c| !c.trim().is_empty()) {
                let bodies = {
                    let mut stmt=tx.prepare("SELECT body FROM records WHERE actor=?1 AND scope=?2 AND status='active' AND id<>?3").map_err(storage)?;
                    let rows = stmt
                        .query_map(params![access.actor_id, scope, id], |r| {
                            r.get::<_, String>(0)
                        })
                        .map_err(storage)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(storage)?
                };
                for body in bodies {
                    let mut other: Record = serde_json::from_str(&body).map_err(storage)?;
                    if other
                        .cue
                        .as_deref()
                        .is_some_and(|c| c.trim().eq_ignore_ascii_case(cue.trim()))
                        && other.text != record.text
                    {
                        other.status = "superseded".into();
                        other.revision += 1;
                        let body = serde_json::to_string(&other).map_err(storage)?;
                        tx.execute("UPDATE records SET revision=?4,status='superseded',body=?5 WHERE actor=?1 AND scope=?2 AND id=?3",params![access.actor_id,scope,other.id,other.revision,body]).map_err(storage)?;
                        tx.execute(
                            "INSERT INTO revisions VALUES(?1,?2,?3,?4,?5)",
                            params![access.actor_id, scope, other.id, other.revision, body],
                        )
                        .map_err(storage)?;
                        tx.execute(
                            "DELETE FROM memory_fts WHERE actor=?1 AND scope=?2 AND id=?3",
                            params![access.actor_id, scope, other.id],
                        )
                        .map_err(storage)?;
                        tx.execute(
                            "INSERT INTO audit VALUES(?1,?2,'superseded',?3)",
                            params![access.actor_id, scope, other.id],
                        )
                        .map_err(storage)?;
                    }
                }
            }
        }
        record.revision += 1;
        record.status = if accept { "active" } else { "retracted" }.into();
        let body = serde_json::to_string(&record).map_err(storage)?;
        tx.execute("UPDATE records SET revision=?4,status=?5,body=?6 WHERE actor=?1 AND scope=?2 AND id=?3",params![access.actor_id,scope,id,record.revision,record.status,body]).map_err(storage)?;
        tx.execute(
            "INSERT INTO revisions VALUES(?1,?2,?3,?4,?5)",
            params![access.actor_id, scope, id, record.revision, body],
        )
        .map_err(storage)?;
        if accept {
            tx.execute(
                "INSERT INTO memory_fts VALUES(?1,?2,?3,?4)",
                params![access.actor_id, scope, id, record.text],
            )
            .map_err(storage)?;
        }
        tx.execute(
            "INSERT INTO audit VALUES(?1,?2,?3,?4)",
            params![
                access.actor_id,
                scope,
                if accept { "confirm" } else { "reject" },
                id
            ],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(record)
    }
}
#[cfg(test)]
mod conflict_tests {
    use super::*;
    #[test]
    fn conflicts_fts_and_host_decision() {
        let db = MemoryService::open(Path::new(":memory:")).unwrap();
        let a = AccessContext {
            actor_id: "a".into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let mut r = Record {
            id: "1".into(),
            revision: 0,
            scope: scope.clone(),
            kind: "fact".into(),
            text: "alpha preference".into(),
            cue: Some("colour".into()),
            source_refs: vec![],
            origin: "userExplicit".into(),
            status: "active".into(),
            confidence: None,
            expires_at: None,
            supersedes: None,
        };
        db.upsert(
            &a,
            "1",
            &[Write {
                record: r.clone(),
                expected_revision: None,
            }],
        )
        .unwrap();
        assert_eq!(db.recall(&a, &scope, "alpha", 0).unwrap().len(), 1);
        assert!(db.recall(&a, &scope, "unmatched", 0).unwrap().is_empty());
        r.id = "2".into();
        r.text = "beta preference".into();
        let pending = db
            .upsert(
                &a,
                "2",
                &[Write {
                    record: r,
                    expected_revision: None,
                }],
            )
            .unwrap()
            .remove(0);
        assert_eq!(pending.status, "pending");
        assert!(db.recall(&a, &scope, "", 0).unwrap().is_empty());
        assert_eq!(
            db.decide(&a, &scope, "2", 1, true).unwrap().status,
            "active"
        );
        assert_eq!(db.recall(&a, &scope, "beta", 0).unwrap().len(), 1);
        assert_eq!(
            db.decide(&a, &scope, "1", 2, true).unwrap().status,
            "active"
        );
        let recalled = db.recall(&a, &scope, "", 0).unwrap();
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].id, "1");
        let old = db.read(&a, &scope, "2").unwrap();
        assert_eq!(old.status, "superseded");
        assert_eq!(old.text, "beta preference");
        assert!(db
            .audit(&a, &scope)
            .unwrap()
            .iter()
            .any(|entry| entry.action == "superseded"));
    }
}
#[cfg(test)]
mod schema_tests {
    use super::*;
    #[test]
    fn newer_schema_is_rejected_without_modification() {
        let path = std::env::temp_dir().join(format!(
            "mh-memory-future-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('kept'); PRAGMA user_version=99;").unwrap();
        }
        assert!(matches!(MemoryService::open(&path),Err(e) if e.code=="UNSUPPORTED_SCHEMA"));
        {
            let db = Connection::open(&path).unwrap();
            let version: i64 = db
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, 99);
            let value: String = db
                .query_row("SELECT value FROM sentinel", [], |r| r.get(0))
                .unwrap();
            assert_eq!(value, "kept");
            let count: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name='records'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        }
        std::fs::remove_file(path).unwrap();
    }
}
