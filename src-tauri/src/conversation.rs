//! Executes a turn using the session's stored branch and the selected engine.
use crate::{
    codex::CodexOptions,
    plugins::PluginManager,
    sessions::{IntegrationState, SessionService},
};
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRunResult {
    pub text: String,
    pub summary: Option<String>,
    pub integration: Option<IntegrationState>,
}

pub async fn run(
    store: &SessionService,
    manager: &PluginManager,
    id: &str,
    model: &str,
    last_message_id: &str,
    options: serde_json::Value,
) -> Result<SessionRunResult, String> {
    let (turns, parent) = store.history(id, last_message_id)?;
    if let Some(codex_model) = model.strip_prefix("codex:") {
        let state = store.integration_state(id, "codex")?;
        let thread = matching_codex_thread(state.as_ref(), model, parent.as_deref());
        let options: CodexOptions = serde_json::from_value(options)
            .map_err(|e| format!("Некорректные настройки Codex: {e}"))?;
        let result = manager
            .codex()?
            .chat(codex_model.into(), thread, turns, options)
            .await?;
        return Ok(SessionRunResult {
            text: result.text,
            summary: Some(result.summary),
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
    let text = manager.providers()?.chat(provider, model, turns).await?;
    Ok(SessionRunResult {
        text,
        summary: None,
        integration: None,
    })
}

fn matching_codex_thread(state: Option<&serde_json::Value>, model: &str, parent: Option<&str>) -> Option<String> {
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
    #[test]
    fn resumes_codex_only_from_matching_saved_response() {
        let state = json!({"model":"codex:model", "threadId":"thread", "anchorMessageId":"answer"});
        assert_eq!(matching_codex_thread(Some(&state), "codex:model", Some("answer")).as_deref(), Some("thread"));
        assert_eq!(matching_codex_thread(Some(&state), "codex:other", Some("answer")), None);
        assert_eq!(matching_codex_thread(Some(&state), "codex:model", Some("other")), None);
        let unanchored = json!({"model":"codex:model", "threadId":"thread", "anchorMessageId":null});
        assert_eq!(matching_codex_thread(Some(&unanchored), "codex:model", None), None);
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
            "one",
            &format!("api:{pid}/test-model"),
            "u2",
            json!({}),
        )
        .await
        .unwrap();
        assert_eq!(result.text, "local reply");
        assert!(result.integration.is_none());
        server.join().unwrap();
        manager.providers().unwrap().delete(&pid).await.unwrap();
        manager.shutdown().await;
    }
}
