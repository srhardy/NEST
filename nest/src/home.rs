//! Where NEST keeps its things.
//!
//! The `.nest/` surface exists at two levels: the project directory
//! (`.nest/` next to `agent.toml`, written by Spoke) and the personal
//! directory (`~/.nest/`, where package installs land). Project wins.

use std::env;
use std::path::{Path, PathBuf};

/// The `.nest/` roots the engine reads.
#[derive(Debug, Clone)]
pub struct NestDirs {
    /// `<project>/.nest/` when the working directory has one.
    pub project: Option<PathBuf>,
    /// `~/.nest/` (or `$NEST_HOME` / `--home`), always present.
    pub home: PathBuf,
}

impl NestDirs {
    /// Resolve dirs for a working directory. `home` overrides `~/.nest`
    /// (used by `--home` / `NEST_HOME`, and by tests to sandbox).
    pub fn resolve(cwd: &Path, home: Option<PathBuf>) -> Self {
        let home = home
            .or_else(|| env::var_os("NEST_HOME").map(PathBuf::from))
            .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".nest")))
            .unwrap_or_else(|| PathBuf::from(".nest"));
        let project = cwd.join(".nest").is_dir().then(|| cwd.join(".nest"));
        Self { project, home }
    }

    /// Agent directories to scan, project first then personal.
    pub fn agent_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(p) = &self.project {
            dirs.push(p.join("agents"));
        }
        dirs.push(self.home.join("agents"));
        dirs
    }

    /// Routing table candidates (project first, personal as fallback).
    pub fn routing_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Some(p) = &self.project {
            files.push(p.join("routing.toml"));
        }
        files.push(self.home.join("routing.toml"));
        files
    }

    /// Provider table candidates — the free-first router reads these.
    pub fn provider_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Some(p) = &self.project {
            files.push(p.join("providers.toml"));
        }
        files.push(self.home.join("providers.toml"));
        files
    }

    /// Service table candidates.
    pub fn services_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Some(p) = &self.project {
            files.push(p.join("services.toml"));
        }
        files.push(self.home.join("services.toml"));
        files
    }

    /// Automation table candidates — the patrol reads these.
    pub fn automation_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Some(p) = &self.project {
            files.push(p.join("automations.toml"));
        }
        files.push(self.home.join("automations.toml"));
        files
    }

    /// Where run receipts rest: `<home>/.nest/roost/`... which is
    /// `<home>/roost/` since `home` already ends in `.nest`.
    pub fn roost(&self) -> PathBuf {
        self.home.join("roost")
    }
}
