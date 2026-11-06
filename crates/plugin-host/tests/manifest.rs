use minerva_plugin_sdk::Manifest;
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
    for id in ["example", "shell", "providers", "codex", "plugin-manager"] {
        let manifest:Manifest=serde_json::from_value(serde_json::json!({"id":id,"version":"1","apiVersion":1,"backend":{"kind":"process","entry":"bin/plugin"}})).unwrap();
        assert!(manifest.validate().is_err(), "accepted builtin id {id}");
    }
}
