//! Perches — one wrapper per agent CLI.
//!
//! A perch knows three things about its harness: how to spot it
//! (`detect` — is the binary on PATH), how to launch it with a bot's
//! persona + task (`launch`), and how to translate its output lines into
//! peeps (`parse`). Wrapping, not rebuilding: the harness is the engine.

use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

use spoke_core::manifest::ToolPolicy;

use crate::peep::Peep;
use crate::spec::AgentSpec;

/// A command ready to spawn.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    /// Extra environment for the child (prompt env vars, feature flags).
    pub env: BTreeMap<String, String>,
}

/// A harness wrapper. Implement `launch` + `parse`; `binary` is used by
/// `detect` (return `""` for always-available builtin perches).
/// `Send + Sync` is required: hatches spawn on background threads and the
/// driver parses stdout/stderr on reader threads.
pub trait Perch: Send + Sync {
    /// Stable id, e.g. `"claude-code"`, `"echo"`.
    fn name(&self) -> &'static str;
    /// Binary looked up on PATH by `detect`. `""` = builtin, always present.
    fn binary(&self) -> &'static str;
    /// Is this harness installed?
    fn detect(&self) -> bool {
        let bin = self.binary();
        if bin.is_empty() {
            return true;
        }
        which(bin).is_some()
    }
    /// Build the spawn command for a bot + task.
    fn launch(&self, agent: &AgentSpec, task: &str) -> Launch;
    /// One output line → one peep. `None` skips the line.
    fn parse(&self, line: &str) -> Option<Peep>;
    /// The one-click install command (run via `sh -c`), when this perch
    /// can install itself. Builtin stubs return `None`.
    fn install_cmd(&self) -> Option<&'static str> {
        None
    }
}

/// PATH lookup — is `bin` an *executable* file somewhere on PATH?
fn which(bin: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .find_map(|dir| candidates(&dir, bin).into_iter().find(|c| is_executable(c)))
    })
}

/// What `bin` could be named in `dir`. On Windows a bare name resolves via
/// `PATHEXT` suffixes (.COM/.EXE/.BAT/.CMD…); elsewhere it's just `bin`.
fn candidates(dir: &std::path::Path, bin: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        if std::path::Path::new(bin).extension().is_some() {
            return vec![dir.join(bin)];
        }
        let exts = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        exts.split(';')
            .filter(|e| !e.is_empty())
            .map(|e| dir.join(format!("{bin}{e}")))
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![dir.join(bin)]
    }
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Public exec-bit check for provider availability (same rule as `which`).
pub fn is_exec_pub(path: &std::path::Path) -> bool {
    is_executable(path)
}

/// The prompt every perch feeds its harness: persona + tool policy + task.
pub fn build_prompt(agent: &AgentSpec, task: &str) -> String {
    let mut prompt = String::new();
    prompt.push_str(agent.body.trim());
    if !agent.permissions.is_empty() {
        prompt.push_str("\n\n## Tool policy\n\n");
        for (tool, policy) in &agent.permissions {
            let word = match policy {
                ToolPolicy::Allow => "allow",
                ToolPolicy::Ask => "ask-first",
                ToolPolicy::Deny => "never",
            };
            prompt.push_str(&format!("- {tool}: {word}\n"));
        }
    }
    prompt.push_str(&format!("\n\n## Task\n\n{task}\n"));
    prompt
}

/// Tool lists for harnesses that take allow/deny flags.
/// `ask` can't prompt in a headless run, so it joins the deny list —
/// the run reports what it couldn't touch instead of pretending.
fn tool_lists(agent: &AgentSpec) -> (Vec<String>, Vec<String>) {
    let mut allow = Vec::new();
    let mut deny = Vec::new();
    for (tool, policy) in &agent.permissions {
        match policy {
            ToolPolicy::Allow => allow.push(tool.clone()),
            ToolPolicy::Ask | ToolPolicy::Deny => deny.push(tool.clone()),
        }
    }
    (allow, deny)
}

// ---------------------------------------------------------------------------
// Preset perches
// ---------------------------------------------------------------------------

/// `claude` (Claude Code CLI): `claude -p <prompt> --output-format
/// stream-json --verbose` — one JSON event per line.
pub struct ClaudeCode;
impl Perch for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude-code"
    }
    fn binary(&self) -> &'static str {
        "claude"
    }
    fn launch(&self, agent: &AgentSpec, task: &str) -> Launch {
        let (allow, deny) = tool_lists(agent);
        let mut args = vec![
            "-p".into(),
            build_prompt(agent, task),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
        ];
        if !allow.is_empty() {
            args.push("--allowedTools".into());
            args.push(allow.join(","));
        }
        if !deny.is_empty() {
            args.push("--disallowedTools".into());
            args.push(deny.join(","));
        }
        Launch {
            program: "claude".into(),
            args,
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => Some(Peep::Step {
                n: 0,
                label: "assistant".into(),
            }),
            Some("result") => {
                let text = v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or_default()
                    .to_string();
                (!text.is_empty()).then_some(Peep::Output { text })
            }
            _ => None,
        }
    }
    fn install_cmd(&self) -> Option<&'static str> {
        Some("npm i -g @anthropic-ai/claude-code")
    }
}

/// `codex` (Codex CLI): `codex exec --json <prompt>` — JSONL turn events.
pub struct Codex;
impl Perch for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }
    fn binary(&self) -> &'static str {
        "codex"
    }
    fn launch(&self, agent: &AgentSpec, task: &str) -> Launch {
        Launch {
            program: "codex".into(),
            args: vec!["exec".into(), "--json".into(), build_prompt(agent, task)],
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let kind = v.get("type").or_else(|| v.get("msg").map(|m| &m["type"]))?;
        match kind.as_str()? {
            "item.completed" | "exec_command_begin" => Some(Peep::Step {
                n: 0,
                label: "item".into(),
            }),
            "turn.completed" | "turn.started" => None,
            _ => Some(Peep::Output { text: line.into() }),
        }
    }
    fn install_cmd(&self) -> Option<&'static str> {
        Some("npm i -g @openai/codex")
    }
}

/// `opencode` (OpenCode CLI): `opencode run <prompt>` — plain lines out.
pub struct OpenCode;
impl Perch for OpenCode {
    fn name(&self) -> &'static str {
        "opencode"
    }
    fn binary(&self) -> &'static str {
        "opencode"
    }
    fn launch(&self, agent: &AgentSpec, task: &str) -> Launch {
        Launch {
            program: "opencode".into(),
            args: vec!["run".into(), build_prompt(agent, task)],
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        (!line.trim().is_empty()).then(|| Peep::Output { text: line.into() })
    }
    fn install_cmd(&self) -> Option<&'static str> {
        Some("npm i -g opencode-ai")
    }
}

/// `echo` — builtin stub perch: prints the prompt and exits 0.
/// Proves the spine end-to-end on machines with no harness installed.
pub struct Echo;
impl Perch for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn binary(&self) -> &'static str {
        ""
    }
    fn launch(&self, agent: &AgentSpec, task: &str) -> Launch {
        let mut env = BTreeMap::new();
        env.insert("NEST_PROMPT".into(), build_prompt(agent, task));
        Launch {
            program: "sh".into(),
            args: vec!["-c".into(), "echo \"$NEST_PROMPT\"".into()],
            env,
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        Some(Peep::Output { text: line.into() })
    }
}

/// `fail` — builtin stub perch: exits 1 immediately. Exists so the
/// fallback ladder is testable without breaking anything real.
pub struct Fail;
impl Perch for Fail {
    fn name(&self) -> &'static str {
        "fail"
    }
    fn binary(&self) -> &'static str {
        ""
    }
    fn launch(&self, _agent: &AgentSpec, _task: &str) -> Launch {
        Launch {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "echo 'stub perch failing as designed' >&2; exit 1".into(),
            ],
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        Some(Peep::Output { text: line.into() })
    }
}

/// Every perch the engine knows about.
pub fn all_perches() -> Vec<Box<dyn Perch>> {
    vec![
        Box::new(ClaudeCode),
        Box::new(Codex),
        Box::new(OpenCode),
        Box::new(Echo),
        Box::new(Fail),
    ]
}

/// Look a perch up by name.
pub fn perch_named(name: &str) -> Option<Box<dyn Perch>> {
    all_perches().into_iter().find(|p| p.name() == name)
}

/// First detected real harness, else the echo stub (the spine always runs).
pub fn default_perch() -> Box<dyn Perch> {
    let all = all_perches();
    all.into_iter()
        .find(|p| p.binary() != "" && p.detect())
        .unwrap_or_else(|| Box::new(Echo))
}

/// One row of the engines table: name, installed?, and the one-click
/// install command when it has one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PerchInfo {
    pub name: &'static str,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<&'static str>,
}

/// Everything installed, for `nest perches` / the shell's engine cards.
pub fn detected_perches() -> Vec<PerchInfo> {
    all_perches()
        .iter()
        .map(|p| PerchInfo {
            name: p.name(),
            installed: p.detect(),
            install: p.install_cmd(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(perms: &[(&str, ToolPolicy)]) -> AgentSpec {
        AgentSpec {
            name: "t".into(),
            title: None,
            job: "job".into(),
            model: None,
            mode: None,
            boss: false,
            max_steps: None,
            permissions: perms.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            skills: vec![],
            mcp: vec![],
            body: "You are a test bot.".into(),
        }
    }

    #[test]
    fn prompt_carries_persona_policy_and_task() {
        let agent = spec(&[("bash", ToolPolicy::Ask), ("read", ToolPolicy::Allow)]);
        let prompt = build_prompt(&agent, "do the thing");
        assert!(prompt.contains("You are a test bot."));
        assert!(prompt.contains("bash: ask-first"));
        assert!(prompt.contains("read: allow"));
        assert!(prompt.ends_with("do the thing\n"));
    }

    #[test]
    fn claude_launch_maps_permissions_to_tool_flags() {
        let agent = spec(&[
            ("Bash", ToolPolicy::Allow),
            ("Write", ToolPolicy::Deny),
            ("Edit", ToolPolicy::Ask),
        ]);
        let launch = ClaudeCode.launch(&agent, "task");
        let joined = launch.args.join(" ");
        assert!(joined.contains("--allowedTools Bash"));
        assert!(joined.contains("--disallowedTools Edit,Write"));
    }

    #[test]
    fn claude_parse_maps_stream_json() {
        let step = r#"{"type":"assistant","message":{"content":"hi"}}"#;
        assert!(matches!(ClaudeCode.parse(step), Some(Peep::Step { .. })));
        let result = r#"{"type":"result","result":"all done"}"#;
        assert!(matches!(
            ClaudeCode.parse(result),
            Some(Peep::Output { .. })
        ));
        assert!(ClaudeCode.parse("not json").is_none());
    }

    #[test]
    fn echo_is_always_detected() {
        assert!(Echo.detect());
        assert!(perch_named("echo").is_some());
        assert!(perch_named("nope").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn which_requires_an_execute_bit() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("fakecli");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        std::env::set_var("PATH", tmp.path());
        assert!(which("fakecli").is_none(), "non-exec file must not count");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(which("fakecli").is_some());
    }
}
