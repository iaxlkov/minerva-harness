use minerva_runtime::{plugins::PluginManager, tools::*};
use serde_json::json;
use std::{path::PathBuf, time::Duration};

const SYSTEM: &[&str] = &[
    "plugin-manager",
    "message-queue",
    "providers",
    "shell",
    "tool-runtime",
    "context",
    "workspace-tools",
    "mcp",
];

#[tokio::test]
async fn system_plugins_are_listed_and_cannot_be_disabled_or_paused() {
    let dir = tempfile::tempdir().unwrap();
    let manager = PluginManager::start(dir.path().join("shell.json"))
        .await
        .unwrap();
    for id in SYSTEM {
        let roster = manager.list().await.unwrap();
        let entry = roster
            .iter()
            .map(|p| serde_json::to_value(p).unwrap())
            .find(|p| p["id"] == *id)
            .unwrap_or_else(|| panic!("missing system plugin {id}"));
        // Queue availability follows the session service, absent in this manager-only fixture.
        assert_eq!(entry["enabled"], *id != "message-queue", "{id}");
        assert_eq!(entry["canToggle"], false, "{id}");
        assert_eq!(entry["origin"], "builtin", "{id}");
        assert!(manager.set_enabled(id, false).await.is_err());
    }
    let paused = manager.set_optional_enabled(false).await.unwrap();
    for entry in paused
        .iter()
        .map(|p| serde_json::to_value(p).unwrap())
        .filter(|p| SYSTEM.contains(&p["id"].as_str().unwrap()))
    {
        assert_eq!(entry["enabled"], entry["id"] != "message-queue");
    }
    manager.shutdown().await;
}

fn call(manager: &PluginManager, root: PathBuf) -> ToolContext {
    ToolContext {
        session_id: "session".into(),
        run_id: "run".into(),
        call_id: "call".into(),
        cwd: Some(root),
        policy: PermissionPolicy::Allow,
        cancellation: Cancellation::default(),
        interactions: manager.harness.interactions.clone(),
        sink: None,
    }
}

#[tokio::test]
async fn disposing_system_plugins_revokes_previously_admitted_native_tools() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("proof.txt"), "safe").unwrap();
    let manager = PluginManager::start(dir.path().join("shell.json"))
        .await
        .unwrap();
    let admitted = manager.harness.tools.snapshot();
    let ctx = call(&manager, dir.path().into());
    assert!(admitted
        .execute("read_file", json!({"path":"proof.txt"}), ctx.clone())
        .await
        .is_ok());
    manager.shutdown().await;
    assert!(
        admitted
            .execute("read_file", json!({"path":"proof.txt"}), ctx)
            .await
            .is_err(),
        "disposed plugin still executes its tool"
    );
}

#[tokio::test]
async fn system_mcp_keeps_real_connections_when_optional_plugins_are_paused() {
    let dir = tempfile::tempdir().unwrap();
    let manager = PluginManager::start(dir.path().join("shell.json"))
        .await
        .unwrap();
    manager.harness.mcp.save(serde_json::from_value(json!({"id":"system-proof","enabled":true,"transport":{"kind":"stdio","command":"node","args":[
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugins/mh-mcp/src/fixture.cjs"), "stdio", dir.path().join("calls.log").to_str().unwrap()
    ]}})).unwrap()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while manager.harness.mcp.list()[0].state != "connected" {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    manager.set_optional_enabled(false).await.unwrap();
    assert_eq!(
        manager.harness.mcp.list()[0].state,
        "connected",
        "system MCP was paused with optional plugins"
    );
    let admitted = manager.tools_for("session").await.unwrap();
    let echo = admitted
        .definitions()
        .into_iter()
        .find(|t| t.name == "mcp.system-proof.echo")
        .expect("system MCP tools disappeared");
    let output = admitted
        .execute(
            &echo.name,
            json!({"text":"alive"}),
            call(&manager, dir.path().into()),
        )
        .await
        .unwrap();
    assert_eq!(output["structuredContent"]["text"], "alive");
    manager.shutdown().await;
    assert!(admitted
        .execute(
            &echo.name,
            json!({"text":"after shutdown"}),
            call(&manager, dir.path().into())
        )
        .await
        .is_err());
    let restored = PluginManager::start(dir.path().join("shell.json"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while restored.harness.mcp.list()[0].state != "connected" {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !serde_json::to_value(&restored.list().await.unwrap()[0]).unwrap()["optionalEnabled"]
            .as_bool()
            .unwrap()
    );
    restored.shutdown().await;
}
