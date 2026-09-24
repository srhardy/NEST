use std::fs;

use spoke_adapters::apply;
use spoke_core::ir::{Op, Plan};

fn plan_with(ops: Vec<Op>) -> Plan {
    let mut plan = Plan::default();
    for op in ops {
        plan.push(op);
    }
    plan
}

fn hook_entry(command: &str) -> serde_json::Value {
    serde_json::json!({ "hooks": [ { "type": "command", "command": command } ] })
}

#[test]
fn appends_hooks_without_touching_user_entries_and_is_idempotent() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let settings_path = project.path().join(".claude/settings.json");
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

    // A user hook that Spoke must never touch or reorder.
    let user_entry = hook_entry("/usr/local/bin/my-own-hook");
    let seeded = serde_json::json!({
        "model": "opus",
        "hooks": { "SessionStart": [ user_entry.clone() ] }
    });
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&seeded).unwrap(),
    )
    .unwrap();

    let spoke_entry = hook_entry("/opt/spoke event session_start");
    let op = Op::MergeJsonArrayAppend {
        path: settings_path.clone(),
        pointer: vec!["hooks".to_string(), "SessionStart".to_string()],
        value: serde_json::json!([spoke_entry.clone()]),
        why: "test".to_string(),
    };

    let changes =
        apply::apply(&plan_with(vec![op.clone()]), home.path(), project.path(), 1).unwrap();
    assert_eq!(changes.iter().filter(|c| c.changed).count(), 1);

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    let entries = parsed["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "user hook preserved, Spoke hook appended");
    assert_eq!(entries[0], user_entry, "user hook stays first");
    assert_eq!(parsed["model"], "opus", "unrelated keys preserved");

    // Second apply must be a no-op.
    let again = apply::apply(&plan_with(vec![op]), home.path(), project.path(), 2).unwrap();
    assert_eq!(again.iter().filter(|c| c.changed).count(), 0, "idempotent");

    // Removal by marker deletes only Spoke's entry.
    let remove = Op::RemoveJsonArrayItems {
        path: settings_path.clone(),
        pointer: vec!["hooks".to_string(), "SessionStart".to_string()],
        marker: "/opt/spoke event".to_string(),
        why: "test removal".to_string(),
    };
    apply::apply(&plan_with(vec![remove]), home.path(), project.path(), 3).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    let entries = parsed["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], user_entry, "only Spoke's entry was removed");
    assert_eq!(parsed["model"], "opus");
}

#[test]
fn failed_mid_plan_rolls_back_files_and_preserves_backup() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let original = project.path().join("first.txt");
    fs::write(&original, "before").unwrap();
    let conflict = project.path().join("blocked");
    fs::write(&conflict, "not a directory").unwrap();
    let plan = plan_with(vec![
        Op::WriteBlock {
            path: original.clone(),
            id: "test".into(),
            contents: "after".into(),
            why: "first write".into(),
        },
        Op::WriteFiles {
            root: conflict.clone(),
            name: "target".into(),
            files: vec![("file.txt".into(), "never writes".into())],
            why: "forced failure".into(),
        },
    ]);

    assert!(apply::apply(&plan, home.path(), project.path(), 123).is_err());
    assert_eq!(fs::read_to_string(&original).unwrap(), "before");
    assert_eq!(fs::read_to_string(&conflict).unwrap(), "not a directory");
    let backup = home.path().join(".spoke/backups/123/first.txt");
    assert_eq!(fs::read_to_string(backup).unwrap(), "before");
}

#[test]
fn failed_transaction_commit_rolls_back_and_keeps_old_lockfile() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let target = project.path().join("managed.txt");
    fs::write(&target, "before").unwrap();
    let plan = plan_with(vec![Op::WriteBlock {
        path: target.clone(),
        id: "test".into(),
        contents: "after".into(),
        why: "write before metadata commit".into(),
    }]);

    let result = apply::apply_transaction(&plan, home.path(), project.path(), 456, || {
        anyhow::bail!("injected lock commit failure")
    });
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "before");
    assert!(!project.path().join("agent.lock").exists());
    assert_eq!(
        fs::read_to_string(home.path().join(".spoke/backups/456/managed.txt")).unwrap(),
        "before"
    );
}

#[test]
fn removal_is_safe_when_the_path_is_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let settings_path = project.path().join("settings.json");
    fs::write(&settings_path, "{\n  \"env\": { \"A\": \"1\" }\n}\n").unwrap();

    let remove = Op::RemoveJsonArrayItems {
        path: settings_path.clone(),
        pointer: vec!["hooks".to_string(), "SessionStart".to_string()],
        marker: "spoke".to_string(),
        why: "no-op".to_string(),
    };
    let changes = apply::apply(&plan_with(vec![remove]), home.path(), project.path(), 1).unwrap();
    assert_eq!(changes.iter().filter(|c| c.changed).count(), 0);
    let content = fs::read_to_string(&settings_path).unwrap();
    assert!(content.contains("\"A\""));
}
