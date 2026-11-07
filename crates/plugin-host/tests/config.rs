use minerva_plugin_host::config::{ConfigStore, PluginConfig};
use serde_json::json;

#[test]
fn defaults_and_updates_are_validated_without_stale_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path()).unwrap();
    let other = store.clone();
    let defaults = store.snapshot().unwrap();
    assert_eq!(defaults.version, 1);
    assert!(defaults.optional_plugins_enabled);
    assert_eq!(defaults.services.shell.timeout_ms, 120_000);
    assert_eq!(defaults.services.agent_loop.shutdown_timeout_ms, 5_000);
    store
        .update(|c| {
            c.optional_plugins_enabled = false;
            Ok(())
        })
        .unwrap();
    other
        .update(|c| {
            c.plugins
                .insert("test.missing".into(), PluginConfig { enabled: true });
            Ok(())
        })
        .unwrap();
    assert!(!store.snapshot().unwrap().optional_plugins_enabled);
    assert!(store.snapshot().unwrap().plugins["test.missing"].enabled);
    assert!(store
        .update(|c| {
            c.services.shell.timeout_ms = 0;
            Ok(())
        })
        .is_err());
    assert_eq!(store.snapshot().unwrap().services.shell.timeout_ms, 120_000);
}

#[test]
fn imports_legacy_once_and_preserves_legacy_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = br#"{"plugins":{"test.old":{"enabled":true,"digest":"abc"}},"optional_enabled":false,"example_enabled":false}"#;
    std::fs::write(dir.path().join("plugin-lock.json"), legacy).unwrap();
    std::fs::write(
        dir.path().join("shell.json"),
        br#"{"timeoutMs":1234,"maxOutputBytes":4567}"#,
    )
    .unwrap();
    let store = ConfigStore::open(dir.path()).unwrap();
    let config = store.snapshot().unwrap();
    assert!(!config.optional_plugins_enabled);
    assert!(!config.plugins["example"].enabled);
    assert!(config.plugins["test.old"].enabled);
    assert_eq!(config.services.shell.timeout_ms, 1234);
    assert_eq!(
        std::fs::read(dir.path().join("plugin-lock.json")).unwrap(),
        legacy
    );
    std::fs::write(dir.path().join("shell.json"), "broken").unwrap();
    assert_eq!(
        ConfigStore::open(dir.path()).unwrap().snapshot().unwrap(),
        config
    );
}

#[test]
fn rejects_invalid_config_without_overwriting_it() {
    let dir = tempfile::tempdir().unwrap();
    for value in [
        json!({}),
        json!({"version":2}),
        json!({"version":1,"unknown":true}),
        json!({"version":1,"plugins":{"../escape":{"enabled":true}}}),
        json!({"version":1,"plugins":{"shell":{"enabled":false}}}),
        json!({"version":1,"services":{"shell":{"timeoutMs":99}}}),
        json!({"version":1,"services":{"shell":{"maxOutputBytes":1000001}}}),
        json!({"version":1,"services":{"agent-loop":{"shutdownTimeoutMs":60001}}}),
        json!({"version":1,"services":{"unknown":{}}}),
        json!({"version":1,"services":{"shell":{"unknown":1}}}),
        json!({"version":1,"services":{"agent-loop":{"unknown":1}}}),
        json!({"version":1,"plugins":{"example":{"enabled":true,"unknown":1}}}),
    ] {
        let bytes = serde_json::to_vec(&value).unwrap();
        let path = dir.path().join("minerva.conf.json");
        std::fs::write(&path, &bytes).unwrap();
        let error = ConfigStore::open(dir.path())
            .err()
            .expect("invalid config accepted");
        assert!(error.contains("minerva.conf.json"), "{error}");
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn concurrent_updates_merge_and_snapshot_revalidates_manual_edits() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path()).unwrap();
    std::thread::scope(|scope| {
        for n in 0..8 {
            let store = store.clone();
            scope.spawn(move || {
                store
                    .update(|c| {
                        c.plugins
                            .insert(format!("test.{n}"), PluginConfig { enabled: false });
                        Ok(())
                    })
                    .unwrap();
            });
        }
    });
    assert_eq!(store.snapshot().unwrap().plugins.len(), 9);
    std::fs::write(dir.path().join("minerva.conf.json"), r#"{"version":2}"#).unwrap();
    assert!(store.snapshot().is_err());
    assert!(store.update(|_| Ok(())).is_err());
}

#[test]
fn missing_configured_package_is_a_roster_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path()).unwrap();
    store
        .update(|c| {
            c.plugins
                .insert("test.missing".into(), PluginConfig { enabled: true });
            Ok(())
        })
        .unwrap();
    let host = minerva_plugin_host::Host::open(dir.path().to_owned(), true).unwrap();
    let roster = host.roster();
    assert_eq!(roster.len(), 1);
    assert!(roster[0].requested_enabled);
    assert_eq!(roster[0].error.as_deref(), Some("plugin package missing"));
}

#[test]
fn rejects_all_system_disables_and_invalid_legacy_shell_before_migration() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path()).unwrap();
    for id in minerva_plugin_sdk::SYSTEM_PLUGIN_IDS {
        assert!(store
            .update(|c| {
                c.plugins
                    .insert((*id).into(), PluginConfig { enabled: false });
                Ok(())
            })
            .is_err());
    }
    let legacy = tempfile::tempdir().unwrap();
    std::fs::write(legacy.path().join("shell.json"), r#"{"timeoutMs":0}"#).unwrap();
    assert!(ConfigStore::open(legacy.path()).is_err());
    assert!(!legacy.path().join("minerva.conf.json").exists());
}
