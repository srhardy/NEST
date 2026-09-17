pub mod package;
pub mod scan;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use package::{Index, PackageManifest};

/// Filesystem store for Spoke packages and the registry index.
pub struct Store {
    pub home: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Materialized {
    pub dir: PathBuf,
    pub source_description: String,
}

impl Store {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }

    pub fn packages_dir(&self) -> PathBuf {
        self.home.join(".spoke").join("packages")
    }

    pub fn registry_dir(&self) -> PathBuf {
        self.home.join(".spoke").join("registry")
    }

    pub fn index_path(&self) -> PathBuf {
        self.registry_dir().join("index.toml")
    }

    /// Resolve a spec: a local path to a package directory, or a name in the index.
    pub fn resolve(&self, spec: &str) -> Result<PackageManifest> {
        let as_path = Path::new(spec);
        if as_path.exists() {
            let manifest_path = if as_path.is_dir() {
                as_path.join("package.toml")
            } else {
                as_path.to_path_buf()
            };
            let mut manifest = PackageManifest::load(&manifest_path)?;
            let base = manifest_path
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf();
            manifest.resolve_source_path(&base);
            return Ok(manifest);
        }

        let index_path = self.index_path();
        if !index_path.exists() {
            bail!(
                "no registry index at {} — pass a local package path instead, or create an index",
                index_path.display()
            );
        }
        let index = Index::load(&index_path)?;
        let entry = index
            .find(spec)
            .ok_or_else(|| anyhow!("package '{spec}' is not in {}", index_path.display()))?;
        let manifest_path = self.registry_dir().join(&entry.manifest);
        let mut manifest = PackageManifest::load(&manifest_path)?;
        // Relative source paths resolve against the manifest's own directory,
        // exactly as they do for a package added by path.
        let base = manifest_path
            .parent()
            .unwrap_or(&self.registry_dir())
            .to_path_buf();
        manifest.resolve_source_path(&base);
        Ok(manifest)
    }

    /// Materialize a package's source into the store so installs are reproducible.
    pub fn fetch(&self, manifest: &PackageManifest) -> Result<Materialized> {
        let dest = self
            .packages_dir()
            .join(&manifest.name)
            .join(&manifest.version);
        if dest.exists() {
            fs::remove_dir_all(&dest)
                .with_context(|| format!("clearing {}", dest.display()))?;
        }
        fs::create_dir_all(&dest)
            .with_context(|| format!("creating {}", dest.display()))?;

        match (&manifest.source.path, &manifest.source.git) {
            (Some(path), _) => {
                let source = Path::new(path);
                if !source.exists() {
                    bail!("package source path does not exist: {}", source.display());
                }
                copy_dir(source, &dest)?;
                Ok(Materialized {
                    dir: dest,
                    source_description: manifest.source.describe(),
                })
            }
            (None, Some(git)) => {
                let mut command = Command::new("git");
                command.args(["clone", "--depth", "1", "--quiet"]);
                if let Some(rev) = &manifest.source.rev {
                    command.args(["--branch", rev]);
                }
                command.arg(git).arg(&dest);
                let output = command
                    .output()
                    .with_context(|| format!("running git clone for {}", git))?;
                if !output.status.success() {
                    bail!(
                        "git clone failed for {}: {}",
                        git,
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                Ok(Materialized {
                    dir: dest,
                    source_description: manifest.source.describe(),
                })
            }
            (None, None) => bail!("package '{}' has no source", manifest.name),
        }
    }

    /// (skill name, files) for every skill the package declares.
    pub fn skill_files(
        &self,
        manifest: &PackageManifest,
        source_dir: &Path,
    ) -> Result<Vec<(String, Vec<(String, String)>)>> {
        let mut out = Vec::new();
        for spec in &manifest.skills {
            let dir = source_dir.join(&spec.path);
            if !dir.exists() {
                bail!(
                    "skill '{}' path does not exist in the package source: {}",
                    spec.name,
                    dir.display()
                );
            }
            let files = collect_files(&dir)?;
            if files.is_empty() {
                bail!("skill '{}' has no files at {}", spec.name, dir.display());
            }
            out.push((spec.name.clone(), files));
        }
        Ok(out)
    }

    /// Deterministic content hash over (relative path, contents) pairs.
    pub fn integrity(files: &[(String, String)]) -> String {
        let mut sorted: Vec<&(String, String)> = files.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut hasher = Sha256::new();
        for (rel, contents) in sorted {
            hasher.update(rel.as_bytes());
            hasher.update(b"\0");
            hasher.update(contents.as_bytes());
            hasher.update(b"\0");
        }
        format!("sha256:{:x}", hasher.finalize())
    }
}

/// Estimated context cost in tokens. bytes/4 is the same heuristic rtk exposes;
/// it is labelled as an estimate everywhere it is shown.
pub fn token_estimate(files: &[(String, String)]) -> u64 {
    files.iter().map(|(_, c)| c.len() as u64 / 4).sum()
}

/// Recursively collect text files under `root`, sorted, skipping VCS/build dirs.
pub fn collect_files(root: &Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if matches!(name.as_str(), ".git" | "node_modules" | "target" | ".venv") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            match fs::read_to_string(&path) {
                Ok(contents) => out.push((rel, contents)),
                Err(_) => continue, // binary or unreadable: skipped, not installed
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    let mut stack = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = stack.pop() {
        fs::create_dir_all(&dst).with_context(|| format!("creating {}", dst.display()))?;
        for entry in fs::read_dir(&src).with_context(|| format!("reading {}", src.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), ".git" | "target") {
                continue;
            }
            let src_path = entry.path();
            let dst_path = dst.join(&name);
            if src_path.is_dir() {
                stack.push((src_path, dst_path));
            } else {
                fs::copy(&src_path, &dst_path).with_context(|| {
                    format!("copying {} -> {}", src_path.display(), dst_path.display())
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    fn fixture_package(root: &Path) -> PathBuf {
        write(
            &root.join("package.toml"),
            r#"
name = "demo-skill"
version = "0.1.0"
license = "MIT"
description = "A demo package"

[source]
path = "src"

[[skills]]
name = "demo-skill"
path = "."
"#,
        );
        write(
            &root.join("src/SKILL.md"),
            "---\nname: demo-skill\ndescription: demo\n---\n\nDo the demo thing.\n",
        );
        root.to_path_buf()
    }

    #[test]
    fn resolves_and_fetches_a_local_package() {
        let home = tempfile::tempdir().unwrap();
        let pkg_root = tempfile::tempdir().unwrap();
        let manifest_path = fixture_package(pkg_root.path());

        let store = Store::new(home.path());
        let manifest = store.resolve(manifest_path.to_str().unwrap()).unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.name, "demo-skill");
        // A relative source path is resolved against the manifest's directory.
        assert!(
            manifest.source.path.as_deref().unwrap().contains("src"),
            "relative source path was resolved: {:?}",
            manifest.source.path
        );

        let materialized = store.fetch(&manifest).unwrap();
        assert!(materialized.dir.join("SKILL.md").exists());
        assert!(materialized.source_description.starts_with("path:"));

        let skills = store.skill_files(&manifest, &materialized.dir).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].0, "demo-skill");
        assert!(skills[0].1.iter().any(|(rel, _)| rel == "SKILL.md"));
    }

    #[test]
    fn integrity_is_stable_and_content_sensitive() {
        let files = vec![
            ("b.md".to_string(), "beta".to_string()),
            ("a.md".to_string(), "alpha".to_string()),
        ];
        let first = Store::integrity(&files);
        let reordered = vec![files[1].clone(), files[0].clone()];
        assert_eq!(first, Store::integrity(&reordered), "order-independent");
        let changed = vec![
            ("a.md".to_string(), "alpha".to_string()),
            ("b.md".to_string(), "BETA".to_string()),
        ];
        assert_ne!(first, Store::integrity(&changed));
        assert!(first.starts_with("sha256:"));
    }

    #[test]
    fn token_estimate_sums_bytes_over_four() {
        let files = vec![("a.md".to_string(), "x".repeat(400))];
        assert_eq!(token_estimate(&files), 100);
    }
}
