//! Requires `cargo build -j1 -p minerva-plugin-host --bin plugin-fixture`.
use minerva_runtime::{plugins::PluginManager, tools::*};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
fn fixture_binary() -> PathBuf {
    let path = std::env::var_os("MINERVA_PLUGIN_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("plugin-fixture{}", std::env::consts::EXE_SUFFIX))
        });
    assert!(path.is_file(),"Build the process fixture first: cargo build -j1 -p minerva-plugin-host --bin plugin-fixture; or set MINERVA_PLUGIN_FIXTURE");
    path
}
async fn installed(root: &Path) -> Arc<PluginManager> {
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let entry = format!("fixture{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(fixture_binary(), source.join(&entry)).unwrap();
    std::fs::write(source.join("plugin.json"),serde_json::to_vec(&json!({"id":"test.tools","version":"1","apiVersion":1,"capabilities":["tools"],"backend":{"kind":"process","entry":entry}})).unwrap()).unwrap();
    let config = root.join("config");
    minerva_plugin_host::install_local(&source, &config).unwrap();
    let manager = Arc::new(
        PluginManager::start(config.join("shell.json"))
            .await
            .unwrap(),
    );
    manager.set_enabled("test.tools", true).await.unwrap();
    manager
}
fn context(manager: &PluginManager, call: &str) -> ToolContext {
    ToolContext {
        session_id: "session".into(),
        run_id: "run-exact".into(),
        call_id: call.into(),
        cwd: None,
        policy: PermissionPolicy::Allow,
        cancellation: Cancellation::default(),
        interactions: manager.harness.interactions.clone(),
        sink: None,
    }
}
fn audit(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("config/plugins/.data/test.tools/tool-audit.jsonl"))
        .unwrap_or_default()
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
async fn wait_audit(root: &Path, method: &str, call: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !audit(root)
            .iter()
            .any(|v| v["method"] == method && v["callId"] == call)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn name(tools: &AdmittedTools, suffix: &str) -> String {
    tools
        .definitions()
        .into_iter()
        .find(|d| d.name.ends_with(suffix))
        .expect("installed plugin tool catalogue must be admitted")
        .name
}
#[tokio::test]
async fn installed_process_tool_executes_and_old_admission_is_revoked() {
    let root = tempfile::tempdir().unwrap();
    let manager = installed(root.path()).await;
    let tools = manager.tools_for("session").await.unwrap();
    let echo = name(&tools, "__echo");
    let output = tools
        .execute(
            &echo,
            json!({"value":{"real":"process"}}),
            context(&manager, "echo-call"),
        )
        .await
        .unwrap();
    assert_eq!(output["value"], json!({"real":"process"}));
    assert_eq!(
        output["runId"],
        serde_json::to_string(&("session", "run-exact")).unwrap()
    );
    assert_eq!(output["callId"], "echo-call");
    assert_eq!(
        output["pid"],
        manager
            .plugin_call("test.tools", "test.pid", json!({}))
            .await
            .unwrap()["pid"]
    );
    let log = audit(root.path());
    assert!(log
        .iter()
        .any(|v| v["method"] == "tools.release" && v["callId"] == "echo-call"));
    manager.set_enabled("test.tools", false).await.unwrap();
    assert!(tools
        .execute(
            &echo,
            json!({"value":"never"}),
            context(&manager, "revoked-call")
        )
        .await
        .unwrap_err()
        .contains("revoked"));
    assert_eq!(audit(root.path()), log);
    assert!(!manager
        .tools_for("session")
        .await
        .unwrap()
        .definitions()
        .iter()
        .any(|d| d.name == echo));
    manager.shutdown().await;
}
#[tokio::test]
async fn cancelled_process_tool_stops_effects_releases_identity_and_keeps_process_responsive() {
    run_interrupted(false).await
}
#[tokio::test]
async fn dropped_process_tool_future_cancels_and_releases_owned_work() {
    run_interrupted(true).await
}
async fn run_interrupted(drop_future: bool) {
    let root = tempfile::tempdir().unwrap();
    let manager = installed(root.path()).await;
    let tools = manager.tools_for("session").await.unwrap();
    let tool = name(&tools, "__delayed_write");
    let ctx = context(
        &manager,
        if drop_future {
            "dropped-call"
        } else {
            "cancelled-call"
        },
    );
    let call = ctx.call_id.clone();
    let cancellation = ctx.cancellation.clone();
    let effect = root.path().join("must-not-exist");
    let pid = manager
        .plugin_call("test.tools", "test.pid", json!({}))
        .await
        .unwrap();
    let task = tokio::spawn({
        let effect = effect.clone();
        async move {
            tools
                .execute(
                    &tool,
                    json!({"value":"later","delayMillis":1500,"effectPath":effect}),
                    ctx,
                )
                .await
        }
    });
    wait_audit(root.path(), "tools.start", &call).await;
    assert_eq!(
        manager
            .plugin_call("test.tools", "test.tool-state", json!({}))
            .await
            .unwrap()["active"],
        1
    );
    if drop_future {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    } else {
        cancellation.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
    wait_audit(root.path(), "tools.cancel", &call).await;
    wait_audit(root.path(), "tools.release", &call).await;
    tokio::time::sleep(Duration::from_millis(1600)).await;
    assert!(!effect.exists());
    assert_eq!(
        manager
            .plugin_call("test.tools", "test.pid", json!({}))
            .await
            .unwrap(),
        pid
    );
    assert_eq!(
        manager
            .plugin_call("test.tools", "test.tool-state", json!({}))
            .await
            .unwrap()["active"],
        0
    );
    for event in audit(root.path()) {
        assert_eq!(
            event["runId"],
            serde_json::to_string(&("session", "run-exact")).unwrap()
        );
        assert_eq!(event["callId"], call);
    }
    manager.shutdown().await;
}
#[tokio::test]
async fn two_sessions_with_identical_run_and_call_ids_cannot_cancel_each_other() {
    let root = tempfile::tempdir().unwrap();
    let manager = installed(root.path()).await;
    let tools = manager.tools_for("session-a").await.unwrap();
    let tool = name(&tools, "__delayed_write");
    let mut a = context(&manager, "same-call");
    a.session_id = "session-a".into();
    let mut b = context(&manager, "same-call");
    b.session_id = "session-b".into();
    let cancel_a = a.cancellation.clone();
    let effect_a = root.path().join("effect-a");
    let effect_b = root.path().join("effect-b");
    let start = |ctx: ToolContext, path: PathBuf| {
        let tools = tools.clone();
        let tool = tool.clone();
        tokio::spawn(async move {
            tools
                .execute(
                    &tool,
                    json!({"value":"session scoped","delayMillis":800,"effectPath":path}),
                    ctx,
                )
                .await
        })
    };
    let first = start(a, effect_a.clone());
    let second = start(b, effect_b.clone());
    tokio::time::timeout(Duration::from_secs(3), async {
        while audit(root.path())
            .iter()
            .filter(|v| v["method"] == "tools.start")
            .count()
            < 2
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // Wait for start replies to settle before cancelling one logical session.
    assert_eq!(
        manager
            .plugin_call("test.tools", "test.tool-state", json!({}))
            .await
            .unwrap()["active"],
        2
    );
    cancel_a.cancel();
    assert!(first.await.unwrap().is_err());
    let output = second.await.unwrap().unwrap();
    assert_eq!(
        output["runId"],
        serde_json::to_string(&("session-b", "run-exact")).unwrap()
    );
    assert!(!effect_a.exists());
    assert_eq!(std::fs::read_to_string(effect_b).unwrap(), "completed");
    assert_eq!(
        manager
            .plugin_call("test.tools", "test.tool-state", json!({}))
            .await
            .unwrap()["active"],
        0
    );
    manager.shutdown().await;
}
#[tokio::test]
async fn model_calls_installed_plugin_tool_then_receives_result_and_continues() {
    use std::io::{Read, Write};
    let root = tempfile::tempdir().unwrap();
    let manager = installed(root.path()).await;
    let tools = manager.tools_for("session").await.unwrap();
    let echo = name(&tools, "__echo");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut bodies = Vec::new();
        for turn in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut headers = Vec::new();
            let mut byte = [0; 1];
            while !headers.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
            }
            let headers = String::from_utf8(headers).unwrap();
            let len = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|length| length.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut body = vec![0; len];
            stream.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            if turn == 0 {
                assert!(body["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["function"]["name"] == echo));
            }
            bodies.push(body);
            let message = if turn == 0 {
                json!({"role":"assistant","content":null,"tool_calls":[{"id":"model-plugin-call","type":"function","function":{"name":echo,"arguments":"{\"value\":{\"from\":\"model\"}}"}}]})
            } else {
                json!({"role":"assistant","content":"continued after plugin tool"})
            };
            let response=json!({"id":format!("reply-{turn}"),"object":"chat.completion","created":1,"model":"fixture","choices":[{"index":0,"message":message,"finish_reason":if turn==0{"tool_calls"}else{"stop"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        }
        bodies
    });
    let mut ctx = context(&manager, "");
    ctx.call_id.clear();
    let model = rig_core::providers::openai::OpenAIConfig::new("fixture")
        .with_base_url(format!("http://{address}"))
        .client()
        .chat("fixture");
    let agent = rig_agent::AgentBuilder::new(model)
        .dynamic_tools(tools.rig_tools(ctx))
        .build();
    let response = agent
        .prompt("Use the installed plugin tool")
        .max_turns(4)
        .add_hook(tools.identity_hook())
        .await
        .unwrap();
    assert!(!response.messages.is_empty());
    let bodies = server.join().unwrap();
    let result = bodies[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    assert_eq!(result["tool_call_id"], "model-plugin-call");
    let output: Value = serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
    assert_eq!(output["value"], json!({"from":"model"}));
    assert_eq!(output["callId"], "model-plugin-call");
    assert!(audit(root.path())
        .iter()
        .any(|v| v["method"] == "tools.start" && v["callId"] == "model-plugin-call"));
    manager.shutdown().await;
}
