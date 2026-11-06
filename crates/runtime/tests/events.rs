use minerva_runtime::sessions::{ContentPart, IntegrationState, SessionMessage, SessionService};
use serde_json::{json, Map};
fn message(id: &str, parent: Option<&str>, role: &str) -> SessionMessage {
    SessionMessage {
        id: id.into(),
        parent_id: parent.map(str::to_owned),
        role: role.into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        parts: vec![ContentPart::text("text", id)],
        extensions: Map::new(),
    }
}
#[test]
fn ordered_scoped_replay_and_snapshot_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.db");
    let s = SessionService::open(&path).unwrap();
    s.create("one").unwrap();
    s.create("two").unwrap();
    s.save("one", message("u", None, "user"), None).unwrap();
    let baseline = s.baseline("one").unwrap();
    assert_eq!(baseline.messages.len(), 1);
    s.set_model("one", "test:model").unwrap();
    let after = s.events_after("one", baseline.cursor, 100).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].kind, "session.updated");
    assert_eq!(s.events_after("two", 0, 100).unwrap()[0].seq, 1);
    assert!(s.events_after("missing", 0, 10).is_err());
    assert!(s.events_after("one", -1, 10).is_err());
    assert!(s.events_after("one", 0, 1001).is_err());
    let events = s.events_after("one", 0, 100).unwrap();
    assert!(events.windows(2).all(|v| v[1].seq == v[0].seq + 1));
    let cursor = baseline.cursor;
    drop(s);
    let s = SessionService::open(&path).unwrap();
    assert_eq!(
        s.events_after("one", cursor, 100).unwrap()[0].seq,
        after[0].seq
    );
}
#[test]
fn rolled_back_write_never_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.db");
    let s = SessionService::open(&path).unwrap();
    s.create("one").unwrap();
    s.save("one", message("u", None, "user"), None).unwrap();
    let before = s.baseline("one").unwrap().cursor;
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_state BEFORE INSERT ON session_integration_state BEGIN SELECT RAISE(ABORT,'forced'); END;").unwrap();
    assert!(s
        .save(
            "one",
            message("a", Some("u"), "assistant"),
            Some(IntegrationState {
                namespace: "test".into(),
                value: json!({})
            })
        )
        .is_err());
    assert!(s.events_after("one", before, 100).unwrap().is_empty());
    assert_eq!(s.baseline("one").unwrap().messages.len(), 1);
}
#[test]
fn migration_preserves_history_and_restart_emits_interruption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.db");
    let s = SessionService::open(&path).unwrap();
    s.create("one").unwrap();
    s.save("one", message("u", None, "user"), None).unwrap();
    s.begin_run("one", "r", "u", &json!({}), None).unwrap();
    drop(s);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER events_session_insert; DROP TRIGGER events_session_update; DROP TRIGGER events_message_insert; DROP TRIGGER events_message_update; DROP TRIGGER events_run_insert; DROP TRIGGER events_run_update; DROP TABLE session_events; PRAGMA user_version=4;").unwrap();
    drop(db);
    let s = SessionService::open(&path).unwrap();
    let events = s.events_after("one", 0, 100).unwrap();
    let backups = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".pre-v5-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let backup = rusqlite::Connection::open(&backups[0]).unwrap();
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        backup
            .query_row("SELECT message_id FROM messages", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "u"
    );
    assert_eq!(
        backup
            .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, "history.snapshot");
    assert_eq!(events[0].data["messages"][0]["id"], "u");
    assert_eq!(events[1].kind, "run.updated");
    assert_eq!(events[1].data["status"], "interrupted");
    let b = s.baseline("one").unwrap();
    assert_eq!(b.messages[0].id, "u");
    assert!(b.active_run.is_none());
    assert_eq!(b.cursor, 2);
}
#[test]
fn checkpoint_and_settlement_publish_replacements() {
    let dir = tempfile::tempdir().unwrap();
    let s = SessionService::open(&dir.path().join("events.db")).unwrap();
    s.create("one").unwrap();
    s.save("one", message("u", None, "user"), None).unwrap();
    s.begin_run("one", "r", "u", &json!({}), None).unwrap();
    let cursor = s.baseline("one").unwrap().cursor;
    s.checkpoint("one", &message("r", Some("u"), "assistant"))
        .unwrap();
    assert_eq!(
        s.baseline("one").unwrap().active_run.unwrap().parts.len(),
        1
    );
    s.fail_run("one", "r", true, "stop").unwrap();
    let events = s.events_after("one", cursor, 100).unwrap();
    assert_eq!(events[0].kind, "message.upsert");
    assert_eq!(events[1].data["status"], "cancelled");
    assert_eq!(
        s.baseline("one").unwrap().messages[1].extensions["execution"]["status"],
        "cancelled"
    );
}
