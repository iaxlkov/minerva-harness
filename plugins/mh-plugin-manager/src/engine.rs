//! Supervised execution for installed model engines. Wire state remains opaque.
use crate::{
    plugins::PluginManager,
    sessions::Turn,
    transcript::{Transcript, TranscriptSink},
};
use minerva_plugin_sdk::engine::EngineSnapshot;
use serde_json::{json, Value};
use tokio::sync::watch;

pub fn model_ref(model: &str) -> Option<(&str, &str)> {
    let (plugin, model) = model.strip_prefix("plugin:")?.split_once('/')?;
    (!plugin.is_empty() && !model.is_empty()).then_some((plugin, model))
}

pub fn resume_state(saved: Option<Value>, model: &str, parent: Option<&str>) -> Option<Value> {
    let saved = saved?;
    if saved["model"] == model && saved["anchorMessageId"].as_str() == Some(parent?) {
        saved.get("value").cloned()
    } else {
        None
    }
}

pub async fn run(
    manager: &PluginManager,
    run_id: &str,
    model: &str,
    turns: Vec<Turn>,
    state: Option<Value>,
    sink: TranscriptSink,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(Transcript, Option<Value>), String> {
    run_with_context(manager, run_id, model, turns, state, sink, cancel, None).await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_with_context(
    manager: &PluginManager,
    run_id: &str,
    model: &str,
    turns: Vec<Turn>,
    state: Option<Value>,
    sink: TranscriptSink,
    cancel: &mut watch::Receiver<bool>,
    context: Option<crate::harness::AgentRunContext>,
) -> Result<(Transcript, Option<Value>), String> {
    let (plugin, model) = model_ref(model).ok_or("Invalid plugin model")?;
    let reference = json!({"runId":run_id});
    let mut lease = crate::plugins::ProcessRunLease::new(
        manager.engine_host(plugin)?,
        plugin,
        "engine",
        reference.clone(),
    );
    let wire_context = match &context {
        Some(ctx) => {
            let settings = ctx.context.workspace_get(&ctx.session_id)?;
            Some(minerva_plugin_sdk::engine::EngineStartContext {
                system_prompt: ctx
                    .context
                    .prepare(&ctx.session_id, &ctx.messages)?
                    .system_prompt,
                cwd: settings.cwd,
                permission_policy: settings.permission_policy,
                tools: ctx.tools.definitions(),
            })
        }
        None => None,
    };
    let tool_sink = std::sync::Arc::new(EngineToolSink {
        transcript: std::sync::Mutex::new(Transcript::default()),
        host_parts: std::sync::Mutex::new(Vec::new()),
        sink: sink.clone(),
    });
    let result = async {
        if *cancel.borrow() {
            return Err("Run cancelled".into());
        }
        manager
            .engine_call(
                plugin,
                "engine.start",
                json!({"runId":run_id,"model":model,"turns":turns,"state":state,"context":wire_context}),
            )
            .await?;
        let mut deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(240);
        let mut previous = None;
        let mut handled=std::collections::HashMap::new();
        let mut calls=std::collections::HashMap::new();
        loop {
            let cancelled = *cancel.borrow();
            let expired = tokio::time::Instant::now() >= deadline;
            if cancelled || expired {
                manager
                    .engine_call(plugin, "engine.cancel", reference.clone())
                    .await?;
            }
            // Always finish a short RPC before checking cancellation, preserving framing.
            let mut snapshot: EngineSnapshot = serde_json::from_value(
                manager
                    .engine_call(plugin, "engine.poll", reference.clone())
                    .await?,
            )
            .map_err(|e| format!("Invalid engine snapshot: {e}"))?;
            snapshot.transcript.validate()?;
            if !matches!(snapshot.status.as_str(),"running"|"completed"|"failed"|"cancelled") {return Err("Invalid engine run status".into());}
            merge_host_parts(&mut snapshot.transcript,&tool_sink.host_parts.lock().map_err(|e|e.to_string())?);
            *tool_sink.transcript.lock().map_err(|e|e.to_string())?=snapshot.transcript.clone();
            let encoded = serde_json::to_value(&snapshot.transcript).map_err(|e| e.to_string())?;
            if !snapshot.transcript.parts.is_empty() && previous.as_ref() != Some(&encoded) {
                sink(&snapshot.transcript)?;
                previous = Some(encoded);
            }
            if snapshot.requests.len()>64 || handled.len()>1024 {return Err("Too many engine interaction requests".into());}
            for request in snapshot.requests.iter().filter(|_|snapshot.status=="running") {
                if !admit_request(request,&mut handled,&mut calls)? {continue;}
                let ctx=context.as_ref().ok_or("Engine requested unavailable host interaction")?;
                let mut tool_context=ctx.tool_context.clone();
                tool_context.call_id=request.call_id.clone();
                tool_context.sink=Some(tool_sink.clone());
                let started=tokio::time::Instant::now();
                let image=request.kind=="tool" && request.tool.as_deref()==Some("generate_image");
                let interaction_deadline=if image { deadline + std::time::Duration::from_secs(960) } else { deadline };
                let pending=engine_interaction(ctx,request,tool_context);
                tokio::pin!(pending);
                let answer=tokio::select! {
                    biased;
                    _=async {if !*cancel.borrow(){let _=cancel.changed().await;}}=>{
                        ctx.tool_context.cancellation.cancel();
                        ctx.tool_context.interactions.cancel_run(&ctx.session_id,&ctx.run_id);
                        return Err("Run cancelled".into());
                    },
                    _=tokio::time::sleep_until(interaction_deadline)=>{
                        ctx.tool_context.cancellation.cancel();
                        return Err("Engine run timed out".into());
                    },
                    answer=&mut pending=>answer?,
                };
                if image { deadline += started.elapsed(); }
                manager.engine_call(plugin,"engine.answer",json!({"runId":run_id,"id":request.id,"decision":answer["decision"],"answer":answer["answer"],"output":answer["output"],"isError":answer["isError"]})).await?;
            }
            snapshot.transcript=tool_sink.transcript.lock().map_err(|e|e.to_string())?.clone();
            if *cancel.borrow() {
                return Err("Run cancelled".into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("Engine run timed out".into());
            }
            match snapshot.status.as_str() {
                "completed" => {
                    snapshot.transcript.validate_complete()?;
                    return Ok((snapshot.transcript, snapshot.state));
                }
                "failed" | "cancelled" => {
                    return Err(snapshot
                        .error
                        .unwrap_or_else(|| "Engine run interrupted".into()))
                }
                "running" => {}
                _ => return Err("Invalid engine run status".into()),
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {},
                _ = cancel.changed() => {},
            }
        }
    }
    .await;
    lease.finish(result.is_err()).await;
    result
}

async fn engine_interaction(
    context: &crate::harness::AgentRunContext,
    request: &minerva_plugin_sdk::engine::EngineInteraction,
    tool_context: crate::tools::ToolContext,
) -> Result<Value, String> {
    match request.kind.as_str() {
        "tool" => {
            let name = request
                .tool
                .as_deref()
                .ok_or("Engine tool request missing name")?;
            let result = context
                .tools
                .execute(name, request.arguments.clone(), tool_context)
                .await;
            Ok(match result {
                Ok(output) => json!({"decision":"result","output":output,"isError":false}),
                Err(error) => json!({"decision":"result","output":{"error":error},"isError":true}),
            })
        }
        "approval" => {
            use crate::tools::PermissionPolicy;
            let decision = match tool_context.policy {
                PermissionPolicy::ReadOnly => "deny".into(),
                PermissionPolicy::Allow => "allow".into(),
                PermissionPolicy::Ask => {
                    tool_context
                        .interactions
                        .request(
                            &tool_context,
                            "approval",
                            request.tool.as_deref().unwrap_or("External operation"),
                            request.prompt.as_deref().unwrap_or("Allow this operation?"),
                            vec![],
                        )
                        .await?
                }
            };
            Ok(json!({"decision":decision}))
        }
        "question" => {
            let answer = tool_context
                .interactions
                .request(
                    &tool_context,
                    "question",
                    request.tool.as_deref().unwrap_or("Question"),
                    request
                        .prompt
                        .as_deref()
                        .ok_or("Engine question missing prompt")?,
                    request.options.clone(),
                )
                .await?;
            Ok(json!({"decision":"answer","answer":answer}))
        }
        _ => Err("Unsupported engine interaction".into()),
    }
}

struct EngineToolSink {
    transcript: std::sync::Mutex<Transcript>,
    host_parts: std::sync::Mutex<Vec<HostToolPart>>,
    sink: TranscriptSink,
}
impl crate::tools::ToolEventSink for EngineToolSink {
    fn emit(
        &self,
        ctx: &crate::tools::ToolContext,
        name: &str,
        args: &Value,
        status: &str,
        result: Option<&Result<Value, String>>,
    ) -> Result<(), String> {
        let mut transcript = self.transcript.lock().map_err(|e| e.to_string())?;
        let index = transcript
            .parts
            .iter()
            .position(|p| p.kind == "tool-call" && p.data["callId"] == ctx.call_id);
        let index = match index {
            Some(index) => index,
            None => {
                transcript.parts.push(crate::transcript::ContentPart::call(
                    crate::transcript::ToolCall {
                        call_id: ctx.call_id.clone(),
                        name: name.into(),
                        arguments: args.clone(),
                    },
                ));
                transcript.parts.len() - 1
            }
        };
        transcript.parts[index].data["name"] = json!(name);
        transcript.parts[index].data["arguments"] = args.clone();
        transcript.parts[index].data["status"] = json!(status);
        if let Some(result) = result {
            transcript.parts[index].data["meta"] = match result {
                Ok(value) => value.clone(),
                Err(error) if name == "generate_image" && transcript.parts[index].data["meta"]["jobId"].is_string() => {
                    let mut previous=transcript.parts[index].data["meta"].clone();
                    previous["error"]=json!(error);
                    previous
                }
                Err(error) => json!({"error":error}),
            };
            if matches!(status, "completed" | "failed" | "cancelled")
                && !transcript
                    .parts
                    .iter()
                    .any(|p| p.kind == "tool-result" && p.data["callId"] == ctx.call_id)
            {
                transcript
                    .parts
                    .push(crate::transcript::ContentPart::result(
                        crate::transcript::ToolResult {
                            call_id: ctx.call_id.clone(),
                            output: match result {
                                Ok(value) => value.clone(),
                                Err(error) => json!({"error":error}),
                            },
                            is_error: result.is_err(),
                        },
                    ));
            }
        }
        let mut host_parts = self.host_parts.lock().map_err(|e| e.to_string())?;
        for (index, part) in transcript.parts.iter().enumerate().filter(|(_, p)| {
            matches!(p.kind.as_str(), "tool-call" | "tool-result")
                && p.data["callId"] == ctx.call_id
        }) {
            if let Some(existing) = host_parts
                .iter_mut()
                .find(|p| p.part.kind == part.kind && p.part.data["callId"] == ctx.call_id)
            {
                existing.part = part.clone();
            } else {
                host_parts.push(HostToolPart {
                    index,
                    part: part.clone(),
                    preceding: index
                        .checked_sub(1)
                        .and_then(|i| transcript.parts.get(i))
                        .cloned(),
                });
            }
        }
        (self.sink)(&transcript)
    }
}

struct HostToolPart {
    index: usize,
    part: crate::transcript::ContentPart,
    preceding: Option<crate::transcript::ContentPart>,
}
fn merge_host_parts(transcript: &mut Transcript, host_parts: &[HostToolPart]) {
    for saved in host_parts {
        let part = &saved.part;
        if let Some(existing) = transcript
            .parts
            .iter_mut()
            .find(|p| p.kind == part.kind && p.data["callId"] == part.data["callId"])
        {
            *existing = part.clone();
        } else {
            let index = saved.index.min(transcript.parts.len());
            // A provider may grow the last text part across a host-tool boundary.
            if let Some(anchor) = saved
                .preceding
                .as_ref()
                .filter(|p| matches!(p.kind.as_str(), "text" | "reasoning"))
            {
                if let Some(previous) = index
                    .checked_sub(1)
                    .and_then(|i| transcript.parts.get_mut(i))
                {
                    if previous.kind == anchor.kind {
                        if let (Some(prefix), Some(text)) =
                            (anchor.data["text"].as_str(), previous.data["text"].as_str())
                        {
                            if let Some(suffix) =
                                text.strip_prefix(prefix).filter(|s| !s.is_empty())
                            {
                                let suffix =
                                    crate::transcript::ContentPart::text(&previous.kind, suffix);
                                *previous = anchor.clone();
                                transcript.parts.insert(index, suffix);
                            }
                        }
                    }
                }
            }
            transcript.parts.insert(index, part.clone());
        }
    }
}

fn admit_request(
    request: &minerva_plugin_sdk::engine::EngineInteraction,
    handled: &mut std::collections::HashMap<String, Value>,
    calls: &mut std::collections::HashMap<(String, String), String>,
) -> Result<bool, String> {
    let valid_id =
        |id: &str| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control);
    if !valid_id(&request.id)
        || !valid_id(&request.call_id)
        || request.prompt.as_ref().is_some_and(|p| p.len() > 65536)
        || request
            .tool
            .as_ref()
            .is_some_and(|p| p.is_empty() || p.len() > 256 || p.chars().any(char::is_control))
        || request.options.len() > 64
        || request.options.iter().any(|o| o.len() > 4096)
    {
        return Err("Invalid engine interaction".into());
    }
    let encoded = serde_json::to_value(request).map_err(|e| e.to_string())?;
    if let Some(previous) = handled.get(&request.id) {
        return if previous == &encoded {
            Ok(false)
        } else {
            Err("Engine changed an existing interaction".into())
        };
    }
    if calls
        .insert(
            (request.kind.clone(), request.call_id.clone()),
            request.id.clone(),
        )
        .is_some()
    {
        return Err("Duplicate engine call identity".into());
    }
    handled.insert(request.id.clone(), encoded);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_image_retains_its_generator_job() {
        use crate::tools::{ToolContext,ToolEventSink};
        let sink=EngineToolSink { transcript:std::sync::Mutex::new(Transcript::default()), host_parts:std::sync::Mutex::new(Vec::new()), sink:std::sync::Arc::new(|_|Ok(())) };
        let ctx=ToolContext {session_id:"s".into(),run_id:"r".into(),call_id:"c".into(),cwd:None,policy:crate::tools::PermissionPolicy::Allow,cancellation:Default::default(),interactions:std::sync::Arc::new(crate::tools::InteractionBroker::default()),sink:None};
        sink.emit(&ctx,"generate_image",&json!({}),"running",Some(&Ok(json!({"generatorId":"codex","jobId":"job"})))).unwrap();
        sink.emit(&ctx,"generate_image",&json!({}),"cancelled",Some(&Err("cancelled".into()))).unwrap();
        let transcript=sink.transcript.lock().unwrap();
        assert_eq!(transcript.parts[0].data["meta"]["jobId"],"job");
        assert_eq!(transcript.parts[0].data["meta"]["error"],"cancelled");
    }
    #[test]
    fn repeated_requests_cannot_change_arguments_or_repeat_effects() {
        let mut handled = std::collections::HashMap::new();
        let mut calls = std::collections::HashMap::new();
        let request:minerva_plugin_sdk::engine::EngineInteraction=serde_json::from_value(json!({"id":"r1","callId":"c1","kind":"tool","tool":"write","arguments":{"text":"one"}})).unwrap();
        assert!(admit_request(&request, &mut handled, &mut calls).unwrap());
        assert!(!admit_request(&request, &mut handled, &mut calls).unwrap());
        let mut changed = request.clone();
        changed.arguments = json!({"text":"two"});
        assert!(admit_request(&changed, &mut handled, &mut calls).is_err());
        let mut duplicate = request.clone();
        duplicate.id = "r2".into();
        assert!(admit_request(&duplicate, &mut handled, &mut calls).is_err());
        let mut control = request;
        control.call_id = "bad\n".into();
        assert!(admit_request(&control, &mut handled, &mut calls).is_err());
    }
    #[test]
    fn later_engine_snapshots_keep_canonical_host_tool_results() {
        let call = crate::transcript::ContentPart::call(crate::transcript::ToolCall {
            call_id: "c".into(),
            name: "read_file".into(),
            arguments: json!({}),
        });
        let result = crate::transcript::ContentPart::result(crate::transcript::ToolResult {
            call_id: "c".into(),
            output: json!({"text":"original"}),
            is_error: false,
        });
        let before = crate::transcript::ContentPart::text("text", "before");
        let host_parts = [
            HostToolPart {
                index: 1,
                part: call.clone(),
                preceding: Some(before.clone()),
            },
            HostToolPart {
                index: 2,
                part: result,
                preceding: Some(call),
            },
        ];
        for parts in [
            vec![
                before.clone(),
                crate::transcript::ContentPart::text("text", "continued"),
            ],
            vec![crate::transcript::ContentPart::text(
                "text",
                "beforecontinued",
            )],
        ] {
            let mut transcript = Transcript {
                parts,
                ..Default::default()
            };
            merge_host_parts(&mut transcript, &host_parts);
            merge_host_parts(&mut transcript, &host_parts);
            assert_eq!(
                transcript
                    .parts
                    .iter()
                    .map(|p| p.kind.as_str())
                    .collect::<Vec<_>>(),
                vec!["text", "tool-call", "tool-result", "text"]
            );
            assert_eq!(transcript.parts[0].data["text"], "before");
            assert_eq!(transcript.parts[2].data["output"]["text"], "original");
            assert_eq!(transcript.parts[3].data["text"], "continued");
        }
    }
    #[test]
    fn state_is_reusable_only_for_the_same_model_and_saved_parent() {
        let state =
            json!({"model":"plugin:fixture/model","anchorMessageId":"a","value":{"opaque":42}});
        assert_eq!(
            resume_state(Some(state.clone()), "plugin:fixture/model", Some("a")),
            Some(json!({"opaque":42}))
        );
        assert_eq!(
            resume_state(Some(state.clone()), "plugin:other/model", Some("a")),
            None
        );
        assert_eq!(
            resume_state(Some(state.clone()), "plugin:fixture/model", Some("b")),
            None
        );
        assert_eq!(
            resume_state(Some(state), "plugin:fixture/model", None),
            None
        );
        assert_eq!(
            model_ref("plugin:fixture/vendor/model"),
            Some(("fixture", "vendor/model"))
        );
        assert_eq!(model_ref("plugin:fixture/"), None);
    }
}
