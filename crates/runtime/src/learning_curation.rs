//! Deterministic checks for configured curation cases.
use mh_skill_curation::ValidationCase;

/// Checks host-provided assertions against an actual bounded model result.
pub(crate) fn check_case(case: &ValidationCase, output: &str) -> Result<(), String> {
    if output.len() > 65_536 {
        return Err("OUTPUT_TOO_LARGE".into());
    }
    if case
        .contains
        .iter()
        .any(|expected| !output.contains(expected))
    {
        return Err("required text missing".into());
    }
    if case
        .not_contains
        .iter()
        .any(|forbidden| output.contains(forbidden))
    {
        return Err("forbidden text present".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evaluates_actual_output() {
        let case = ValidationCase {
            input: "task".into(),
            contains: vec!["correct".into()],
            not_contains: vec!["wrong".into()],
        };
        assert!(check_case(&case, "correct answer").is_ok());
        assert!(check_case(&case, "wrong answer").is_err());
        assert!(check_case(&case, "correct but wrong").is_err());
    }
}

use crate::learning::Learning;
use mh_skill_curation::{GateConfig, Proposal, Verdict};
use minerva_plugin_sdk::{
    learning::{AccessContext, Scope},
    skills::{Invocation, LoadSkill},
};
use serde_json::{json, Value};
use std::sync::Arc;

fn provider(model: &str) -> Option<&str> {
    let rest = model.strip_prefix("api:")?;
    let (provider, name) = rest.split_once('/')?;
    if provider.is_empty() || name.is_empty() {
        None
    } else {
        Some(provider)
    }
}
impl Learning {
    pub(crate) async fn start_curation_evaluation(
        self: &Arc<Self>,
        access: &AccessContext,
        id: &str,
    ) -> Result<Proposal, String> {
        let (proposal, gate) = self
            .curation
            .begin_evaluation(access, id)
            .map_err(|e| e.to_string())?;
        let valid = provider(&gate.executor_model).is_some()
            && provider(&gate.reviewers[0]).is_some()
            && provider(&gate.reviewers[1]).is_some()
            && provider(&gate.reviewers[0]) != provider(&gate.reviewers[1])
            && proposal.candidate.len() <= 12_000;
        if !valid {
            let _ = self.curation.fail_evaluation(access, id);
            return Err(
                "CONFIG_REQUIRED: native executor and two independent reviewer providers required"
                    .into(),
            );
        }
        let host = self.clone();
        let actor = access.clone();
        let job_id = id.to_string();
        let scope = proposal.scope.clone();
        let task_proposal = proposal.clone();
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        let (admit, ready) = tokio::sync::oneshot::channel::<()>();
        let worker = tokio::spawn(async move {
            if ready.await.is_err() {
                return;
            }
            if host
                .evaluate_curation(&actor, &task_proposal, &gate, receiver)
                .await
                .is_err()
            {
                let _ = host.curation.fail_evaluation(&actor, &job_id);
            }
        });
        let supervisor_id = format!("curation:{id}");
        if let Err(error) = self
            .execution
            .register_custom(access, &scope, &supervisor_id, cancel, worker)
            .await
        {
            let _ = self.curation.fail_evaluation(access, id);
            return Err(error);
        }
        let _ = admit.send(());
        Ok(proposal)
    }
    async fn curation_trial(
        &self,
        access: &AccessContext,
        scope: &Scope,
        model: &str,
        task: &str,
        cancel: &tokio::sync::watch::Receiver<bool>,
    ) -> Result<(String, String), String> {
        if *cancel.borrow() || !self.enabled("skill-curation") {
            return Err("curation disabled".into());
        }
        let job = self
            .execution_call(
                "experiments",
                "create",
                access,
                scope,
                json!({"task":task,"model":model,"budget":1,"k":1}),
            )
            .await?;
        let id = job["id"].as_str().ok_or("missing job id")?.to_owned();
        self.execution_call("experiments", "start", access, scope, json!({"id":id}))
            .await?;
        loop {
            if *cancel.borrow() || !self.enabled("skill-curation") {
                let _ = self
                    .execution_call("experiments", "cancel", access, scope, json!({"id":id}))
                    .await;
                return Err("curation disabled".into());
            }
            let view = self
                .execution_call("experiments", "get", access, scope, json!({"id":id}))
                .await?;
            match view["state"].as_str() {
                Some("completed") => break,
                Some("failed" | "cancelled" | "interrupted" | "budgetExceeded") => {
                    return Err("evaluation trial failed".into())
                }
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let result = self
            .execution_call("experiments", "compare", access, scope, json!({"id":id}))
            .await?;
        let digest = result["artifact"]
            .as_str()
            .ok_or("missing trial artifact")?;
        let scope_key = serde_json::to_string(scope).map_err(|e| e.to_string())?;
        let data = self
            .experiments
            .artifact(&access.actor_id, &scope_key, &id, digest)
            .map_err(|e| format!("{e:?}"))?;
        let snapshot: Value = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        let text = snapshot["parts"]
            .as_array()
            .ok_or("missing output")?
            .iter()
            .filter(|p| p["kind"] == "text")
            .filter_map(|p| p["data"]["text"].as_str())
            .collect::<String>();
        if text.is_empty() || text.len() > 65_536 {
            return Err("invalid output".into());
        }
        Ok((id, text))
    }
    async fn evaluate_curation(
        &self,
        access: &AccessContext,
        proposal: &Proposal,
        gate: &GateConfig,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), String> {
        self.verify_sources(access, &proposal.sources)?;
        mh_skills::validate_candidate(&proposal.skill, &proposal.candidate)
            .map_err(|e| e.to_string())?;
        let baseline = self
            .skills
            .load(
                access,
                &proposal.scope,
                &LoadSkill {
                    name: proposal.skill.clone(),
                    revision: Some(proposal.base_revision.clone()),
                },
                Invocation::Selected,
            )
            .map_err(|e| e.to_string())?;
        if baseline.body.len() > 12_000 {
            return Err("baseline too large".into());
        }
        let mut evidence = Vec::new();
        let mut pass = true;
        for case in std::iter::once(&gate.trigger).chain(gate.holdouts.iter()) {
            let before = format!(
                "Use the following skill instructions for this task:\n{}\n\nTask:\n{}",
                baseline.body, case.input
            );
            let after = format!(
                "Use the following skill instructions for this task:\n{}\n\nTask:\n{}",
                proposal.candidate, case.input
            );
            let (before_id, before_output) = self
                .curation_trial(
                    access,
                    &proposal.scope,
                    &gate.executor_model,
                    &before,
                    &cancel,
                )
                .await?;
            let (after_id, after_output) = self
                .curation_trial(
                    access,
                    &proposal.scope,
                    &gate.executor_model,
                    &after,
                    &cancel,
                )
                .await?;
            let old = check_case(case, &before_output).is_ok();
            let new = check_case(case, &after_output).is_ok();
            pass &= new || !old;
            if std::ptr::eq(case, &gate.trigger) {
                pass &= new;
            }
            evidence.push(format!(
                "{before_id} -> {after_id}: baseline={old}, candidate={new}"
            ));
        }
        let mut verdicts = vec![
            Verdict {
                id: "schema".into(),
                passed: true,
                evidence: "candidate schema and source references validated".into(),
            },
            Verdict {
                id: "regression".into(),
                passed: pass,
                evidence: evidence.join("; "),
            },
        ];
        for (index, model) in gate.reviewers.iter().enumerate() {
            let task=format!("Review this skill change. Return exactly JSON with boolean approved and nonempty reason.\nBefore:\n{}\nAfter:\n{}\nReason for change: {}",baseline.body,proposal.candidate,proposal.rationale);
            let (job, output) = self
                .curation_trial(access, &proposal.scope, model, &task, &cancel)
                .await?;
            let review: Value =
                serde_json::from_str(&output).map_err(|_| "invalid reviewer response")?;
            let approved = review["approved"]
                .as_bool()
                .ok_or("invalid reviewer response")?;
            let reason = review["reason"]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 2048)
                .ok_or("invalid reviewer reason")?;
            verdicts.push(Verdict {
                id: if index == 0 {
                    "reviewer.primary"
                } else {
                    "reviewer.secondary"
                }
                .into(),
                passed: approved,
                evidence: format!("{job}: {reason}"),
            });
        }
        self.curation
            .finish_evaluation(access, &proposal.id, verdicts)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use mh_plugin_manager::optional::OptionalPlugins;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    #[tokio::test]
    async fn actual_mock_models_gate_publish_and_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = crate::Runtime::start(crate::RuntimeConfig {
            tenant_id: "t".into(),
            workspace_id: "w".into(),
            profile: "local".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
        })
        .await
        .unwrap();
        runtime.sessions.create("parent").unwrap();
        use crate::sessions::{ContentPart, SessionMessage};
        runtime
            .sessions
            .save(
                "parent",
                SessionMessage {
                    id: "msg".into(),
                    parent_id: None,
                    role: "user".into(),
                    created_at: "2026-01-01".into(),
                    parts: vec![ContentPart::text("text", "source correction")],
                    extensions: Default::default(),
                },
                None,
            )
            .unwrap();
        let source_event = runtime
            .sessions
            .events_after("parent", 0, 100)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "message.upsert")
            .unwrap();
        let access = runtime.learning.access("parent").unwrap();
        let scope = Scope::Session {
            session_id: "parent".into(),
        };
        for plugin in ["skills", "session-query", "experiments", "skill-curation"] {
            runtime.learning.set_enabled(plugin, true).await.unwrap();
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..6 {
                let (mut sock, _) = listener.accept().unwrap();
                sock.set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let n = sock.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(i) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..i]);
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= i + 4 + len {
                            break;
                        }
                    }
                }
                let req = String::from_utf8_lossy(&request);
                let output = if req.contains("Review this skill change") {
                    r#"{"approved":true,"reason":"verified"}"#
                } else {
                    "correct"
                };
                let chunk = json!({"id":"x","object":"chat.completion.chunk","created":1,"model":"model","choices":[{"index":0,"delta":{"role":"assistant","content":output},"finish_reason":null}]});
                let end = json!({"id":"x","object":"chat.completion.chunk","created":1,"model":"model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                let body = format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n");
                write!(sock,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let providers: Vec<String> = (0..3)
            .map(|_| crate::learning_execution::test_credentials::id())
            .collect();
        let _cleanup = crate::learning_execution::test_credentials::Cleanup::new(
            runtime.plugins.providers().unwrap(),
            providers.clone(),
        );
        for provider in &providers {
            runtime
                .plugins
                .providers()
                .unwrap()
                .save(crate::providers::ProviderInput {
                    id: provider.clone(),
                    name: provider.clone(),
                    base_url: format!("http://{addr}/v1"),
                    models: vec!["model".into()],
                    api_key: Some("test".into()),
                    is_new: true,
                    protocol: "openai-chat".into(),
                })
                .await
                .unwrap();
        }
        let baseline = runtime
            .learning
            .skills
            .publish(
                &access,
                &scope,
                "example",
                "---\nname: example\ndescription: old\n---\nold",
                None,
            )
            .unwrap();
        let case = mh_skill_curation::ValidationCase {
            input: "answer".into(),
            contains: vec!["correct".into()],
            not_contains: vec!["wrong".into()],
        };
        let gate = GateConfig {
            executor_model: format!("api:{}/model", providers[0]),
            reviewers: [
                format!("api:{}/model", providers[1]),
                format!("api:{}/model", providers[2]),
            ],
            trigger: case.clone(),
            holdouts: vec![case],
            max_calls: 6,
        };
        runtime
            .learning
            .curation
            .configure_gate(&access, &scope, &gate)
            .unwrap();
        let p = Proposal {
            id: "p".into(),
            actor: String::new(),
            scope: scope.clone(),
            sources: vec![minerva_plugin_sdk::learning::SourceRef {
                session_id: "parent".into(),
                message_id: Some("msg".into()),
                event_seq: Some(source_event.seq),
                run_id: None,
                content_digest: format!("{:x}", Sha256::digest(b"source correction")),
            }],
            skill: "example".into(),
            base_revision: baseline.revision.clone(),
            candidate: "---\nname: example\ndescription: new\n---\nnew".into(),
            diff: "new".into(),
            rationale: "correction".into(),
            applicability: "example".into(),
            negative_example: "wrong".into(),
            validators: vec![],
            reviewers: vec![],
            state: mh_skill_curation::State::Draft,
            evidence: vec![],
            published_revision: None,
        };
        runtime.learning.curation.propose(&access, p).unwrap();
        runtime
            .learning
            .start_curation_evaluation(&access, "p")
            .await
            .unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let p = runtime.learning.curation.get(&access, "p").unwrap();
                if p.state != mh_skill_curation::State::Evaluating {
                    break p;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(result.state, mh_skill_curation::State::Ready);
        let publisher = mh_skill_curation::ManagedSkillsPublisher(&runtime.learning.skills);
        let published = runtime
            .learning
            .curation
            .publish(&access, "p", &publisher)
            .unwrap();
        assert_eq!(published.state, mh_skill_curation::State::Published);
        assert_eq!(
            runtime.learning.skills.list(&access, &scope).unwrap().items[0].revision,
            published.published_revision.unwrap()
        );
        runtime
            .learning
            .curation
            .rollback(&access, "p", &publisher)
            .unwrap();
        assert_eq!(
            runtime.learning.skills.list(&access, &scope).unwrap().items[0].revision,
            baseline.revision
        );
        server.join().unwrap();
    }
}
