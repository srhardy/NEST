use std::path::{Path, PathBuf};

/// Spoke's runtime store (backups, event log, caches).
pub fn spoke_home(home: &Path) -> PathBuf {
    home.join(".spoke")
}

pub fn backups_dir(home: &Path, unix_secs: u64) -> PathBuf {
    spoke_home(home).join("backups").join(unix_secs.to_string())
}

/// Where a harness keeps its user-level configuration, if known.
pub fn harness_root(home: &Path, harness_id: &str) -> Option<PathBuf> {
    match harness_id {
        "claude-code" => Some(home.join(".claude")),
        "codex" => Some(home.join(".codex")),
        "gemini" => Some(home.join(".gemini")),
        "opencode" => Some(home.join(".config").join("opencode")),
        _ => None,
    }
}

pub fn flatten_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "__")
}

/// Readable backup filename: relative to `base` when the file lives under it.
pub fn backup_file_name(path: &Path, base: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(relative) => flatten_path(relative),
        Err(_) => flatten_path(path),
    }
}
