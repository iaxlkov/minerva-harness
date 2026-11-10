use minerva_runtime::{
    sessions::{ContentPart, SessionMessage},
    Runtime, RuntimeConfig,
};
use serde_json::{json, Value};

async fn call(
    runtime: &Runtime,
    id: &str,
    method: &str,
    session: &str,
    scope: Value,
    payload: Value,
) -> Result<Value, String> {
    runtime
        .plugins
        .plugin_call(
            id,
            method,
            json!({"sessionId":session,"scope":scope,"payload":payload}),
        )
        .await
}
fn config(path: &std::path::Path) -> RuntimeConfig {
    RuntimeConfig {
        tenant_id: "local".into(),
        workspace_id: "default".into(),
        profile: "desktop".into(),
        data_dir: path.join("data"),
        config_dir: path.join("config"),
    }
}
#[tokio::test]
async fn optional_learning_persists_and_enforces_scope() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let rt = Runtime::start(cfg.clone()).await.unwrap();
    let roster = rt.plugins.list().await.unwrap();
    for id in [
        "memory",
        "skills",
        "session-query",
        "experience",
        "experiments",
        "improvement",
        "skill-curation",
    ] {
        assert!(roster.iter().any(|p| p.id == id), "missing {id}");
    }
    rt.sessions.create("first").unwrap();
    rt.sessions.create("second").unwrap();
    {
        let db = rt.plugins.harness.context.db().unwrap();
        db.execute(
            "INSERT INTO projects(id,title,path) VALUES('w1','one',NULL),('w2','two',NULL)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO project_sessions VALUES('first','w1'),('second','w2')",
            [],
        )
        .unwrap();
    }
    let s1 = json!({"kind":"workspace","workspaceId":"w1"});
    assert!(call(&rt, "memory", "list", "first", s1.clone(), json!({}))
        .await
        .is_err());
    rt.plugins.set_optional_enabled(true).await.unwrap();
    rt.plugins.set_enabled("memory", true).await.unwrap();
    call(
        &rt,
        "memory",
        "remember",
        "first",
        s1.clone(),
        json!({"id":"fact1","text":"Remember the violet lantern","requestId":"req1"}),
    )
    .await
    .unwrap();
    let record = call(
        &rt,
        "memory",
        "read",
        "first",
        s1.clone(),
        json!({"id":"fact1"}),
    )
    .await
    .unwrap();
    assert_eq!(record["text"], "Remember the violet lantern");
    assert_eq!(
        call(&rt, "memory", "list", "first", s1.clone(), json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(call(
        &rt,
        "memory",
        "read",
        "second",
        s1.clone(),
        json!({"id":"fact1"})
    )
    .await
    .is_err());
    let context = rt
        .plugins
        .harness
        .context
        .prepare_enriched("first", &[])
        .await
        .unwrap();
    assert!(context
        .system_prompt
        .contains("Remember the violet lantern"));
    rt.plugins.set_optional_enabled(false).await.unwrap();
    assert!(call(&rt, "memory", "list", "first", s1.clone(), json!({}))
        .await
        .is_err());
    assert!(!rt
        .plugins
        .harness
        .context
        .prepare_enriched("first", &[])
        .await
        .unwrap()
        .system_prompt
        .contains("Remember the violet lantern"));
    rt.plugins.set_optional_enabled(true).await.unwrap();
    rt.plugins.set_enabled("session-query", true).await.unwrap();
    rt.plugins.set_enabled("skills", true).await.unwrap();
    rt.sessions
        .save(
            "first",
            SessionMessage {
                id: "message1".into(),
                parent_id: None,
                role: "user".into(),
                created_at: "2026-01-01".into(),
                parts: vec![ContentPart::text("text", "the cobalt telescope")],
                extensions: Default::default(),
            },
            None,
        )
        .unwrap();
    let found = call(
        &rt,
        "session-query",
        "search",
        "first",
        s1.clone(),
        json!({"query":"cobalt"}),
    )
    .await
    .unwrap();
    let source = found["hits"][0]["source"].clone();
    assert!(source.is_object(), "{found}");
    let event = call(
        &rt,
        "session-query",
        "read",
        "first",
        s1.clone(),
        json!({"source":source.clone()}),
    )
    .await
    .unwrap();
    assert_eq!(event["text"], "the cobalt telescope");
    assert!(call(
        &rt,
        "session-query",
        "read",
        "second",
        s1.clone(),
        json!({"source":source})
    )
    .await
    .is_err());
    let skill = "---\nname: demo\ndescription: Example skill\n---\nUse this procedure.\n";
    call(
        &rt,
        "skills",
        "publish",
        "first",
        s1.clone(),
        json!({"name":"demo","body":skill}),
    )
    .await
    .unwrap();
    let loaded = call(
        &rt,
        "skills",
        "load",
        "first",
        s1.clone(),
        json!({"name":"demo"}),
    )
    .await
    .unwrap();
    assert!(loaded.to_string().contains("Use this procedure"));
    let tools = rt.plugins.tools_for("first").await.unwrap();
    let loaded = tools
        .execute(
            "load_skill",
            json!({"name":"demo"}),
            minerva_runtime::tools::ToolContext {
                session_id: "first".into(),
                run_id: "skill-test".into(),
                call_id: "load".into(),
                cwd: None,
                policy: minerva_runtime::tools::PermissionPolicy::ReadOnly,
                cancellation: minerva_runtime::tools::Cancellation::default(),
                interactions: rt.plugins.harness.interactions.clone(),
                sink: None,
            },
        )
        .await
        .unwrap();
    assert!(loaded["instructions"]
        .as_str()
        .unwrap()
        .contains("Use this procedure"));
    rt.plugins.set_enabled("skills", false).await.unwrap();
    assert!(!rt
        .plugins
        .tools_for("first")
        .await
        .unwrap()
        .definitions()
        .iter()
        .any(|tool| tool.name == "load_skill"));
    let tools = rt.plugins.tools_for("first").await.unwrap();
    for name in [
        "session_search",
        "session_event_search",
        "session_event_read",
        "session_trace",
        "session_event_trace",
    ] {
        assert!(tools.definitions().iter().any(|tool| tool.name == name));
    }
    let tool_context = |policy| minerva_runtime::tools::ToolContext {
        session_id: "first".into(),
        run_id: "memory-test".into(),
        call_id: "remember".into(),
        cwd: None,
        policy,
        cancellation: minerva_runtime::tools::Cancellation::default(),
        interactions: rt.plugins.harness.interactions.clone(),
        sink: None,
    };
    assert!(tools
        .execute(
            "memory_remember",
            json!({"text":"Tool memory retained"}),
            tool_context(minerva_runtime::tools::PermissionPolicy::ReadOnly)
        )
        .await
        .is_err());
    let first = tools
        .execute(
            "memory_remember",
            json!({"text":"Tool memory retained"}),
            tool_context(minerva_runtime::tools::PermissionPolicy::Allow),
        )
        .await
        .unwrap();
    let repeated = tools
        .execute(
            "memory_remember",
            json!({"text":"Tool memory retained"}),
            tool_context(minerva_runtime::tools::PermissionPolicy::Allow),
        )
        .await
        .unwrap();
    assert_eq!(first, repeated);
    rt.shutdown().await.unwrap();
    drop(rt);
    let reopened = Runtime::start(cfg).await.unwrap();
    let roster = reopened.plugins.list().await.unwrap();
    assert!(roster
        .iter()
        .any(|p| p.id == "memory" && p.requested_enabled));
    assert_eq!(
        call(
            &reopened,
            "memory",
            "read",
            "first",
            s1,
            json!({"id":"fact1"})
        )
        .await
        .unwrap()["text"],
        "Remember the violet lantern"
    );
    reopened.shutdown().await.unwrap();
}
