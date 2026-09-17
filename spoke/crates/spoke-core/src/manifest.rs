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
}

fn default_harnesses() -> Vec<String> {
    vec!["claude-code".to_string(), "codex".to_string()]
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
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
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
        Ok(())
    }

    pub fn template() -> &'static str {
        TEMPLATE
    }
}

pub const TEMPLATE: &str = r#"# agent.toml — Spoke manifest. One file, every harness.
version = 1

# Harnesses Spoke projects this manifest into.
harnesses = ["claude-code", "codex"]

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
"#;
