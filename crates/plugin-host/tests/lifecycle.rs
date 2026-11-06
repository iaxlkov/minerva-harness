use minerva_plugin_host::{install_local, Host};
use serde_json::json;
#[tokio::test]
async fn install_enable_call_disable_restore_and_digest_lock() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("bin")).unwrap();
    let executable = if cfg!(windows) {
        "bin/plugin.exe"
    } else {
        "bin/plugin"
    };
    std::fs::copy(
        std::env::var("MINERVA_EXAMPLE_BIN")
            .unwrap_or_else(|_| env!("CARGO_BIN_EXE_plugin-fixture").into()),
        source.join(executable),
    )
    .unwrap();
    std::fs::write(source.join("plugin.json"),serde_json::to_vec(&json!({"id":"test.review","version":"1.0.0","apiVersion":1,"backend":{"kind":"process","entry":executable}})).unwrap()).unwrap();
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let mut host = Host::open(config.clone(), true).unwrap();
    host.set_enabled("test.review", true).await.unwrap();
    assert_eq!(
        host.call(
            "test.review",
            "review.summary",
            json!({"files":[{"content":"a\nb"}]})
        )
        .await
        .unwrap()["lineCount"],
        2
    );
    host.shutdown().await;
    let mut host = Host::open(config.clone(), true).unwrap();
    host.restore().await.unwrap();
    assert!(host.roster()[0].1);
    host.set_enabled("test.review", false).await.unwrap();
    assert!(host
        .call("test.review", "review.summary", json!({}))
        .await
        .is_err());
    host.set_enabled("test.review", true).await.unwrap();
    host.shutdown().await;
    let mut hosted = Host::open(config.clone(), false).unwrap();
    assert!(hosted.restore().await.is_err());
    std::fs::write(config.join("plugins/test.review/extra"), "tampered").unwrap();
    let mut tampered = Host::open(config, true).unwrap();
    assert!(tampered.restore().await.is_err());
}

#[tokio::test]
async fn protocol_timeout_and_invalid_response_are_bounded() {
    use minerva_plugin_host::{Package, Process};
    let temp = tempfile::tempdir().unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_plugin-fixture"),
        temp.path().join("fixture.exe"),
    )
    .unwrap();
    std::fs::write(temp.path().join("plugin.json"),r#"{"id":"test.fixture","version":"1","apiVersion":1,"backend":{"kind":"process","entry":"fixture.exe"}}"#).unwrap();
    let package = Package::load(temp.path()).unwrap();
    let marker = temp.path().join("child-survived");
    let mut process = Process::start(&package, std::time::Duration::from_secs(3))
        .await
        .unwrap();
    process
        .call("test.spawn", json!({"marker":marker}))
        .await
        .unwrap();
    process.stop().await;
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    assert!(
        !marker.exists(),
        "plugin descendant survived process cleanup"
    );
    for method in ["test.hang", "test.invalid"] {
        let mut process = Process::start(&package, std::time::Duration::from_millis(300))
            .await
            .unwrap();
        assert!(process.call(method, json!({})).await.is_err());
    }
}
#[tokio::test]
async fn idle_exit_is_reaped_and_enable_restarts() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_fixture_package(&source, "test.crash");
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let mut host = Host::open(config, true).unwrap();
    host.set_enabled("test.crash", true).await.unwrap();
    let marker = temp.path().join("idle-helper-survived");
    host.call("test.crash", "test.spawn", json!({"marker":marker}))
        .await
        .unwrap();
    host.call("test.crash", "test.exit", json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!host.roster()[0].1, "idle exited process remains enabled");
    host.set_enabled("test.crash", true).await.unwrap();
    assert!(host.roster()[0].1);
    host.call("test.crash", "test.exit", json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // Enabling again must reap/restart even when no roster request happens first.
    host.set_enabled("test.crash", true).await.unwrap();
    host.call("test.crash", "review.summary", json!({}))
        .await
        .unwrap();
    host.shutdown().await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(!marker.exists(), "idle-crash helper survived reaping");
}
fn make_fixture_package(path: &std::path::Path, id: &str) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_plugin-fixture"),
        path.join("fixture.exe"),
    )
    .unwrap();
    std::fs::write(path.join("plugin.json"),serde_json::to_vec(&json!({"id":id,"version":"1","apiVersion":1,"backend":{"kind":"process","entry":"fixture.exe"}})).unwrap()).unwrap();
}
#[tokio::test]
async fn installer_and_stale_host_merge_persisted_state() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let a = temp.path().join("a");
    make_fixture_package(&a, "test.first");
    install_local(&a, &config).unwrap();
    let mut stale = Host::open(config.clone(), true).unwrap();
    let b = temp.path().join("b");
    make_fixture_package(&b, "test.second");
    let c = temp.path().join("c");
    make_fixture_package(&c, "test.third");
    let mut one = std::process::Command::new(env!("CARGO_BIN_EXE_plugin-fixture"))
        .arg("--install")
        .arg(&b)
        .arg(&config)
        .spawn()
        .unwrap();
    let mut two = std::process::Command::new(env!("CARGO_BIN_EXE_plugin-fixture"))
        .arg("--install")
        .arg(&c)
        .arg(&config)
        .spawn()
        .unwrap();
    assert!(one.wait().unwrap().success());
    assert!(two.wait().unwrap().success());
    stale.set_enabled("test.first", true).await.unwrap();
    stale.shutdown().await;
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(config.join("plugin-lock.json")).unwrap()).unwrap();
    assert_eq!(lock["plugins"].as_object().unwrap().len(), 3);
    assert_eq!(lock["plugins"]["test.first"]["enabled"], true);
    assert_eq!(lock["plugins"]["test.second"]["enabled"], false);
}

#[tokio::test]
async fn helpers_spawned_before_initialize_belong_to_cleanup_scope() {
    use minerva_plugin_host::{Package, Process};
    let temp = tempfile::tempdir().unwrap();
    let package_path = temp.path().join("package");
    make_fixture_package(&package_path, "test.startup");
    let marker = temp.path().join("startup-child-survived");
    std::fs::write(
        package_path.join("startup-helper-marker"),
        marker.to_str().unwrap(),
    )
    .unwrap();
    let package = Package::load(&package_path).unwrap();
    let mut process = Process::start(&package, std::time::Duration::from_secs(3))
        .await
        .unwrap();
    process.stop().await;
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    assert!(!marker.exists(), "startup helper escaped cleanup scope");
}
