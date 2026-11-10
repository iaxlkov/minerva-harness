use minerva_runtime::{Runtime, RuntimeConfig};

#[tokio::test]
async fn second_runtime_is_rejected_before_opening_live_storage() {
    let temp = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        tenant_id: "owner".into(),
        workspace_id: "workspace".into(),
        profile: "local".into(),
        data_dir: temp.path().join("data"),
        config_dir: temp.path().join("config"),
    };
    let first = Runtime::start(config.clone()).await.unwrap();
    first.create_session("live", None).unwrap();
    let second = Runtime::start(config.clone()).await;
    assert!(matches!(second, Err(ref error) if error.to_string().contains("another runtime")));
    assert_eq!(first.sessions.get("live").unwrap().id, "live");
    first.shutdown().await.unwrap();
    let reopened = Runtime::start(config).await.unwrap();
    drop(first);
    assert_eq!(reopened.sessions.get("live").unwrap().id, "live");
    reopened.shutdown().await.unwrap();
}
