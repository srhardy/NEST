pub mod package;
pub mod scan;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use package::{ensure_contained, safe_relative_path, Index, PackageManifest};

/// Filesystem store for Spoke packages and the registry index.
pub struct Store {
    home: PathBuf,
}

struct StagingCleanup(PathBuf);

impl Drop for StagingCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub type PackageFiles = Vec<(String, Vec<(String, String)>)>;

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
            let manifest_path = fs::canonicalize(&manifest_path)
                .with_context(|| format!("resolving {}", manifest_path.display()))?;
            let mut manifest = PackageManifest::load(&manifest_path)?;
            let base = manifest_path
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf();
            manifest.resolve_source_path(&base)?;
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
        index.validate()?;
        let entry = index
            .find(spec)
            .ok_or_else(|| anyhow!("package '{spec}' is not in {}", index_path.display()))?;
        let registry_root = fs::canonicalize(self.registry_dir())?;
        let relative_manifest = safe_relative_path(&entry.manifest)?;
        let manifest_path = registry_root.join(relative_manifest);
        let manifest_path = ensure_contained(&registry_root, &manifest_path, true)?;
        let mut manifest = PackageManifest::load(&manifest_path)?;
        if manifest.name != entry.name {
            bail!(
                "registry entry '{}' points to package '{}'",
                entry.name,
                manifest.name
            );
        }
        manifest.validate()?;
        // Relative source paths resolve against the manifest's own directory,
        // exactly as they do for a package added by path.
        let base = manifest_path
            .parent()
            .unwrap_or(&registry_root)
            .to_path_buf();
        manifest.resolve_source_path(&base)?;
        Ok(manifest)
    }

    /// Materialize into a staging directory; preserve the previous cache until
    /// the new copy/clone has completed successfully.
    pub fn fetch(&self, manifest: &PackageManifest) -> Result<Materialized> {
        manifest.validate()?;
        let packages_root = self.packages_dir();
        fs::create_dir_all(&packages_root)
            .with_context(|| format!("creating {}", packages_root.display()))?;
        let packages_root_canonical = fs::canonicalize(&packages_root)?;
        let package_root = packages_root.join(&manifest.name);
        if package_root.exists() {
            ensure_contained(
                &packages_root_canonical,
                &fs::canonicalize(&package_root)?,
                true,
            )?;
        } else {
            fs::create_dir_all(&package_root)
                .with_context(|| format!("creating {}", package_root.display()))?;
        }
        let package_root_canonical = fs::canonicalize(&package_root)?;
        let dest = package_root.join(&manifest.version);
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let staging = package_root.join(format!("staging-{nonce}"));
        let _staging_cleanup = StagingCleanup(staging.clone());
        let previous = package_root.join(format!("previous-{nonce}"));
        match (&manifest.source.path, &manifest.source.git) {
            (Some(path), _) => {
                let source = Path::new(path);
                if !source.exists() {
                    bail!("package source path does not exist: {}", source.display());
                }
                let source = fs::canonicalize(source)?;
                fs::create_dir(&staging)
                    .with_context(|| format!("creating {}", staging.display()))?;
                copy_dir(&source, &staging)?;
            }
            (None, Some(git)) => {
                let rev = manifest.source.rev.as_deref().unwrap_or_default();
                let sha_pin =
                    !rev.is_empty() && rev.len() >= 7 && rev.chars().all(|c| c.is_ascii_hexdigit());
                let output = if sha_pin {
                    Command::new("git")
                        .args(["clone", "--quiet"])
                        .arg(git)
                        .arg(&staging)
                        .output()
                        .with_context(|| format!("running git clone for {git}"))?
                } else {
                    let mut command = Command::new("git");
                    command.args(["clone", "--depth", "1", "--quiet"]);
                    if !rev.is_empty() {
                        command.args(["--branch", rev]);
                    }
                    command.arg(git).arg(&staging);
                    command
                        .output()
                        .with_context(|| format!("running git clone for {git}"))?
                };
                if !output.status.success() {
                    let _ = fs::remove_dir_all(&staging);
                    bail!(
                        "git clone failed for {}: {}",
                        git,
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                if sha_pin {
                    let output = Command::new("git")
                        .arg("-C")
                        .arg(&staging)
                        .args(["checkout", "--quiet", rev])
                        .output()
                        .with_context(|| format!("checking out {rev}"))?;
                    if !output.status.success() {
                        let _ = fs::remove_dir_all(&staging);
                        bail!(
                            "cannot pin {} to {}: {}",
                            git,
                            rev,
                            String::from_utf8_lossy(&output.stderr).trim()
                        );
                    }
                }
            }
            (None, None) => bail!("package '{}' has no source", manifest.name),
        }
        if dest.exists() {
            ensure_contained(&package_root_canonical, &fs::canonicalize(&dest)?, true)?;
            fs::rename(&dest, &previous)
                .with_context(|| format!("staging existing cache {}", dest.display()))?;
        }
        if let Err(error) = fs::rename(&staging, &dest) {
            if previous.exists() {
                let _ = fs::rename(&previous, &dest);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(error)
                .with_context(|| format!("installing package cache {}", dest.display()));
        }
        if previous.exists() {
            fs::remove_dir_all(&previous)
                .with_context(|| format!("removing previous cache {}", previous.display()))?;
        }
        Ok(Materialized {
            dir: dest,
            source_description: manifest.source.describe(),
        })
    }

    /// (skill name, files) for every skill the package declares.
    pub fn skill_files(
        &self,
        manifest: &PackageManifest,
        source_dir: &Path,
    ) -> Result<PackageFiles> {
        manifest.validate()?;
        let source_dir = fs::canonicalize(source_dir)?;
        let mut out = Vec::new();
        for spec in &manifest.skills {
            let rel = safe_relative_path(&spec.path)?;
            let dir = ensure_contained(&source_dir, &source_dir.join(rel), true)?;
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

    /// (agent name, files) for every agent the package declares. An agent's
    /// path may be a directory (collected recursively) or a single .md file.
    pub fn agent_files(
        &self,
        manifest: &PackageManifest,
        source_dir: &Path,
    ) -> Result<PackageFiles> {
        manifest.validate()?;
        let source_dir = fs::canonicalize(source_dir)?;
        let mut out = Vec::new();
        for spec in &manifest.agents {
            let rel = safe_relative_path(&spec.path)?;
            let path = ensure_contained(&source_dir, &source_dir.join(rel), true)?;
            if !path.exists() {
                bail!(
                    "agent '{}' path does not exist in the package source: {}",
                    spec.name,
                    path.display()
                );
            }
            let files = if path.is_dir() {
                collect_files(&path)?
            } else {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("{}.md", spec.name));
                let contents = fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                vec![(name, contents)]
            };
            if files.is_empty() {
                bail!("agent '{}' has no files at {}", spec.name, path.display());
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
/// Symlinks are rejected rather than followed so package content cannot read
/// outside its declared root.
pub fn collect_files(root: &Path) -> Result<Vec<(String, String)>> {
    let root = fs::canonicalize(root).with_context(|| format!("resolving {}", root.display()))?;
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                bail!(
                    "symlinks are not allowed in package content: {}",
                    path.display()
                );
            }
            if file_type.is_dir() {
                if matches!(name.as_str(), ".git" | "node_modules" | "target" | ".venv") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .with_context(|| format!("path {} escaped package root", path.display()))?
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
    let from = fs::canonicalize(from).with_context(|| format!("resolving {}", from.display()))?;
    let to = to.to_path_buf();
    let mut stack = vec![(from, to.clone())];
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
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                bail!(
                    "symlinks are not allowed in package source: {}",
                    src_path.display()
                );
            }
            if file_type.is_dir() {
                stack.push((src_path, dst_path));
            } else if file_type.is_file() {
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

    fn toml_string(value: &str) -> String {
        toml::Value::String(value.to_string()).to_string()
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

    /// A real git repo holding a package, with two commits — pinning a sha
    /// must land the older revision's content.
    fn git_package_repo(dir: &Path) -> String {
        fixture_package(dir);
        let commit = |msg: &str| {
            let run = |args: &[&str]| {
                let status = std::process::Command::new("git")
                    .arg("-C")
                    .arg(dir)
                    .args(args)
                    .status()
                    .unwrap();
                assert!(status.success(), "git {args:?} failed");
            };
            run(&["add", "-A"]);
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(["commit", "-qm", msg])
                .status()
                .unwrap();
            assert!(status.success());
        };
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(dir)
            .status()
            .unwrap();
        assert!(status.success());
        commit("v1");
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let v1 = String::from_utf8(out.stdout).unwrap().trim().to_string();
        write(
            &dir.join("src/SKILL.md"),
            "---\nname: demo-skill\ndescription: v2\n---\n\nv2 body.\n",
        );
        commit("v2");
        v1
    }

    #[test]
    fn fetches_a_git_package_pinned_to_a_sha() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let sha = git_package_repo(repo.path());
        let store = Store::new(home.path());
        let manifest: PackageManifest = toml::from_str(&format!(
            "name = \"demo-skill\"\nversion = \"0.1.0\"\n\n\
             [source]\ngit = {}\nrev = {}\n\n\
             [[skills]]\nname = \"demo-skill\"\npath = \".\"\n",
            toml_string(&repo.path().display().to_string()),
            toml_string(&sha)
        ))
        .unwrap();
        let materialized = store.fetch(&manifest).unwrap();
        let body = std::fs::read_to_string(materialized.dir.join("src/SKILL.md")).unwrap();
        assert!(
            body.contains("demo"),
            "pinned checkout lost content: {body}"
        );
        assert!(!body.contains("v2"), "pin did not land the old sha: {body}");
    }

    #[test]
    fn fetches_a_git_package_pinned_to_a_branch() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        git_package_repo(repo.path());
        let store = Store::new(home.path());
        let branch = String::from_utf8(
            std::process::Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        let manifest: PackageManifest = toml::from_str(&format!(
            "name = \"demo-skill\"\nversion = \"0.1.0\"\n\n\
             [source]\ngit = {}\nrev = {}\n\n\
             [[skills]]\nname = \"demo-skill\"\npath = \".\"\n",
            toml_string(&repo.path().display().to_string()),
            toml_string(&branch)
        ))
        .unwrap();
        let materialized = store.fetch(&manifest).unwrap();
        let body = std::fs::read_to_string(materialized.dir.join("src/SKILL.md")).unwrap();
        assert!(body.contains("v2"), "branch pin missed latest: {body}");
    }

    #[test]
    fn registry_manifest_path_cannot_escape_registry_root() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path());
        let registry = store.registry_dir();
        let outside = registry.parent().unwrap().join("outside");
        fs::create_dir_all(&outside).unwrap();
        write(
            &outside.join("package.toml"),
            "name = \"escape\"\nversion = \"1\"\n\n[source]\npath = \".\"\n\n[[skills]]\nname = \"x\"\n",
        );
        fs::create_dir_all(&registry).unwrap();
        write(
            &registry.join("index.toml"),
            "[[package]]\nname = \"escape\"\nmanifest = \"../outside/package.toml\"\n",
        );
        assert!(store.resolve("escape").is_err());
        assert!(outside.join("package.toml").is_file());
    }

    #[test]
    fn registry_index_identity_must_match_package_manifest() {
        let home = tempfile::tempdir().unwrap();
        let registry = Store::new(home.path()).registry_dir();
        write(
            &registry.join("index.toml"),
            "[[package]]\nname = \"listed\"\nmanifest = \"pkg/package.toml\"\n",
        );
        write(
            &registry.join("pkg/package.toml"),
            "name = \"different\"\nversion = \"1\"\n\n[source]\npath = \"src\"\n\n[[skills]]\nname = \"x\"\n",
        );
        fs::create_dir_all(registry.join("pkg/src")).unwrap();
        assert!(Store::new(home.path()).resolve("listed").is_err());
    }

    #[test]
    fn failed_fetch_preserves_previous_cached_package() {
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let store = Store::new(home.path());
        let manifest = PackageManifest {
            name: "stable-package".into(),
            version: "1.0.0".into(),
            description: String::new(),
            license: None,
            tier: None,
            source: package::Source {
                path: Some(source.path().to_string_lossy().to_string()),
                git: None,
                rev: None,
            },
            skills: vec![package::SkillSpec {
                name: "stable-skill".into(),
                path: ".".into(),
            }],
            agents: vec![],
            requires: package::Requires::default(),
        };
        write(&source.path().join("first.txt"), "valid source");
        let cached = store.fetch(&manifest).unwrap();
        write(&cached.dir.join("sentinel.txt"), "old cache");
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("secret.txt"), "secret");
        let link = source.path().join("external");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        match std::os::windows::fs::symlink_dir(outside.path(), &link) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(1314) => return,
            Err(e) => panic!("cannot create test symlink: {e}"),
        }
        assert!(store.fetch(&manifest).is_err());
        assert_eq!(
            fs::read_to_string(cached.dir.join("sentinel.txt")).unwrap(),
            "old cache"
        );
        let package_root = cached.dir.parent().unwrap();
        assert!(!fs::read_dir(package_root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with("staging-")));
    }

    #[test]
    fn package_collection_rejects_external_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("secret.txt"), "secret-sentinel");
        let link = root.path().join("external");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        match std::os::windows::fs::symlink_dir(outside.path(), &link) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(1314) => return,
            Err(e) => panic!("cannot create test symlink: {e}"),
        }
        assert!(collect_files(root.path()).is_err());
        assert_eq!(
            fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
            "secret-sentinel"
        );
    }

    #[test]
    fn local_package_copy_rejects_symlink_and_leaves_external_sentinel() {
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("secret.txt"), "secret-sentinel");
        let link = source.path().join("external");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        match std::os::windows::fs::symlink_dir(outside.path(), &link) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(1314) => return,
            Err(e) => panic!("cannot create test symlink: {e}"),
        }
        let manifest = PackageManifest {
            name: "safe".into(),
            version: "1.0".into(),
            description: String::new(),
            license: None,
            tier: None,
            source: package::Source {
                path: Some(source.path().to_string_lossy().to_string()),
                git: None,
                rev: None,
            },
            skills: vec![package::SkillSpec {
                name: "safe-skill".into(),
                path: ".".into(),
            }],
            agents: vec![],
            requires: package::Requires::default(),
        };
        // `fetch` accepts a resolved path source; the package walker must reject its link.
        let materialized = Store::new(home.path()).fetch(&manifest).unwrap();
        assert!(collect_files(&materialized.dir).is_err());
        assert_eq!(
            fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
            "secret-sentinel"
        );
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
