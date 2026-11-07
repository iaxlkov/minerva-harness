use super::*;
use crate::sessions::SessionMessage;
use serde_json::{json, Map};
use std::fs;

fn message(id: &str, role: &str, text: &str) -> SessionMessage {
    SessionMessage {
        id: id.into(),
        parent_id: None,
        role: role.into(),
        created_at: "1".into(),
        parts: vec![crate::transcript::ContentPart::text("text", text)],
        extensions: Map::new(),
    }
}

#[test]
fn settings_are_atomic_scoped_and_survive_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("context.sqlite3");
    let service = ContextService::open(&path).unwrap();
    assert_eq!(service.workspace_get("one").unwrap().cwd, None);
    let settings = WorkspaceSettings {
        cwd: Some(dir.path().to_str().unwrap().into()),
        ..Default::default()
    };
    service.workspace_save("one", settings.clone()).unwrap();
    let mut invalid = settings;
    invalid.context_window = 0;
    assert!(service.workspace_save("one", invalid).is_err());
    assert!(service.workspace_get("two").unwrap().cwd.is_none());
    drop(service);
    assert!(ContextService::open(&path)
        .unwrap()
        .workspace_get("one")
        .unwrap()
        .cwd
        .is_some());
}

#[test]
fn instructions_and_selected_skills_refresh_before_next_step() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".agents/skills/check")).unwrap();
    fs::write(dir.path().join("AGENTS.md"), "first instruction").unwrap();
    fs::write(
        dir.path().join(".agents/skills/check/SKILL.md"),
        "---\nname: check\ndescription: Runs checks\n---\nsecret skill body",
    )
    .unwrap();
    let service = ContextService::open(&dir.path().join("state.sqlite3")).unwrap();
    service
        .workspace_save(
            "one",
            WorkspaceSettings {
                cwd: Some(dir.path().to_str().unwrap().into()),
                ..Default::default()
            },
        )
        .unwrap();
    let prepared = service.prepare("one", &[]).unwrap();
    assert!(prepared.system_prompt.contains("first instruction"));
    assert_eq!(prepared.skills[0].name, "check");
    assert!(!prepared.system_prompt.contains("secret skill body"));
    assert!(service
        .load_skill("one", "check")
        .unwrap()
        .contains("secret skill body"));
    fs::write(dir.path().join("AGENTS.md"), "second instruction").unwrap();
    assert!(service
        .prepare("one", &[])
        .unwrap()
        .system_prompt
        .contains("second instruction"));
    assert!(service
        .prepare("one", &[])
        .unwrap()
        .system_prompt
        .contains("secret skill body"));
    assert!(service.load_skill("one", "../check").is_err());
}

#[test]
fn summary_is_anchored_and_never_changes_history() {
    let dir = tempfile::tempdir().unwrap();
    let service = ContextService::open(&dir.path().join("state.sqlite3")).unwrap();
    let mut history = vec![
        message("u1", "user", "old question"),
        message("a1", "assistant", "old answer"),
        message("u2", "user", "recent question"),
        message("a2", "assistant", "recent answer"),
    ];
    let before = serde_json::to_string(&history).unwrap();
    let prefix = compact_prefix(&history, 2);
    assert_eq!(prefix.len(), 2);
    service
        .save_summary("one", &prefix, "old conversation summary")
        .unwrap();
    let prepared = service.prepare("one", &history).unwrap();
    assert_eq!(prepared.turns.len(), 2);
    assert!(prepared.system_prompt.contains("old conversation summary"));
    assert_eq!(serde_json::to_string(&history).unwrap(), before);
    history[0].parts[0].data = json!({"text":"branch changed"});
    assert_eq!(service.prepare("one", &history).unwrap().turns.len(), 4);
}

#[test]
fn attachment_is_decoded_durable_private_and_not_a_path() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let service = ContextService::open(&path).unwrap();
    let mut data = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut data, image::ImageFormat::Png)
        .unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(data.into_inner());
    let reference = service
        .put_attachment("one", "picture.png", "image/png", &encoded)
        .unwrap();
    assert!(service
        .put_attachment("one", "bad.png", "image/jpeg", &encoded)
        .is_err());
    assert!(service
        .put_attachment("one", "bad.png", "image/png", "dGV4dA==")
        .is_err());
    assert!(service.get_attachment("two", &reference.id).is_err());
    assert!(service.get_attachment("one", "../../secret").is_err());
    drop(service);
    let reloaded = ContextService::open(&path)
        .unwrap()
        .get_attachment("one", &reference.id)
        .unwrap();
    assert_eq!(reloaded.data, encoded);
    assert!(reloaded.bytes > 0);
}

#[test]
fn tool_outputs_are_pruned_in_projection_only() {
    let dir = tempfile::tempdir().unwrap();
    let service = ContextService::open(&dir.path().join("state.sqlite3")).unwrap();
    let mut history = vec![message("a1", "assistant", "")];
    history[0].parts = vec![crate::transcript::ContentPart {
        kind: "tool-result".into(),
        data: json!({"callId":"tool1","output":"x".repeat(50000),"isError":false}),
    }];
    let prepared = service.prepare("one", &history).unwrap();
    assert!(
        prepared.turns[0].parts[0].data["output"]
            .as_str()
            .unwrap()
            .len()
            < 10000
    );
    assert_eq!(prepared.turns[0].parts[0].data["callId"], "tool1");
    assert_eq!(
        history[0].parts[0].data["output"].as_str().unwrap().len(),
        50000
    );
}

#[test]
fn native_replay_pruning_preserves_wire_call_identity() {
    let mut replay = json!([{"role":"user","content":[{"type":"toolresult","call":{"provider":"call1"},"name":"read","content":[{"type":"text","text":"x".repeat(50000)}]}]}]);
    prune_replay(&mut replay);
    let block = &replay[0]["content"][0];
    assert_eq!(block["call"], json!({"provider":"call1"}));
    assert!(block["content"].to_string().len() < 1000);
    let _: Vec<rig_core::message::Message> = serde_json::from_value(replay).unwrap();
}

#[test]
fn incomplete_tool_pair_is_not_compacted() {
    let mut history = vec![
        message("u1", "user", "one"),
        message("a1", "assistant", ""),
        message("u2", "user", "two"),
        message("a2", "assistant", "answer"),
    ];
    history[1].parts = vec![crate::transcript::ContentPart {
        kind: "tool-call".into(),
        data: json!({"callId":"pending","name":"read","arguments":{}}),
    }];
    assert!(compact_prefix(&history, 2).is_empty());
}

#[test]
fn running_or_interrupted_assistant_is_not_compacted() {
    for status in ["running", "interrupted", "failed", "cancelled"] {
        let mut history = vec![
            message("u1", "user", "one"),
            message("a1", "assistant", "partial"),
            message("u2", "user", "two"),
            message("a2", "assistant", "answer"),
        ];
        history[1]
            .extensions
            .insert("execution".into(), json!({"status":status}));
        assert!(compact_prefix(&history, 2).is_empty(), "status {status}");
    }
}

#[test]
fn budget_counts_provider_replay_and_respects_auto_compact() {
    let dir = tempfile::tempdir().unwrap();
    let service = ContextService::open(&dir.path().join("state.sqlite3")).unwrap();
    service
        .workspace_save(
            "one",
            WorkspaceSettings {
                context_window: 1024,
                ..Default::default()
            },
        )
        .unwrap();
    let mut history = vec![message("a1", "assistant", "small display")];
    history[0].extensions.insert("replay".into(),json!({"rig-v1":[{"role":"assistant","content":[{"type":"text","text":"x".repeat(6000)}]}]}));
    let prepared = service.prepare("one", &history).unwrap();
    assert!(prepared.budget.estimated_tokens > 1500);
    assert!(prepared.budget.should_compact);
    assert_eq!(prepared.budget.available_tokens, 0);
    service
        .workspace_save(
            "one",
            WorkspaceSettings {
                context_window: 1024,
                auto_compact: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        !service
            .prepare("one", &history)
            .unwrap()
            .budget
            .should_compact
    );
}

#[test]
fn project_instruction_chain_keeps_parent_then_child_order() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::create_dir_all(dir.path().join("child")).unwrap();
    fs::write(dir.path().join("AGENTS.md"), "parent-policy").unwrap();
    fs::write(dir.path().join("child/CLAUDE.md"), "child-policy").unwrap();
    let instructions = files::instructions(&dir.path().join("child")).unwrap();
    assert!(
        instructions.find("parent-policy").unwrap() < instructions.find("child-policy").unwrap()
    );
}

#[test]
fn catalogue_reads_folded_skill_descriptions_without_loading_body() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".agents/skills/check")).unwrap();
    fs::write(dir.path().join(".agents/skills/check/SKILL.md"),"---\r\nname: check\r\ndescription: >\r\n  Run the checks\r\n  before finishing.\r\nlicense: MIT\r\n---\r\nPrivate body").unwrap();
    let skills = files::discover_skills(dir.path()).unwrap();
    assert_eq!(
        skills[0].info.description,
        "Run the checks before finishing."
    );
}

#[cfg(windows)]
#[test]
fn skill_symlink_outside_project_is_denied_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("SKILL.md"), "outside secret").unwrap();
    fs::create_dir_all(dir.path().join(".agents/skills")).unwrap();
    // Junctions do not require the Windows symbolic-link privilege.
    let link = dir.path().join(".agents/skills/outside");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link.to_str().unwrap().replace('/', "\\"))
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(files::discover_skills(dir.path()).is_err());
}

#[test]
fn oversized_instruction_is_rejected_instead_of_unbounded_read() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("AGENTS.md"),
        "x".repeat(files::MAX_SOURCE + 1),
    )
    .unwrap();
    let service = ContextService::open(&dir.path().join("state.sqlite3")).unwrap();
    service
        .workspace_save(
            "one",
            WorkspaceSettings {
                cwd: Some(dir.path().to_str().unwrap().into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(service.prepare("one", &[]).is_err());
}

#[cfg(unix)]
#[test]
fn skill_symlink_outside_project_is_denied() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("SKILL.md"), "outside secret").unwrap();
    fs::create_dir_all(dir.path().join(".agents/skills")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(".agents/skills/outside")).unwrap();
    assert!(files::discover_skills(dir.path()).is_err());
}
