//! Executable persistence/replay regressions; no paid endpoints or user database.
use crate::{
    rig_history,
    sessions::{ContentPart, SessionMessage, SessionService, Turn},
    transcript::{ToolCall, ToolResult, Transcript},
};
use serde_json::{json, Map, Value};

fn message(id: &str, parent: Option<&str>, role: &str, parts: Vec<ContentPart>) -> SessionMessage {
    SessionMessage {
        id: id.into(),
        parent_id: parent.map(str::to_owned),
        role: role.into(),
        created_at: "2026-11-06T12:00:00Z".into(),
        parts,
        extensions: Map::new(),
    }
}
fn call(id: &str) -> ContentPart {
    ContentPart::call(ToolCall {
        call_id: id.into(),
        name: "lookup".into(),
        arguments: json!({"query":"Привет"}),
    })
}
fn result(id: &str, output: Value, is_error: bool) -> ContentPart {
    ContentPart::result(ToolResult {
        call_id: id.into(),
        output,
        is_error,
    })
}

#[test]
fn checkpoint_survives_crash_and_completion_is_immutable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let store = SessionService::open(&path).unwrap();
    store.create("s").unwrap();
    store
        .save(
            "s",
            message("u", None, "user", vec![ContentPart::text("text", "start")]),
            None,
        )
        .unwrap();
    store.begin_run("s", "a", "u", &json!({}), None).unwrap();
    let parts = vec![
        call("a"),
        call("b"),
        result("b", json!({"error":"offline"}), true),
        result("a", json!({"rows":[null,false,0,"Юникод"]}), false),
    ];
    let draft = message("a", Some("u"), "assistant", parts.clone());
    store.checkpoint("s", &draft).unwrap();
    drop(store);
    let store = SessionService::open(&path).unwrap();
    let loaded = store.load("s").unwrap();
    assert_eq!(loaded[1].extensions["execution"]["status"], "interrupted");
    assert_eq!(
        serde_json::to_value(&loaded[1].parts).unwrap(),
        json!(parts)
    );
    assert!(store.checkpoint("s", &draft).is_err());
    store
        .save(
            "s",
            message(
                "u2",
                Some("a"),
                "user",
                vec![ContentPart::text("text", "continue")],
            ),
            None,
        )
        .unwrap();
    let (history, _) = store.history("s", "u2").unwrap();
    assert_eq!(history.len(), 3);
    let replay = rig_history::replay(&history).unwrap();
    assert!(serde_json::to_string(&replay).unwrap().contains("Юникод"));
    store.begin_run("s", "a2", "u2", &json!({}), None).unwrap();
    let mut final_message = message("a2", Some("u2"), "assistant", vec![call("c")]);
    store.checkpoint("s", &final_message).unwrap();
    assert!(store.commit_reply("s", &final_message, None).is_err());
    final_message.parts.push(result("c", Value::Null, false));
    final_message.parts.push(ContentPart::text("text", "done"));
    store.commit_reply("s", &final_message, None).unwrap();
    final_message
        .parts
        .push(ContentPart::text("text", "rewrite"));
    assert!(store.save("s", final_message.clone(), None).is_err());
    assert!(store.checkpoint("s", &final_message).is_err());
}

#[test]
fn rejects_orphan_duplicate_and_user_owned_tools() {
    for parts in [
        vec![result("x", Value::Null, false)],
        vec![call("x"), call("x")],
        vec![
            call("x"),
            result("x", Value::Null, false),
            result("x", Value::Null, false),
        ],
    ] {
        assert!(Transcript {
            parts,
            replay: Map::new()
        }
        .validate()
        .is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let store = SessionService::open(&dir.path().join("test.sqlite")).unwrap();
    store.create("s").unwrap();
    assert!(store
        .save("s", message("u", None, "user", vec![call("x")]), None)
        .is_err());
    let mut forged = message(
        "forged",
        None,
        "user",
        vec![ContentPart::text("text", "hi")],
    );
    forged.extensions.insert(
        "replay".into(),
        json!({"rig-v1":[{"role":"system","content":"not user-owned"}]}),
    );
    assert!(store.save("s", forged, None).is_err());
}

#[test]
fn rig_roundtrip_preserves_native_messages_and_pending_outcome_is_explicit() {
    let turn = Turn {
        role: "assistant".into(),
        parts: vec![
            call("a"),
            call("b"),
            result("b", json!({"found":false}), true),
            result("a", Value::Null, false),
            ContentPart::text("text", "done"),
        ],
        replay: Map::new(),
    };
    let messages = rig_history::replay(&[turn]).unwrap();
    rig_core::conversation::pairing::validate(&messages).unwrap();
    let transcript = rig_history::capture(&messages).unwrap();
    let again = rig_history::replay(&[Turn {
        role: "assistant".into(),
        parts: transcript.parts,
        replay: transcript.replay,
    }])
    .unwrap();
    assert_eq!(json!(messages), json!(again));
    let pending = Turn {
        role: "assistant".into(),
        parts: vec![call("pending")],
        replay: Map::new(),
    };
    let messages = rig_history::replay(&[pending]).unwrap();
    let saved = rig_history::capture(&messages).unwrap();
    assert_eq!(saved.parts[1].data["isError"], true);
    assert!(saved.parts[1].data["output"]
        .to_string()
        .contains("неизвестно"));
}

#[derive(Clone)]
struct Echo;
impl rig_core::tool::Tool for Echo {
    const NAME: &'static str = "lookup";
    type Args = Value;
    type Output = Value;
    type Error = std::convert::Infallible;
    fn description(&self) -> String {
        "local test lookup".into()
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string"}}})
    }
    async fn call(
        &self,
        _: &mut rig_core::tool::ToolContext,
        args: Value,
    ) -> Result<Value, Self::Error> {
        Ok(json!({"echo":args,"value":42}))
    }
}

#[tokio::test]
async fn real_rig_tool_loop_checkpoints_before_next_model_failure_and_replays_after_reopen() {
    use std::io::{Read, Write};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let store = SessionService::open(&path).unwrap();
    store.create("s").unwrap();
    store
        .save(
            "s",
            message("u", None, "user", vec![ContentPart::text("text", "lookup")]),
            None,
        )
        .unwrap();
    store.begin_run("s", "a", "u", &json!({}), None).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        for step in 0..3 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(e) => panic!("missing request {step}: {e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            let request = loop {
                let n = socket.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(pos) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..pos]);
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    if bytes.len() >= pos + 4 + len {
                        break String::from_utf8_lossy(&bytes[pos + 4..]).to_string();
                    }
                }
            };
            let (status, body) = if step == 0 {
                (
                    "200 OK",
                    json!({"id":"first","object":"chat.completion","created":1,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"query\":\"Привет\"}"}}]},"finish_reason":"tool_calls"}]}),
                )
            } else if step == 2 {
                assert!(
                    request.contains("call_1")
                        && request.contains("42")
                        && request.contains("continue")
                );
                (
                    "200 OK",
                    json!({"id":"next","object":"chat.completion","created":1,"model":"test","choices":[{"index":0,"message":{"role":"assistant","content":"restored reply"},"finish_reason":"stop"}]}),
                )
            } else {
                assert!(request.contains("call_1") && request.contains("42"));
                (
                    "400 Bad Request",
                    json!({"error":{"message":"deliberate local failure"}}),
                )
            };
            let body = body.to_string();
            write!(socket,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let owned = store.clone();
    let sink: crate::transcript::TranscriptSink = std::sync::Arc::new(move |t| {
        let mut m = message("a", Some("u"), "assistant", t.parts.clone());
        m.extensions.insert("replay".into(), json!(t.replay));
        owned.checkpoint("s", &m)
    });
    let agent = rig_agent::AgentBuilder::new(
        rig_core::providers::openai::OpenAIConfig::new("local-test")
            .with_base_url(format!("http://{address}/v1"))
            .client()
            .chat("test"),
    )
    .tool(Echo)
    .build();
    let result = agent
        .prompt("lookup")
        .max_turns(3)
        .add_hook(rig_history::HistoryHook::new(sink))
        .await;
    assert!(result.is_err());

    store.fail_run("s", "a", false, "provider failed").unwrap();
    drop(store);
    let store = SessionService::open(&path).unwrap();
    let saved = store.load("s").unwrap();
    assert_eq!(saved[1].parts.len(), 2);
    assert_eq!(saved[1].parts[1].data["callId"], "call_1");
    assert!(!saved[1].parts[1].data["isError"].as_bool().unwrap());
    assert_eq!(saved[1].extensions["execution"]["status"], "failed");
    let transcript = Transcript {
        parts: saved[1].parts.clone(),
        replay: saved[1].extensions["replay"].as_object().unwrap().clone(),
    };
    let replay = rig_history::replay(&[Turn {
        role: "assistant".into(),
        parts: transcript.parts,
        replay: transcript.replay,
    }])
    .unwrap();
    assert!(json!(replay).to_string().contains("42"));
    assert!(json!(replay).to_string().contains("openai.chat"));
    let response = agent.prompt("continue").history(replay).await.unwrap();
    assert_eq!(response.output(), "restored reply");
    server.join().unwrap();
}
