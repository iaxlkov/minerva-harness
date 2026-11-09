//! Real plugin processes verify host discovery, choice, routing and no fallback.
use minerva_runtime::{plugins::PluginManager, tools::*};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};
fn package(root: &Path, id: &str, available: bool, fail: bool) {
    let source = root.join(id);
    std::fs::create_dir_all(&source).unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("plugin-fixture{}", std::env::consts::EXE_SUFFIX));
    let entry = format!("fixture{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(binary, source.join(&entry)).unwrap();
    std::fs::write(source.join("plugin.json"),json!({"id":id,"version":"1","apiVersion":1,"capabilities":["tools","image-generation"],"backend":{"kind":"process","entry":entry}}).to_string()).unwrap();
    std::fs::write(
        source.join("image-generator.json"),
        json!({"available":available,"name":id,"fail":fail}).to_string(),
    )
    .unwrap();
    minerva_plugin_host::install_local(&source, &root.join("config")).unwrap();
}
fn context(manager: &PluginManager, call: &str) -> ToolContext {
    ToolContext {
        session_id: "s".into(),
        run_id: "r".into(),
        call_id: call.into(),
        cwd: None,
        policy: PermissionPolicy::Allow,
        cancellation: Cancellation::default(),
        interactions: manager.harness.interactions.clone(),
        sink: None,
    }
}
fn starts(root: &Path, id: &str) -> usize {
    std::fs::read_to_string(
        root.join("config/plugins/.data")
            .join(id)
            .join("tool-audit.jsonl"),
    )
    .unwrap_or_default()
    .lines()
    .filter(|s| serde_json::from_str::<Value>(s).unwrap()["method"] == "tools.start")
    .count()
}
#[tokio::test]
async fn image_generators_are_extensible_and_never_silently_fall_back() {
    let root = tempfile::tempdir().unwrap();
    package(root.path(), "test.image-a", true, true);
    package(root.path(), "test.image-b", true, false);
    package(root.path(), "test.offline", false, false);
    let manager = Arc::new(
        PluginManager::start(root.path().join("config/shell.json"))
            .await
            .unwrap(),
    );
    manager.set_enabled("test.image-b", true).await.unwrap();
    manager.set_enabled("test.offline", true).await.unwrap();
    let tools = manager.tools_for("s").await.unwrap();
    assert!(!tools
        .definitions()
        .iter()
        .any(|d| d.name.ends_with("__generate_image")));
    let single = tools
        .execute(
            "generate_image",
            json!({"prompt":"one image"}),
            context(&manager, "single"),
        )
        .await
        .unwrap();
    assert_eq!(single["generatorId"], "test.image-b");
    assert!(manager.harness.interactions.list("s").is_empty());
    let status = tools
        .execute(
            "image_status",
            single.clone(),
            context(&manager, "bad-status"),
        )
        .await;
    assert!(status.is_err()); // schema rejects extra arguments
    let status = tools
        .execute(
            "image_status",
            json!({"generatorId":single["generatorId"],"jobId":single["jobId"]}),
            context(&manager, "status"),
        )
        .await
        .unwrap();
    assert_eq!(status["status"], "completed");
    manager.set_enabled("test.image-a", true).await.unwrap();
    let tools = manager.tools_for("s").await.unwrap();
    let remembered = tools
        .execute(
            "generate_image",
            json!({"prompt":"another image"}),
            context(&manager, "remembered"),
        )
        .await
        .unwrap();
    assert_eq!(remembered["generatorId"], "test.image-b");
    assert!(manager.harness.interactions.list("s").is_empty());
    let mut ctx = context(&manager, "choice");
    ctx.session_id = "other".into();
    let worker = ctx.clone();
    let task = tokio::spawn(async move {
        tools
            .execute("generate_image", json!({"prompt":"one image"}), worker)
            .await
    });
    let question = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(q) = manager.harness.interactions.list("other").pop() {
                break q;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(question.options, vec!["test.image-a", "test.image-b"]);
    let before_b = starts(root.path(), "test.image-b");
    assert_eq!(starts(root.path(), "test.image-a"), 0);
    assert_eq!(starts(root.path(), "test.offline"), 0);
    assert!(manager.harness.interactions.answer(InteractionAnswer {
        session_id: "other".into(),
        id: question.id,
        decision: "answer".into(),
        answer: Some("test.image-a".into())
    }));
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("Selected generator failed"));
    assert_eq!(starts(root.path(), "test.image-a"), 1);
    assert_eq!(starts(root.path(), "test.image-b"), before_b);
    let mut repeated = context(&manager, "remember-failure");
    repeated.session_id = "other".into();
    let repeated = manager
        .tools_for("other")
        .await
        .unwrap()
        .execute(
            "generate_image",
            json!({"prompt":"same generator"}),
            repeated,
        )
        .await;
    assert!(repeated.unwrap_err().contains("Selected generator failed"));
    assert!(manager.harness.interactions.list("other").is_empty());
    assert_eq!(starts(root.path(), "test.image-b"), before_b);
    manager.set_enabled("test.image-a", false).await.unwrap();
    manager.set_enabled("test.image-b", false).await.unwrap();
    let tools = manager.tools_for("s").await.unwrap();
    assert!(tools
        .execute(
            "generate_image",
            json!({"prompt":"one image"}),
            context(&manager, "offline")
        )
        .await
        .unwrap_err()
        .contains("нет доступных"));
    assert_eq!(starts(root.path(), "test.offline"), 0);
    manager.set_enabled("test.offline", false).await.unwrap();
    assert!(!manager
        .tools_for("s")
        .await
        .unwrap()
        .definitions()
        .iter()
        .any(|d| d.name == "generate_image"));
    manager.shutdown().await;
}
