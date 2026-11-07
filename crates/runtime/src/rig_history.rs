//! Rig wire/history conversion and durable execution observations.
use crate::{
    sessions::{ContentPart, Turn},
    transcript::{ToolCall, ToolResult, Transcript, TranscriptSink},
};
use rig_agent::agent::hook::{
    AgentHook, HookContext, ModelTurnAction, ModelTurnFinished, ObservationAction, OutcomeAction,
    OutcomeEvent, ReasoningDelta, TextDelta,
};
use rig_core::message::{AssistantContent, AssistantMessage, Message, UserContent};
use serde_json::json;
use std::sync::{Arc, Mutex};

pub fn capture(messages: &[Message]) -> Result<Transcript, String> {
    let mut transcript = Transcript::default();
    for message in messages {
        match message {
            Message::Assistant(m) => {
                for part in &m.content {
                    match part {
                        AssistantContent::Text(t) => {
                            transcript.parts.push(ContentPart::text("text", &t.text))
                        }
                        AssistantContent::Reasoning(r) => transcript
                            .parts
                            .push(ContentPart::text("reasoning", &r.text)),
                        AssistantContent::ToolCall(c) => {
                            transcript.parts.push(ContentPart::call(ToolCall {
                                call_id: c.id.to_string(),
                                name: c.function.name.to_string(),
                                arguments: json!(c.function.arguments),
                            }))
                        }
                        other => transcript.parts.push(ContentPart {
                            kind: "extension".into(),
                            data: json!({"provider":"rig","content":other}),
                        }),
                    }
                }
            }
            Message::User { content } => {
                for part in content {
                    match part {
                        UserContent::ToolResult(r) => {
                            transcript.parts.push(ContentPart::result(ToolResult {
                                call_id: r.call.to_string(),
                                output: json!(r.content),
                                is_error: r.is_error,
                            }))
                        }
                        // Corrective feedback belongs to the provider transcript, not user-authored text.
                        other => transcript.parts.push(ContentPart {
                            kind: "extension".into(),
                            data: json!({"provider":"rig","content":other}),
                        }),
                    }
                }
            }
            Message::System { .. } => {
                return Err("Неожиданное system-сообщение в результате Rig.".into())
            }
        }
    }
    transcript.replay.insert("rig-v1".into(), json!(messages));
    transcript.validate()?;
    Ok(transcript)
}

pub fn replay(turns: &[Turn]) -> Result<Vec<Message>, String> {
    let mut messages = Vec::new();
    for turn in turns {
        if let Some(saved) = turn.replay.get("rig-v1") {
            let restored: Vec<Message> = serde_json::from_value(saved.clone())
                .map_err(|e| format!("Не удалось прочитать историю Rig: {e}"))?;
            // A checkpoint may end before a tool returned. Complete only the wire
            // context with an explicit unknown outcome; never execute it on load.
            messages.extend(close_pending(restored)?);
            continue;
        }
        if turn.role == "user" {
            messages.push(user_message(turn)?);
            continue;
        }
        let mut assistant = Vec::new();
        for part in &turn.parts {
            match part.kind.as_str() {
                "text" => assistant.push(AssistantContent::text(
                    part.data["text"].as_str().unwrap_or_default(),
                )),
                "reasoning" => {} // Provider-independent reasoning is display-only.
                "tool-call" => {
                    let call: ToolCall =
                        serde_json::from_value(part.data.clone()).map_err(|e| e.to_string())?;
                    let id = rig_core::message::ProviderCallId::new(call.call_id)
                        .ok_or("Пустой ID инструмента")?;
                    let name =
                        rig_core::message::ToolName::new(call.name).map_err(|e| e.to_string())?;
                    assistant.push(AssistantContent::ToolCall(
                        rig_core::message::ToolCall::new(
                            id.into(),
                            rig_core::message::ToolFunction::new(name, call.arguments),
                        ),
                    ));
                }
                "tool-result" => {
                    if !assistant.is_empty() {
                        messages.push(Message::Assistant(AssistantMessage::new(std::mem::take(
                            &mut assistant,
                        ))));
                    }
                    let result: ToolResult =
                        serde_json::from_value(part.data.clone()).map_err(|e| e.to_string())?;
                    let call = messages
                        .iter()
                        .rev()
                        .filter_map(|m| {
                            if let Message::Assistant(a) = m {
                                Some(a)
                            } else {
                                None
                            }
                        })
                        .flat_map(|a| a.tool_calls())
                        .find(|c| c.id.to_string() == result.call_id)
                        .ok_or("Результат без вызова инструмента.")?;
                    let content = vec![rig_core::message::ToolResultContent::Json {
                        value: result.output,
                    }];
                    let mut answer = call.result(content);
                    answer.is_error = result.is_error;
                    messages.push(Message::User {
                        content: vec![UserContent::ToolResult(answer)],
                    });
                }
                _ => return Err("Неизвестный тип истории для Rig.".into()),
            }
        }
        if !assistant.is_empty() {
            messages.push(Message::Assistant(AssistantMessage::new(assistant)));
        }
    }
    close_pending(messages)
}

pub fn user_message(turn: &Turn) -> Result<Message, String> {
    use rig_core::message::{DocumentSourceKind, Image, ImageMediaType};
    let mut content = Vec::new();
    for part in &turn.parts {
        match part.kind.as_str() {
            "text" => content.push(UserContent::text(
                part.data["text"].as_str().unwrap_or_default(),
            )),
            "image" => {
                let mime = part.data["mimeType"]
                    .as_str()
                    .ok_or("Image MIME type missing")?;
                let media_type = match mime {
                    "image/png" => ImageMediaType::PNG,
                    "image/jpeg" => ImageMediaType::JPEG,
                    "image/gif" => ImageMediaType::GIF,
                    "image/webp" => ImageMediaType::WEBP,
                    _ => return Err("Unsupported image MIME type".into()),
                };
                let data = part.data["data"]
                    .as_str()
                    .ok_or("Image bytes must be hydrated before provider call")?;
                content.push(UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(data.into()),
                    media_type: Some(media_type),
                    ..Default::default()
                }));
            }
            _ => return Err("Unsupported user content for provider".into()),
        }
    }
    Ok(Message::User { content })
}

fn close_pending(messages: Vec<Message>) -> Result<Vec<Message>, String> {
    // Close each tool batch before the next assistant/user prompt, retaining
    // parallel results and exact original IDs.
    let mut out = Vec::new();
    let mut pending = Vec::<rig_core::message::ToolCall>::new();
    let flush = |out: &mut Vec<Message>, pending: &mut Vec<rig_core::message::ToolCall>| {
        if !pending.is_empty() {
            let content = pending.drain(..).map(|call| {
                let mut result = call.result(vec![rig_core::message::ToolResultContent::text(
                    "Результат не был получен до прерывания. Состояние выполнения неизвестно; не повторяй действие автоматически."
                )]);
                result.is_error = true;
                UserContent::ToolResult(result)
            }).collect();
            push_message(out, Message::User { content });
        }
    };
    for message in messages {
        match &message {
            Message::User { content }
                if content
                    .iter()
                    .all(|c| matches!(c, UserContent::ToolResult(_))) =>
            {
                for part in content {
                    if let UserContent::ToolResult(r) = part {
                        pending.retain(|c| c.id != r.call);
                    }
                }
            }
            _ => flush(&mut out, &mut pending),
        }
        if let Message::Assistant(a) = &message {
            pending.extend(a.tool_calls().cloned());
        }
        push_message(&mut out, message);
    }
    flush(&mut out, &mut pending);
    rig_core::conversation::pairing::validate(&out)
        .map_err(|e| format!("Некорректная цепочка инструментов: {e}"))?;
    Ok(out)
}

fn push_message(out: &mut Vec<Message>, message: Message) {
    if let Message::User { content } = &message {
        if content
            .iter()
            .all(|p| matches!(p, UserContent::ToolResult(_)))
        {
            if let Some(Message::User { content: previous }) = out.last_mut() {
                if previous
                    .iter()
                    .all(|p| matches!(p, UserContent::ToolResult(_)))
                {
                    previous.extend(content.clone());
                    return;
                }
            }
        }
    }
    out.push(message);
}

#[derive(Default)]
struct HistoryState {
    messages: Vec<Message>,
    model_message: Option<AssistantMessage>,
    active: Vec<ContentPart>,
    reasoning_parts: std::collections::HashMap<usize, usize>,
    tool_states: std::collections::HashMap<String, serde_json::Value>,
    usage: Vec<serde_json::Value>,
}

#[derive(Clone)]
pub struct HistoryHook {
    state: Arc<Mutex<HistoryState>>,
    sink: TranscriptSink,
}

impl HistoryHook {
    pub fn new(sink: TranscriptSink) -> Self {
        Self {
            state: Arc::new(Mutex::new(HistoryState::default())),
            sink,
        }
    }
    pub fn finish(&self, messages: &[Message]) -> Result<Transcript, String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        state.messages = messages.to_vec();
        state.active.clear();
        state.reasoning_parts.clear();
        self.publish(&state)?;
        self.snapshot(&state)
    }
    fn snapshot(&self, state: &HistoryState) -> Result<Transcript, String> {
        let mut transcript = capture(&state.messages)?;
        if !state.usage.is_empty() {
            transcript.replay.insert("usage".into(), json!(state.usage));
        }
        for part in &mut transcript.parts {
            if let Some(metadata) = part.data["callId"]
                .as_str()
                .and_then(|call| state.tool_states.get(call))
            {
                if let (Some(target), Some(extra)) =
                    (part.data.as_object_mut(), metadata.as_object())
                {
                    target.extend(
                        extra
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone())),
                    );
                }
            }
        }
        Ok(transcript)
    }
    fn publish(&self, state: &HistoryState) -> Result<(), String> {
        let mut native = state.messages.clone();
        let active_text = state
            .active
            .iter()
            .filter(|p| p.kind == "text")
            .filter_map(|p| p.data["text"].as_str())
            .map(AssistantContent::text)
            .collect::<Vec<_>>();
        if !active_text.is_empty() {
            native.push(Message::Assistant(AssistantMessage::new(active_text)));
        }
        let mut transcript = self.snapshot(state)?;
        transcript.parts.extend(state.active.clone());
        transcript.replay.insert("rig-v1".into(), json!(native));
        (self.sink)(&transcript)
    }
    fn fragment(&self, kind: &str, part: Option<usize>, delta: &str) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        let index = if let Some(part) = part {
            if let Some(index) = state.reasoning_parts.get(&part) {
                *index
            } else {
                let index = state.active.len();
                state.active.push(ContentPart::text(kind, ""));
                state.reasoning_parts.insert(part, index);
                index
            }
        } else if state.active.last().is_some_and(|p| p.kind == kind) {
            state.active.len() - 1
        } else {
            state.active.push(ContentPart::text(kind, ""));
            state.active.len() - 1
        };
        let mut text = state.active[index].data["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        text.push_str(delta);
        state.active[index].data["text"] = json!(text);
        self.publish(&state)
    }
}
impl AgentHook for HistoryHook {
    async fn on_text_delta(&self, _: &HookContext, event: TextDelta<'_>) -> ObservationAction {
        match self.fragment("text", None, event.delta) {
            Ok(()) => ObservationAction::Continue,
            Err(error) => ObservationAction::Stop(error),
        }
    }
    async fn on_reasoning_delta(
        &self,
        _: &HookContext,
        event: ReasoningDelta<'_>,
    ) -> ObservationAction {
        match self.fragment("reasoning", Some(event.part.index()), event.delta) {
            Ok(()) => ObservationAction::Continue,
            Err(error) => ObservationAction::Stop(error),
        }
    }
    async fn on_model_turn_finished(
        &self,
        _: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let save = || -> Result<(), String> {
            let mut state = self.state.lock().map_err(|e| e.to_string())?;
            let native = state.model_message.take();
            state.messages.push(Message::Assistant(
                native
                    .unwrap_or_default()
                    .with_content(event.content.clone()),
            ));
            state.active.clear();
            state.reasoning_parts.clear();
            state
                .usage
                .push(serde_json::to_value(event.usage).map_err(|e| e.to_string())?);
            self.publish(&state)
        };
        match save() {
            Ok(()) => ModelTurnAction::Continue,
            Err(e) => ModelTurnAction::Stop(e),
        }
    }
    async fn on_outcome(&self, _: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        if let Some(response) = event.completion() {
            match self.state.lock() {
                Ok(mut state) => {
                    state.model_message = match response.message() {
                        Some(Message::Assistant(a)) => Some(a),
                        _ => None,
                    }
                }
                Err(e) => return OutcomeAction::stop(e.to_string()),
            }
        }
        let Some(id) = event.call_id else {
            return OutcomeAction::Proceed;
        };
        let save = || -> Result<(), String> {
            let mut state = self.state.lock().map_err(|e| e.to_string())?;
            let call = state
                .messages
                .iter()
                .rev()
                .filter_map(|m| {
                    if let Message::Assistant(a) = m {
                        Some(a)
                    } else {
                        None
                    }
                })
                .flat_map(|a| a.tool_calls())
                .find(|c| &c.id == id)
                .ok_or("Не найден вызов инструмента Rig")?;
            let answer = if let Some(result) = event.tool_result() {
                call.answer(result)
            } else {
                let mut answer = call.result(vec![rig_core::message::ToolResultContent::text(
                    format!("{:?}", event.outcome),
                )]);
                answer.is_error = true;
                answer
            };
            let existing = state.messages.iter_mut().find_map(|message| match message {
                Message::User { content } => content.iter_mut().find(|part| matches!(part, UserContent::ToolResult(result) if result.call == answer.call)),
                _ => None,
            });
            if let Some(existing) = existing {
                *existing = UserContent::ToolResult(answer);
            } else {
                state.messages.push(Message::User {
                    content: vec![UserContent::ToolResult(answer)],
                });
            }
            self.publish(&state)
        };
        match save() {
            Ok(()) => OutcomeAction::Proceed,
            Err(e) => OutcomeAction::stop(e),
        }
    }
}

impl crate::tools::ToolEventSink for HistoryHook {
    fn emit(
        &self,
        context: &crate::tools::ToolContext,
        _: &str,
        _: &serde_json::Value,
        status: &str,
        result: Option<&Result<serde_json::Value, String>>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        let mut metadata = json!({"status":status});
        if let Some(result) = result {
            metadata["meta"] = match result {
                Ok(output) => output.clone(),
                Err(error) => json!({"error":error}),
            };
        }
        if let Some(result) =
            result.filter(|_| matches!(status, "completed" | "failed" | "cancelled"))
        {
            let call = state
                .messages
                .iter()
                .rev()
                .filter_map(|m| match m {
                    Message::Assistant(a) => Some(a),
                    _ => None,
                })
                .flat_map(|a| a.tool_calls())
                .find(|c| c.id.to_string() == context.call_id)
                .cloned();
            if let Some(call) = call {
                // Persist real host results before Rig commits its batch; sibling cancellation may stop it.
                let mut answer = match result {
                    Ok(value) => call.result(vec![rig_core::message::ToolResultContent::Json {
                        value: value.clone(),
                    }]),
                    Err(error) => {
                        call.result(vec![rig_core::message::ToolResultContent::text(error)])
                    }
                };
                answer.is_error = result.is_err();
                state.messages.push(Message::User {
                    content: vec![UserContent::ToolResult(answer)],
                });
            }
        }
        state.tool_states.insert(context.call_id.clone(), metadata);
        self.publish(&state)
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    #[test]
    fn running_tool_progress_is_metadata_until_terminal_result() {
        use crate::tools::{ToolContext, ToolEventSink};
        use rig_core::message::{ProviderCallId, ToolFunction, ToolName};
        let saved = Arc::new(Mutex::new(Vec::new()));
        let observed = saved.clone();
        let hook = HistoryHook::new(Arc::new(move |t| {
            observed.lock().unwrap().push(t.clone());
            Ok(())
        }));
        let call = rig_core::message::ToolCall::new(
            ProviderCallId::new("shell-call").unwrap().into(),
            ToolFunction::new(ToolName::new("shell").unwrap(), json!({})),
        );
        hook.state
            .lock()
            .unwrap()
            .messages
            .push(Message::Assistant(AssistantMessage::new(vec![
                AssistantContent::ToolCall(call),
            ])));
        let context = ToolContext {
            session_id: "session".into(),
            run_id: "run".into(),
            call_id: "shell-call".into(),
            cwd: None,
            policy: crate::tools::PermissionPolicy::ReadOnly,
            cancellation: Default::default(),
            interactions: Arc::new(crate::tools::InteractionBroker::default()),
            sink: None,
        };
        hook.emit(
            &context,
            "shell",
            &json!({}),
            "running",
            Some(&Ok(json!({"stdout":"partial"}))),
        )
        .unwrap();
        let running = saved.lock().unwrap().last().unwrap().clone();
        assert_eq!(running.parts[0].data["status"], "running");
        assert!(!running.parts.iter().any(|p| p.kind == "tool-result"));
        assert!(!running.replay["rig-v1"].to_string().contains("toolresult"));
        hook.emit(
            &context,
            "shell",
            &json!({}),
            "completed",
            Some(&Ok(json!({"stdout":"complete"}))),
        )
        .unwrap();
        let terminal = saved.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            terminal
                .parts
                .iter()
                .filter(|p| p.kind == "tool-result")
                .count(),
            1
        );
        let replay: Vec<Message> =
            serde_json::from_value(terminal.replay["rig-v1"].clone()).unwrap();
        assert_eq!(
            replay
                .iter()
                .filter_map(|m| match m {
                    Message::User { content } => Some(content),
                    _ => None,
                })
                .flatten()
                .filter(|c| matches!(c, UserContent::ToolResult(_)))
                .count(),
            1
        );
    }
    #[test]
    fn streaming_snapshots_preserve_order_and_recover_visible_text() {
        let saved = std::sync::Arc::new(Mutex::new(Vec::new()));
        let observed = saved.clone();
        let hook = HistoryHook::new(std::sync::Arc::new(move |t| {
            observed.lock().unwrap().push(t.clone());
            Ok(())
        }));
        hook.fragment("reasoning", Some(0), "inspect").unwrap();
        hook.fragment("text", None, "Hello").unwrap();
        hook.fragment("text", None, " world").unwrap();
        hook.fragment("reasoning", Some(2), "check").unwrap();
        hook.fragment("text", None, " done").unwrap();
        let snapshots = saved.lock().unwrap();
        assert_eq!(snapshots.len(), 5);
        let last = snapshots.last().unwrap();
        assert_eq!(
            last.parts
                .iter()
                .map(|p| p.kind.as_str())
                .collect::<Vec<_>>(),
            ["reasoning", "text", "reasoning", "text"]
        );
        assert_eq!(last.parts[1].data["text"], "Hello world");
        let replay: Vec<Message> = serde_json::from_value(last.replay["rig-v1"].clone()).unwrap();
        assert!(
            !replay.is_empty(),
            "interrupted streamed text must remain model context"
        );
    }
}
