use super::*;
use crate::{
    credential_store::MemoryCredentialStore,
    tools::{InteractionBroker, PermissionPolicy, ToolContext},
};
fn context() -> ToolContext {
    ToolContext {
        session_id: "session".into(),
        run_id: "run".into(),
        call_id: "call".into(),
        cwd: None,
        policy: PermissionPolicy::Allow,
        cancellation: Cancellation::default(),
        interactions: Arc::new(InteractionBroker::default()),
        sink: None,
    }
}
fn fixture() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/mcp/fixture.cjs")
        .to_string_lossy()
        .into_owned()
}
fn node() -> String {
    #[cfg(windows)]
    let output = std::process::Command::new("where.exe")
        .arg("node.exe")
        .output()
        .unwrap();
    #[cfg(not(windows))]
    let output = std::process::Command::new("which")
        .arg("node")
        .output()
        .unwrap();
    assert!(output.status.success(), "MCP fixtures require Node");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .trim()
        .into()
}
fn manager(dir: &std::path::Path) -> (Arc<McpManager>, Arc<ToolRegistry>) {
    let registry = Arc::new(ToolRegistry::default());
    (
        McpManager::new(
            dir.join("mcp.json"),
            CredentialScope::new("test").unwrap(),
            Arc::new(MemoryCredentialStore::default()),
            registry.clone(),
            EndpointPolicy::Local,
        )
        .unwrap(),
        registry,
    )
}
async fn ready(manager: &McpManager) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if manager.list().iter().any(|e| e.state == "connected") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("MCP connected");
}
async fn log_count(path: &std::path::Path, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .count()
                >= count
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
async fn exercise(
    manager: Arc<McpManager>,
    registry: Arc<ToolRegistry>,
    config: McpServerConfig,
    log: &std::path::Path,
) {
    manager
        .save(McpServerInput {
            config,
            credentials: None,
        })
        .await
        .unwrap();
    ready(&manager).await;
    let mut admitted = registry.snapshot();
    assert_eq!(admitted.definitions().len(), 4);
    let result = admitted
        .execute("mcp.fixture.echo", json!({"text":"roundtrip"}), context())
        .await
        .unwrap();
    assert_eq!(result["content"][0]["text"], "roundtrip");
    let error = admitted
        .execute("mcp.fixture.fail", json!({}), context())
        .await
        .unwrap_err();
    assert!(error.contains("isError") && error.contains("true"));
    assert!(admitted
        .execute(
            "mcp.fixture.echo",
            json!({"text":"invalid-output"}),
            context()
        )
        .await
        .unwrap_err()
        .contains("invalid tool output"));
    manager.set_paused(true).await.unwrap();
    assert!(registry.snapshot().definitions().is_empty());
    assert!(admitted
        .execute("mcp.fixture.echo", json!({}), context())
        .await
        .is_err());
    manager.set_paused(false).await.unwrap();
    manager.set_paused(false).await.unwrap();
    ready(&manager).await;
    admitted = registry.snapshot();
    assert!(admitted
        .execute("mcp.fixture.echo", json!({"unknown":1}), context())
        .await
        .is_err());
    let ctx = context();
    let cancel = ctx.cancellation.clone();
    let pending = admitted.clone();
    let call =
        tokio::spawn(async move { pending.execute("mcp.fixture.hang", json!({}), ctx).await });
    log_count(log, 4).await;
    cancel.cancel();
    assert!(call.await.unwrap().is_err());
    // Reconnect may rediscover, but must never replay the hanging effect.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|l| *l == "hang")
            .count(),
        1
    );
    ready(&manager).await;
    assert!(registry
        .snapshot()
        .execute("mcp.fixture.disconnect", json!({}), context())
        .await
        .is_err());
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|l| *l == "disconnect")
            .count(),
        1
    );
    manager.delete("fixture").await.unwrap();
    assert!(registry.snapshot().definitions().is_empty());
    assert!(admitted
        .execute("mcp.fixture.echo", json!({}), context())
        .await
        .is_err());
    manager.shutdown().await;
}
#[tokio::test]
async fn stdio_model_facing_calls_cancellation_reconnect_and_revoke() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("effects");
    let (m, r) = manager(dir.path());
    exercise(
        m,
        r,
        McpServerConfig {
            id: "fixture".into(),
            enabled: true,
            transport: McpTransport::Stdio {
                command: node(),
                args: vec![
                    fixture(),
                    "stdio".into(),
                    log.to_string_lossy().into_owned(),
                ],
            },
        },
        &log,
    )
    .await;
}
#[tokio::test]
async fn http_model_facing_calls_cancellation_reconnect_and_revoke() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("effects");
    let port = dir.path().join("port");
    let mut child = Command::new(node())
        .args([
            fixture(),
            "http".into(),
            log.to_string_lossy().into_owned(),
            port.to_string_lossy().into_owned(),
        ])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !port.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let (m, r) = manager(dir.path());
    exercise(
        m,
        r,
        McpServerConfig {
            id: "fixture".into(),
            enabled: true,
            transport: McpTransport::StreamableHttp {
                url: format!(
                    "http://127.0.0.1:{}/mcp",
                    std::fs::read_to_string(port).unwrap()
                ),
            },
        },
        &log,
    )
    .await;
    let _ = child.kill().await;
}
#[tokio::test]
async fn secrets_are_scoped_redacted_and_not_in_settings() {
    let dir = tempfile::tempdir().unwrap();
    let (m, _) = manager(dir.path());
    let input: McpServerInput=serde_json::from_value(json!({"id":"fixture","enabled":false,"transport":{"kind":"stdio","command":"unused"},"credentials":{"env":{"TOKEN":"super-secret"},"headers":{"authorization":"hidden"}}})).unwrap();
    assert!(!format!("{input:?}").contains("super-secret"));
    let view = m.save(input).await.unwrap();
    assert!(view[0].has_credentials);
    assert!(!serde_json::to_string(&view)
        .unwrap()
        .contains("super-secret"));
    assert!(!std::fs::read_to_string(dir.path().join("mcp.json"))
        .unwrap()
        .contains("hidden"));
    assert!(m.registry.snapshot().definitions().is_empty());
    assert_ne!(
        m.credential_key("fixture"),
        McpManager::new(
            dir.path().join("other.json"),
            CredentialScope::new("other").unwrap(),
            m.credentials.clone(),
            m.registry.clone(),
            EndpointPolicy::Local
        )
        .unwrap()
        .credential_key("fixture")
    );
    m.delete("fixture").await.unwrap();
    assert!(m.get_credentials("fixture").unwrap().env.is_empty());
}

#[tokio::test]
async fn unrelated_servers_do_not_share_call_queue() {
    let dir = tempfile::tempdir().unwrap();
    let (m, r) = manager(dir.path());
    for id in ["slow", "fast"] {
        m.save(McpServerInput {
            config: McpServerConfig {
                id: id.into(),
                enabled: true,
                transport: McpTransport::Stdio {
                    command: node(),
                    args: vec![
                        fixture(),
                        "stdio".into(),
                        dir.path().join(id).to_string_lossy().into_owned(),
                    ],
                },
            },
            credentials: None,
        })
        .await
        .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        while r.snapshot().definitions().len() != 8 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let ctx = context();
    let cancellation = ctx.cancellation.clone();
    let admitted = r.snapshot();
    let pending =
        tokio::spawn(async move { admitted.execute("mcp.slow.hang", json!({}), ctx).await });
    log_count(&dir.path().join("slow"), 1).await;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        r.snapshot()
            .execute("mcp.fast.echo", json!({"text":"independent"}), context()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result["content"][0]["text"], "independent");
    cancellation.cancel();
    assert!(pending.await.unwrap().is_err());
    m.shutdown().await;
}

#[tokio::test]
async fn empty_and_disabled_hosted_managers_create_no_connections() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(ToolRegistry::default());
    let m = McpManager::new(
        dir.path().join("empty.json"),
        CredentialScope::new("hosted").unwrap(),
        Arc::new(MemoryCredentialStore::default()),
        registry.clone(),
        EndpointPolicy::Hosted,
    )
    .unwrap();
    m.start().await;
    assert!(m.list().is_empty());
    assert!(registry.snapshot().definitions().is_empty());
    for transport in [
        McpTransport::Stdio {
            command: "unused".into(),
            args: vec![],
        },
        McpTransport::StreamableHttp {
            url: "http://127.0.0.1/mcp".into(),
        },
        McpTransport::StreamableHttp {
            url: "https://example.com/mcp".into(),
        },
    ] {
        assert!(m
            .save(McpServerInput {
                config: McpServerConfig {
                    id: "denied".into(),
                    enabled: true,
                    transport
                },
                credentials: None
            })
            .await
            .is_err());
    }
    assert!(m.list().is_empty());
    m.shutdown().await;
}
