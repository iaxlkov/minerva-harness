use minerva_plugin_host::{install_local, Host};
use serde_json::json;
async fn host(root: &std::path::Path) -> Host {
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let entry = format!("fixture{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(env!("CARGO_BIN_EXE_plugin-fixture"), source.join(&entry)).unwrap();
    std::fs::write(source.join("plugin.json"),serde_json::to_vec(&json!({"id":"test.frames","version":"1","apiVersion":1,"backend":{"kind":"process","entry":entry}})).unwrap()).unwrap();
    let config = root.join("config");
    install_local(&source, &config).unwrap();
    let host = Host::open(config, true).unwrap();
    host.set_enabled("test.frames", true).await.unwrap();
    host
}
#[tokio::test]
async fn engine_start_has_larger_outgoing_budget_but_ordinary_requests_do_not() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path()).await;
    let bytes = 32 * 1024 * 1024 - 1024;
    let result = host
        .call(
            "test.frames",
            "engine.start",
            json!({"payload":"x".repeat(bytes)}),
        )
        .await
        .unwrap();
    assert_eq!(result["bytes"], bytes);
    assert!(host
        .call(
            "test.frames",
            "test.pid",
            json!({"payload":"x".repeat(bytes)})
        )
        .await
        .unwrap_err()
        .contains("request exceeds limit"));
    host.shutdown().await;
}
#[tokio::test]
async fn engine_start_rejects_outgoing_frames_larger_than_32_mib() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path()).await;
    assert!(host
        .call(
            "test.frames",
            "engine.start",
            json!({"payload":"x".repeat(32*1024*1024)})
        )
        .await
        .unwrap_err()
        .contains("request exceeds limit"));
    host.shutdown().await;
}
#[tokio::test]
async fn engine_start_response_remains_limited_to_one_mib() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path()).await;
    let error = host
        .call(
            "test.frames",
            "engine.start",
            json!({"responseBytes":1024*1024}),
        )
        .await
        .unwrap_err();
    assert!(error.contains("response exceeds limit"), "{error}");
    assert!(!host.roster()[0].enabled);
    host.shutdown().await;
}
