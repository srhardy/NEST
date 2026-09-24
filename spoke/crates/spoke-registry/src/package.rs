use std::path::{Component, Path, PathBuf};

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
    pub fn resolve_source_path(&mut self, manifest_dir: &Path) -> Result<()> {
        if let Some(path) = &self.source.path {
            let candidate = safe_relative_path(path)?;
            let manifest_dir = std::fs::canonicalize(manifest_dir).with_context(|| {
                format!("resolving package directory {}", manifest_dir.display())
            })?;
            let resolved = manifest_dir.join(candidate);
            let resolved = ensure_contained(&manifest_dir, &resolved, true)?;
            self.source.path = Some(resolved.to_string_lossy().to_string());
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        validate_component(&self.name, "package name")?;
        validate_component(&self.version, "package version")?;
        self.source.validate()?;
        for (kind, specs) in [("skill", &self.skills), ("agent", &self.agents)] {
            let mut names = std::collections::BTreeSet::new();
            for spec in specs {
                validate_component(&spec.name, &format!("{kind} name"))?;
                safe_relative_path(&spec.path)?;
                if !names.insert(&spec.name) {
                    bail!(
                        "package '{}' declares duplicate {kind} '{}'",
                        self.name,
                        spec.name
                    );
                }
            }
        }
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

/// Package paths use portable `/`-separated relative syntax only.
pub fn safe_relative_path(raw: &str) -> Result<PathBuf> {
    if raw.trim().is_empty() || raw.contains('\\') || raw.contains(':') {
        bail!("unsafe relative path '{raw}'");
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        bail!("absolute path '{raw}' is not allowed");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("path '{raw}' escapes its package root")
            }
        }
    }
    Ok(path.to_path_buf())
}

/// Names become a single target directory, never an arbitrary path.
pub fn validate_component(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        bail!("unsafe {label} '{value}' — expected one path component");
    }
    Ok(())
}

/// Ensure a resolved path remains beneath root and contains no symlink
/// components. Package paths are not permitted to redirect outside their tree.
pub fn ensure_contained(root: &Path, candidate: &Path, must_exist: bool) -> Result<PathBuf> {
    let root_abs = std::fs::canonicalize(root)
        .with_context(|| format!("resolving package root {}", root.display()))?;
    let relative = candidate.strip_prefix(root).with_context(|| {
        format!(
            "path {} is outside package root {}",
            candidate.display(),
            root.display()
        )
    })?;
    let mut resolved = root_abs.clone();
    for component in relative.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            _ => bail!("path {} escapes package root", candidate.display()),
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&resolved) {
            if metadata.file_type().is_symlink() {
                bail!(
                    "symlink package path is not allowed: {}",
                    resolved.display()
                );
            }
            let actual = std::fs::canonicalize(&resolved)
                .with_context(|| format!("resolving package path {}", resolved.display()))?;
            if !actual.starts_with(&root_abs) {
                bail!(
                    "path {} escapes package root {}",
                    candidate.display(),
                    root.display()
                );
            }
            resolved = actual;
        }
    }
    if must_exist && !resolved.exists() {
        bail!("package path does not exist: {}", candidate.display());
    }
    if !resolved.starts_with(&root_abs) {
        bail!(
            "path {} escapes package root {}",
            candidate.display(),
            root.display()
        );
    }
    Ok(resolved)
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

    pub fn validate(&self) -> Result<()> {
        let mut names = std::collections::BTreeSet::new();
        for entry in &self.packages {
            validate_component(&entry.name, "registry package name")?;
            safe_relative_path(&entry.manifest)?;
            if !names.insert(&entry.name) {
                bail!("registry index declares duplicate package '{}'", entry.name);
            }
        }
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&IndexEntry> {
        self.packages.iter().find(|entry| entry.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_reject_traversal_absolute_and_windows_forms() {
        for value in [
            "../outside",
            "a/../../outside",
            "/outside",
            "C:/outside",
            "C:\\outside",
            "\\\\server\\share",
            "a\\b",
        ] {
            assert!(safe_relative_path(value).is_err(), "accepted {value:?}");
        }
        assert_eq!(
            safe_relative_path("./src/skill").unwrap(),
            PathBuf::from("./src/skill")
        );
    }

    #[test]
    fn components_reject_paths_and_empty_names() {
        for value in ["", "  ", ".", "..", "a/b", "a\\b", "C:"] {
            assert!(
                validate_component(value, "name").is_err(),
                "accepted {value:?}"
            );
        }
        assert!(validate_component("starter-bots", "name").is_ok());
    }

    #[test]
    fn registry_index_rejects_traversal_and_duplicate_names() {
        let bad_path = Index {
            packages: vec![IndexEntry {
                name: "safe".into(),
                manifest: "../outside/package.toml".into(),
            }],
        };
        assert!(bad_path.validate().is_err());

        let duplicate = Index {
            packages: vec![
                IndexEntry {
                    name: "same".into(),
                    manifest: "a/package.toml".into(),
                },
                IndexEntry {
                    name: "same".into(),
                    manifest: "b/package.toml".into(),
                },
            ],
        };
        assert!(duplicate.validate().is_err());
    }
}
