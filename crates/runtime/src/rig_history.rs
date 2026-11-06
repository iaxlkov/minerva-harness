//! Rig wire/history conversion and durable execution observations.
use crate::{
    sessions::{ContentPart, Turn},
    transcript::{ToolCall, ToolResult, Transcript, TranscriptSink},
};
use rig_agent::agent::hook::{
    AgentHook, HookContext, ModelTurnAction, ModelTurnFinished, OutcomeAction, OutcomeEvent,
};
use rig_core::message::{AssistantContent, AssistantMessage, Message, UserContent};
use serde_json::json;
use std::sync::Mutex;

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
            messages.push(Message::user(turn.text()));
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

pub struct HistoryHook {
    messages: Mutex<Vec<Message>>,
    sink: TranscriptSink,
    model_message: Mutex<Option<AssistantMessage>>,
}
impl HistoryHook {
    pub fn new(sink: TranscriptSink) -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            sink,
            model_message: Mutex::new(None),
        }
    }
}
impl AgentHook for HistoryHook {
    async fn on_model_turn_finished(
        &self,
        _: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let save = || -> Result<(), String> {
            let mut messages = self.messages.lock().map_err(|e| e.to_string())?;
            let native = self.model_message.lock().map_err(|e| e.to_string())?.take();
            messages.push(Message::Assistant(
                native
                    .unwrap_or_default()
                    .with_content(event.content.clone()),
            ));
            (self.sink)(&capture(&messages)?)
        };
        match save() {
            Ok(()) => ModelTurnAction::Continue,
            Err(e) => ModelTurnAction::Stop(e),
        }
    }
    async fn on_outcome(&self, _: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        if let Some(response) = event.completion() {
            match self.model_message.lock() {
                Ok(mut slot) => {
                    *slot = match response.message() {
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
            let mut messages = self.messages.lock().map_err(|e| e.to_string())?;
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
            messages.push(Message::User {
                content: vec![UserContent::ToolResult(answer)],
            });
            (self.sink)(&capture(&messages)?)
        };
        match save() {
            Ok(()) => OutcomeAction::Proceed,
            Err(e) => OutcomeAction::stop(e),
        }
    }
}
