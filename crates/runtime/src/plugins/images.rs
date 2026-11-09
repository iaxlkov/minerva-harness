//! Host-owned image selection is independent of the calling chat model.
use super::run_plugin_tool;
use crate::tools::{ToolContext, ToolDefinition, ToolRegistry};
use minerva_plugin_host::Host;
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
pub(super) const OWNER: &str = "native.images";
pub(super) fn is_generator(manifest: &minerva_plugin_sdk::Manifest) -> bool {
    manifest
        .capabilities
        .iter()
        .any(|c| c == "image-generation")
}
pub(super) fn has_generators(host: &Host) -> bool {
    host.roster()
        .iter()
        .any(|p| p.enabled && p.manifest.as_ref().is_some_and(is_generator))
}
struct Selections {
    store: Arc<crate::context::ContextService>,
    locks: Mutex<BTreeMap<String, Weak<tokio::sync::Mutex<()>>>>,
}
impl Selections {
    fn new(store: Arc<crate::context::ContextService>) -> Self {
        Self {
            store,
            locks: Mutex::new(BTreeMap::new()),
        }
    }
    async fn choose(
        &self,
        context: &ToolContext,
        providers: &[(String, String)],
    ) -> Result<String, String> {
        // Concurrent calls in one session share one question; other sessions can choose independently.
        let lock = {
            let mut locks = self
                .locks
                .lock()
                .map_err(|_| "image selection lock poisoned")?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            let lock = locks
                .get(&context.session_id)
                .and_then(Weak::upgrade)
                .unwrap_or_default();
            locks.insert(context.session_id.clone(), Arc::downgrade(&lock));
            lock
        };
        let _guard = lock.lock().await;
        let saved: Option<String> = self
            .store
            .db()?
            .query_row(
                "SELECT generator FROM image_generator_choice WHERE session=?1",
                [&context.session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = saved
            .as_ref()
            .filter(|id| providers.iter().any(|(available, _)| available == *id))
        {
            return Ok(id.clone());
        }
        let id = choose(context, providers, saved.is_some()).await?;
        self.store.db()?.execute("INSERT INTO image_generator_choice(session,generator) VALUES (?1,?2) ON CONFLICT(session) DO UPDATE SET generator=excluded.generator",params![context.session_id,id]).map_err(|e|e.to_string())?;
        Ok(id)
    }
}
async fn choose(
    context: &ToolContext,
    providers: &[(String, String)],
    replacing: bool,
) -> Result<String, String> {
    match providers {
        [] => Err("Сейчас нет доступных генераторов изображений".into()),
        [(id, _)] if !replacing => Ok(id.clone()),
        _ => {
            let options: Vec<_> = providers
                .iter()
                .map(|(id, name)| {
                    if providers.iter().filter(|(_, n)| n == name).count() > 1 {
                        format!("{name} ({id})")
                    } else {
                        name.clone()
                    }
                })
                .collect();
            let selected = context
                .interactions
                .request(
                    context,
                    "question",
                    "Генератор изображения",
                    if replacing {
                        "Выбранный для этой сессии генератор недоступен. Какой использовать дальше?"
                    } else {
                        "Каким генератором создавать изображения в этой сессии?"
                    },
                    options.clone(),
                )
                .await?;
            providers
                .iter()
                .zip(options)
                .find(|(_, label)| label == &selected)
                .map(|((id, _), _)| id.clone())
                .ok_or_else(|| "Неизвестный генератор".into())
        }
    }
}
pub(super) fn register(
    registry: &ToolRegistry,
    host: Arc<Host>,
    store: Arc<crate::context::ContextService>,
) -> Result<(), String> {
    let selections = Arc::new(Selections::new(store));
    let generation = ToolDefinition {
        name: "generate_image".into(), description: "Create one image. The host remembers the generator for this session and asks when a choice is needed. Returns a background job; use image_status later. Never retry an uncertain submission.".into(),
        input_schema: json!({"type":"object","properties":{"prompt":{"type":"string","minLength":1,"maxLength":12000}},"required":["prompt"],"additionalProperties":false}), output_schema: json!({"type":"object"}), side_effect: true,
    };
    let status = ToolDefinition {
        name: "image_status".into(), description: "Collect an image job using generatorId and jobId returned by generate_image. Wait between polls; never regenerate a running job.".into(),
        input_schema: json!({"type":"object","properties":{"generatorId":{"type":"string","maxLength":100},"jobId":{"type":"string","minLength":1,"maxLength":128}},"required":["generatorId","jobId"],"additionalProperties":false}), output_schema: json!({"type":"object"}), side_effect: false,
    };
    for (definition, generate) in [(generation, true), (status, false)] {
        let host = host.clone();
        let selections = selections.clone();
        registry.register_with_timeout(
            OWNER,
            definition,
            Duration::from_secs(600),
            move |context, args| {
                let host = host.clone();
                let selections = selections.clone();
                Box::pin(async move {
                    let id = if generate {
                        let mut providers = Vec::new();
                        for entry in host
                            .roster()
                            .into_iter()
                            .filter(|p| p.enabled && p.manifest.as_ref().is_some_and(is_generator))
                        {
                            // Probe health without submitting an image request.
                            if let Ok(probe) =
                                host.call(&entry.id, "images.available", json!({})).await
                            {
                                if probe["available"] == true {
                                    let label = probe["name"]
                                        .as_str()
                                        .filter(|s| !s.is_empty() && s.len() <= 120)
                                        .unwrap_or(&entry.id)
                                        .to_owned();
                                    providers.push((entry.id, label));
                                }
                            }
                        }
                        selections.choose(&context, &providers).await?
                    } else {
                        let id = args["generatorId"].as_str().ok_or("Missing generatorId")?;
                        if !host.roster().iter().any(|p| {
                            p.id == id && p.enabled && p.manifest.as_ref().is_some_and(is_generator)
                        }) {
                            return Err("Генератор отключён или удалён".into());
                        }
                        id.to_owned()
                    };
                    let arguments = if generate {
                        args
                    } else {
                        json!({"jobId":args["jobId"]})
                    };
                    let mut result = run_plugin_tool(
                        host,
                        &id,
                        if generate {
                            "generate_image"
                        } else {
                            "image_status"
                        },
                        arguments,
                        context,
                    )
                    .await?;
                    if !result.is_object() {
                        return Err("Invalid image generator result".into());
                    }
                    result["generatorId"] = json!(id);
                    Ok(result)
                })
            },
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{Cancellation, InteractionBroker, PermissionPolicy};
    fn context() -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            run_id: "r".into(),
            call_id: "c".into(),
            cwd: None,
            policy: PermissionPolicy::Allow,
            cancellation: Cancellation::default(),
            interactions: Arc::new(InteractionBroker::default()),
            sink: None,
        }
    }
    async fn answer(context: &ToolContext, label: &str) {
        let question = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(question) = context.interactions.list(&context.session_id).pop() {
                    break question;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(context.interactions.list(&context.session_id).len(), 1);
        assert!(context
            .interactions
            .answer(crate::tools::InteractionAnswer {
                session_id: context.session_id.clone(),
                id: question.id,
                decision: "answer".into(),
                answer: Some(label.into())
            }));
    }
    #[tokio::test]
    async fn session_choice_survives_restart_is_scoped_and_deduplicates_questions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.sqlite3");
        let selections = Arc::new(Selections::new(Arc::new(
            crate::context::ContextService::open(&path).unwrap(),
        )));
        let providers = vec![
            ("home".into(), "Qwen".into()),
            ("codex".into(), "ChatGPT".into()),
        ];
        let ctx = context();
        let mut pending = vec![];
        for _ in 0..2 {
            let selections = selections.clone();
            let providers = providers.clone();
            let ctx = ctx.clone();
            pending.push(tokio::spawn(async move {
                selections.choose(&ctx, &providers).await
            }));
        }
        answer(&ctx, "ChatGPT").await;
        for task in pending {
            assert_eq!(task.await.unwrap().unwrap(), "codex");
        }
        assert!(ctx.interactions.list("s").is_empty());
        let mut other = context();
        other.session_id = "other".into();
        let worker = other.clone();
        let store = selections.clone();
        let available = providers.clone();
        let task = tokio::spawn(async move { store.choose(&worker, &available).await });
        answer(&other, "Qwen").await;
        assert_eq!(task.await.unwrap().unwrap(), "home");
        drop(selections);
        let restored = Arc::new(Selections::new(Arc::new(
            crate::context::ContextService::open(&path).unwrap(),
        )));
        assert_eq!(restored.choose(&ctx, &providers).await.unwrap(), "codex");
        assert_eq!(restored.choose(&other, &providers).await.unwrap(), "home");
        // Losing the selected generator requires confirmation even with only one alternative.
        let store = restored.clone();
        let worker = ctx.clone();
        let task = tokio::spawn(async move {
            store
                .choose(&worker, &[("home".into(), "Qwen".into())])
                .await
        });
        answer(&ctx, "Qwen").await;
        assert_eq!(task.await.unwrap().unwrap(), "home");
        assert_eq!(restored.choose(&ctx, &providers).await.unwrap(), "home");
        assert!(restored.choose(&ctx, &[]).await.is_err());
        assert_eq!(restored.choose(&ctx, &providers).await.unwrap(), "home");
    }
    #[tokio::test]
    async fn zero_one_and_two_generators_require_the_right_choice() {
        let ctx = context();
        assert!(choose(&ctx, &[], false).await.is_err());
        assert_eq!(
            choose(&ctx, &[("home".into(), "Qwen дома".into())], false)
                .await
                .unwrap(),
            "home"
        );
        assert!(ctx.interactions.list("s").is_empty());
        let worker = ctx.clone();
        let task = tokio::spawn(async move {
            choose(
                &worker,
                &[
                    ("home".into(), "Qwen дома".into()),
                    ("codex".into(), "ChatGPT".into()),
                ],
                false,
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        let question = ctx.interactions.list("s").pop().unwrap();
        assert!(ctx.interactions.answer(crate::tools::InteractionAnswer {
            session_id: "s".into(),
            id: question.id,
            decision: "answer".into(),
            answer: Some("ChatGPT".into())
        }));
        assert_eq!(task.await.unwrap().unwrap(), "codex");
    }
}
