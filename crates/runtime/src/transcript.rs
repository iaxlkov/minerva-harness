//! Provider-neutral, ordered execution transcript. Provider replay stays opaque.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct ContentPart {
    pub kind: String,
    #[ts(type = "JsonValue")]
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    #[ts(type = "JsonValue")]
    pub arguments: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(ts_rs::TS)]
pub struct ToolResult {
    pub call_id: String,
    #[ts(type = "JsonValue")]
    pub output: Value,
    pub is_error: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct Transcript {
    pub parts: Vec<ContentPart>,
    #[serde(default)]
    #[ts(type = "Record<string, JsonValue>")]
    pub replay: Map<String, Value>,
}
pub type TranscriptSink = Arc<dyn Fn(&Transcript) -> Result<(), String> + Send + Sync>;

impl ContentPart {
    pub fn text(kind: &str, text: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            data: serde_json::json!({"text":text.into()}),
        }
    }
    pub fn call(call: ToolCall) -> Self {
        Self {
            kind: "tool-call".into(),
            data: serde_json::to_value(call).expect("tool call serializes"),
        }
    }
    pub fn result(result: ToolResult) -> Self {
        Self {
            kind: "tool-result".into(),
            data: serde_json::to_value(result).expect("tool result serializes"),
        }
    }
}

impl Transcript {
    pub fn validate_complete(&self) -> Result<(), String> {
        self.validate()?;
        if self.parts.iter().filter(|p| p.kind == "tool-call").count()
            != self
                .parts
                .iter()
                .filter(|p| p.kind == "tool-result")
                .count()
        {
            return Err("Ход завершился без результата одного из инструментов.".into());
        }
        Ok(())
    }
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter(|p| p.kind == "text")
            .filter_map(|p| p.data["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    // Pending calls are valid in interrupted snapshots; dangling or duplicate
    // results are never valid. IDs are scoped to one assistant response.
    pub fn validate(&self) -> Result<(), String> {
        let mut calls = HashSet::new();
        let mut results = HashSet::new();
        for part in &self.parts {
            match part.kind.as_str() {
                "text" | "reasoning" if part.data["text"].is_string() => {}
                "extension" if part.data.is_object() => {}
                "tool-call" => {
                    let call: ToolCall =
                        serde_json::from_value(part.data.clone()).map_err(|e| e.to_string())?;
                    if call.call_id.is_empty()
                        || call.name.is_empty()
                        || !calls.insert(call.call_id)
                    {
                        return Err("Некорректный или повторный вызов инструмента.".into());
                    }
                }
                "tool-result" => {
                    let result: ToolResult =
                        serde_json::from_value(part.data.clone()).map_err(|e| e.to_string())?;
                    if !calls.contains(&result.call_id) || !results.insert(result.call_id) {
                        return Err(
                            "Результат без вызова или повторный результат инструмента.".into()
                        );
                    }
                }
                _ => return Err("Некорректное содержимое истории.".into()),
            }
        }
        Ok(())
    }
}
