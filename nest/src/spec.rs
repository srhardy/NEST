//! Agent specs — what the engine loads from `.nest/agents/`.
//!
//! Spoke renders each `[[bots]]` manifest entry to `.nest/agents/<name>.md`
//! (frontmatter + persona body); packages install agents under
//! `~/.nest/agents/<name>/`. `AgentSpec` is the flattened shape the engine
//! reads either way.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use spoke_core::manifest::{BotMode, Route, Service, ToolPolicy};

/// A loadable bot: frontmatter fields plus the persona body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub job: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: Option<BotMode>,
    #[serde(default)]
    pub boss: bool,
    #[serde(default)]
    pub max_steps: Option<u32>,
    #[serde(default)]
    pub permissions: BTreeMap<String, ToolPolicy>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub mcp: Vec<String>,
    /// The markdown body after the frontmatter — the persona.
    #[serde(skip)]
    pub body: String,
}

impl AgentSpec {
    /// The label the roster shows.
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// Parse `---\n<yaml>\n---\n<body>` into an `AgentSpec`.
/// Returns `None` for files without a frontmatter fence.
pub fn parse_agent_md(text: &str) -> Result<Option<AgentSpec>> {
    let text = text.trim_start();
    if !text.starts_with("---") {
        return Ok(None);
    }
    let after_open = &text[3..];
    let close = after_open
        .find("\n---")
        .context("agent file has an unterminated frontmatter fence")?;
    let yaml = &after_open[..close];
    let body = after_open[close + 4..]
        .trim_start_matches(['-', '\n'])
        .to_string();
    let mut spec: AgentSpec =
        serde_yaml::from_str(yaml).context("agent frontmatter is not valid YAML")?;
    spec.body = body;
    Ok(Some(spec))
}

/// The markdown file an agent lives in, for one agents-dir entry:
/// `<dir>/<name>.md` (manifest bots) or `<dir>/<name>/<something>.md`
/// (package-installed agents — prefer `<name>.md`, else first *.md).
fn agent_entry_file(entry: &Path) -> Option<PathBuf> {
    if entry.is_file() && entry.extension().is_some_and(|e| e == "md") {
        return Some(entry.to_path_buf());
    }
    if entry.is_dir() {
        let name_md = entry.join(format!("{}.md", entry.file_name()?.to_string_lossy()));
        if name_md.exists() {
            return Some(name_md);
        }
        let mut mds: Vec<PathBuf> = fs::read_dir(entry)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        mds.sort();
        return mds.into_iter().next();
    }
    None
}

/// Load every agent found under the given `agents/` dirs.
/// Earlier dirs win on name collisions (project shadows personal).
/// Strict: the first bad file fails the whole load.
pub fn load_agents(dirs: &[PathBuf]) -> Result<Vec<AgentSpec>> {
    let (agents, warnings) = load_agents_lenient(dirs);
    if let Some(warning) = warnings.into_iter().next() {
        anyhow::bail!(warning);
    }
    Ok(agents)
}

/// Like `load_agents` but per-file problems become warnings instead of
/// failing the whole roster — callers that prefer a partial roster (the
/// shell) get every good bot plus the list of files that didn't parse.
pub fn load_agents_lenient(dirs: &[PathBuf]) -> (Vec<AgentSpec>, Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    let mut agents = Vec::new();
    let mut warnings = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let entries = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                warnings.push(format!("cannot read {}: {e}", dir.display()));
                continue;
            }
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let Some(file) = agent_entry_file(&path) else {
                continue;
            };
            let text = match fs::read_to_string(&file) {
                Ok(t) => t,
                Err(e) => {
                    warnings.push(format!("cannot read {}: {e}", file.display()));
                    continue;
                }
            };
            match parse_agent_md(&text) {
                Ok(Some(spec)) => {
                    if seen.insert(spec.name.clone()) {
                        agents.push(spec);
                    }
                }
                Ok(None) => {}
                Err(e) => warnings.push(format!("invalid agent file {}: {e:#}", file.display())),
            }
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    (agents, warnings)
}

/// A declared LLM provider — kind determines how it's called.
/// `command`: any CLI taking the prompt on stdin (`program`+`args`).
/// `ollama`:  shorthand for `ollama run <model>` — the free-forever local.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderSpec {
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub model: Option<String>,
    /// free | cheap | paid — the router tries free first, then cheap, then paid.
    #[serde(default = "default_cost")]
    pub cost: String,
}

fn default_kind() -> String {
    "command".into()
}
fn default_cost() -> String {
    "paid".into()
}

/// Load `[[providers]]` tables — first file wins on name collisions.
pub fn load_providers(files: &[PathBuf]) -> (Vec<ProviderSpec>, Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    let mut providers = Vec::new();
    let mut warnings = Vec::new();
    for file in files {
        if !file.is_file() {
            continue;
        }
        let text = match fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("cannot read {}: {e}", file.display()));
                continue;
            }
        };
        #[derive(Deserialize)]
        struct Table {
            #[serde(default)]
            providers: Vec<ProviderSpec>,
        }
        match toml::from_str::<Table>(&text) {
            Ok(t) => {
                for p in t.providers {
                    if seen.insert(p.name.clone()) {
                        providers.push(p);
                    }
                }
            }
            Err(e) => warnings.push(format!("invalid provider file {}: {e}", file.display())),
        }
    }
    (providers, warnings)
}

/// Load a `[[route]]` table file (`route` key — matches the Spoke projection).
pub fn load_routes(files: &[PathBuf]) -> Result<Vec<Route>> {
    #[derive(Deserialize)]
    struct RoutingFile {
        #[serde(default)]
        route: Vec<Route>,
    }
    for file in files {
        if file.is_file() {
            let text = fs::read_to_string(file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            let parsed: RoutingFile = toml::from_str(&text)
                .with_context(|| format!("invalid routing table {}", file.display()))?;
            return Ok(parsed.route);
        }
    }
    Ok(Vec::new())
}

/// Load a `[[service]]` table file (`service` key).
pub fn load_services(files: &[PathBuf]) -> Result<Vec<Service>> {
    #[derive(Deserialize)]
    struct ServicesFile {
        #[serde(default)]
        service: Vec<Service>,
    }
    for file in files {
        if file.is_file() {
            let text = fs::read_to_string(file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            let parsed: ServicesFile = toml::from_str(&text)
                .with_context(|| format!("invalid services table {}", file.display()))?;
            return Ok(parsed.service);
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\nname: doc-keeper\ntitle: \"Doc Keeper\"\njob: \"keep docs in sync\"\nmode: subagent\nboss: true\nmax_steps: 25\npermissions:\n  bash: ask\n  read: allow\nskills: [markdown-tools]\n---\n\n# Doc Keeper\n\nYou keep docs tidy.\n";
        let spec = parse_agent_md(text).unwrap().unwrap();
        assert_eq!(spec.name, "doc-keeper");
        assert_eq!(spec.title(), "Doc Keeper");
        assert_eq!(spec.mode, Some(BotMode::Subagent));
        assert!(spec.boss);
        assert_eq!(spec.max_steps, Some(25));
        assert_eq!(spec.permissions["bash"], ToolPolicy::Ask);
        assert_eq!(spec.skills, vec!["markdown-tools".to_string()]);
        assert!(spec.body.contains("You keep docs tidy."));
    }

    #[test]
    fn skips_files_without_frontmatter() {
        assert!(parse_agent_md("# just a readme\n").unwrap().is_none());
    }

    #[test]
    fn loads_both_file_and_dir_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(agents.join("pack-bot")).unwrap();
        fs::write(
            agents.join("plain.md"),
            "---\nname: plain\njob: x\n---\nbody\n",
        )
        .unwrap();
        fs::write(
            agents.join("pack-bot").join("pack-bot.md"),
            "---\nname: pack-bot\njob: y\n---\nbody\n",
        )
        .unwrap();
        let loaded = load_agents(&[agents]).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "pack-bot");
        assert_eq!(loaded[1].name, "plain");
    }

    #[test]
    fn project_shadows_personal_on_name_clash() {
        let tmp = tempfile::tempdir().unwrap();
        let project_agents = tmp.path().join("p/agents");
        let home_agents = tmp.path().join("h/agents");
        fs::create_dir_all(&project_agents).unwrap();
        fs::create_dir_all(&home_agents).unwrap();
        for dir in [&project_agents, &home_agents] {
            fs::write(dir.join("a.md"), "---\nname: a\njob: x\n---\nb\n").unwrap();
        }
        fs::write(
            project_agents.join("a.md"),
            "---\nname: a\njob: project-version\n---\nb\n",
        )
        .unwrap();
        let loaded = load_agents(&[project_agents, home_agents]).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].job, "project-version");
    }
}
