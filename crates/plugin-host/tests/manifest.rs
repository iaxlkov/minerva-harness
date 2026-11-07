use minerva_plugin_sdk::Manifest;
#[test]
fn ui_requires_native_module_sdk() {
    let mut manifest: Manifest = serde_json::from_value(serde_json::json!({
        "id":"test.ui", "version":"1", "apiVersion":1,
        "ui":{"entry":"ui/client.js", "sdkVersion":2}
    }))
    .unwrap();
    assert!(manifest.validate().is_ok());
    manifest.ui.as_mut().unwrap().sdk_version = 1;
    assert!(manifest.validate().is_err());
}
#[test]
fn rejects_version_and_traversal() {
    let mut m: Manifest = serde_json::from_str(r#"{"id":"test.example","version":"1.0.0","apiVersion":2,"backend":{"kind":"process","entry":"bin/example"}}"#).unwrap();
    assert!(m.validate().is_err());
    m.api_version = 1;
    m.backend.as_mut().unwrap().entry = "../escape".into();
    assert!(m.validate().is_err());
}
#[test]
fn builtin_ids_are_reserved() {
    for id in ["example", "shell", "providers", "plugin-manager"] {
        let manifest:Manifest=serde_json::from_value(serde_json::json!({"id":id,"version":"1","apiVersion":1,"backend":{"kind":"process","entry":"bin/plugin"}})).unwrap();
        assert!(manifest.validate().is_err(), "accepted builtin id {id}");
    }
}
