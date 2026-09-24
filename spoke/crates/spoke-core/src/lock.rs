use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const LOCK_FILE: &str = "agent.lock";

/// Resolved package set. Written by the packages spoke (M1); created empty for now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lock {
    pub version: u32,
    #[serde(default)]
    pub packages: Vec<LockedPackage>,
}

impl Default for Lock {
    fn default() -> Self {
        Self {
            version: 1,
            packages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedPackage {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    /// Estimated context cost in tokens (bytes/4 heuristic; labelled as an estimate).
    #[serde(default)]
    pub tokens: u64,
    /// Skill names installed by this package.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Agent (bot) names installed by this package.
    #[serde(default)]
    pub agents: Vec<String>,
    /// Absolute paths Spoke wrote for this package (one entry per harness per skill).
    #[serde(default)]
    pub installed_paths: Vec<String>,
}

impl Lock {
    pub fn load(project: &Path) -> Result<Self> {
        let path = project.join(LOCK_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let lock =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(lock)
    }

    pub fn save(&self, project: &Path) -> Result<PathBuf> {
        let path = project.join(LOCK_FILE);
        let mut body = serde_json::to_string_pretty(self).context("serializing lockfile")?;
        body.push('\n');
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let tmp = project.join(format!(".{LOCK_FILE}.{nonce}.tmp"));
        std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
        if path.exists() {
            let previous = project.join(format!(".{LOCK_FILE}.{nonce}.previous"));
            std::fs::rename(&path, &previous)
                .with_context(|| format!("staging previous lock {}", path.display()))?;
            if let Err(error) = std::fs::rename(&tmp, &path) {
                let _ = std::fs::rename(&previous, &path);
                return Err(error).with_context(|| format!("replacing {}", path.display()));
            }
            if let Err(error) = std::fs::remove_file(&previous) {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::rename(&previous, &path);
                return Err(error).with_context(|| format!("removing {}", previous.display()));
            }
        } else {
            std::fs::rename(&tmp, &path)
                .with_context(|| format!("installing {}", path.display()))?;
        }
        Ok(path)
    }

    pub fn get(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn upsert(&mut self, package: LockedPackage) {
        match self.packages.iter_mut().find(|p| p.name == package.name) {
            Some(existing) => *existing = package,
            None => self.packages.push(package),
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<LockedPackage> {
        let index = self.packages.iter().position(|p| p.name == name)?;
        Some(self.packages.remove(index))
    }
}
