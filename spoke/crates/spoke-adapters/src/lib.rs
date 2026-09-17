pub mod apply;
pub mod claude_code;
pub mod codex;
pub mod nest;

use std::path::{Path, PathBuf};

use spoke_core::ir::{Capabilities, Op, Plan};
use spoke_core::manifest::Manifest;

/// Everything an adapter needs to render a projection.
pub struct Ctx<'a> {
    pub home: &'a Path,
    pub project: &'a Path,
    pub manifest: &'a Manifest,
}

impl<'a> Ctx<'a> {
    pub fn new(home: &'a Path, project: &'a Path, manifest: &'a Manifest) -> Self {
        Self {
            home,
            project,
            manifest,
        }
    }

    /// Instruction lines from the manifest, one per line, with a trailing newline.
    pub fn instruction_lines(&self) -> String {
        let mut body = String::new();
        for line in &self.manifest.instructions.extra {
            body.push_str(line);
            body.push('\n');
        }
        body
    }
}

/// A coding-agent harness Spoke can project into.
pub trait Harness {
    fn id(&self) -> &'static str;
    fn display(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    /// User-level config root (used for detection and backups).
    fn root(&self, home: &Path) -> PathBuf;
    fn detect(&self, home: &Path) -> bool {
        self.root(home).exists()
    }
    /// Where this harness loads skills from (SKILL.md directories).
    fn skills_root(&self, home: &Path) -> PathBuf {
        self.root(home).join("skills")
    }
    /// Where this harness loads agents from (agent .md directories),
    /// e.g. `~/.nest/agents` for packages that ship bot definitions.
    fn agents_root(&self, home: &Path) -> PathBuf {
        self.root(home).join("agents")
    }
    fn plan(&self, ctx: &Ctx<'_>) -> Plan;
}

/// Install a skill directory into a harness's skills root.
pub fn skill_install_op(
    harness: &dyn Harness,
    home: &Path,
    skill: &str,
    files: Vec<(String, String)>,
    source: &str,
) -> Op {
    Op::WriteFiles {
        root: harness.skills_root(home),
        name: skill.to_string(),
        files,
        why: format!("install skill '{skill}' from {source}"),
    }
}

/// Remove an installed skill directory.
pub fn skill_remove_op(harness: &dyn Harness, home: &Path, skill: &str) -> Op {
    Op::DeleteDir {
        path: harness.skills_root(home).join(skill),
        why: format!("remove skill '{skill}'"),
    }
}

/// Install an agent directory (bot definition files) into a harness's agents root.
pub fn agent_install_op(
    harness: &dyn Harness,
    home: &Path,
    agent: &str,
    files: Vec<(String, String)>,
    source: &str,
) -> Op {
    Op::WriteFiles {
        root: harness.agents_root(home),
        name: agent.to_string(),
        files,
        why: format!("install agent '{agent}' from {source}"),
    }
}

pub fn registry() -> Vec<Box<dyn Harness>> {
    vec![
        Box::new(claude_code::ClaudeCode),
        Box::new(codex::Codex),
        Box::new(nest::Nest),
    ]
}

pub fn known_ids() -> Vec<&'static str> {
    registry().iter().map(|h| h.id()).collect()
}

pub fn find(id: &str) -> Option<Box<dyn Harness>> {
    registry().into_iter().find(|h| h.id() == id)
}

/// MCP server as a Claude Code `.mcp.json` entry.
pub(crate) fn claude_mcp_entry(server: &spoke_core::manifest::McpServer) -> serde_json::Value {
    match (&server.command, &server.url) {
        (Some(command), _) => serde_json::json!({
            "type": "stdio",
            "command": command,
            "args": server.args,
        }),
        (None, Some(url)) => serde_json::json!({
            "type": "http",
            "url": url,
        }),
        (None, None) => serde_json::Value::Null,
    }
}

/// MCP server as a Codex `config.toml` table body (headers omitted).
pub(crate) fn codex_mcp_table(server: &spoke_core::manifest::McpServer) -> String {
    match (&server.command, &server.url) {
        (Some(command), _) => {
            let args = server
                .args
                .iter()
                .map(|a| toml_string(a))
                .collect::<Vec<_>>()
                .join(", ");
            format!("command = {}\nargs = [{}]", toml_string(command), args)
        }
        (None, Some(url)) => format!("url = {}", toml_string(url)),
        (None, None) => String::new(),
    }
}

pub(crate) fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

/// Telemetry beacons Spoke disables by default, honouring the complaints gathered in research.
pub const TELEMETRY_ENV: &[(&str, &str)] = &[
    ("DO_NOT_TRACK", "1"),
    ("SUPERPOWERS_DISABLE_TELEMETRY", "1"),
    ("HEADROOM_BEACON", "off"),
    ("SCARF_NO_ANALYTICS", "1"),
    ("RTK_TELEMETRY_DISABLED", "1"),
];
