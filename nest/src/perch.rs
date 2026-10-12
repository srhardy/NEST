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

use crate::hatchling::FailureKind;
use crate::home::NestDirs;
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
    /// Build the spawn command for a bot + task. `session` is the
    /// harness's own conversation id when resuming — perches without
    /// resume semantics ignore it and start fresh.
    fn launch(
        &self,
        agent: &AgentSpec,
        task: &str,
        dirs: &NestDirs,
        session: Option<&str>,
    ) -> Launch;
    /// One output line → one peep. `None` skips the line.
    fn parse(&self, line: &str) -> Option<Peep>;
    /// Why did a failed run fail? The default sniffs the output tail for
    /// the usual "you weren't allowed" markers — `Denied` vs `Exit` is
    /// what tells the brood's ladder whether the next candidate is a
    /// retry or a different action. Perches with richer exit signals
    /// (structured error events, known exit codes) can override.
    fn failure_kind(&self, tail: &[String]) -> FailureKind {
        const DENIED: [&str; 5] = [
            "permission denied",
            "operation not permitted",
            "access denied",
            "not permitted",
            "not allowed",
        ];
        let denied = tail.iter().any(|line| {
            let line = line.to_lowercase();
            DENIED.iter().any(|marker| line.contains(marker))
        });
        if denied {
            FailureKind::Denied
        } else {
            FailureKind::Exit
        }
    }
    /// The one-click install command (run via `sh -c`, or `cmd /C` on Windows), when this perch
    /// can install itself. Builtin stubs return `None`.
    fn install_cmd(&self) -> Option<&'static str> {
        None
    }
    /// Can this perch hand the tool policy to the harness as a real
    /// control (allow/deny flags, permission env) rather than prompt
    /// text? Stubs return false — their policy is advisory only.
    fn enforces_tools(&self) -> bool {
        false
    }
    /// Real egress control. No perch has one today — declared `egress`
    /// is prompt text everywhere, and the run says so on the tape.
    fn enforces_egress(&self) -> bool {
        false
    }
    /// Real spend-budget control (`budget:` is money, distinct from the
    /// engine's step/second rails). Advisory on every perch today.
    fn enforces_budget(&self) -> bool {
        false
    }
    /// Does this perch parse structured events (stream-json / JSONL)?
    /// A structured perch emitting only plain lines means the upstream
    /// CLI changed its output format — worth a warning, not silence.
    fn structured_output(&self) -> bool {
        false
    }
}

/// PATH lookup honoring PATHEXT on Windows — the shared "is this
/// executable installed" check for perches and providers.
pub fn which(bin: &str) -> Option<PathBuf> {
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

/// When a failed run's output says the harness isn't signed in, the one
/// line that tells a newcomer how to fix it. NEST wraps CLIs that keep
/// their own login, so the remedy lives in that CLI, not in NEST.
pub fn auth_hint(perch: &str, tail: &[String]) -> Option<String> {
    const MARKERS: [&str; 6] = [
        "not logged in",
        "please run /login",
        "invalid api key",
        "authentication_error",
        "unauthorized",
        "not authenticated",
    ];
    let signed_out = tail.iter().any(|line| {
        let line = line.to_lowercase();
        MARKERS.iter().any(|m| line.contains(m))
    });
    if !signed_out {
        return None;
    }
    let fix = match perch {
        "claude-code" => "run `claude`, then /login (or set ANTHROPIC_API_KEY)",
        "codex" => "run `codex login` (or set OPENAI_API_KEY)",
        "opencode" => "run `opencode auth login`",
        _ => "sign in to that CLI directly, then retry",
    };
    Some(format!("⚠ {perch} is not signed in — {fix}"))
}

/// Turn a bare program name into something `Command` can actually spawn.
/// Windows `CreateProcess` only appends `.exe`, so npm-installed CLIs
/// (`claude.cmd`, `codex.cmd`) are detected by `which` yet "not found" at
/// spawn. Resolve through PATH/PATHEXT, and unwrap npm shims to their real
/// target: Rust refuses multi-line args to batch files, and prompts are
/// multi-line. Returns the program plus any args that must precede the
/// caller's (a `node` shim's script path). Non-Windows is a passthrough.
pub fn resolve_program(program: &str) -> (String, Vec<String>) {
    if !cfg!(windows) {
        return (program.to_string(), Vec::new());
    }
    let Some(path) = which(program) else {
        return (program.to_string(), Vec::new());
    };
    let is_batch = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if is_batch {
        if let Some(dir) = path.parent() {
            if let Some(target) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| parse_npm_shim(&text, dir))
            {
                return match target {
                    ShimTarget::Exe(exe) => (exe.to_string_lossy().into_owned(), Vec::new()),
                    ShimTarget::Script(script) => {
                        let node = which("node")
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "node".into());
                        (node, vec![script.to_string_lossy().into_owned()])
                    }
                };
            }
        }
    }
    (path.to_string_lossy().into_owned(), Vec::new())
}

#[derive(Debug, PartialEq)]
pub enum ShimTarget {
    Exe(PathBuf),
    Script(PathBuf),
}

/// Find the real target an npm `.cmd` shim launches: the last quoted
/// `%dp0%\…` path ending in `.exe` (a native binary) or `.js/.cjs/.mjs`
/// (run under node). `node.exe` itself is the interpreter, never the target.
pub fn parse_npm_shim(text: &str, shim_dir: &std::path::Path) -> Option<ShimTarget> {
    let mut found = None;
    for token in text.split('"') {
        let Some(rest) = token.strip_prefix("%dp0%") else {
            continue;
        };
        let rest = rest.trim_start_matches(['\\', '/']);
        if rest.is_empty() {
            continue;
        }
        let path = rest
            .split(['\\', '/'])
            .fold(shim_dir.to_path_buf(), |acc, part| acc.join(part));
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        if name == "node.exe" {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext.eq_ignore_ascii_case("exe") {
            found = Some(ShimTarget::Exe(path));
        } else if ["js", "cjs", "mjs"]
            .iter()
            .any(|e| ext.eq_ignore_ascii_case(e))
        {
            found = Some(ShimTarget::Script(path));
        }
    }
    found
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

/// The prompt every perch feeds its harness: persona + tool policy +
/// spend budget + routines + network egress + skills + task. A declared
/// skill's `SKILL.md` resolves project-first (same precedence as agents);
/// a missing one is named in the prompt so the run reports it instead of
/// silently ignoring it.
pub fn build_prompt(agent: &AgentSpec, task: &str, dirs: &NestDirs) -> String {
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
    if let Some(egress) = agent.egress.as_deref() {
        let note = match egress {
            "none" => "work offline — local files only, no network fetches",
            "same-project" => "project remotes and registries only — no open web",
            _ => "open network access is allowed for this bot",
        };
        prompt.push_str(&format!("\n## Network\n\n- egress: {egress} — {note}\n"));
    }
    if let Some(budget) = agent.budget.as_deref() {
        prompt.push_str(&format!(
            "\n## Budget\n\n- budget: {budget} — stay inside it\n"
        ));
    }
    if !agent.routines.is_empty() {
        prompt.push_str("\n## Routines\n\n");
        for r in &agent.routines {
            prompt.push_str(&format!("- {r}\n"));
        }
    }
    if !agent.skills.is_empty() {
        prompt.push_str("\n\n## Skills\n\n");
        for name in &agent.skills {
            match dirs
                .skill(name)
                .and_then(|p| std::fs::read_to_string(p).ok())
            {
                Some(body) => prompt.push_str(&format!("### {name}\n\n{}\n\n", body.trim())),
                None => prompt.push_str(&format!("- {name}: missing — no SKILL.md found\n")),
            }
        }
    }
    if let Some(memory) = crate::memory::prompt_block(dirs) {
        prompt.push_str(&format!("\n\n## Memory\n\n{memory}\n"));
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
    fn launch(
        &self,
        agent: &AgentSpec,
        task: &str,
        dirs: &NestDirs,
        session: Option<&str>,
    ) -> Launch {
        let (allow, deny) = tool_lists(agent);
        let mut args = vec![
            "-p".into(),
            build_prompt(agent, task, dirs),
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
        if let Some(session) = session {
            args.push("--resume".into());
            args.push(session.into());
        }
        Launch {
            program: "claude".into(),
            args,
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        // the init event announces the session id — surface it once so
        // the transcript can resume the next turn on the harness
        if v.get("type").and_then(|t| t.as_str()) == Some("system") {
            if let Some(id) = v.get("session_id").and_then(|id| id.as_str()) {
                return Some(Peep::Session { id: id.into() });
            }
        }
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
    fn enforces_tools(&self) -> bool {
        true
    }
    fn structured_output(&self) -> bool {
        true
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
    fn launch(
        &self,
        agent: &AgentSpec,
        task: &str,
        dirs: &NestDirs,
        session: Option<&str>,
    ) -> Launch {
        // Codex has only global sandbox/approval controls. Since NEST runs
        // headlessly and cannot answer approvals, writes are disabled unless
        // every write-capable tool is explicitly allowed.
        const WRITE_TOOLS: [&str; 8] = [
            "write",
            "edit",
            "bash",
            "shell",
            "exec",
            "apply",
            "patch",
            "powershell",
        ];
        let explicitly_allows_writes = WRITE_TOOLS.iter().all(|write_tool| {
            agent.permissions.iter().any(|(tool, policy)| {
                tool.eq_ignore_ascii_case(write_tool) && *policy == ToolPolicy::Allow
            })
        });
        let sandbox = if explicitly_allows_writes {
            "workspace-write"
        } else {
            "read-only"
        };
        let mut args: Vec<String> = match session {
            // `codex exec resume <id> [prompt]` — continue the thread
            Some(id) => vec!["exec".into(), "resume".into(), id.into()],
            None => vec!["exec".into()],
        };
        args.extend([
            "--json".into(),
            "--sandbox".into(),
            sandbox.into(),
            "--ask-for-approval".into(),
            "never".into(),
            build_prompt(agent, task, dirs),
        ]);
        Launch {
            program: "codex".into(),
            args,
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let kind = v.get("type").or_else(|| v.get("msg").map(|m| &m["type"]))?;
        if kind.as_str()? == "thread.started" {
            // the thread id is codex's resume token — surface it so the
            // transcript can pick this conversation back up
            return v
                .get("thread_id")
                .and_then(|id| id.as_str())
                .map(|id| Peep::Session { id: id.into() });
        }
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
    fn enforces_tools(&self) -> bool {
        true
    }
    fn structured_output(&self) -> bool {
        true
    }
}

/// `opencode` (OpenCode CLI): `opencode run <prompt>` — plain lines out.
///
/// Permissions ride `OPENCODE_PERMISSION` (inlined JSON permission config,
/// same shape as config.json's `permission` object): `allow` stays allow,
/// `ask` can't prompt in a headless run so it joins `deny`. `write` folds
/// into opencode's `edit` (which covers edit/write/patch); unknown names
/// pass through verbatim and the harness ignores what it doesn't know.
pub struct OpenCode;
impl Perch for OpenCode {
    fn name(&self) -> &'static str {
        "opencode"
    }
    fn binary(&self) -> &'static str {
        "opencode"
    }
    fn launch(
        &self,
        agent: &AgentSpec,
        task: &str,
        dirs: &NestDirs,
        session: Option<&str>,
    ) -> Launch {
        let mut env = BTreeMap::new();
        if !agent.permissions.is_empty() {
            let mut perm = serde_json::Map::new();
            for (tool, policy) in &agent.permissions {
                let name = tool.to_lowercase();
                let name = if name == "write" { "edit".into() } else { name };
                let action = match policy {
                    ToolPolicy::Allow => "allow",
                    ToolPolicy::Ask | ToolPolicy::Deny => "deny",
                };
                perm.insert(name, serde_json::Value::String(action.into()));
            }
            env.insert(
                "OPENCODE_PERMISSION".into(),
                serde_json::Value::Object(perm).to_string(),
            );
        }
        let mut args = vec!["run".into()];
        if let Some(session) = session {
            args.push("--session".into());
            args.push(session.into());
        }
        args.push(build_prompt(agent, task, dirs));
        Launch {
            program: "opencode".into(),
            args,
            env,
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        (!line.trim().is_empty()).then(|| Peep::Output { text: line.into() })
    }
    fn install_cmd(&self) -> Option<&'static str> {
        Some("npm i -g opencode-ai")
    }
    fn enforces_tools(&self) -> bool {
        true
    }
}

/// `pi` (Pi coding-agent CLI): `pi --mode json -p <prompt>` — one JSON
/// event per line, final response on stdout, exits when done.
///
/// Permissions map to pi's tool flags: asked-or-denied tools join
/// `--exclude-tools` (pi's denylist). Allowed tools need no flag — pi
/// enables its builtins by default, so `allow` stays the default and only
/// denials are passed. Anything finer stays prompt-only via `build_prompt`.
pub struct Pi;
impl Perch for Pi {
    fn name(&self) -> &'static str {
        "pi"
    }
    fn binary(&self) -> &'static str {
        "pi"
    }
    fn launch(
        &self,
        agent: &AgentSpec,
        task: &str,
        dirs: &NestDirs,
        _session: Option<&str>,
    ) -> Launch {
        let mut denied: Vec<String> = agent
            .permissions
            .iter()
            .filter(|(_, policy)| matches!(policy, ToolPolicy::Ask | ToolPolicy::Deny))
            .map(|(tool, _)| tool.to_lowercase())
            .collect();
        denied.sort();
        denied.dedup();
        let mut args = vec![
            "--mode".into(),
            "json".into(),
            "-p".into(),
            build_prompt(agent, task, dirs),
        ];
        if !denied.is_empty() {
            args.push("--exclude-tools".into());
            args.push(denied.join(","));
        }
        Launch {
            program: "pi".into(),
            args,
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let v: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => return Some(Peep::Output { text: line.into() }),
        };
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind.contains("tool") {
            return Some(Peep::Step {
                n: 0,
                label: kind.into(),
            });
        }
        for key in ["text", "message", "content", "output"] {
            if let Some(text) = v.get(key).and_then(|t| t.as_str()) {
                if !text.is_empty() {
                    return Some(Peep::Output { text: text.into() });
                }
            }
        }
        Some(Peep::Output { text: line.into() })
    }
    fn install_cmd(&self) -> Option<&'static str> {
        Some("npm install -g --ignore-scripts @earendil-works/pi-coding-agent")
    }
    fn enforces_tools(&self) -> bool {
        true
    }
    fn structured_output(&self) -> bool {
        true
    }
}

/// How the builtin stub perches run a one-line script: `sh -c` where there
/// is a POSIX shell, PowerShell on Windows (every Windows has it; `cmd`'s
/// `echo` can't print a multi-line prompt). Output is forced to UTF-8.
fn stub_shell(unix: &str, windows: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "powershell".into(),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                format!("[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); {windows}"),
            ],
        )
    } else {
        ("sh".into(), vec!["-c".into(), unix.into()])
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
    fn launch(
        &self,
        agent: &AgentSpec,
        task: &str,
        dirs: &NestDirs,
        _session: Option<&str>,
    ) -> Launch {
        let mut env = BTreeMap::new();
        env.insert("NEST_PROMPT".into(), build_prompt(agent, task, dirs));
        let (program, args) = stub_shell(
            "echo \"$NEST_PROMPT\"",
            "[Console]::Out.WriteLine($env:NEST_PROMPT)",
        );
        Launch { program, args, env }
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
    fn launch(
        &self,
        _agent: &AgentSpec,
        _task: &str,
        _dirs: &NestDirs,
        _session: Option<&str>,
    ) -> Launch {
        let (program, args) = stub_shell(
            "echo 'stub perch failing as designed' >&2; exit 1",
            "[Console]::Error.WriteLine('stub perch failing as designed'); exit 1",
        );
        Launch {
            program,
            args,
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
        Box::new(Pi),
        Box::new(Echo),
        Box::new(Fail),
    ]
}

/// Look a perch up by name.
pub fn perch_named(name: &str) -> Option<Box<dyn Perch>> {
    all_perches().into_iter().find(|p| p.name() == name)
}

/// First detected real harness, else the echo stub (the spine always runs).
/// `NEST_PERCH` pins one perch by name — for tests and daemon setups that
/// must not depend on which agent CLIs happen to be installed on the box.
pub fn default_perch() -> Box<dyn Perch> {
    if let Ok(name) = env::var("NEST_PERCH") {
        if let Some(perch) = perch_named(&name) {
            return perch;
        }
    }
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

    #[test]
    fn npm_shim_with_native_exe_resolves_to_the_exe() {
        let shim = "@ECHO off\r\nSETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n";
        let dir = std::path::Path::new("npm");
        let want = dir
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin")
            .join("claude.exe");
        assert_eq!(parse_npm_shim(shim, dir), Some(ShimTarget::Exe(want)));
    }

    #[test]
    fn npm_shim_with_node_script_resolves_to_the_script_not_node() {
        let shim = "IF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n)\r\nendLocal & goto #_undefined_# 2>NUL || title %COMMAND_LINE% & \"%_prog%\"  \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\r\n";
        let dir = std::path::Path::new("npm");
        let want = dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin")
            .join("codex.js");
        assert_eq!(parse_npm_shim(shim, dir), Some(ShimTarget::Script(want)));
    }

    #[test]
    fn signed_out_output_gets_a_per_perch_hint() {
        let tail = vec!["Not logged in · Please run /login".to_string()];
        let hint = auth_hint("claude-code", &tail).unwrap();
        assert!(hint.contains("claude") && hint.contains("/login"), "{hint}");
        assert!(auth_hint("codex", &tail).unwrap().contains("codex login"));
        assert!(auth_hint("pi", &tail).is_some());
        assert_eq!(auth_hint("claude-code", &["all good".to_string()]), None);
        assert_eq!(auth_hint("claude-code", &[]), None);
    }

    #[test]
    fn unrecognised_shim_has_no_target() {
        assert_eq!(
            parse_npm_shim("@echo off\r\necho hi\r\n", std::path::Path::new("x")),
            None
        );
    }

    fn spec(perms: &[(&str, ToolPolicy)]) -> AgentSpec {
        AgentSpec {
            name: "t".into(),
            title: None,
            job: "job".into(),
            model: None,
            mode: None,
            boss: false,
            max_steps: None,
            max_seconds: None,
            permissions: perms.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            skills: vec![],
            mcp: vec![],
            budget: None,
            routines: Vec::new(),
            egress: None,
            body: "You are a test bot.".into(),
        }
    }

    #[test]
    fn prompt_carries_persona_policy_and_task() {
        let agent = spec(&[("bash", ToolPolicy::Ask), ("read", ToolPolicy::Allow)]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let prompt = build_prompt(&agent, "do the thing", &dirs);
        assert!(prompt.contains("You are a test bot."));
        assert!(prompt.contains("bash: ask-first"));
        assert!(prompt.contains("read: allow"));
        assert!(prompt.ends_with("do the thing\n"));
    }

    #[test]
    fn prompt_carries_egress_policy() {
        let mut agent = spec(&[]);
        agent.egress = Some("none".into());
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let prompt = build_prompt(&agent, "task", &dirs);
        assert!(prompt.contains("egress: none"));
        assert!(prompt.contains("work offline"));
    }

    #[test]
    fn prompt_carries_budget_and_routines() {
        let mut agent = spec(&[]);
        agent.budget = Some("daily-50k".into());
        agent.routines = vec!["morning-triage".into()];
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let prompt = build_prompt(&agent, "task", &dirs);
        assert!(prompt.contains("budget: daily-50k"));
        assert!(prompt.contains("morning-triage"));
    }

    #[test]
    fn claude_launch_maps_permissions_to_tool_flags() {
        let agent = spec(&[
            ("Bash", ToolPolicy::Allow),
            ("Write", ToolPolicy::Deny),
            ("Edit", ToolPolicy::Ask),
        ]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let launch = ClaudeCode.launch(&agent, "task", &dirs, None);
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
    fn claude_resume_adds_the_flag_and_parse_surfaces_the_session_id() {
        let agent = spec(&[]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        // fresh runs get no resume flag; a transcript session adds it
        assert!(!ClaudeCode
            .launch(&agent, "task", &dirs, None)
            .args
            .join(" ")
            .contains("--resume"));
        let resumed = ClaudeCode.launch(&agent, "task", &dirs, Some("sess-42"));
        let joined = resumed.args.join(" ");
        assert!(joined.contains("--resume sess-42"));

        // the init event announces the id; other events keep their
        // normal peep shape (every stream-json event carries session_id)
        let init = r#"{"type":"system","subtype":"init","session_id":"sess-42"}"#;
        assert!(matches!(
            ClaudeCode.parse(init),
            Some(Peep::Session { id }) if id == "sess-42"
        ));
        let mid = r#"{"type":"assistant","session_id":"sess-42","message":{}}"#;
        assert!(matches!(ClaudeCode.parse(mid), Some(Peep::Step { .. })));
    }

    #[test]
    fn codex_resume_uses_exec_resume_and_thread_id_is_the_session() {
        let agent = spec(&[]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let resumed = Codex.launch(&agent, "task", &dirs, Some("thread-7"));
        assert_eq!(resumed.args[0..3], ["exec", "resume", "thread-7"]);
        assert!(resumed.args.join(" ").contains("--json"));

        let started = r#"{"type":"thread.started","thread_id":"thread-7"}"#;
        assert!(matches!(
            Codex.parse(started),
            Some(Peep::Session { id }) if id == "thread-7"
        ));
    }

    #[test]
    fn opencode_resume_passes_the_session_flag() {
        let agent = spec(&[]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let resumed = OpenCode.launch(&agent, "task", &dirs, Some("ses_9"));
        assert_eq!(resumed.args[1..3], ["--session", "ses_9"]);
    }

    #[test]
    fn echo_is_always_detected() {
        assert!(Echo.detect());
        assert!(perch_named("echo").is_some());
        assert!(perch_named("pi").is_some());
        assert!(perch_named("nope").is_none());
    }

    #[test]
    fn codex_launch_defaults_to_read_only_and_never_ask() {
        let agent = spec(&[]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let launch = Codex.launch(&agent, "task", &dirs, None);
        let joined = launch.args.join(" ");
        assert!(joined.contains("--sandbox read-only"));
        assert!(joined.contains("--ask-for-approval never"));
    }

    #[test]
    fn codex_requires_explicit_write_grant_and_honors_denials() {
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let partial = spec(&[("write", ToolPolicy::Allow)]);
        assert!(Codex
            .launch(&partial, "task", &dirs, None)
            .args
            .join(" ")
            .contains("--sandbox read-only"));

        let all_write_tools_allowed = [
            "write",
            "edit",
            "bash",
            "shell",
            "exec",
            "apply",
            "patch",
            "powershell",
        ]
        .into_iter()
        .map(|tool| (tool, ToolPolicy::Allow))
        .collect::<Vec<_>>();
        let granted = spec(&all_write_tools_allowed);
        assert!(Codex
            .launch(&granted, "task", &dirs, None)
            .args
            .join(" ")
            .contains("--sandbox workspace-write"));

        let denied = spec(&[("write", ToolPolicy::Allow), ("bash", ToolPolicy::Deny)]);
        assert!(Codex
            .launch(&denied, "task", &dirs, None)
            .args
            .join(" ")
            .contains("--sandbox read-only"));

        let only_read = spec(&[("read", ToolPolicy::Allow)]);
        assert!(Codex
            .launch(&only_read, "task", &dirs, None)
            .args
            .join(" ")
            .contains("--sandbox read-only"));
    }

    #[test]
    fn opencode_launch_carries_permission_env() {
        let agent = spec(&[
            ("bash", ToolPolicy::Allow),
            ("edit", ToolPolicy::Ask),
            ("Write", ToolPolicy::Deny),
        ]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let launch = OpenCode.launch(&agent, "task", &dirs, None);
        let perm: serde_json::Value =
            serde_json::from_str(launch.env.get("OPENCODE_PERMISSION").unwrap()).unwrap();
        assert_eq!(perm["bash"], "allow");
        assert_eq!(perm["edit"], "deny");
    }

    #[test]
    fn opencode_launch_skips_env_without_permissions() {
        let agent = spec(&[]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        assert!(OpenCode.launch(&agent, "task", &dirs, None).env.is_empty());
    }

    #[test]
    fn pi_launch_uses_json_print_mode_with_denylist() {
        let agent = spec(&[
            ("read", ToolPolicy::Allow),
            ("Edit", ToolPolicy::Deny),
            ("Bash", ToolPolicy::Ask),
        ]);
        let dirs = NestDirs::resolve(
            &PathBuf::from("/"),
            Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        );
        let launch = Pi.launch(&agent, "task", &dirs, None);
        let joined = launch.args.join(" ");
        assert!(joined.contains("--mode json"));
        assert!(joined.contains("-p"));
        assert!(joined.contains("--exclude-tools bash,edit"));
        assert!(Pi.install_cmd().is_some());
    }

    #[test]
    fn pi_parse_maps_events() {
        let step = r#"{"type":"tool_use","tool":"bash"}"#;
        assert!(matches!(Pi.parse(step), Some(Peep::Step { .. })));
        let text = r#"{"type":"message","text":"all done"}"#;
        assert!(matches!(Pi.parse(text), Some(Peep::Output { .. })));
        assert!(Pi.parse("").is_none());
        assert!(matches!(Pi.parse("plain line"), Some(Peep::Output { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn which_requires_an_execute_bit() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("fakecli");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        // prepend, don't replace — tests share one process and blanking
        // PATH would break every other test's `sh`/binary lookups
        let original = env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![tmp.path().to_path_buf()];
        paths.extend(env::split_paths(&original));
        env::set_var("PATH", env::join_paths(paths).unwrap());
        assert!(which("fakecli").is_none(), "non-exec file must not count");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(which("fakecli").is_some());
        env::set_var("PATH", original);
    }
}
