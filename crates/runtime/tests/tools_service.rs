use minerva_runtime::tools::*;
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[tokio::test]
async fn admission_is_immutable_and_revocation_rejects_dispatch() {
    let registry = ToolRegistry::default();
    registry
        .register("plugin", definition("echo", false), |_, args| {
            Box::pin(async move { Ok(args) })
        })
        .unwrap();
    let admitted = registry.snapshot();
    registry.revoke_owner("plugin");
    assert_eq!(admitted.definitions().len(), 1);
    assert!(admitted
        .execute("echo", json!({}), context(None))
        .await
        .unwrap_err()
        .contains("revoked"));
}
#[tokio::test]
async fn schema_validation_prevents_body_execution() {
    let registry = ToolRegistry::default();
    let mut def = definition("echo", false);
    def.input_schema = json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false});
    registry
        .register("test", def, |_, args| Box::pin(async move { Ok(args) }))
        .unwrap();
    assert!(registry
        .snapshot()
        .execute("echo", json!({"value":"bad"}), context(None))
        .await
        .is_err());
}
#[tokio::test]
async fn write_requires_permission_and_paths_stay_contained() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let admitted = registry.snapshot();
    let mut ctx = context(Some(dir.path().into()));
    assert!(admitted
        .execute(
            "write_file",
            json!({"path":"a.txt","content":"one"}),
            ctx.clone()
        )
        .await
        .is_err());
    assert!(!dir.path().join("a.txt").exists());
    ctx.policy = PermissionPolicy::Allow;
    admitted
        .execute(
            "write_file",
            json!({"path":"a.txt","content":"one"}),
            ctx.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one"
    );
    assert!(admitted
        .execute(
            "write_file",
            json!({"path":"../escape.txt","content":"bad"}),
            ctx.clone()
        )
        .await
        .is_err());
    assert!(admitted
        .execute(
            "patch_file",
            json!({"path":"a.txt","oldText":"missing","newText":"two"}),
            ctx
        )
        .await
        .is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one"
    );
}
#[tokio::test]
async fn answers_are_scoped_once_and_cancelled_requests_disappear() {
    let broker = Arc::new(InteractionBroker::default());
    let mut ctx = context(None);
    ctx.interactions = broker.clone();
    let task = tokio::spawn({
        let ctx = ctx.clone();
        async move {
            ctx.interactions
                .request(&ctx, "approval", "write", "approve?", vec![])
                .await
        }
    });
    tokio::task::yield_now().await;
    let item = broker.list("session").pop().unwrap();
    assert!(!broker.answer(InteractionAnswer {
        session_id: "other".into(),
        id: item.id.clone(),
        decision: "allow".into(),
        answer: None
    }));
    assert!(broker.answer(InteractionAnswer {
        session_id: "session".into(),
        id: item.id.clone(),
        decision: "allow".into(),
        answer: None
    }));
    assert!(!broker.answer(InteractionAnswer {
        session_id: "session".into(),
        id: item.id,
        decision: "allow".into(),
        answer: None
    }));
    assert_eq!(task.await.unwrap().unwrap(), "allow");
    let task = tokio::spawn({
        let ctx = ctx.clone();
        async move {
            ctx.interactions
                .request(&ctx, "question", "question", "answer?", vec![])
                .await
        }
    });
    tokio::task::yield_now().await;
    ctx.cancellation.cancel();
    assert!(task.await.unwrap().is_err());
    assert!(broker.list("session").is_empty());
}
fn definition(name: &str, side_effect: bool) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: name.into(),
        input_schema: json!({"type":"object"}),
        output_schema: json!({}),
        side_effect,
    }
}
fn context(cwd: Option<std::path::PathBuf>) -> ToolContext {
    ToolContext {
        session_id: "session".into(),
        run_id: "run".into(),
        call_id: "call".into(),
        cwd,
        policy: PermissionPolicy::ReadOnly,
        cancellation: Cancellation::default(),
        interactions: Arc::new(InteractionBroker::default()),
        sink: None,
    }
}
#[tokio::test]
async fn timeout_cancels_only_the_call_and_does_not_hang_on_uncooperative_handlers() {
    let registry = ToolRegistry::default();
    registry
        .register_with_timeout(
            "test",
            definition("slow", false),
            Duration::from_millis(10),
            |_, _| Box::pin(std::future::pending()),
        )
        .unwrap();
    let ctx = context(None);
    let result = tokio::time::timeout(
        Duration::from_secs(7),
        registry.snapshot().execute("slow", json!({}), ctx.clone()),
    )
    .await
    .unwrap();
    assert!(result.unwrap_err().contains("timed out"));
    assert!(!ctx.cancellation.is_cancelled());
}
#[tokio::test]
async fn shell_output_and_cancellation_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let admitted = registry.snapshot();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    let result = admitted
        .execute("shell", json!({"command":"echo hello"}), ctx.clone())
        .await
        .unwrap();
    assert!(result["stdout"].as_str().unwrap().contains("hello"));
    #[cfg(windows)]
    let command = "Start-Sleep -Seconds 30";
    #[cfg(not(windows))]
    let command = "sleep 30";
    let task = tokio::spawn({
        let ctx = ctx.clone();
        async move {
            admitted
                .execute("shell", json!({"command":command}), ctx)
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    ctx.cancellation.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(7), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
}

#[tokio::test]
async fn rig_dispatch_preserves_exact_call_identity_and_original_arguments() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut bodies = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut raw = Vec::new();
            let mut byte = [0u8; 1];
            while !raw.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                raw.push(byte[0]);
            }
            let headers = String::from_utf8(raw).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            bodies.push(serde_json::from_slice::<serde_json::Value>(&body).unwrap());
            let message = if index == 0 {
                json!({"role":"assistant","content":null,"tool_calls":[{"id":"wire-call-exact","type":"function","function":{"name":"echo","arguments":"{\"value\":42}"}}]})
            } else {
                json!({"role":"assistant","content":"done"})
            };
            let response=json!({"id":format!("reply-{index}"),"object":"chat.completion","created":1,"model":"fixture","choices":[{"index":0,"message":message,"finish_reason":if index==0{"tool_calls"}else{"stop"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        }
        bodies
    });
    let registry = ToolRegistry::default();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut def = definition("echo", false);
    def.input_schema = json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false});
    registry
        .register("test", def, {
            let seen = seen.clone();
            move |ctx, args| {
                seen.lock().unwrap().push((ctx.call_id, args.clone()));
                Box::pin(async move { Ok(args) })
            }
        })
        .unwrap();
    let admitted = registry.snapshot();
    let mut ctx = context(None);
    ctx.call_id.clear();
    let model = rig_core::providers::openai::OpenAIConfig::new("test")
        .with_base_url(format!("http://{address}"))
        .client()
        .chat("fixture");
    let agent = rig_agent::AgentBuilder::new(model)
        .dynamic_tools(admitted.rig_tools(ctx))
        .build();
    let result = agent
        .prompt("call echo")
        .max_turns(4)
        .add_hook(admitted.identity_hook())
        .await
        .unwrap();
    assert!(!result.messages.is_empty());
    assert_eq!(
        *seen.lock().unwrap(),
        vec![("wire-call-exact".to_string(), json!({"value":42}))]
    );
    let bodies = server.join().unwrap();
    assert!(bodies[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["role"] == "tool" && m["tool_call_id"] == "wire-call-exact"));
}
#[tokio::test]
async fn cancelled_pending_answer_is_rejected_before_waiter_is_polled() {
    let ctx = context(None);
    let broker = ctx.interactions.clone();
    let waiter = tokio::spawn({
        let ctx = ctx.clone();
        async move {
            ctx.interactions
                .request(&ctx, "approval", "test", "approve?", vec![])
                .await
        }
    });
    tokio::task::yield_now().await;
    let pending = broker.list("session").pop().unwrap();
    ctx.cancellation.cancel();
    assert!(!broker.answer(InteractionAnswer {
        session_id: "session".into(),
        id: pending.id,
        decision: "allow".into(),
        answer: None
    }));
    assert!(waiter.await.unwrap().is_err());
}
#[tokio::test]
async fn symlink_or_junction_escape_is_denied_for_reads_and_writes() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "unchanged").unwrap();
    let link = workspace.path().join("escape");
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let admitted = registry.snapshot();
    let mut ctx = context(Some(workspace.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    assert!(admitted
        .execute(
            "read_file",
            json!({"path":"escape/secret.txt"}),
            ctx.clone()
        )
        .await
        .is_err());
    assert!(admitted
        .execute(
            "write_file",
            json!({"path":"escape/secret.txt","content":"escaped"}),
            ctx
        )
        .await
        .is_err());
    assert_eq!(
        std::fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
        "unchanged"
    );
}
#[tokio::test]
async fn ask_policy_allows_exactly_one_call_then_asks_again() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let admitted = registry.snapshot();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Ask;
    let broker = ctx.interactions.clone();
    let task = tokio::spawn({
        let ctx = ctx.clone();
        let admitted = admitted.clone();
        async move {
            admitted
                .execute("write_file", json!({"path":"a","content":"first"}), ctx)
                .await
        }
    });
    tokio::task::yield_now().await;
    let pending = broker.list("session").pop().unwrap();
    assert!(!dir.path().join("a").exists());
    assert!(broker.answer(InteractionAnswer {
        session_id: "session".into(),
        id: pending.id,
        decision: "allow".into(),
        answer: None
    }));
    task.await.unwrap().unwrap();
    ctx.call_id = "second".into();
    let task = tokio::spawn({
        let ctx = ctx.clone();
        async move {
            admitted
                .execute("write_file", json!({"path":"a","content":"second"}), ctx)
                .await
        }
    });
    tokio::task::yield_now().await;
    let pending = broker.list("session").pop().unwrap();
    assert_eq!(pending.call_id, "second");
    assert!(broker.answer(InteractionAnswer {
        session_id: "session".into(),
        id: pending.id,
        decision: "deny".into(),
        answer: None
    }));
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a")).unwrap(),
        "first"
    );
}
#[cfg(windows)]
#[tokio::test]
async fn shell_reaps_descendants_even_after_the_parent_exits() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    let result=registry.snapshot().execute("shell",json!({"command":"$p = Start-Process -WindowStyle Hidden -FilePath powershell.exe -ArgumentList '-NoProfile -NonInteractive -Command Start-Sleep -Seconds 30' -PassThru; Write-Output $p.Id"}),ctx).await.unwrap();
    let pid = result["stdout"]
        .as_str()
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 1 }}"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "shell leaked descendant {pid}");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_patches_do_not_overwrite_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let admitted = registry.snapshot();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    let content = (0..32).map(|i| format!("[{i}] ")).collect::<String>();
    std::fs::write(dir.path().join("a"), content).unwrap();
    let mut tasks = Vec::new();
    let barrier = Arc::new(tokio::sync::Barrier::new(32));
    for i in 0..32 {
        let admitted = admitted.clone();
        let ctx = ctx.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            admitted
                .execute(
                    "patch_file",
                    json!({"path":"a","oldText":format!("[{i}]"),"newText":format!("done-{i}")}),
                    ctx,
                )
                .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let content = std::fs::read_to_string(dir.path().join("a")).unwrap();
    for i in 0..32 {
        assert!(content.contains(&format!("done-{i} ")), "lost patch {i}");
    }
}
#[tokio::test]
async fn shell_handles_multiline_unicode_scripts_and_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    #[cfg(windows)]
    let command =
        "if ($true) {\n Write-Output '\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}'\n}\nexit 7";
    #[cfg(not(windows))]
    let command = "printf '\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}'; exit 7";
    let result = registry
        .snapshot()
        .execute("shell", json!({"command":command}), ctx)
        .await
        .unwrap();
    assert_eq!(result["exitCode"], 7);
    assert!(result["stdout"]
        .as_str()
        .unwrap()
        .contains("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}"));
}
#[tokio::test]
async fn native_read_search_and_successful_patch_return_canonical_results() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "first\nneedle\nlast\n").unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let admitted = registry.snapshot();
    let mut ctx = context(Some(dir.path().into()));
    assert_eq!(
        admitted
            .execute("read_file", json!({"path":"a.txt"}), ctx.clone())
            .await
            .unwrap()["content"],
        "first\nneedle\nlast\n"
    );
    let search = admitted
        .execute("search_files", json!({"pattern":"needle"}), ctx.clone())
        .await
        .unwrap();
    assert_eq!(search["matches"][0]["line"], 2);
    assert!(search["matches"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with("a.txt"));
    ctx.policy = PermissionPolicy::Allow;
    let patch = admitted
        .execute(
            "patch_file",
            json!({"path":"a.txt","oldText":"needle","newText":"changed"}),
            ctx,
        )
        .await
        .unwrap();
    assert!(patch["diff"].as_str().unwrap().contains("+changed"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "first\nchanged\nlast\n"
    );
}
#[tokio::test]
async fn output_schema_is_enforced_and_durable_sink_failure_prevents_side_effects() {
    let registry = ToolRegistry::default();
    let mut def = definition("wrong", false);
    def.output_schema = json!({"type":"integer"});
    registry
        .register("test", def, |_, _| Box::pin(async { Ok(json!("wrong")) }))
        .unwrap();
    assert!(registry
        .snapshot()
        .execute("wrong", json!({}), context(None))
        .await
        .unwrap_err()
        .contains("invalid tool output"));
    struct Reject;
    impl ToolEventSink for Reject {
        fn emit(
            &self,
            _: &ToolContext,
            _: &str,
            _: &serde_json::Value,
            _: &str,
            _: Option<&Result<serde_json::Value, String>>,
        ) -> Result<(), String> {
            Err("disk full".into())
        }
    }
    register_native(&registry).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    ctx.sink = Some(Arc::new(Reject));
    assert!(registry
        .snapshot()
        .execute("write_file", json!({"path":"a","content":"never"}), ctx)
        .await
        .is_err());
    assert!(!dir.path().join("a").exists());
}
#[tokio::test]
async fn bounded_search_reports_files_it_cannot_fully_inspect() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("large.txt"), "needle\n".repeat(12000)).unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let result = registry
        .snapshot()
        .execute(
            "search_files",
            json!({"pattern":"needle"}),
            context(Some(dir.path().into())),
        )
        .await
        .unwrap();
    assert_eq!(result["truncated"], true);
}
#[cfg(windows)]
#[tokio::test]
async fn failed_atomic_replacement_keeps_original_and_removes_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readonly.txt");
    std::fs::write(&path, "original").unwrap();
    let original_permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    let result = registry
        .snapshot()
        .execute(
            "write_file",
            json!({"path":"readonly.txt","content":"replacement"}),
            ctx,
        )
        .await;
    let content = std::fs::read_to_string(&path).unwrap();
    let count = std::fs::read_dir(dir.path()).unwrap().count();
    std::fs::set_permissions(&path, original_permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(content, "original");
    assert_eq!(count, 1, "failed edit leaked a temporary file");
}
#[tokio::test]
async fn native_read_and_patch_reject_directories() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("directory")).unwrap();
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let mut ctx = context(Some(dir.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    for name in ["read_file", "patch_file"] {
        let args = if name == "read_file" {
            json!({"path":"directory"})
        } else {
            json!({"path":"directory","oldText":"x","newText":"y"})
        };
        assert!(registry
            .snapshot()
            .execute(name, args, ctx.clone())
            .await
            .is_err());
    }
}
#[cfg(unix)]
#[test]
fn fifo_sources_are_rejected_in_guarded_child() {
    use std::process::Command;
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fifo_sources_child_probe", "--nocapture"])
        .env("MINERVA_FIFO_CHILD", "1")
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("FIFO source open blocked the child process")
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(unix)]
#[test]
fn fifo_sources_child_probe() {
    if std::env::var("MINERVA_FIFO_CHILD").as_deref() != Ok("1") {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(root.path().join("fifo"))
        .status()
        .unwrap()
        .success());
    let registry = ToolRegistry::default();
    register_native(&registry).unwrap();
    let mut ctx = context(Some(root.path().into()));
    ctx.policy = PermissionPolicy::Allow;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for name in ["read_file", "patch_file"] {
        let args = if name == "read_file" {
            json!({"path":"fifo"})
        } else {
            json!({"path":"fifo","oldText":"x","newText":"y"})
        };
        assert!(runtime
            .block_on(registry.snapshot().execute(name, args, ctx.clone()))
            .is_err());
    }
    assert!(std::process::Command::new("mkfifo")
        .arg(root.path().join("AGENTS.md"))
        .status()
        .unwrap()
        .success());
    let service =
        minerva_runtime::context::ContextService::open(&root.path().join("context.sqlite3"))
            .unwrap();
    service
        .workspace_save(
            "fifo",
            minerva_runtime::context::WorkspaceSettings {
                cwd: Some(root.path().to_str().unwrap().into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(service.prepare("fifo", &[]).is_err());
}
