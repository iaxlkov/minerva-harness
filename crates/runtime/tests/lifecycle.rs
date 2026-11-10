use minerva_runtime::{Runtime, RuntimeConfig};
fn config(dir: &std::path::Path) -> RuntimeConfig {
    RuntimeConfig {
        tenant_id: "local".into(),
        workspace_id: "default".into(),
        data_dir: dir.join("data"),
        config_dir: dir.join("config"),
        profile: "desktop".into(),
    }
}
#[tokio::test]
async fn independent_runtimes_isolate_storage_and_release_locks_on_shutdown() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let first = Runtime::start(config(a.path())).await.unwrap();
    let second = Runtime::start(config(b.path())).await.unwrap();
    first.sessions.create("one").unwrap();
    assert!(second.sessions.list().unwrap().is_empty());
    assert!(Runtime::start(config(a.path())).await.is_err());
    first.shutdown().await.unwrap();
    assert!(first.sessions.list().is_err());
    let reopened = Runtime::start(config(a.path())).await.unwrap();
    assert_eq!(reopened.sessions.list().unwrap().len(), 1);
    reopened.shutdown().await.unwrap();
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_plugin_configuration_releases_database_for_retry() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    std::fs::create_dir_all(&cfg.config_dir).unwrap();
    let shell = cfg.config_dir.join("shell.json");
    std::fs::write(&shell, "not json").unwrap();
    assert!(Runtime::start(cfg.clone()).await.is_err());
    std::fs::remove_file(shell).unwrap();
    let runtime = Runtime::start(cfg).await.unwrap();
    assert_eq!(
        runtime.plugins.list().await.unwrap().len(),
        minerva_plugin_sdk::SYSTEM_PLUGIN_IDS.len()
            + minerva_plugin_sdk::OPTIONAL_HOST_PLUGIN_IDS.len()
            + 1
    );
    runtime.shutdown().await.unwrap();
    assert!(runtime.plugins.shell().is_err());
}
#[tokio::test]
async fn invalid_scope_is_rejected_before_creating_directories() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.workspace_id.clear();
    assert!(Runtime::start(cfg.clone()).await.is_err());
    assert!(!cfg.data_dir.exists());
}
