use minerva_plugin_host::{install_local, Host};
use serde_json::json;

#[test]
fn broken_package_does_not_abort_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let broken = temp.path().join("plugins/test.broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("plugin.json"), "invalid").unwrap();
    let host = Host::open(temp.path().to_owned(), true).unwrap();
    let roster = host.roster();
    assert_eq!(roster[0].id, "test.broken");
    assert!(roster[0].manifest.is_none());
    assert!(roster[0].error.is_some());
    assert!(!roster[0].enabled);
}

#[tokio::test]
async fn restore_isolates_bad_packages_and_retry_revalidates() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    for id in ["test.bad", "test.good", "test.missing"] {
        let source = temp.path().join(id);
        make_fixture_package(&source, id);
        install_local(&source, &config).unwrap();
    }
    let lock_path = config.join("plugin-lock.json");
    let mut lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    for entry in lock["plugins"].as_object_mut().unwrap().values_mut() {
        entry["enabled"] = json!(true);
    }
    std::fs::write(&lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();
    let bad_manifest = config.join("plugins/test.bad/plugin.json");
    let original = std::fs::read(&bad_manifest).unwrap();
    std::fs::write(&bad_manifest, "bad manifest").unwrap();
    std::fs::rename(
        config.join("plugins/test.missing"),
        temp.path().join("removed"),
    )
    .unwrap();
    let host = Host::open(config.clone(), true).unwrap();
    host.restore().await.unwrap();
    let roster = host.roster();
    assert!(!roster[0].enabled && roster[0].error.is_some());
    assert!(roster[1].enabled && roster[1].error.is_none());
    assert!(!roster[2].enabled && roster[2].error.is_some());
    host.set_enabled("test.bad", false).await.unwrap();
    host.set_enabled("test.missing", false).await.unwrap();
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    assert_eq!(lock["plugins"]["test.bad"]["enabled"], false);
    assert_eq!(lock["plugins"]["test.missing"]["enabled"], false);
    std::fs::write(&bad_manifest, original).unwrap();
    host.set_enabled("test.bad", true).await.unwrap();
    assert!(host.roster()[0].enabled);
    host.shutdown().await;
}

async fn wait_for_marker(marker: &std::path::Path) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !marker.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn redundant_enable_during_rpc_keeps_the_running_process() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_fixture_package(&source, "test.running");
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let host = std::sync::Arc::new(Host::open(config, true).unwrap());
    host.set_enabled("test.running", true).await.unwrap();
    let original = host
        .call("test.running", "test.pid", json!({}))
        .await
        .unwrap();
    let generation = host.roster()[0].generation;
    let marker = temp.path().join("started");
    let worker = host.clone();
    let params = json!({"marker":marker});
    let rpc = tokio::spawn(async move { worker.call("test.running", "test.delay", params).await });
    wait_for_marker(&marker).await;
    host.set_enabled("test.running", true).await.unwrap();
    assert_eq!(rpc.await.unwrap().unwrap(), original);
    assert_eq!(
        host.call("test.running", "test.pid", json!({}))
            .await
            .unwrap(),
        original
    );
    assert_eq!(host.roster()[0].generation, generation);
    host.shutdown().await;
}

#[tokio::test]
async fn failed_initialize_does_not_suppress_restoration_of_healthy_plugin() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    for id in ["test.bad", "test.good"] {
        let source = temp.path().join(id);
        make_fixture_package(&source, id);
        if id == "test.bad" {
            std::fs::write(source.join("bad-handshake"), "").unwrap();
        }
        install_local(&source, &config).unwrap();
    }
    let host = Host::open(config.clone(), true).unwrap();
    assert!(host.set_enabled("test.bad", true).await.is_err());
    host.set_enabled("test.good", true).await.unwrap();
    host.shutdown().await;
    let host = Host::open(config.clone(), true).unwrap();
    host.restore().await.unwrap();
    let roster = host.roster();
    assert!(!roster[0].enabled);
    assert_eq!(roster[0].error.as_deref(), Some("incompatible handshake"));
    assert!(roster[1].enabled);
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(config.join("plugin-lock.json")).unwrap()).unwrap();
    assert_eq!(lock["plugins"]["test.bad"]["enabled"], true);
    host.shutdown().await;
}

#[tokio::test]
async fn hung_rpc_is_isolated_and_cancelled_process_cannot_be_reused() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    for id in ["test.slow", "test.good"] {
        let source = temp.path().join(id);
        make_fixture_package(&source, id);
        install_local(&source, &config).unwrap();
    }
    let host = std::sync::Arc::new(Host::open(config, true).unwrap());
    host.set_enabled("test.slow", true).await.unwrap();
    host.set_enabled("test.good", true).await.unwrap();
    let marker = temp.path().join("started");
    let worker = host.clone();
    let params = json!({"marker":marker});
    let slow = tokio::spawn(async move { worker.call("test.slow", "test.hang", params).await });
    wait_for_marker(&marker).await;
    assert!(host.roster().iter().all(|p| p.enabled));
    tokio::time::timeout(
        std::time::Duration::from_millis(500),
        host.call("test.good", "review.summary", json!({})),
    )
    .await
    .unwrap()
    .unwrap();
    let worker = host.clone();
    let queued =
        tokio::spawn(async move { worker.call("test.slow", "review.summary", json!({})).await });
    tokio::task::yield_now().await;
    slow.abort();
    assert!(slow.await.unwrap_err().is_cancelled());
    assert!(queued.await.unwrap().is_err());
    let failed = host
        .roster()
        .into_iter()
        .find(|p| p.id == "test.slow")
        .unwrap();
    assert!(!failed.enabled && failed.error.is_some());
    host.set_enabled("test.slow", true).await.unwrap();
    host.call("test.slow", "review.summary", json!({}))
        .await
        .unwrap();
    host.shutdown().await;
}

#[tokio::test]
async fn disable_fences_queued_calls_and_old_catalogue_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_fixture_package(&source, "test.slow");
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let host = std::sync::Arc::new(Host::open(config, true).unwrap());
    host.set_enabled("test.slow", true).await.unwrap();
    let generation = host.roster()[0].generation;
    host.set_catalog_error("test.slow", generation, Some("bad catalog".into()));
    assert_eq!(host.roster()[0].error.as_deref(), Some("bad catalog"));
    let marker = temp.path().join("started");
    let worker = host.clone();
    let params = json!({"marker":marker});
    let slow = tokio::spawn(async move { worker.call("test.slow", "test.hang", params).await });
    wait_for_marker(&marker).await;
    let worker = host.clone();
    let queued =
        tokio::spawn(async move { worker.call("test.slow", "review.summary", json!({})).await });
    tokio::task::yield_now().await;
    let worker = host.clone();
    let disable = tokio::spawn(async move { worker.set_enabled("test.slow", false).await });
    tokio::time::timeout(std::time::Duration::from_millis(500), async {
        while host.roster()[0].enabled {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    slow.abort();
    let _ = slow.await;
    assert!(queued.await.unwrap().is_err());
    disable.await.unwrap().unwrap();
    host.set_enabled("test.slow", true).await.unwrap();
    host.set_catalog_error("test.slow", generation, Some("stale catalog".into()));
    assert!(host.roster()[0].error.is_none());
    host.call("test.slow", "review.summary", json!({}))
        .await
        .unwrap();
    host.shutdown().await;
}

#[tokio::test]
async fn engine_package_can_start_with_explicit_local_environment() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_plugin-fixture"),
        source.join("fixture.exe"),
    )
    .unwrap();
    std::fs::write(source.join("client.js"), "exports.sdkVersion = 2;").unwrap();
    std::fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&json!({
            "id":"test.engine","version":"1","apiVersion":1,
            "capabilities":["model-engine","local-environment"],
            "backend":{"kind":"process","entry":"fixture.exe"},
            "ui":{"entry":"client.js","sdkVersion":2}
        }))
        .unwrap(),
    )
    .unwrap();
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let host = Host::open(config, true).unwrap();
    host.set_enabled("test.engine", true).await.unwrap();
    assert_eq!(
        host.ui_document("test.engine").unwrap(),
        "exports.sdkVersion = 2;"
    );
    assert!(host
        .call("test.engine", "test.error", json!({}))
        .await
        .is_err());
    assert!(host
        .call("test.engine", "review.summary", json!({}))
        .await
        .is_ok());
    host.set_enabled("test.engine", false).await.unwrap();
    assert!(host.ui_document("test.engine").is_err());
    host.shutdown().await;
}
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
    let host = Host::open(config.clone(), true).unwrap();
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
    let host = Host::open(config.clone(), true).unwrap();
    host.restore().await.unwrap();
    assert!(host.roster()[0].enabled);
    host.set_enabled("test.review", false).await.unwrap();
    assert!(host
        .call("test.review", "review.summary", json!({}))
        .await
        .is_err());
    host.set_enabled("test.review", true).await.unwrap();
    host.shutdown().await;
    let hosted = Host::open(config.clone(), false).unwrap();
    hosted.restore().await.unwrap();
    assert!(hosted.roster()[0].error.is_some());
    std::fs::write(config.join("plugins/test.review/extra"), "tampered").unwrap();
    let tampered = Host::open(config, true).unwrap();
    tampered.restore().await.unwrap();
    assert!(tampered.roster()[0].error.is_some());
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
    let host = Host::open(config, true).unwrap();
    host.set_enabled("test.crash", true).await.unwrap();
    let marker = temp.path().join("idle-helper-survived");
    host.call("test.crash", "test.spawn", json!({"marker":marker}))
        .await
        .unwrap();
    host.call("test.crash", "test.exit", json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !host.roster()[0].enabled,
        "idle exited process remains enabled"
    );
    host.set_enabled("test.crash", true).await.unwrap();
    assert!(host.roster()[0].enabled);
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
    let stale = Host::open(config.clone(), true).unwrap();
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

#[tokio::test]
async fn optional_pause_preserves_selection_across_restart() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_fixture_package(&source, "test.optional");
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let host = Host::open(config.clone(), true).unwrap();
    host.set_enabled("test.optional", true).await.unwrap();
    host.set_optional_enabled(false).await.unwrap();
    assert!(!host.roster()[0].enabled);
    assert!(host.roster()[0].requested_enabled);
    host.set_enabled("test.optional", false).await.unwrap();
    assert!(!host.roster()[0].requested_enabled);
    host.set_enabled("test.optional", true).await.unwrap();
    assert!(!host.roster()[0].enabled);
    host.set_example_enabled(false).await.unwrap();
    let host = Host::open(config, true).unwrap();
    host.restore().await.unwrap();
    assert!(!host.optional_enabled());
    assert!(!host.example_enabled());
    assert!(!host.roster()[0].enabled);
    host.set_optional_enabled(true).await.unwrap();
    assert!(host.roster()[0].enabled);
    host.shutdown().await;
}

#[tokio::test]
async fn declared_services_route_and_provider_failure_blocks_consumers() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    for (id, extra) in [
        ("test.provider", json!({"provides":{"echo":1}})),
        ("test.consumer", json!({"requires":{"echo":1}})),
    ] {
        let source = temp.path().join(id);
        make_fixture_package(&source, id);
        let path = source.join("plugin.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        if id == "test.consumer" {
            std::fs::write(source.join("initialize-service"), "").unwrap();
        }
        std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        install_local(&source, &config).unwrap();
    }
    let host = Host::open(config.clone(), true).unwrap();
    host.set_enabled("test.provider", true).await.unwrap();
    host.set_enabled("test.consumer", true).await.unwrap();
    host.shutdown().await;
    let host = Host::open(config, true).unwrap();
    host.restore().await.unwrap();
    assert!(host.roster().iter().all(|p| p.enabled));
    assert_eq!(
        host.call(
            "test.consumer",
            "test.service",
            json!({"service":"echo","version":1,"method":"test.pid","params":{}})
        )
        .await
        .unwrap(),
        host.call("test.provider", "test.pid", json!({}))
            .await
            .unwrap()
    );
    assert!(host
        .call(
            "test.consumer",
            "test.service",
            json!({"service":"sessions","version":1,"method":"list","params":{}})
        )
        .await
        .is_err());
    for params in [
        json!({"service":"echo","version":2,"method":"test.pid","params":{}}),
        json!({"service":"echo","version":1,"method":"initialize","params":{}}),
        json!({"service":"echo","version":1,"method":"engine.turn","params":{}}),
        json!({"service":"echo","version":1,"method":"test.pid","params":{},"caller":"test.provider"}),
    ] {
        assert!(host
            .call("test.consumer", "test.service", params)
            .await
            .is_err());
    }
    host.call("test.provider", "test.exit", json!({}))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(host.roster().iter().all(|p| !p.enabled));
    host.set_enabled("test.provider", true).await.unwrap();
    assert!(host.roster().iter().all(|p| p.enabled));
    host.set_enabled("test.provider", false).await.unwrap();
    assert!(host.roster().iter().all(|p| !p.enabled));
    assert!(host.roster()[0].requested_enabled);
    host.shutdown().await;
}

#[tokio::test]
async fn invalid_service_graphs_are_diagnostic_and_never_execute() {
    for (requirements, providers, expected) in [
        (json!({"absent":1}), vec![], "missing"),
        (
            json!({"echo":2}),
            vec![json!({"echo":1})],
            "version mismatch",
        ),
        (
            json!({"echo":1}),
            vec![json!({"echo":1}), json!({"echo":1})],
            "ambiguous",
        ),
        (json!({"echo":1}), vec![json!({"echo":1})], "cycle"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        for (id, requires, provides) in
            std::iter::once(("test.consumer".to_string(), requirements, json!({"back":1}))).chain(
                providers.into_iter().enumerate().map(|(i, provides)| {
                    (
                        format!("test.provider{i}"),
                        if expected == "cycle" {
                            json!({"back":1})
                        } else {
                            json!({})
                        },
                        provides,
                    )
                }),
            )
        {
            let source = temp.path().join(&id);
            make_fixture_package(&source, &id);
            let path = source.join("plugin.json");
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            manifest["requires"] = requires;
            manifest["provides"] = provides;
            std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            install_local(&source, &config).unwrap();
        }
        let lock_path = config.join("plugin-lock.json");
        let mut lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
        for plugin in lock["plugins"].as_object_mut().unwrap().values_mut() {
            plugin["enabled"] = json!(true);
        }
        std::fs::write(lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();
        let host = Host::open(config, true).unwrap();
        host.restore().await.unwrap();
        let consumer = host
            .roster()
            .into_iter()
            .find(|p| p.id == "test.consumer")
            .unwrap();
        assert!(!consumer.enabled);
        assert!(consumer.error.unwrap().contains(expected));
        host.shutdown().await;
    }
}

#[tokio::test]
async fn master_pause_fences_inflight_success() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_fixture_package(&source, "test.delay");
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let host = std::sync::Arc::new(Host::open(config, true).unwrap());
    host.set_enabled("test.delay", true).await.unwrap();
    let marker = temp.path().join("started");
    let worker = host.clone();
    let params = json!({"marker":marker});
    let rpc = tokio::spawn(async move { worker.call("test.delay", "test.delay", params).await });
    wait_for_marker(&marker).await;
    host.set_optional_enabled(false).await.unwrap();
    assert!(rpc.await.unwrap().is_err());
}

#[tokio::test]
async fn cancelled_master_pause_cleans_every_process_tree() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config");
    let shutdown_marker = temp.path().join("shutdown-started");
    for id in ["test.first", "test.second"] {
        let source = temp.path().join(id);
        make_fixture_package(&source, id);
        if id == "test.first" {
            std::fs::write(
                source.join("hang-shutdown"),
                shutdown_marker.to_str().unwrap(),
            )
            .unwrap();
        }
        install_local(&source, &config).unwrap();
    }
    let host = std::sync::Arc::new(Host::open(config, true).unwrap());
    for id in ["test.first", "test.second"] {
        host.set_enabled(id, true).await.unwrap();
    }
    for id in ["test.first", "test.second"] {
        host.call(
            id,
            "test.spawn",
            json!({"marker":temp.path().join(format!("{id}-survived"))}),
        )
        .await
        .unwrap();
    }
    let worker = host.clone();
    let pause = tokio::spawn(async move { worker.set_optional_enabled(false).await });
    wait_for_marker(&shutdown_marker).await;
    pause.abort();
    assert!(pause.await.unwrap_err().is_cancelled());
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    for id in ["test.first", "test.second"] {
        assert!(
            !temp.path().join(format!("{id}-survived")).exists(),
            "descendant {id} survived cancelled pause"
        );
    }
    assert!(host.roster().iter().all(|p| !p.enabled));
}

#[test]
fn legacy_lock_defaults_optional_and_example_to_selected() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("plugin-lock.json"), r#"{"plugins":{}}"#).unwrap();
    let host = Host::open(temp.path().to_owned(), true).unwrap();
    assert!(host.optional_enabled());
    assert!(host.example_enabled());
}

#[tokio::test]
async fn repaired_consumer_is_revalidated_before_dependency_resolution() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_fixture_package(&source, "test.repaired");
    let path = source.join("plugin.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["requires"] = json!({"missing-service":1});
    let original = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(&path, &original).unwrap();
    let config = temp.path().join("config");
    install_local(&source, &config).unwrap();
    let installed = config.join("plugins/test.repaired/plugin.json");
    std::fs::write(&installed, "broken").unwrap();
    let host = Host::open(config, true).unwrap();
    assert!(host.set_enabled("test.repaired", true).await.is_err());
    std::fs::write(installed, original).unwrap();
    let error = host.set_enabled("test.repaired", true).await.unwrap_err();
    assert!(error.contains("missing or unsupported service"), "{error}");
    assert!(!host.roster()[0].enabled);
    assert!(host.roster()[0].requested_enabled);
}
