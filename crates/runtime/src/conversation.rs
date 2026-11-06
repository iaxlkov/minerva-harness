//! Executes a turn using the session's stored branch and the selected engine.
use crate::{
    codex::CodexOptions,
    plugins::PluginManager,
    runs::Runs,
    sessions::{ContentPart, IntegrationState, SessionMessage, SessionService},
};
use serde_json::{json, Map};
#[derive(serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub model: String,
    pub last_message_id: String,
    pub response_id: String,
    pub created_at: String,
    #[ts(type = "JsonValue")]
    pub options: serde_json::Value,
}
#[derive(Debug, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionRunResult {
    pub parts: Vec<ContentPart>,
    #[serde(skip)]
    replay: Map<String, serde_json::Value>,
    #[serde(skip)]
    integration: Option<IntegrationState>,
}

// A dropped IPC task must not leave the session permanently marked running.
struct Settlement<'a> {
    store: &'a SessionService,
    session: &'a str,
    id: String,
    settled: bool,
}
impl Drop for Settlement<'_> {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.store.fail_run(
                self.session,
                &self.id,
                false,
                "Запуск прерван до сохранения результата.",
            );
        }
    }
}

pub async fn run(
    store: &SessionService,
    manager: &PluginManager,
    runs: &Runs,
    id: &str,
    request: RunRequest,
) -> Result<SessionRunResult, String> {
    let mut guard = runs.acquire(id, &request.response_id)?;
    run_inner(store, manager, id, request, &mut guard, false).await
}

pub(crate) async fn run_inner(
    store: &SessionService,
    manager: &PluginManager,
    id: &str,
    request: RunRequest,
    guard: &mut crate::runs::RunGuard,
    accepted: bool,
) -> Result<SessionRunResult, String> {
    let (turns, parent) = store.history(id, &request.last_message_id)?;
    if turns
        .last()
        .is_none_or(|m| m.role != "user" || m.text().trim().is_empty())
        || request.created_at.is_empty()
    {
        return Err("Нужно непустое сообщение пользователя и время ответа.".into());
    }
    // Capture reusable state before invalidating it: a failed remote turn may
    // already have mutated the server's thread, so the old anchor is unsafe.
    let state = store.integration_state(id, "codex")?;
    if !accepted {
        store.begin_run(id, &request.response_id, &request.last_message_id,
        &json!({"model":request.model,"options":request.options,"lastMessageId":request.last_message_id,"preamble":if request.model.starts_with("api:"){Some(crate::providers::PREAMBLE)}else{None}}),
        request.model.starts_with("codex:").then_some("codex"))?;
    }
    if accepted && request.model.starts_with("codex:") {
        store.invalidate_integration(id, "codex")?;
    }
    let mut settlement = Settlement {
        store,
        session: id,
        id: request.response_id.clone(),
        settled: false,
    };
    let durable_store = store.clone();
    let session_id = id.to_owned();
    let response_id = request.response_id.clone();
    let parent_id = request.last_message_id.clone();
    let created_at = request.created_at.clone();
    let sink: crate::transcript::TranscriptSink = std::sync::Arc::new(move |transcript| {
        let mut extensions = Map::new();
        extensions.insert("replay".into(), serde_json::json!(transcript.replay));
        durable_store.checkpoint(
            &session_id,
            &SessionMessage {
                id: response_id.clone(),
                parent_id: Some(parent_id.clone()),
                role: "assistant".into(),
                created_at: created_at.clone(),
                parts: transcript.parts.clone(),
                extensions,
            },
        )
    });
    let result = tokio::select! {
        biased;
        _ = async { if !*guard.cancel.borrow() { let _ = guard.cancel.changed().await; } } => Err("Запрос отменён.".to_string()),
        result = execute(manager,&request,turns,parent,state,sink) => result,
    };
    let result = result.and_then(|result| {
        if result.parts.is_empty() {
            return Err("Модель не вернула содержимое ответа.".into());
        }
        let mut extensions = Map::new();
        extensions.insert("replay".into(), json!(result.replay));
        let message = SessionMessage {
            id:request.response_id.clone(), parent_id:Some(request.last_message_id),
            role:"assistant".into(), created_at:request.created_at,
            parts:result.parts.clone(), extensions,
        };
        let state = result.integration.as_ref().map(|s| IntegrationState {
            namespace:s.namespace.clone(),
            value:json!({"model":request.model,"threadId":s.value["threadId"],"anchorMessageId":message.id}),
        });
        store.commit_reply(id, &message, state.as_ref())?;
        Ok(result)
    });
    if let Err(error) = &result {
        store.fail_run(id, &request.response_id, *guard.cancel.borrow(), error)?;
    }
    settlement.settled = true;
    result
}

async fn execute(
    manager: &PluginManager,
    request: &RunRequest,
    turns: Vec<crate::sessions::Turn>,
    parent: Option<String>,
    state: Option<serde_json::Value>,
    sink: crate::transcript::TranscriptSink,
) -> Result<SessionRunResult, String> {
    let model = request.model.as_str();
    if let Some(codex_model) = model.strip_prefix("codex:") {
        let thread = matching_codex_thread(state.as_ref(), model, parent.as_deref());
        let options: CodexOptions = serde_json::from_value(request.options.clone())
            .map_err(|e| format!("Некорректные настройки Codex: {e}"))?;
        let result = manager
            .codex()?
            .chat_recorded(codex_model.into(), thread, turns, options, sink)
            .await?;
        return Ok(SessionRunResult {
            parts: result.transcript.parts,
            replay: result.transcript.replay,
            integration: Some(IntegrationState {
                namespace: "codex".into(),
                value: serde_json::json!({"model":model,"threadId":result.thread_id}),
            }),
        });
    }
    let value = model
        .strip_prefix("api:")
        .ok_or("Выберите модель в настройках.")?;
    let (provider, model) = value.split_once('/').ok_or("Некорректный ID модели")?;
    let transcript = manager
        .providers()?
        .chat_recorded(provider, model, turns, sink)
        .await?;
    Ok(SessionRunResult {
        parts: transcript.parts,
        replay: transcript.replay,
        integration: None,
    })
}

fn matching_codex_thread(
    state: Option<&serde_json::Value>,
    model: &str,
    parent: Option<&str>,
) -> Option<String> {
    let state = state?;
    let parent = parent?;
    if state["model"] != model || state["anchorMessageId"].as_str()? != parent {
        return None;
    }
    state["threadId"].as_str().map(str::to_owned)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers::ProviderInput,
        sessions::{ContentPart, SessionMessage},
    };
    use serde_json::{json, Map};
    use std::io::{Read, Write};
    #[tokio::test]
    async fn failure_and_cancellation_are_not_model_messages_and_release_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = SessionService::open(&path).unwrap();
        store.create("one").unwrap();
        store
            .save(
                "one",
                SessionMessage {
                    id: "u1".into(),
                    parent_id: None,
                    role: "user".into(),
                    created_at: "2026-11-06".into(),
                    parts: vec![ContentPart {
                        kind: "text".into(),
                        data: json!({"text":"question"}),
                    }],
                    extensions: Map::new(),
                },
                None,
            )
            .unwrap();
        let manager = PluginManager::start(dir.path().join("shell.json"))
            .await
            .unwrap();
        let runs = Runs::default();
        let request = |model: String, id: &str| RunRequest {
            model,
            last_message_id: "u1".into(),
            response_id: id.into(),
            created_at: "2026-11-06".into(),
            options: json!({}),
        };
        assert!(run(
            &store,
            &manager,
            &runs,
            "one",
            request("api:missing/model".into(), "failed")
        )
        .await
        .is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let provider = format!("canceltest{}", std::process::id());
        manager
            .providers()
            .unwrap()
            .save(ProviderInput {
                id: provider.clone(),
                name: "Test".into(),
                base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
                models: vec!["model".into()],
                api_key: Some("temporary-local-key".into()),
                is_new: true,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        let task = run(
            &store,
            &manager,
            &runs,
            "one",
            request(format!("api:{provider}/model"), "cancelled"),
        );
        let cancel = async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            assert!(runs.cancel("one", "cancelled").unwrap());
        };
        let (result, ()) = tokio::join!(task, cancel);
        assert!(result.unwrap_err().contains("отменён"));
        let mut dropped = Box::pin(run(
            &store,
            &manager,
            &runs,
            "one",
            request(format!("api:{provider}/model"), "dropped"),
        ));
        tokio::select! { result=&mut dropped => panic!("request unexpectedly finished: {result:?}"), _=tokio::time::sleep(std::time::Duration::from_millis(20)) => {} }
        drop(dropped);
        assert_eq!(store.load("one").unwrap().len(), 1);
        let db = rusqlite::Connection::open(&path).unwrap();
        for (id, expected) in [
            ("failed", "failed"),
            ("cancelled", "cancelled"),
            ("dropped", "failed"),
        ] {
            let status: String = db
                .query_row("SELECT status FROM runs WHERE id=?1", [id], |r| r.get(0))
                .unwrap();
            assert_eq!(status, expected);
        }
        assert!(runs.acquire("one", "next").is_ok());
        manager
            .providers()
            .unwrap()
            .delete(&provider)
            .await
            .unwrap();
        manager.shutdown().await;
    }
    #[test]
    fn resumes_codex_only_from_matching_saved_response() {
        let state = json!({"model":"codex:model", "threadId":"thread", "anchorMessageId":"answer"});
        assert_eq!(
            matching_codex_thread(Some(&state), "codex:model", Some("answer")).as_deref(),
            Some("thread")
        );
        assert_eq!(
            matching_codex_thread(Some(&state), "codex:other", Some("answer")),
            None
        );
        assert_eq!(
            matching_codex_thread(Some(&state), "codex:model", Some("other")),
            None
        );
        let unanchored =
            json!({"model":"codex:model", "threadId":"thread", "anchorMessageId":null});
        assert_eq!(
            matching_codex_thread(Some(&unanchored), "codex:model", None),
            None
        );
    }
    #[tokio::test]
    async fn sends_stored_branch_and_keeps_sessions_separate() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionService::open(&dir.path().join("history.sqlite")).unwrap();
        store.create("one").unwrap();
        store.create("two").unwrap();
        let message = |id: &str, parent: Option<&str>, role: &str, text: &str| SessionMessage {
            id: id.into(),
            parent_id: parent.map(str::to_owned),
            role: role.into(),
            created_at: "2026-01-01".into(),
            parts: vec![ContentPart {
                kind: "text".into(),
                data: json!({"text":text}),
            }],
            extensions: Map::new(),
        };
        store
            .save("one", message("u1", None, "user", "first question"), None)
            .unwrap();
        store
            .save(
                "one",
                message("a1", Some("u1"), "assistant", "first response"),
                None,
            )
            .unwrap();
        store
            .save(
                "one",
                message("u2", Some("a1"), "user", "next question"),
                None,
            )
            .unwrap();
        store
            .save(
                "two",
                message("secret", None, "user", "other session only"),
                None,
            )
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 2048];
            loop {
                let n = socket.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
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
            let request = String::from_utf8_lossy(&data);
            assert!(request.contains("first question"));
            assert!(request.contains("first response"));
            assert!(request.contains("next question"));
            assert!(!request.contains("other session only"));
            let body = r#"{"id":"test","object":"chat.completion","created":1,"model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"local reply"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let manager = PluginManager::start(dir.path().join("shell.json"))
            .await
            .unwrap();
        let pid = format!("sessiontest{}", std::process::id());
        manager
            .providers()
            .unwrap()
            .save(ProviderInput {
                id: pid.clone(),
                name: "Test".into(),
                base_url: format!("http://{address}/v1"),
                models: vec!["test-model".into()],
                api_key: Some("temporary-test-key".into()),
                is_new: true,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        let result = run(
            &store,
            &manager,
            &Runs::default(),
            "one",
            RunRequest {
                model: format!("api:{pid}/test-model"),
                last_message_id: "u2".into(),
                response_id: "a2".into(),
                created_at: "2026-01-01".into(),
                options: json!({}),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.parts[0].data["text"], "local reply");
        assert!(result.integration.is_none());
        let saved = store.load("one").unwrap();
        assert_eq!(saved.last().unwrap().id, "a2");
        assert_eq!(saved.last().unwrap().parts[0].data["text"], "local reply");
        server.join().unwrap();
        manager.providers().unwrap().delete(&pid).await.unwrap();
        manager.shutdown().await;
    }
}
