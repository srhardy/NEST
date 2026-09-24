use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "agent.toml";

/// The project truth. Every harness projection is derived from this file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(default = "default_harnesses")]
    pub harnesses: Vec<String>,
    #[serde(default)]
    pub instructions: Instructions,
    #[serde(default)]
    pub skills: Vec<SkillRef>,
    #[serde(default)]
    pub mcp: Vec<McpServer>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub bots: Vec<Bot>,
    #[serde(default)]
    pub automations: Vec<Automation>,
    #[serde(default)]
    pub routing: Vec<Route>,
    #[serde(default)]
    pub services: Vec<Service>,
}

fn default_harnesses() -> Vec<String> {
    vec![
        "nest".to_string(),
        "claude-code".to_string(),
        "codex".to_string(),
    ]
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            harnesses: default_harnesses(),
            instructions: Instructions::default(),
            skills: Vec::new(),
            mcp: Vec::new(),
            budget: Budget::default(),
            bots: Vec::new(),
            automations: Vec::new(),
            routing: Vec::new(),
            services: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Instructions {
    /// Extra instruction lines bridged into every harness.
    /// Canonical long-form project instructions live in AGENTS.md.
    #[serde(default)]
    pub extra: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl McpServer {
    pub fn is_stdio(&self) -> bool {
        self.command.is_some()
    }
    pub fn is_remote(&self) -> bool {
        self.url.is_some()
    }
    pub fn validate(&self) -> Result<()> {
        if self.is_stdio() == self.is_remote() {
            anyhow::bail!(
                "mcp server '{}' must set exactly one of `command` (stdio) or `url` (remote)",
                self.name
            );
        }
        Ok(())
    }
}

/// A bot definition. The Nest harness projects each into
/// `.nest/agents/<name>.md`; other harnesses warn (they have no roster file yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bot {
    /// kebab-case id — also the generated filename (`<name>.md`).
    pub name: String,
    /// Display name shown in the roster (defaults to `name`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The one job this bot is for. One job, no scope creep.
    #[serde(default)]
    pub job: String,
    /// Persona lines — the body of the generated agent file.
    #[serde(default)]
    pub voice: Vec<String>,
    /// Explicit "not my job" list — keeps the bot in its lane.
    #[serde(default)]
    pub anti_jobs: Vec<String>,
    /// Skill names this bot carries.
    #[serde(default)]
    pub skills: Vec<String>,
    /// MCP server names this bot may use (entries from `[[mcp]]`).
    #[serde(default)]
    pub mcp: Vec<String>,
    /// Model tier hint (concise | balanced | full — mirrors budget.preset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Knowledge notes seeded into the bot.
    #[serde(default)]
    pub memory: Vec<String>,
    /// Lifecycle mode: `subagent` bots spool on demand, return results, and
    /// auto-cleanup (the council); `primary` bots are foreground conversants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<BotMode>,
    /// Per-tool permission policy, e.g. `permissions = { bash = "ask", read = "allow" }`.
    /// Missing tools are decided by the harness, not by Spoke.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<String, ToolPolicy>,
    /// Marks the boss bot — the default receiver for un-routed intents and the
    /// delegator to the council. At most one bot may set `boss = true`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub boss: bool,
    /// Step budget for a single run (Bob-style `steps:` guardrail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    /// Per-bot spend budget — a preset name (concise | balanced | full)
    /// or a `daily-NNNk`-style cap. Consulted by the engine before a run;
    /// unknown shapes warn, never block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<String>,
    /// Automation names this bot owns — must each match a declared
    /// `[[automations]]` entry. Rendered into the agent file so the
    /// roster shows who answers which routine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routines: Vec<String>,
    /// Network egress policy: `none` (default — local files only),
    /// `same-project` (project remotes/registries), or `open`. Rendered
    /// into the agent file as a policy note; harnesses without an egress
    /// control surface it prompt-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<String>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// How a bot runs when invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BotMode {
    /// Foreground conversant — the user talks to it directly.
    Primary,
    /// Spools as a delegated subtask with scoped permissions, returns results,
    /// and auto-cleans up (the council).
    Subagent,
}

/// What a bot may do with a given tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolPolicy {
    Allow,
    Ask,
    Deny,
}

/// An intent-dispatch rule: when a task matches `patterns`, route it to `bot`.
/// The Boss pattern — the boss bot consults this table to delegate to the
/// council. Rendered to `.nest/routing.toml` by the Nest harness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    /// Keywords / patterns that trigger this route.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Target `[[bots]]` name.
    pub bot: String,
    /// Escalation ladder — bot names tried in order if the target fails.
    #[serde(default)]
    pub fallback: Vec<String>,
}

impl Route {
    pub fn validate(&self) -> Result<()> {
        if self.patterns.is_empty() {
            anyhow::bail!(
                "route for bot '{}' has no patterns — add at least one",
                self.bot
            );
        }
        if self.bot.trim().is_empty() {
            anyhow::bail!("route has an empty bot name");
        }
        for f in &self.fallback {
            if f.trim().is_empty() {
                anyhow::bail!("route for bot '{}' has an empty fallback entry", self.bot);
            }
        }
        Ok(())
    }
}

/// A declared external service the project depends on (memory layer, crawler,
/// autonomous worker…). Rendered to `.nest/services.toml` so the app can start
/// and health-check them — the orchestrate.sh table as data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    /// TCP port the service listens on (localhost).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Health-check path, e.g. `/api/v1/stats/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    /// Command used to start the service when it is down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
}

impl Service {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("service has an empty name");
        }
        Ok(())
    }
}

impl Bot {
    /// Bot names become filenames; keep them lowercase-kebab and path-safe.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("bot has an empty name");
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            anyhow::bail!(
                "bot name '{}' must be lowercase-kebab (a-z, 0-9, '-', '_') — it becomes a filename",
                self.name
            );
        }
        if self.job.trim().is_empty() {
            anyhow::bail!(
                "bot '{}' must declare a job — one sentence, one job",
                self.name
            );
        }
        if let Some(e) = &self.egress {
            if !["none", "same-project", "open"].contains(&e.as_str()) {
                anyhow::bail!(
                    "bot '{}' egress '{e}' must be one of: none, same-project, open",
                    self.name
                );
            }
        }
        if let Some(b) = &self.budget {
            let b = b.trim();
            let preset = ["concise", "balanced", "full"].contains(&b);
            let daily = b.strip_prefix("daily-").is_some_and(|rest| {
                rest.strip_suffix('k')
                    .is_some_and(|n| n.parse::<u32>().is_ok_and(|n| n > 0))
            });
            if !preset && !daily {
                anyhow::bail!(
                    "bot '{}' budget '{b}' must be a preset (concise|balanced|full) or daily-NNNk",
                    self.name
                );
            }
        }
        for r in &self.routines {
            if r.trim().is_empty() {
                anyhow::bail!("bot '{}' has an empty routines entry", self.name);
            }
        }
        Ok(())
    }
}

/// A trigger-driven run (Devin-style automation), projected to
/// `.nest/automations.toml` by the Nest harness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub name: String,
    /// Cron expression (5 fields) for recurring runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// Event trigger, e.g. \"file-change:src/**\". Free-form for now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// What the run should do.
    #[serde(default)]
    pub prompt: String,
    /// Optional `[[bots]]` name to run as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Automation {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("automation has an empty name");
        }
        if self.schedule.is_some() == self.event.is_some() {
            anyhow::bail!(
                "automation '{}' must set exactly one of `schedule` (cron) or `event`",
                self.name
            );
        }
        if let Some(schedule) = &self.schedule {
            if schedule.split_whitespace().count() != 5 {
                anyhow::bail!(
                    "automation '{}' schedule '{}' is not a 5-field cron expression",
                    self.name,
                    schedule
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    #[serde(default = "default_preset")]
    pub preset: String,
}

fn default_preset() -> String {
    "balanced".to_string()
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            preset: default_preset(),
        }
    }
}

impl Manifest {
    pub fn load(project: &Path) -> Result<Self> {
        let path = project.join(MANIFEST_FILE);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let manifest: Manifest =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(manifest)
    }

    pub fn save(&self, project: &Path) -> Result<PathBuf> {
        let path = project.join(MANIFEST_FILE);
        let body = toml::to_string_pretty(self).context("serializing manifest")?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    pub fn enabled(&self, harness_id: &str) -> bool {
        self.harnesses.iter().any(|h| h == harness_id)
    }

    pub fn validate(&self) -> Result<()> {
        for server in &self.mcp {
            server.validate()?;
        }
        for bot in &self.bots {
            bot.validate()?;
        }
        for automation in &self.automations {
            automation.validate()?;
        }
        for bot in &self.bots {
            for r in &bot.routines {
                if !self.automations.iter().any(|a| a.name == *r) {
                    anyhow::bail!(
                        "bot '{}' claims routine '{r}' — no [[automations]] entry declares it",
                        bot.name
                    );
                }
            }
        }
        for route in &self.routing {
            route.validate()?;
        }
        for service in &self.services {
            service.validate()?;
        }
        if self.bots.iter().filter(|b| b.boss).count() > 1 {
            anyhow::bail!("at most one bot may be marked `boss = true`");
        }
        for (kind, names) in [
            (
                "bot",
                self.bots
                    .iter()
                    .map(|b| b.name.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                "automation",
                self.automations
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                "service",
                self.services
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>(),
            ),
        ] {
            let mut seen = std::collections::HashSet::new();
            for name in names {
                if !seen.insert(name) {
                    anyhow::bail!("duplicate {kind} name '{name}'");
                }
            }
        }
        Ok(())
    }

    pub fn template() -> &'static str {
        TEMPLATE
    }
}

pub const TEMPLATE: &str = r#"# agent.toml — Spoke manifest. One file, every harness.
version = 1

# Harnesses Spoke projects this manifest into.
harnesses = ["nest", "claude-code", "codex"]

# Skill packages. Installed by the packages spoke (M1) via `spoke add <name>`.
skills = []

# Extra instruction lines bridged into every harness.
# Long-form project instructions live in AGENTS.md (Spoke only manages its marked block there).
[instructions]
extra = []

# MCP servers, registered with every enabled harness in that harness's native format.
# [[mcp]]
# name = "spoke-fetch"
# command = "spoke"
# args = ["mcp", "fetch"]

# Token/context policy for the budget spoke (M2).
[budget]
preset = "balanced"   # concise | balanced | full

# Bots — your roster. Nest projects each into .nest/agents/<name>.md.
# `boss = true` marks the orchestrator (at most one): it receives un-routed
# intents and delegates to the council via [[routing]]. `mode = "subagent"`
# bots spool on demand and auto-clean. `permissions` sets per-tool policy
# (allow | ask | deny); `max_steps` is a per-run step budget; `budget`
# (concise|balanced|full or daily-NNNk) is the spend budget; `routines`
# names the [[automations]] this bot owns; `egress`
# (none | same-project | open) is the network policy.
# [[bots]]
# name = "eggbot"
# title = "Dr. Eggbot"
# job = "Walks you through building a bot, then writes it"
# voice = ["You are a friendly bot-builder who asks one question at a time."]
# anti_jobs = ["Do not write application code", "Do not browse the web"]
# model = "balanced"
# mode = "primary"
# boss = true
# max_steps = 30
# budget = "balanced"
# permissions = { bash = "ask", edit = "ask", read = "allow" }

# Routing — the Boss's dispatch table: keywords/patterns -> bot, with an
# optional escalation ladder (`fallback` bots tried in order on failure).
# [[routing]]
# patterns = ["debug", "fix bug", "stack trace", "crash"]
# bot = "debugger"
# fallback = ["council"]

# Services — external daemons the project depends on. Rendered to
# .nest/services.toml so the app can start and health-check them.
# [[services]]
# name = "openmemory"
# port = 8765
# health = "/api/v1/stats/"
# start = "docker compose -f ~/mem0/openmemory up -d"

# Automations — trigger-driven runs. Exactly one of `schedule` (5-field cron)
# or `event`; Nest projects them into .nest/automations.toml.
# [[automations]]
# name = "morning-triage"
# schedule = "0 9 * * *"
# prompt = "Summarize yesterday's changes"
# bot = "eggbot"
"#;
