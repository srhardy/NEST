use std::fs;

use spoke_adapters::{apply, find, Ctx};
use spoke_core::manifest::{Automation, Bot};
use spoke_core::{Manifest, McpServer};

fn manifest_with_mcp() -> Manifest {
    let mut manifest = Manifest::default();
    manifest
        .instructions
        .extra
        .push("Prefer small diffs.".to_string());
    manifest.mcp.push(McpServer {
        name: "spoke-fetch".to_string(),
        command: Some("spoke".to_string()),
        args: vec!["mcp".to_string(), "fetch".to_string()],
        url: None,
    });
    manifest
}

#[test]
fn claude_projection_preserves_user_content_and_is_idempotent() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    fs::create_dir_all(home.path().join(".claude")).expect("claude dir");
    fs::write(project.path().join("CLAUDE.md"), "# My notes\nkeep me\n").expect("seed");

    let manifest = manifest_with_mcp();
    let harness = find("claude-code").expect("adapter");
    let ctx = Ctx::new(home.path(), project.path(), &manifest);
    let plan = harness.plan(&ctx);
    let changes = apply::apply(&plan, home.path(), project.path(), 1).expect("apply");
    assert!(changes.iter().any(|c| c.changed));

    let claude_md = fs::read_to_string(project.path().join("CLAUDE.md")).expect("read");
    assert!(claude_md.contains("keep me"), "user content preserved");
    assert!(claude_md.contains("<!-- spoke:begin instructions -->"));
    assert!(claude_md.contains("@AGENTS.md"), "bridges to AGENTS.md");
    assert!(claude_md.contains("Prefer small diffs."));

    let mcp: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(project.path().join(".mcp.json")).expect("read"))
            .expect("json");
    assert_eq!(mcp["mcpServers"]["spoke-fetch"]["command"], "spoke");
    assert_eq!(mcp["mcpServers"]["spoke-fetch"]["type"], "stdio");

    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.path().join(".claude/settings.json")).expect("read"),
    )
    .expect("json");
    assert_eq!(settings["env"]["DO_NOT_TRACK"], "1");

    let second = apply::apply(&plan, home.path(), project.path(), 2).expect("second apply");
    assert!(
        second.iter().all(|c| !c.changed),
        "second apply must change nothing"
    );
}

#[test]
fn claude_settings_merge_keeps_existing_env_entries() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    fs::create_dir_all(project.path().join(".claude")).expect("dir");
    fs::write(
        project.path().join(".claude/settings.json"),
        "{\n  \"env\": {\n    \"MY_FLAG\": \"keep\"\n  },\n  \"model\": \"opus\"\n}\n",
    )
    .expect("seed");

    let manifest = Manifest::default();
    let harness = find("claude-code").expect("adapter");
    let plan = harness.plan(&Ctx::new(home.path(), project.path(), &manifest));
    apply::apply(&plan, home.path(), project.path(), 1).expect("apply");

    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.path().join(".claude/settings.json")).expect("read"),
    )
    .expect("json");
    assert_eq!(settings["env"]["MY_FLAG"], "keep");
    assert_eq!(settings["env"]["HEADROOM_BEACON"], "off");
    assert_eq!(settings["model"], "opus");
}

#[test]
fn codex_projection_merges_toml_and_surfaces_capability_gap() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    fs::create_dir_all(home.path().join(".codex")).expect("codex dir");
    fs::create_dir_all(project.path().join(".codex")).expect("project codex dir");
    fs::write(
        project.path().join(".codex/config.toml"),
        "model = \"gpt-5\"\n# keep this comment\n",
    )
    .expect("seed");

    let manifest = manifest_with_mcp();
    let harness = find("codex").expect("adapter");
    let ctx = Ctx::new(home.path(), project.path(), &manifest);
    let plan = harness.plan(&ctx);
    assert!(
        plan.warnings.iter().any(|w| w.contains("telemetry-off")),
        "capability gap must be surfaced, not dropped: {:?}",
        plan.warnings
    );

    apply::apply(&plan, home.path(), project.path(), 1).expect("apply");

    let config = fs::read_to_string(project.path().join(".codex/config.toml")).expect("read");
    assert!(config.contains("model = \"gpt-5\""), "existing keys kept");
    assert!(config.contains("# keep this comment"), "comments kept");
    assert!(config.contains("[mcp_servers.spoke-fetch]"));
    assert!(config.contains("command = \"spoke\""));

    let agents = fs::read_to_string(project.path().join("AGENTS.md")).expect("read");
    assert!(agents.contains("<!-- spoke:begin instructions -->"));
    assert!(agents.contains("Prefer small diffs."));
}

#[test]
fn nest_projection_writes_agents_md_and_mcp_idempotently() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    fs::write(project.path().join("AGENTS.md"), "# My notes\nkeep me\n").expect("seed");

    // Detection is directory-based: absent until ~/.nest exists.
    let manifest = manifest_with_mcp();
    let harness = find("nest").expect("adapter");
    assert!(!harness.detect(home.path()));
    fs::create_dir_all(home.path().join(".nest")).expect("cc dir");
    assert!(harness.detect(home.path()));
    assert_eq!(
        harness.skills_root(home.path()),
        home.path().join(".nest/skills")
    );

    let ctx = Ctx::new(home.path(), project.path(), &manifest);
    let plan = harness.plan(&ctx);
    assert!(
        plan.warnings.iter().any(|w| w.contains("not projected")),
        "unprojected surfaces must be warned: {:?}",
        plan.warnings
    );
    let changes = apply::apply(&plan, home.path(), project.path(), 1).expect("apply");
    assert!(changes.iter().any(|c| c.changed));

    let agents = fs::read_to_string(project.path().join("AGENTS.md")).expect("read");
    assert!(agents.contains("keep me"), "user content preserved");
    assert!(agents.contains("<!-- spoke:begin instructions -->"));
    assert!(agents.contains("Prefer small diffs."));

    let mcp: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.path().join(".nest/mcp.json")).expect("read"),
    )
    .expect("json");
    // Nest accepts the Claude-shaped type/transport = stdio|http|sse.
    assert_eq!(mcp["mcpServers"]["spoke-fetch"]["command"], "spoke");
    assert_eq!(mcp["mcpServers"]["spoke-fetch"]["type"], "stdio");

    let second = apply::apply(&plan, home.path(), project.path(), 2).expect("second apply");
    assert!(
        second.iter().all(|c| !c.changed),
        "second apply must change nothing"
    );
}

#[test]
fn nest_projection_renders_bots_and_automations() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    fs::create_dir_all(home.path().join(".nest")).expect("nest dir");

    let mut manifest = manifest_with_mcp();
    manifest.bots.push(Bot {
        name: "eggbot".to_string(),
        title: Some("Dr. Eggbot".to_string()),
        job: "Walks you through building a bot, then writes it".to_string(),
        voice: vec!["You are a friendly bot-builder.".to_string()],
        anti_jobs: vec!["Do not write application code".to_string()],
        skills: vec!["bot-authoring".to_string()],
        mcp: vec!["spoke-fetch".to_string(), "ghost".to_string()],
        model: Some("balanced".to_string()),
        memory: vec!["Templates live in the catalog".to_string()],
    });
    manifest.automations.push(Automation {
        name: "morning-triage".to_string(),
        schedule: Some("0 9 * * *".to_string()),
        event: None,
        prompt: "Summarize yesterday's changes".to_string(),
        bot: Some("eggbot".to_string()),
        enabled: true,
    });
    manifest.validate().expect("manifest valid");

    let harness = find("nest").expect("adapter");
    let plan = harness.plan(&Ctx::new(home.path(), project.path(), &manifest));
    // Dangling refs warn, never drop: 'bot-authoring' skill and 'ghost' mcp
    // are not declared in the manifest.
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("bot-authoring") && w.contains("eggbot")),
        "dangling skill ref must warn: {:?}",
        plan.warnings
    );
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("ghost") && w.contains("eggbot")),
        "dangling mcp ref must warn: {:?}",
        plan.warnings
    );

    apply::apply(&plan, home.path(), project.path(), 1).expect("apply");

    let agent =
        fs::read_to_string(project.path().join(".nest/agents/eggbot.md")).expect("read agent");
    assert!(agent.contains("name: eggbot"));
    assert!(agent.contains("title: \"Dr. Eggbot\""));
    assert!(
        agent.contains("job: \"Walks you through building a bot, then writes it\""),
        "job in frontmatter: {agent}"
    );
    assert!(agent.contains("model: \"balanced\""));
    assert!(agent.contains("skills: [\"bot-authoring\"]"));
    assert!(agent.contains("mcp: [\"spoke-fetch\", \"ghost\"]"));
    assert!(agent.contains("# Dr. Eggbot"));
    assert!(agent.contains("You are a friendly bot-builder."));
    assert!(agent.contains("## Not my job"));
    assert!(agent.contains("- Do not write application code"));
    assert!(agent.contains("## Memory"));
    assert!(agent.contains("edit the manifest, not this file"));

    let automations = fs::read_to_string(project.path().join(".nest/automations.toml"))
        .expect("read automations");
    assert!(automations.contains("[[automation]]"));
    assert!(automations.contains("name = \"morning-triage\""));
    assert!(automations.contains("schedule = \"0 9 * * *\""));
    assert!(automations.contains("prompt = \"Summarize yesterday's changes\""));
    assert!(automations.contains("bot = \"eggbot\""));
    assert!(automations.contains("enabled = true"));

    let second = apply::apply(&plan, home.path(), project.path(), 2).expect("second apply");
    assert!(
        second.iter().all(|c| !c.changed),
        "second apply must change nothing"
    );
}

#[test]
fn manifest_validation_rejects_bad_bots_and_automations() {
    let mut manifest = Manifest::default();
    manifest.bots.push(Bot {
        name: "Bad Name".to_string(),
        title: None,
        job: "x".to_string(),
        voice: vec![],
        anti_jobs: vec![],
        skills: vec![],
        mcp: vec![],
        model: None,
        memory: vec![],
    });
    assert!(manifest.validate().is_err(), "uppercase name must fail");

    manifest.bots[0].name = "ok-name".to_string();
    manifest.bots[0].job = String::new();
    assert!(manifest.validate().is_err(), "empty job must fail");

    manifest.bots[0].job = "one job".to_string();
    manifest.bots.push(manifest.bots[0].clone());
    assert!(manifest.validate().is_err(), "duplicate bot name must fail");

    manifest.bots.truncate(1);
    manifest.automations.push(Automation {
        name: "a".to_string(),
        schedule: Some("0 9 * * *".to_string()),
        event: Some("file-change:src/**".to_string()),
        prompt: String::new(),
        bot: None,
        enabled: true,
    });
    assert!(
        manifest.validate().is_err(),
        "schedule XOR event must be enforced"
    );

    manifest.automations[0].event = None;
    manifest.automations[0].schedule = Some("daily".to_string());
    assert!(manifest.validate().is_err(), "non-cron schedule must fail");

    manifest.automations[0].schedule = Some("0 9 * * *".to_string());
    manifest.validate().expect("fixed manifest is valid");
}
