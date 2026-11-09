use minerva_runtime::{Runtime, RuntimeConfig};
use serde_json::json;
use std::sync::Arc;
#[tokio::test]
async fn editor_uses_session_workspace_and_rejects_stale_save() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Arc::new(
        Runtime::start(RuntimeConfig {
            tenant_id: "local".into(),
            workspace_id: "default".into(),
            data_dir: dir.path().join("data"),
            config_dir: dir.path().join("config"),
            profile: "desktop".into(),
        })
        .await
        .unwrap(),
    );
    runtime.create_session("editor-test", None).unwrap();
    let file = dir.path().join("data/default-workspace/proof.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let list = runtime
        .harness_call("mh-editor.list", json!({"sessionId":"editor-test"}))
        .await
        .unwrap();
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "proof.rs")
    );
    let read = runtime
        .harness_call(
            "mh-editor.read",
            json!({"sessionId":"editor-test","path":"proof.rs"}),
        )
        .await
        .unwrap();
    let version = read["version"].as_str().unwrap();
    std::fs::write(&file, "external").unwrap();
    assert!(runtime.harness_call("mh-editor.save",json!({"sessionId":"editor-test","path":"proof.rs","version":version,"text":"my code"})).await.is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "external");
    let read = runtime
        .harness_call(
            "mh-editor.read",
            json!({"sessionId":"editor-test","path":"proof.rs"}),
        )
        .await
        .unwrap();
    runtime.harness_call("mh-editor.save",json!({"sessionId":"editor-test","path":"proof.rs","version":read["version"],"text":"my code"})).await.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "my code");
    assert!(
        runtime
            .harness_call(
                "mh-editor.read",
                json!({"sessionId":"editor-test","path":"../secret"})
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .harness_call(
                "mh-editor.read",
                json!({"sessionId":"not-a-session","path":"proof.rs"})
            )
            .await
            .is_err()
    );
    runtime.shutdown().await.unwrap();
}
