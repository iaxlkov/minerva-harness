use minerva_runtime::{Runtime, RuntimeConfig, StartRunRequest};
use std::sync::Arc;
#[tokio::test]
async fn accepted_request_survives_caller_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: "t".into(),
            workspace_id: "w".into(),
            profile: "local".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        })
        .await
        .unwrap(),
    );
    runtime.sessions.create("s").unwrap();
    let request:StartRunRequest=serde_json::from_value(serde_json::json!({"sessionId":"s","requestId":"r","parentId":null,"message":{"id":"u","text":"hello"},"model":"api:missing/model","options":{}})).unwrap();
    let handle = runtime.start_run(request.clone()).await.unwrap();
    assert_eq!(runtime.sessions.get("s").unwrap().title, "hello");
    drop(handle);
    runtime.start_run(request.clone()).await.unwrap();
    let mut changed = request.clone();
    changed.message.text = "different".into();
    assert!(runtime.start_run(changed).await.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(runtime.get_run("s", "r").unwrap().status, "failed");
    assert_eq!(runtime.sessions.load("s").unwrap().len(), 1);
    runtime.shutdown().await.unwrap();
}

async fn provider_ownership_case(fail_commit: bool) {
    use minerva_runtime::providers::ProviderInput;
    use std::io::{Read, Write};
    let dir = tempfile::tempdir().unwrap();
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: "t".into(),
            workspace_id: "w".into(),
            profile: "local".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        })
        .await
        .unwrap(),
    );
    runtime.sessions.create("s").unwrap();
    runtime.sessions.create("other").unwrap();
    if fail_commit {
        let db = rusqlite::Connection::open(dir.path().join("data/sessions.sqlite3")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF status ON runs WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'forced final commit failure'); END;").unwrap();
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut data = Vec::new();
        let mut buf = [0; 2048];
        loop {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buf[..n]);
            if let Some(pos) = data.windows(4).position(|v| v == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..pos]);
                let len = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if data.len() >= pos + 4 + len {
                    break;
                }
            }
        }
        seen_tx.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(String::from_utf8_lossy(&data).contains("\"stream\":true"));
        let body = concat!(
            "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"owned reply\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let provider = format!(
        "ownership{}{}",
        std::process::id(),
        if fail_commit { "fail" } else { "ok" }
    );
    runtime
        .plugins
        .providers()
        .unwrap()
        .save(ProviderInput {
            id: provider.clone(),
            name: "Test".into(),
            base_url: format!("http://{address}/v1"),
            models: vec!["model".into()],
            api_key: Some("temporary-key".into()),
            is_new: true,
            protocol: "openai-chat".into(),
        })
        .await
        .unwrap();
    let request:StartRunRequest=serde_json::from_value(serde_json::json!({"sessionId":"s","requestId":"r","parentId":null,"message":{"id":"u","text":"hello"},"model":format!("api:{provider}/model"),"options":{}})).unwrap();
    drop(runtime.start_run(request.clone()).await.unwrap());
    tokio::time::timeout(std::time::Duration::from_secs(5), seen_rx)
        .await
        .unwrap()
        .unwrap();
    runtime.start_run(request.clone()).await.unwrap();
    let mut other = request.clone();
    other.request_id = "r2".into();
    other.message.id = "u2".into();
    assert!(runtime.start_run(other.clone()).await.is_err());
    other.session_id = "other".into();
    runtime.start_run(other).await.unwrap();
    assert!(runtime.cancel_run("other", "r2").unwrap());
    assert!(runtime.cancel_run("other", "r2").unwrap());
    for _ in 0..100 {
        if runtime.get_run("s", "r").unwrap().status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        runtime.get_run("s", "r").unwrap().status,
        if fail_commit { "failed" } else { "completed" }
    );
    if fail_commit {
        assert!(runtime
            .get_run("s", "r")
            .unwrap()
            .error
            .unwrap()
            .contains("forced final commit failure"));
    }
    assert_eq!(
        runtime.get_run("s", "r").unwrap().parts[0].data["text"],
        "owned reply"
    );
    let messages = runtime.sessions.load("s").unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].created_at.contains('T') && messages[0].created_at.ends_with('Z'));
    let parser = rusqlite::Connection::open_in_memory().unwrap();
    let parsed: Option<f64> = parser
        .query_row("SELECT julianday(?1)", [&messages[0].created_at], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        parsed.is_some(),
        "server timestamp must parse as an ISO UTC instant"
    );

    runtime.start_run(request).await.unwrap();
    server.join().unwrap();
    runtime
        .plugins
        .providers()
        .unwrap()
        .delete(&provider)
        .await
        .unwrap();
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_cancels_owned_run_and_closes_store_after_settlement() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: "t".into(),
            workspace_id: "w".into(),
            profile: "local".into(),
            data_dir: data.clone(),
            config_dir: dir.path().join("config"),
        })
        .await
        .unwrap(),
    );
    runtime.sessions.create("s").unwrap();
    let request:StartRunRequest=serde_json::from_value(serde_json::json!({"sessionId":"s","requestId":"r","parentId":null,"message":{"id":"u","text":"hello"},"model":"api:missing/model","options":{}})).unwrap();
    runtime.start_run(request.clone()).await.unwrap();
    runtime.shutdown().await.unwrap();
    assert!(runtime.start_run(request).await.is_err());
    let db = rusqlite::Connection::open(data.join("sessions.sqlite3")).unwrap();
    let status: String = db
        .query_row("SELECT status FROM runs WHERE id='r'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "cancelled");
}

#[tokio::test]
async fn provider_executes_once_after_handle_disconnect_and_cancel_shutdown_drain() {
    provider_ownership_case(false).await;
}
#[tokio::test]
async fn final_commit_failure_settles_as_failed_without_reexecution() {
    provider_ownership_case(true).await;
}
