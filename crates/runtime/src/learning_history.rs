//! Host-authorized history projection; session event data remains authoritative.
use mh_context::ContextService;
use mh_session_query::{Cursor, Event, HistoryService, HistorySource, Page, Surface};
use mh_sessions::sessions::{SessionEvent, SessionService};
use minerva_plugin_sdk::learning::{AccessContext, LearningError, Scope, SourceRef};
use sha2::{Digest, Sha256};
use std::sync::Arc;

type Result<T> = std::result::Result<T, LearningError>;
fn unavailable(_: impl std::fmt::Display) -> LearningError {
    LearningError::new("SOURCE_UNAVAILABLE", "history source unavailable")
}
fn denied() -> LearningError {
    LearningError::new("NOT_FOUND_OR_FORBIDDEN", "history source unavailable")
}
pub struct RuntimeHistory {
    sessions: Arc<SessionService>,
    context: Arc<ContextService>,
    actor: String,
    fingerprint: std::sync::Mutex<Option<String>>,
}
impl RuntimeHistory {
    pub fn new(sessions: Arc<SessionService>, context: Arc<ContextService>, actor: String) -> Self {
        Self {
            sessions,
            context,
            actor,
            fingerprint: std::sync::Mutex::new(None),
        }
    }
    fn workspace(&self, session: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.context
            .db()
            .map_err(unavailable)?
            .query_row(
                "SELECT project FROM project_sessions WHERE session=?1",
                [session],
                |r| r.get(0),
            )
            .optional()
            .map_err(unavailable)
    }
    fn authorized(&self, access: &AccessContext, session: &str) -> Result<Option<String>> {
        if access.actor_id != self.actor || self.actor.is_empty() {
            return Err(denied());
        }
        self.sessions.get(session).map_err(|_| denied())?;
        let workspace = self.workspace(session)?;
        if session != access.session_id && (workspace.is_none() || workspace != access.workspace_id)
        {
            return Err(denied());
        }
        Ok(workspace)
    }
    fn project(&self, event: &SessionEvent, workspace: Option<String>) -> Option<Event> {
        if event.kind != "message.upsert" {
            return None;
        }
        let role = event.data.get("role")?.as_str()?;
        if role != "user" && role != "assistant" {
            return None;
        }
        let message = event.data.get("id")?.as_str()?.to_owned();
        let mut text = String::new();
        for part in event.data.get("parts")?.as_array()? {
            if part.get("kind").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = part
                    .get("data")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(t);
                }
            }
        }
        if text.trim().is_empty() || text.len() > 1_048_576 {
            return None;
        }
        let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        Some(Event {
            actor: self.actor.clone(),
            workspace,
            source: SourceRef {
                session_id: event.session_id.clone(),
                message_id: Some(message),
                event_seq: Some(event.seq),
                run_id: None,
                content_digest: hash.clone(),
            },
            text,
            surface: Surface::Current,
            revision: hash,
            inherited: false,
        })
    }
    fn project_many(&self, event: &SessionEvent, workspace: Option<String>) -> Vec<Event> {
        if event.kind != "history.snapshot" {
            return self.project(event, workspace).into_iter().collect();
        }
        event
            .data
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|messages| {
                messages
                    .iter()
                    .filter_map(|message| {
                        self.project(
                            &SessionEvent {
                                session_id: event.session_id.clone(),
                                seq: event.seq,
                                kind: "message.upsert".into(),
                                data: message.clone(),
                            },
                            workspace.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn refresh(&self, history: &HistoryService) -> Result<()> {
        let mut guard = self.fingerprint.lock().map_err(unavailable)?;
        let mut all = Vec::new();
        let mut bytes = 0usize;
        let mut provenance = std::collections::BTreeMap::new();
        for session in self.sessions.list().map_err(unavailable)? {
            let workspace = self.workspace(&session.id)?;
            let mut cursor = 0;
            let mut latest = std::collections::BTreeMap::new();
            let mut session_bytes = 0usize;
            loop {
                let batch = self
                    .sessions
                    .events_after(&session.id, cursor, 1000)
                    .map_err(unavailable)?;
                if batch.is_empty() {
                    break;
                }
                for event in &batch {
                    if event.kind == "fork.provenance" {
                        if let (Some(parent), Some(original), Some(message)) = (
                            event.data.get("sourceSessionId").and_then(|x| x.as_str()),
                            event.data.get("sourceMessageId").and_then(|x| x.as_str()),
                            event.data.get("messageId").and_then(|x| x.as_str()),
                        ) {
                            if parent.len() <= 256 && original.len() <= 256 && message.len() <= 256
                            {
                                if provenance.len() >= 100_000 {
                                    return Err(LearningError::new(
                                        "LIMIT_EXCEEDED",
                                        "too many fork links",
                                    ));
                                }
                                provenance.insert(
                                    (session.id.clone(), message.to_string()),
                                    (parent.to_string(), original.to_string()),
                                );
                            }
                        }
                    }
                    for item in self.project_many(event, workspace.clone()) {
                        session_bytes = session_bytes.saturating_add(item.text.len() + 256);
                        if bytes.saturating_add(session_bytes) > 64 * 1024 * 1024 {
                            return Err(LearningError::new(
                                "LIMIT_EXCEEDED",
                                "history projection exceeds 64 MiB",
                            ));
                        }
                        if let Some(old) =
                            latest.insert(item.source.message_id.clone().unwrap_or_default(), item)
                        {
                            let old: Event = old;
                            session_bytes = session_bytes.saturating_sub(old.text.len() + 256);
                        }
                    }
                }
                cursor = batch.last().unwrap().seq;
                if batch.len() < 1000 {
                    break;
                }
            }
            for event in latest.into_values() {
                bytes = bytes
                    .checked_add(event.text.len() + event.source.session_id.len() + 256)
                    .ok_or_else(|| unavailable("size"))?;
                if bytes > 64 * 1024 * 1024 {
                    return Err(LearningError::new(
                        "LIMIT_EXCEEDED",
                        "history projection exceeds 64 MiB",
                    ));
                }
                all.push(event);
            }
        }
        let original: std::collections::BTreeMap<_, _> = all
            .iter()
            .map(|e| {
                (
                    (
                        e.source.session_id.clone(),
                        e.source.message_id.clone().unwrap_or_default(),
                    ),
                    e.clone(),
                )
            })
            .collect();
        let mut aliases = Vec::new();
        all.retain(|e| {
            let key = (
                e.source.session_id.clone(),
                e.source.message_id.clone().unwrap_or_default(),
            );
            match provenance
                .get(&key)
                .filter(|parent| !provenance.contains_key(*parent))
                .and_then(|parent| original.get(parent))
            {
                Some(parent)
                    if parent.workspace.is_some()
                        && parent.workspace == e.workspace
                        && parent.source.content_digest == e.source.content_digest =>
                {
                    aliases.push(mh_session_query::Alias {
                        actor: self.actor.clone(),
                        fork_session: e.source.session_id.clone(),
                        source: parent.source.clone(),
                    });
                    false
                }
                _ => true,
            }
        });
        aliases.sort_by(|a, b| {
            (
                &a.fork_session,
                &a.source.session_id,
                a.source.event_seq,
                &a.source.message_id,
            )
                .cmp(&(
                    &b.fork_session,
                    &b.source.session_id,
                    b.source.event_seq,
                    &b.source.message_id,
                ))
        });
        all.sort_by(|a, b| {
            (&a.source.session_id, a.source.event_seq)
                .cmp(&(&b.source.session_id, b.source.event_seq))
        });
        let mut hash = Sha256::new();
        for e in &all {
            hash.update(e.source.session_id.as_bytes());
            hash.update(e.source.event_seq.unwrap_or(0).to_le_bytes());
            hash.update(e.source.message_id.as_deref().unwrap_or("").as_bytes());
            hash.update(e.source.content_digest.as_bytes());
            hash.update(e.workspace.as_deref().unwrap_or("").as_bytes());
        }
        for alias in &aliases {
            hash.update(alias.fork_session.as_bytes());
            hash.update(alias.source.session_id.as_bytes());
            hash.update(alias.source.message_id.as_deref().unwrap_or("").as_bytes());
        }
        let fingerprint = format!("{:x}", hash.finalize());
        if guard.as_ref() == Some(&fingerprint) {
            return Ok(());
        }
        history.rebuild_with_aliases(0, &all, &aliases)?;
        *guard = Some(fingerprint);
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        history: &HistoryService,
        access: &AccessContext,
        scope: &Scope,
        session: Option<&str>,
        query: &str,
        limit: usize,
        cursor: Option<&Cursor>,
    ) -> Result<Page> {
        if access.actor_id != self.actor {
            return Err(denied());
        }
        if let Some(target) = session {
            let target_workspace = self.authorized(access, target)?;
            if let Scope::Workspace { workspace_id } = scope {
                if target_workspace.as_ref() != Some(workspace_id) {
                    return Err(denied());
                }
            }
        }
        let mut page = history.search(access, scope, session, query, limit, cursor)?;
        for hit in &mut page.hits {
            self.authorized(access, &hit.source.session_id)?;
            if let Scope::Workspace { workspace_id } = scope {
                if self.workspace(&hit.source.session_id)?.as_ref() != Some(workspace_id) {
                    return Err(denied());
                }
            }
            history.read_event(self, access, scope, &hit.source)?;
            hit.text = hit.text.chars().take(400).collect();
        }
        Ok(page)
    }
}
impl HistorySource for RuntimeHistory {
    fn read(&self, access: &AccessContext, source: &SourceRef) -> Result<Event> {
        let workspace = self.authorized(access, &source.session_id)?;
        let seq = source.event_seq.ok_or_else(denied)?;
        let batch = self
            .sessions
            .events_after(&source.session_id, seq.saturating_sub(1), 1)
            .map_err(unavailable)?;
        let event = batch
            .first()
            .filter(|e| e.seq == seq)
            .and_then(|e| {
                self.project_many(e, workspace)
                    .into_iter()
                    .find(|x| x.source.message_id == source.message_id)
            })
            .ok_or_else(denied)?;
        if event.source != *source {
            return Err(LearningError::new("SOURCE_CHANGED", "source changed"));
        }
        let messages = self
            .sessions
            .load(&source.session_id)
            .map_err(unavailable)?;
        let message = messages
            .iter()
            .find(|m| Some(m.id.as_str()) == source.message_id.as_deref())
            .ok_or_else(denied)?;
        let current = SessionEvent {
            session_id: source.session_id.clone(),
            seq,
            kind: "message.upsert".into(),
            data: serde_json::to_value(message).map_err(unavailable)?,
        };
        if self
            .project(&current, event.workspace.clone())
            .is_none_or(|e| e.source.content_digest != source.content_digest)
        {
            return Err(LearningError::new("SOURCE_CHANGED", "source changed"));
        }
        Ok(event)
    }
    fn trace(
        &self,
        access: &AccessContext,
        session: &str,
        event: Option<i64>,
    ) -> Result<Vec<SourceRef>> {
        let workspace = self.authorized(access, session)?;
        let seq = event.ok_or_else(denied)?;
        let rows = self
            .sessions
            .events_after(session, seq.saturating_sub(1), 1)
            .map_err(unavailable)?;
        let mut result: Vec<SourceRef> = rows
            .first()
            .filter(|e| e.seq == seq)
            .map(|e| {
                self.project_many(e, workspace.clone())
                    .into_iter()
                    .map(|e| e.source)
                    .collect()
            })
            .unwrap_or_default();
        if result.len() != 1 {
            return Ok(result);
        }
        let current = result[0].clone();
        if self.read(access, &current).is_err() {
            return Err(denied());
        }
        let mut cursor = 0;
        let mut scanned = 0usize;
        let mut link = None;
        loop {
            let batch = self
                .sessions
                .events_after(session, cursor, 1000)
                .map_err(unavailable)?;
            if batch.is_empty() {
                break;
            }
            scanned += batch.len();
            if scanned > 100_000 {
                return Err(LearningError::new(
                    "LIMIT_EXCEEDED",
                    "too many events to trace",
                ));
            }
            for e in &batch {
                if e.kind == "fork.provenance"
                    && e.data.get("messageId").and_then(|v| v.as_str())
                        == current.message_id.as_deref()
                {
                    if let (Some(parent), Some(message)) = (
                        e.data.get("sourceSessionId").and_then(|v| v.as_str()),
                        e.data.get("sourceMessageId").and_then(|v| v.as_str()),
                    ) {
                        link = Some((parent.to_owned(), message.to_owned()));
                    }
                }
            }
            cursor = batch.last().unwrap().seq;
            if batch.len() < 1000 {
                break;
            }
        }
        if let Some((parent, message)) = link {
            if let Ok(parent_workspace) = self.authorized(access, &parent) {
                if workspace.is_some() && parent_workspace == workspace {
                    let mut pos = 0;
                    let mut latest = None;
                    let mut scanned = 0;
                    loop {
                        let batch = self
                            .sessions
                            .events_after(&parent, pos, 1000)
                            .map_err(unavailable)?;
                        if batch.is_empty() {
                            break;
                        }
                        scanned += batch.len();
                        if scanned > 100_000 {
                            return Err(LearningError::new(
                                "LIMIT_EXCEEDED",
                                "too many events to trace",
                            ));
                        }
                        for event in &batch {
                            for candidate in self.project_many(event, parent_workspace.clone()) {
                                if candidate.source.message_id.as_deref() == Some(&message)
                                    && candidate.source.content_digest == current.content_digest
                                {
                                    latest = Some(candidate.source);
                                }
                            }
                        }
                        pos = batch.last().unwrap().seq;
                        if batch.len() < 1000 {
                            break;
                        }
                    }
                    if let Some(source) = latest {
                        if self.read(access, &source).is_ok() {
                            result.push(source);
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mh_sessions::sessions::{ContentPart, SessionMessage};
    #[test]
    fn real_events_are_scoped_and_searchable() {
        let base = std::env::temp_dir().join(format!(
            "mh-history-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let sessions = Arc::new(SessionService::open(&base.join("sessions.sqlite")).unwrap());
        let context = Arc::new(ContextService::open(&base.join("context.sqlite")).unwrap());
        sessions.create("s").unwrap();
        sessions.create("other").unwrap();
        {
            let db = context.db().unwrap();
            db.execute(
                "INSERT INTO projects(id,title,path) VALUES('w','w',NULL)",
                [],
            )
            .unwrap();
            db.execute("INSERT INTO project_sessions VALUES('s','w')", [])
                .unwrap();
        }
        sessions
            .save(
                "s",
                SessionMessage {
                    id: "m".into(),
                    parent_id: None,
                    role: "user".into(),
                    created_at: "2026-01-01".into(),
                    parts: vec![ContentPart::text("text", "unique visible words")],
                    extensions: Default::default(),
                },
                None,
            )
            .unwrap();
        let adapter = RuntimeHistory::new(sessions, context, "actor".into());
        let history = HistoryService::open(&base.join("history.sqlite")).unwrap();
        adapter.refresh(&history).unwrap();
        let access = AccessContext {
            actor_id: "actor".into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let page = adapter
            .search(&history, &access, &scope, None, "unique", 10, None)
            .unwrap();
        assert_eq!(page.hits.len(), 1);
        let source = &page.hits[0].source;
        assert_eq!(
            history
                .read_event(&adapter, &access, &scope, source)
                .unwrap()
                .text,
            "unique visible words"
        );
        let mut wrong = access.clone();
        wrong.actor_id = "other".into();
        assert!(adapter
            .search(&history, &wrong, &scope, None, "unique", 10, None)
            .is_err());
        adapter.refresh(&history).unwrap();
    }
}
#[cfg(test)]
mod snapshot_fork_tests {
    use super::*;
    use mh_sessions::sessions::{ContentPart, SessionMessage};
    #[test]
    fn snapshot_multiple_messages_and_real_fork_are_readable() {
        let temp = tempfile::tempdir().unwrap();
        let session_path = temp.path().join("sessions.sqlite");
        let sessions = Arc::new(SessionService::open(&session_path).unwrap());
        let context = Arc::new(ContextService::open(&temp.path().join("context.sqlite")).unwrap());
        sessions.create("source").unwrap();
        for (id, text) in [("m1", "legacy amber"), ("m2", "legacy indigo")] {
            sessions
                .save(
                    "source",
                    SessionMessage {
                        id: id.into(),
                        parent_id: None,
                        role: "user".into(),
                        created_at: "2026-01-01".into(),
                        parts: vec![ContentPart::text("text", text)],
                        extensions: Default::default(),
                    },
                    None,
                )
                .unwrap();
        }
        let snapshot = serde_json::to_string(&sessions.baseline("source").unwrap()).unwrap();
        {
            let db = rusqlite::Connection::open(&session_path).unwrap();
            db.execute("DELETE FROM session_events WHERE session_id='source'", [])
                .unwrap();
            db.execute(
                "INSERT INTO session_events VALUES('source',1,'history.snapshot',?1)",
                [snapshot],
            )
            .unwrap();
        }
        sessions.fork("source", "fork").unwrap();
        {
            let db = rusqlite::Connection::open(&session_path).unwrap();
            db.execute(
                "DELETE FROM session_events WHERE session_id='fork' AND kind='fork.provenance'",
                [],
            )
            .unwrap();
        }
        sessions.fork("source", "newfork").unwrap();
        {
            let db = context.db().unwrap();
            db.execute("INSERT INTO projects(id,title) VALUES('w','w')", [])
                .unwrap();
            db.execute(
                "INSERT INTO project_sessions VALUES('source','w'),('fork','w'),('newfork','w')",
                [],
            )
            .unwrap();
        }
        let adapter = RuntimeHistory::new(sessions, context, "actor".into());
        let index = HistoryService::open(&temp.path().join("history.sqlite")).unwrap();
        adapter.refresh(&index).unwrap();
        let access = AccessContext {
            actor_id: "actor".into(),
            session_id: "source".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let page = adapter
            .search(&index, &access, &scope, Some("source"), "legacy", 10, None)
            .unwrap();
        assert_eq!(page.hits.len(), 2);
        for hit in page.hits {
            assert_eq!(hit.source.event_seq, Some(1));
            assert_eq!(
                index
                    .read_event(&adapter, &access, &scope, &hit.source)
                    .unwrap()
                    .text,
                hit.text
            );
        }
        let fork = adapter
            .search(&index, &access, &scope, Some("fork"), "legacy", 10, None)
            .unwrap();
        assert_eq!(fork.hits.len(), 2);
        for hit in fork.hits {
            assert_eq!(
                index
                    .read_event(&adapter, &access, &scope, &hit.source)
                    .unwrap()
                    .text,
                hit.text
            );
        }
        let inherited = adapter
            .search(&index, &access, &scope, Some("newfork"), "legacy", 10, None)
            .unwrap();
        assert_eq!(inherited.hits.len(), 2);
        for hit in inherited.hits {
            assert_eq!(hit.source.session_id, "source");
            assert_eq!(
                index
                    .read_event(&adapter, &access, &scope, &hit.source)
                    .unwrap()
                    .text,
                hit.text
            );
        }
        let all = adapter
            .search(&index, &access, &scope, None, "legacy", 10, None)
            .unwrap();
        assert_eq!(all.hits.len(), 4);
        assert!(all
            .hits
            .iter()
            .all(|hit| hit.source.session_id != "newfork"));
        for hit in all.hits {
            assert_eq!(
                index
                    .read_event(&adapter, &access, &scope, &hit.source)
                    .unwrap()
                    .text,
                hit.text
            );
        }
        let original_source = adapter
            .search(&index, &access, &scope, Some("newfork"), "amber", 10, None)
            .unwrap()
            .hits[0]
            .source
            .clone();
        {
            let db = rusqlite::Connection::open(&session_path).unwrap();
            db.execute("DELETE FROM session_events WHERE session_id='source'", [])
                .unwrap();
            db.execute("DELETE FROM messages WHERE session_id='source'", [])
                .unwrap();
            db.execute("DELETE FROM sessions WHERE id='source'", [])
                .unwrap();
        }
        adapter.refresh(&index).unwrap();
        assert!(index
            .read_event(&adapter, &access, &scope, &original_source)
            .is_err());
        let recovered = adapter
            .search(&index, &access, &scope, Some("newfork"), "amber", 10, None)
            .unwrap();
        assert_eq!(recovered.hits[0].source.session_id, "newfork");
    }
}
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTraceNode {
    pub session_id: String,
    pub parent: Option<String>,
    pub opaque_parent: bool,
    pub depth: usize,
}
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTrace {
    pub nodes: Vec<SessionTraceNode>,
    pub truncated: bool,
}
impl RuntimeHistory {
    /// Follows only host-recorded fork links; inaccessible parents remain opaque.
    pub fn trace_session(
        &self,
        access: &AccessContext,
        scope: &Scope,
        target: &str,
    ) -> Result<SessionTrace> {
        access.authorize(scope)?;
        let target_workspace = self.authorized(access, target)?;
        match scope {
            Scope::Session { session_id } if session_id != target => return Err(denied()),
            Scope::Workspace { workspace_id }
                if target_workspace.as_ref() != Some(workspace_id) =>
            {
                return Err(denied())
            }
            Scope::User => return Err(denied()),
            _ => {}
        }
        let sessions = self.sessions.list().map_err(unavailable)?;
        if sessions.len() > 1000 {
            return Err(LearningError::new(
                "LIMIT_EXCEEDED",
                "too many sessions to trace",
            ));
        }
        let mut visible = std::collections::BTreeSet::new();
        for session in sessions {
            let allowed = match scope {
                Scope::Session { session_id } => session.id == *session_id,
                Scope::Workspace { workspace_id } => {
                    self.workspace(&session.id)?.as_ref() == Some(workspace_id)
                }
                Scope::User => false,
            };
            if allowed {
                visible.insert(session.id);
            }
        }
        if !visible.contains(target) {
            return Err(denied());
        }
        let mut parents = std::collections::BTreeMap::new();
        let mut scanned = 0usize;
        for session in &visible {
            let mut cursor = 0;
            loop {
                let batch = self
                    .sessions
                    .events_after(session, cursor, 1000)
                    .map_err(unavailable)?;
                if batch.is_empty() {
                    break;
                }
                scanned = scanned.saturating_add(batch.len());
                if scanned > 100_000 {
                    return Err(LearningError::new(
                        "LIMIT_EXCEEDED",
                        "too many events to trace",
                    ));
                }
                for e in &batch {
                    if e.kind == "fork.provenance" {
                        if let Some(parent) = e.data.get("sourceSessionId").and_then(|x| x.as_str())
                        {
                            match parents.get(session) {
                                None => {
                                    parents.insert(session.clone(), Some(parent.to_string()));
                                }
                                Some(Some(existing)) if existing != parent => {
                                    parents.insert(session.clone(), None);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                cursor = batch.last().unwrap().seq;
                if batch.len() < 1000 {
                    break;
                }
            }
        }
        let mut adjacency: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for (child, parent) in &parents {
            if let Some(parent) = parent {
                if visible.contains(parent) {
                    adjacency
                        .entry(parent.clone())
                        .or_default()
                        .push(child.clone());
                    adjacency
                        .entry(child.clone())
                        .or_default()
                        .push(parent.clone());
                }
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut queue = std::collections::VecDeque::from([(target.to_owned(), 0usize)]);
        let mut nodes = Vec::new();
        let mut truncated = false;
        while let Some((id, depth)) = queue.pop_front() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if nodes.len() == 1000 {
                truncated = true;
                break;
            }
            let parent = parents
                .get(&id)
                .and_then(|p| p.as_ref())
                .filter(|p| visible.contains(*p))
                .cloned();
            let opaque_parent = parents
                .get(&id)
                .is_some_and(|p| p.as_ref().is_none_or(|v| !visible.contains(v)));
            nodes.push(SessionTraceNode {
                session_id: id.clone(),
                parent,
                opaque_parent,
                depth,
            });
            if let Some(neighbors) = adjacency.get(&id) {
                for next in neighbors {
                    if !seen.contains(next) {
                        if depth >= 32 {
                            truncated = true
                        } else {
                            queue.push_back((next.clone(), depth + 1));
                        }
                    }
                }
            }
        }
        Ok(SessionTrace { nodes, truncated })
    }
}
#[cfg(test)]
mod session_trace_tests {
    use super::*;
    use mh_sessions::sessions::{ContentPart, SessionMessage};
    #[test]
    fn explicit_forks_are_scoped_and_foreign_parent_is_opaque() {
        let temp = tempfile::tempdir().unwrap();
        let sessions =
            Arc::new(SessionService::open(&temp.path().join("sessions.sqlite")).unwrap());
        let context = Arc::new(ContextService::open(&temp.path().join("context.sqlite")).unwrap());
        sessions.create("parent").unwrap();
        sessions
            .save(
                "parent",
                SessionMessage {
                    id: "m".into(),
                    parent_id: None,
                    role: "user".into(),
                    created_at: "time".into(),
                    parts: vec![ContentPart::text("text", "source text")],
                    extensions: Default::default(),
                },
                None,
            )
            .unwrap();
        sessions.fork("parent", "child").unwrap();
        sessions.fork("child", "grandchild").unwrap();
        sessions.fork("parent", "foreign").unwrap();
        {
            let db = context.db().unwrap();
            db.execute(
                "INSERT INTO projects(id,title) VALUES('w','w'),('other','other')",
                [],
            )
            .unwrap();
            db.execute("INSERT INTO project_sessions VALUES('parent','w'),('child','w'),('grandchild','w'),('foreign','other')",[]).unwrap();
        }
        let adapter = RuntimeHistory::new(sessions.clone(), context, "actor".into());
        let access = AccessContext {
            actor_id: "actor".into(),
            session_id: "child".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let tree = adapter.trace_session(&access, &scope, "child").unwrap();
        assert_eq!(tree.nodes.len(), 3);
        assert!(!serde_json::to_string(&tree).unwrap().contains("foreign"));
        assert!(adapter.trace_session(&access, &scope, "foreign").is_err());
        let foreign = AccessContext {
            actor_id: "actor".into(),
            session_id: "foreign".into(),
            workspace_id: Some("other".into()),
            allow_user_scope: false,
        };
        let tree = adapter
            .trace_session(
                &foreign,
                &Scope::Workspace {
                    workspace_id: "other".into(),
                },
                "foreign",
            )
            .unwrap();
        assert_eq!(tree.nodes.len(), 1);
        assert!(tree.nodes[0].opaque_parent);
        assert!(!serde_json::to_string(&tree)
            .unwrap()
            .contains("\"parent\":\"parent\""));
        let events = sessions.events_after("child", 0, 100).unwrap();
        let message = events.iter().find(|e| e.kind == "message.upsert").unwrap();
        let refs = adapter.trace(&access, "child", Some(message.seq)).unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[1].session_id, "parent");
    }
}
