use spoke_core::{Manifest, McpServer};

#[test]
fn template_parses_and_has_defaults() {
    let manifest: Manifest = toml::from_str(Manifest::template()).expect("template must parse");
    assert_eq!(manifest.version, 1);
    assert!(manifest.enabled("claude-code"));
    assert!(manifest.enabled("codex"));
    assert_eq!(manifest.budget.preset, "balanced");
    assert!(manifest.mcp.is_empty());
    assert!(manifest.skills.is_empty());
}

/// Guards against TOML table-nesting bugs: every documented key must live at the level
/// the Manifest schema expects, not accidentally inside a preceding `[table]`.
#[test]
fn template_keys_live_at_top_level() {
    let raw: toml::Value = toml::from_str(Manifest::template()).expect("template must parse");
    let table = raw.as_table().expect("root table");

    for key in ["version", "harnesses", "skills", "instructions", "budget"] {
        assert!(
            table.contains_key(key),
            "'{key}' must be a top-level key in the template"
        );
    }
    let instructions = table["instructions"]
        .as_table()
        .expect("instructions table");
    assert!(
        instructions.contains_key("extra") && !instructions.contains_key("skills"),
        "skills must not be nested inside [instructions]"
    );
}

#[test]
fn save_then_load_roundtrips() {
    let dir = tempfile::tempdir().expect("tempdir");
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

    manifest.save(dir.path()).expect("save");
    let loaded = Manifest::load(dir.path()).expect("load");
    assert_eq!(manifest, loaded);
}

#[test]
fn mcp_server_requires_exactly_one_transport() {
    let both = McpServer {
        name: "x".to_string(),
        command: Some("cmd".to_string()),
        args: vec![],
        url: Some("https://example.com".to_string()),
    };
    assert!(both.validate().is_err());

    let neither = McpServer {
        name: "x".to_string(),
        command: None,
        args: vec![],
        url: None,
    };
    assert!(neither.validate().is_err());

    let stdio = McpServer {
        name: "x".to_string(),
        command: Some("cmd".to_string()),
        args: vec![],
        url: None,
    };
    assert!(stdio.validate().is_ok());
}
