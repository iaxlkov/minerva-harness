//! App Server item lifecycle -> neutral transcript. No database or UI dependency.
use crate::{
    sessions::ContentPart,
    transcript::{ToolCall, ToolResult, Transcript},
};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct CodexHistory {
    transcript: Transcript,
    positions: HashMap<(String, String), usize>,
    completed: std::collections::HashSet<String>,
}
impl CodexHistory {
    fn put(&mut self, id: &str, part: ContentPart) {
        let key = (id.to_owned(), part.kind.clone());
        if let Some(index) = self.positions.get(&key) {
            self.transcript.parts[*index] = part;
        } else {
            self.positions.insert(key, self.transcript.parts.len());
            self.transcript.parts.push(part);
        }
    }
    pub fn item(&mut self, item: &Value, completed: bool) -> Result<(), String> {
        let id = item["id"].as_str().ok_or("У события Codex нет ID")?;
        let kind = item["type"].as_str().ok_or("У события Codex нет типа")?;
        if self.completed.contains(id) {
            return Ok(());
        }
        self.transcript
            .replay
            .entry("codex-items-v1")
            .or_insert_with(|| json!({}))[id] = item.clone();
        match kind {
            "userMessage" => {}
            "agentMessage" | "plan" => {
                if let Some(text) = item["text"].as_str() {
                    self.put(id, ContentPart::text("text", text));
                }
            }
            "reasoning" => {
                let text = item["summary"]
                    .as_array()
                    .or_else(|| item["content"].as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                self.put(id, ContentPart::text("reasoning", text));
            }
            "commandExecution"
            | "fileChange"
            | "mcpToolCall"
            | "dynamicToolCall"
            | "collabAgentToolCall"
            | "webSearch"
            | "imageView"
            | "imageGeneration"
            | "sleep" => {
                let (name, args) = match kind {
                    "commandExecution" => (
                        "shell".to_string(),
                        json!({"command":item["command"],"cwd":item["cwd"]}),
                    ),
                    "mcpToolCall" => (
                        format!(
                            "{}.{}",
                            item["server"].as_str().unwrap_or("mcp"),
                            item["tool"].as_str().unwrap_or("tool")
                        ),
                        item["arguments"].clone(),
                    ),
                    "dynamicToolCall" => (
                        item["tool"].as_str().unwrap_or(kind).to_owned(),
                        item["arguments"].clone(),
                    ),
                    "webSearch" => (
                        kind.to_string(),
                        json!({"query":item["query"],"action":item["action"]}),
                    ),
                    _ => (kind.to_string(), item.clone()),
                };
                self.put(
                    id,
                    ContentPart::call(ToolCall {
                        call_id: id.into(),
                        name,
                        arguments: args,
                    }),
                );
                if completed {
                    let failed = matches!(
                        item["status"].as_str(),
                        Some("failed" | "declined" | "cancelled")
                    ) || item["success"] == false
                        || item["exitCode"].as_i64().is_some_and(|v| v != 0)
                        || !item["error"].is_null();
                    self.put(
                        id,
                        ContentPart::result(ToolResult {
                            call_id: id.into(),
                            output: item.clone(),
                            is_error: failed,
                        }),
                    );
                }
            }
            _ => self.put(
                id,
                ContentPart {
                    kind: "extension".into(),
                    data: json!({"provider":"codex","content":item}),
                },
            ),
        }
        if completed {
            self.completed.insert(id.into());
        }
        self.transcript.validate()
    }
    pub fn snapshot(&self) -> Transcript {
        self.transcript.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_calls_failures_and_duplicate_completion_keep_identity_and_order() {
        let mut history = CodexHistory::default();
        let a = json!({"id":"a","type":"commandExecution","command":"echo Привет","cwd":"C:/dev","status":"inProgress"});
        let b = json!({"id":"b","type":"mcpToolCall","server":"s","tool":"lookup","arguments":{"id":42},"status":"inProgress"});
        history.item(&a, false).unwrap();
        history.item(&b, false).unwrap();
        let mut b = b;
        b["status"] = json!("failed");
        b["error"] = json!({"message":"offline"});
        history.item(&b, true).unwrap();
        history.item(&b, true).unwrap();
        let mut a = a;
        a["status"] = json!("completed");
        a["aggregatedOutput"] = json!("Привет\n");
        a["exitCode"] = json!(0);
        history.item(&a, true).unwrap();
        let parts = history.snapshot().parts;
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[2].data["callId"], "b");
        assert_eq!(parts[2].data["isError"], true);
        assert_eq!(parts[3].data["output"]["aggregatedOutput"], "Привет\n");
    }
}
