use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// A package manifest (package.toml): skills and/or agent (bot) definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// system | curated | experimental (the tiering convention used by existing catalogs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    pub source: Source,
    #[serde(default)]
    pub skills: Vec<SkillSpec>,
    /// Agent (bot) definitions shipped by this package — directories (or single
    /// .md files) installed into each harness's agents root.
    #[serde(default)]
    pub agents: Vec<SkillSpec>,
    #[serde(default)]
    pub requires: Requires,
}

impl PackageManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let manifest: Self =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(manifest)
    }

    /// Resolve a relative `source.path` against the manifest's own directory,
    /// so a package is portable and `./src` means what it looks like it means.
    pub fn resolve_source_path(&mut self, manifest_dir: &Path) {
        if let Some(path) = &self.source.path {
            let candidate = Path::new(path);
            if candidate.is_relative() {
                // `components()` drops the `.` segments, so displayed paths stay clean.
                let resolved: std::path::PathBuf =
                    manifest_dir.join(candidate).components().collect();
                self.source.path = Some(resolved.to_string_lossy().to_string());
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("package name is empty");
        }
        if self.version.trim().is_empty() {
            bail!("package '{}' declares no version", self.name);
        }
        self.source.validate()?;
        if self.skills.is_empty() && self.agents.is_empty() {
            bail!(
                "package '{}' declares no skills or agents — add one or more [[skills]] or [[agents]] entries (name + path)",
                self.name
            );
        }
        for (kind, specs) in [("skill", &self.skills), ("agent", &self.agents)] {
            for spec in specs {
                if spec.name.trim().is_empty() {
                    bail!("package '{}' has a {kind} with an empty name", self.name);
                }
            }
        }
        Ok(())
    }
}

/// Where the package content comes from. Exactly one of `path` or `git` must be set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
}

impl Source {
    pub fn validate(&self) -> Result<()> {
        if self.path.is_some() == self.git.is_some() {
            bail!("source must set exactly one of `path` or `git`");
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        match (&self.path, &self.git) {
            (Some(path), _) => format!("path:{path}"),
            (None, Some(git)) => match &self.rev {
                Some(rev) => format!("git:{git}#{rev}"),
                None => format!("git:{git}"),
            },
            _ => "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillSpec {
    pub name: String,
    /// Path to the skill directory, relative to the package source root.
    #[serde(default = "default_dot")]
    pub path: String,
}

fn default_dot() -> String {
    ".".to_string()
}

/// Declared runtime requirements. Not wired yet — surfaced as warnings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Requires {
    #[serde(default)]
    pub mcp: Vec<String>,
}

/// A registry index (index.toml): a tap-style list of package manifests.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Index {
    #[serde(default, rename = "package")]
    pub packages: Vec<IndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    /// Manifest path relative to the index directory.
    pub manifest: String,
}

impl Index {
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let index: Self =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(index)
    }

    pub fn find(&self, name: &str) -> Option<&IndexEntry> {
        self.packages.iter().find(|entry| entry.name == name)
    }
}
