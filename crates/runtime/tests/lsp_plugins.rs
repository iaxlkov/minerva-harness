use minerva_runtime::{
    Runtime, RuntimeConfig,
    tools::{Cancellation, PermissionPolicy, ToolContext},
};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires Python 3"]
async fn installed_language_plugin_follows_enable_and_pause_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("language-package");
    std::fs::create_dir(&source).unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("plugin-fixture{}", std::env::consts::EXE_SUFFIX));
    let entry = format!("fixture{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(binary, source.join(&entry)).unwrap();
    std::fs::write(source.join("plugin.json"), json!({
        "id":"test.lang", "version":"1", "apiVersion":1,
        "backend":{"kind":"process", "entry":entry},
        "languageServers":[{"id":"sample","command":"python","args":[concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugins/mh-lsp/tests/fixtures/mh_lsp_server.py")],"extensions":{".sample":"sample"}}]
    }).to_string()).unwrap();
    let config = dir.path().join("config");
    minerva_plugin_host::install_local(&source, &config).unwrap();
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: "local".into(),
            workspace_id: "default".into(),
            data_dir: dir.path().join("data"),
            config_dir: config,
            profile: "desktop".into(),
        })
        .await
        .unwrap(),
    );
    runtime.create_session("test-session", None).unwrap();
    let params = json!({"sessionId":"test-session", "serverId":"plugin:test.lang/sample"});
    assert!(
        runtime
            .harness_call("mh-lsp.start", params.clone())
            .await
            .is_err()
    );
    runtime
        .plugins
        .set_enabled("test.lang", true)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .harness_call("mh-lsp.start", params.clone())
            .await
            .unwrap()["running"],
        true
    );
    let open =
        json!({"sessionId":"test-session", "path":"new.sample", "version":1, "text":"unsaved"});
    assert_eq!(
        runtime.harness_call("mh-lsp.open", open).await.unwrap(),
        "plugin:test.lang/sample"
    );
    let diag = json!({"sessionId":"test-session", "path":"new.sample"});
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let diagnostics = runtime
                .harness_call("mh-lsp.diagnostics", diag.clone())
                .await
                .unwrap();
            if diagnostics
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let query = json!({"sessionId":"test-session", "path":"new.sample", "operation":"hover", "line":0, "character":0});
    assert_eq!(
        runtime.harness_call("mh-lsp.query", query).await.unwrap()["contents"],
        "unsaved"
    );
    let tools = runtime.plugins.tools_for("test-session").await.unwrap();
    let ctx = ToolContext {
        session_id: "test-session".into(),
        run_id: "run".into(),
        call_id: "call".into(),
        cwd: Some(dir.path().join("data/default-workspace")),
        policy: PermissionPolicy::ReadOnly,
        cancellation: Cancellation::default(),
        interactions: runtime.plugins.harness.interactions.clone(),
        sink: None,
    };
    std::fs::write(
        dir.path().join("data/default-workspace/disk.sample"),
        "on disk",
    )
    .unwrap();
    let disk = tools
        .execute(
            "lsp",
            json!({"path":"disk.sample", "operation":"hover", "line":0, "character":0}),
            ctx.clone(),
        )
        .await
        .unwrap();
    assert_eq!(disk["result"]["contents"], "on disk");
    let opened_after_query =
        json!({"sessionId":"test-session", "path":"disk.sample", "version":1, "text":"new buffer"});
    runtime
        .harness_call("mh-lsp.open", opened_after_query)
        .await
        .unwrap();
    runtime
        .harness_call(
            "mh-lsp.close",
            json!({"sessionId":"test-session", "path":"disk.sample"}),
        )
        .await
        .unwrap();
    let output = tools
        .execute(
            "lsp",
            json!({"path":"new.sample", "operation":"hover", "line":0, "character":0}),
            ctx,
        )
        .await
        .unwrap();
    assert_eq!(output["result"]["contents"], "unsaved");
    let traversal =
        json!({"sessionId":"test-session", "path":"../escape.sample", "version":1, "text":"bad"});
    assert!(
        runtime
            .harness_call("mh-lsp.open", traversal)
            .await
            .is_err()
    );
    runtime.plugins.set_optional_enabled(false).await.unwrap();
    assert!(
        runtime
            .harness_call("mh-lsp.start", params.clone())
            .await
            .is_err()
    );
    runtime.plugins.set_optional_enabled(true).await.unwrap();
    assert_eq!(
        runtime
            .harness_call("mh-lsp.start", params.clone())
            .await
            .unwrap()["running"],
        true
    );
    runtime
        .plugins
        .set_enabled("test.lang", false)
        .await
        .unwrap();
    assert!(runtime.harness_call("mh-lsp.start", params).await.is_err());
    runtime.shutdown().await.unwrap();
}
