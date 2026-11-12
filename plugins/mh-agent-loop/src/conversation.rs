//! Executes a turn using the session's stored branch and the selected engine.
use crate::{
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
    let messages = store.branch(id, &request.last_message_id)?;
    let parent = messages.last().and_then(|m| m.parent_id.clone());
    if messages.last().is_none_or(|m| {
        m.role != "user"
            || !m.parts.iter().any(|p| {
                matches!(p.kind.as_str(), "image" | "file")
                    || p.kind == "text"
                        && p.data["text"]
                            .as_str()
                            .is_some_and(|t| !t.trim().is_empty())
            })
    }) || request.created_at.is_empty()
    {
        return Err("Нужно непустое сообщение пользователя и время ответа.".into());
    }
    let namespace = crate::engine::model_ref(&request.model).map(|(id, _)| format!("plugin:{id}"));
    if namespace.is_some() && request.options.get("toolAllowlist").is_some() {
        return Err("toolAllowlist is unsupported by external engines".into());
    }
    let state = match &namespace {
        Some(namespace) => crate::engine::resume_state(
            store.integration_state(id, namespace)?,
            &request.model,
            parent.as_deref(),
        ),
        None => None,
    };
    if let Some(namespace) = &namespace {
        store.invalidate_integration(id, namespace)?;
    }
    if !accepted {
        store.begin_run(id, &request.response_id, &request.last_message_id,
        &json!({"model":request.model,"options":request.options,"lastMessageId":request.last_message_id,"preamble":if request.model.starts_with("api:"){Some(crate::providers::PREAMBLE)}else{None}}),
        None)?;
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
    let result: Result<SessionRunResult, String> = async {
        let mut prepared = manager.harness.context.prepare_enriched(id, &messages).await?;
        if prepared.budget.should_compact && !crate::context::compact_prefix(&messages, 4).is_empty() {
            manager
                .compact_messages_cancellable(id, &request.model, &messages, &mut guard.cancel)
                .await?;
            prepared = manager.harness.context.prepare_enriched(id, &messages).await?;
        }
        let mut turns = prepared.turns;
        let mut image_bytes = 0usize;
        for turn in &mut turns {
            for part in &mut turn.parts {
                if part.kind == "image" {
                    let image = manager.harness.context.get_attachment(
                        id,
                        part.data["id"].as_str().ok_or("Invalid image reference")?,
                    )?;
                    image_bytes = image_bytes.saturating_add(image.bytes);
                    if image_bytes > 20 * 1024 * 1024 {
                        return Err("Inline images exceed the 20 MiB context limit; compact earlier turns or start a new branch with fewer images".into());
                    }
                    part.data = serde_json::to_value(image).map_err(|e| e.to_string())?;
                }
            }
        }
        let cancellation = crate::tools::Cancellation::default();
        let _cleanup = crate::harness::RunCleanup {
            cancellation: cancellation.clone(),
            interactions: manager.harness.interactions.clone(),
            session: id.into(),
            run: request.response_id.clone(),
        };
        let settings = manager.harness.context.workspace_get(id)?;
        let allowlist = match request.options.get("toolAllowlist") {
            None => None,
            Some(value) => {
                let names = value.as_array().ok_or("toolAllowlist must be an array")?;
                if names.len() > 128 { return Err("toolAllowlist exceeds limit".into()); }
                Some(names.iter().map(|v| v.as_str().filter(|s| !s.is_empty() && s.len() <= 128).map(str::to_owned).ok_or("invalid toolAllowlist entry")).collect::<Result<Vec<_>,_>>()?)
            }
        };
        let run_context = crate::harness::AgentRunContext {
            session_id: id.into(),
            run_id: request.response_id.clone(),
            tools: tokio::select! {
                biased;
                _ = async { if !*guard.cancel.borrow() { let _ = guard.cancel.changed().await; } } => return Err("Run cancelled".into()),
                tools = manager.tools_for(id) => { let tools = tools?; match &allowlist { Some(names) => tools.restrict_to(names), None => tools } },
            },
            context: manager.harness.context.clone(),
            messages,
            reasoning_effort: match request.options.get("reasoningEffort") {
                None => None,
                Some(value) => Some(value.as_str().filter(|s| !s.is_empty() && s.len() <= 32).ok_or("invalid reasoningEffort")?.to_owned()),
            },
            tool_context: crate::tools::ToolContext {
                session_id: id.into(),
                run_id: request.response_id.clone(),
                call_id: String::new(),
                cwd: settings.cwd.map(std::path::PathBuf::from),
                policy: settings.permission_policy,
                cancellation,
                interactions: manager.harness.interactions.clone(),
                sink: None,
            },
        };
        if *guard.cancel.borrow() { return Err("Run cancelled".into()); }
        if let Some(namespace) = namespace {
            crate::engine::run_with_context(manager, &serde_json::to_string(&(id, &request.response_id)).map_err(|e| e.to_string())?, &request.model, turns, state, sink, &mut guard.cancel,Some(run_context)).await
                .map(|(transcript, state)| SessionRunResult {
                    parts: transcript.parts, replay: transcript.replay,
                    integration: state.map(|value| IntegrationState { namespace, value: json!({"model":request.model,"anchorMessageId":request.response_id,"value":value}) }),
                })
        } else {
            tokio::select! {
                biased;
                _ = async { if !*guard.cancel.borrow() { let _ = guard.cancel.changed().await; } } => Err("Запрос отменён.".to_string()),
                result = execute(manager,&request,turns,sink,run_context) => result,
            }
        }
    }.await;
    let result = result.and_then(|result| {
        if *guard.cancel.borrow() {
            return Err("Run cancelled".into());
        }
        if result.parts.is_empty() {
            return Err("Модель не вернула содержимое ответа.".into());
        }
        let mut extensions = Map::new();
        extensions.insert("replay".into(), json!(result.replay));
        let message = SessionMessage {
            id: request.response_id.clone(),
            parent_id: Some(request.last_message_id),
            role: "assistant".into(),
            created_at: request.created_at,
            parts: result.parts.clone(),
            extensions,
        };
        store.commit_reply(id, &message, result.integration.as_ref())?;
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
    sink: crate::transcript::TranscriptSink,
    context: crate::harness::AgentRunContext,
) -> Result<SessionRunResult, String> {
    let model = request.model.as_str();
    let value = model
        .strip_prefix("api:")
        .ok_or("Выберите модель в настройках.")?;
    let (provider, model) = value.split_once('/').ok_or("Некорректный ID модели")?;
    let transcript = manager
        .providers()?
        .chat_agent(provider, model, turns, sink, context)
        .await?;
    Ok(SessionRunResult {
        parts: transcript.parts,
        replay: transcript.replay,
        integration: None,
    })
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
    async fn cancellation_during_automatic_compaction_settles_without_summary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = SessionService::open(&path).unwrap();
        store.create("one").unwrap();
        for index in 0..9 {
            store
                .save(
                    "one",
                    SessionMessage {
                        id: format!("m{index}"),
                        parent_id: (index > 0).then(|| format!("m{}", index - 1)),
                        role: if index % 2 == 0 { "user" } else { "assistant" }.into(),
                        created_at: "now".into(),
                        parts: vec![ContentPart::text("text", "history".repeat(200))],
                        extensions: Map::new(),
                    },
                    None,
                )
                .unwrap();
        }
        let manager = PluginManager::start(dir.path().join("shell.json"))
            .await
            .unwrap();
        manager
            .harness
            .context
            .workspace_save(
                "one",
                crate::context::WorkspaceSettings {
                    context_window: 1024,
                    ..Default::default()
                },
            )
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let (close_tx, close_rx) = std::sync::mpsc::channel();
        let wire = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            while !String::from_utf8_lossy(&request).contains("Summarize") {
                let count = socket.read(&mut buf).unwrap();
                assert!(count > 0 && request.len() < 1024 * 1024);
                request.extend_from_slice(&buf[..count]);
            }
            let _ = seen_tx.send(());
            let _ = close_rx.recv_timeout(std::time::Duration::from_secs(5));
        });
        let provider = format!("compactcancel{}", std::process::id());
        manager
            .providers()
            .unwrap()
            .save(ProviderInput {
                id: provider.clone(),
                name: "fixture".into(),
                base_url: format!("http://{address}/v1"),
                models: vec!["model".into()],
                api_key: Some("test-only".into()),
                is_new: true,
                protocol: "openai-chat".into(),
            })
            .await
            .unwrap();
        let runs = Runs::default();
        let task = run(
            &store,
            &manager,
            &runs,
            "one",
            RunRequest {
                model: format!("api:{provider}/model"),
                last_message_id: "m8".into(),
                response_id: "reply".into(),
                created_at: "now".into(),
                options: json!({}),
            },
        );
        let cancel = async {
            seen_rx.await.unwrap();
            assert!(runs.cancel("one", "reply").unwrap());
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            tokio::join!(task, cancel)
        })
        .await
        .unwrap();
        close_tx.send(()).unwrap();
        wire.join().unwrap();
        assert!(result.unwrap_err().to_lowercase().contains("cancel"));
        let status: String = rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT status FROM runs WHERE id='reply'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "cancelled");
        let messages = store.load("one").unwrap();
        assert_eq!(messages.len(), 9);
        assert!(!manager
            .harness
            .context
            .prepare("one", &messages)
            .unwrap()
            .system_prompt
            .contains("Summary of earlier"));
        manager
            .providers()
            .unwrap()
            .delete(&provider)
            .await
            .unwrap();
        manager.shutdown().await;
    }
    #[tokio::test]
    async fn context_preparation_failure_records_actual_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = SessionService::open(&path).unwrap();
        store.create("one").unwrap();
        store
            .save(
                "one",
                SessionMessage {
                    id: "user".into(),
                    parent_id: None,
                    role: "user".into(),
                    created_at: "now".into(),
                    parts: vec![ContentPart::text("text", "hello")],
                    extensions: Map::new(),
                },
                None,
            )
            .unwrap();
        let manager = PluginManager::start(dir.path().join("shell.json"))
            .await
            .unwrap();
        manager
            .harness
            .context
            .workspace_save(
                "one",
                crate::context::WorkspaceSettings {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
            )
            .unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "x".repeat(1024 * 1024)).unwrap();
        let error = run(
            &store,
            &manager,
            &Runs::default(),
            "one",
            RunRequest {
                model: "api:missing/model".into(),
                last_message_id: "user".into(),
                response_id: "reply".into(),
                created_at: "now".into(),
                options: json!({}),
            },
        )
        .await
        .unwrap_err();
        let saved: (String, String) = rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT status,error FROM runs WHERE id='reply'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(saved, ("failed".into(), error));
        manager.shutdown().await;
    }
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
            assert!(String::from_utf8_lossy(&data).contains("\"stream\":true"));
            let body = concat!(
                "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"local reply\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            );
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
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
        let saved = store.load("one").unwrap();
        assert_eq!(saved.last().unwrap().id, "a2");
        assert_eq!(saved.last().unwrap().parts[0].data["text"], "local reply");
        server.join().unwrap();
        manager.providers().unwrap().delete(&pid).await.unwrap();
        manager.shutdown().await;
    }
}
