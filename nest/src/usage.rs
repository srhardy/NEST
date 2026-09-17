//! Usage — CodexBar-style per-provider limit windows.
//!
//! Each provider with a declared `limit`/`window` gets a counter file in
//! `~/.nest/usage/<name>.json`: `{window_start, count}` — a rolling window
//! anchored at the first call. The router skips providers over their
//! limit and the picker shows "used/limit · resets at …" per row.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::spec::ProviderSpec;

/// Parse a window like "5h", "24h", "7d", "1w" into seconds.
pub fn window_secs(w: &str) -> Option<u64> {
    let (num, unit) = w.split_at(w.len().checked_sub(1)?);
    let n: u64 = num.trim().parse().ok()?;
    match unit {
        "h" => Some(n * 3600),
        "d" => Some(n * 86400),
        "w" => Some(n * 604800),
        _ => None,
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Counter {
    window_start: u64,
    count: u64,
}

/// What the picker shows: how much of this window's limit is spent.
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    pub used: u64,
    pub limit: u64,
    /// unix seconds when the current window rolls over
    pub resets_at: u64,
    /// true when the window is spent — router skips the provider
    pub exhausted: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn counter_file(home: &Path, name: &str) -> PathBuf {
    home.join("usage").join(format!("{name}.json"))
}

fn load(home: &Path, name: &str, window: u64) -> Counter {
    let path = counter_file(home, name);
    let mut c: Counter = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // window expired → fresh counter anchored now
    if now() >= c.window_start + window {
        c = Counter {
            window_start: now(),
            count: 0,
        };
    } else if c.window_start == 0 {
        c.window_start = now();
    }
    c
}

/// Current usage for a provider — None when it declares no limit.
pub fn status(home: &Path, spec: &ProviderSpec) -> Option<Usage> {
    let limit = spec.limit?;
    let window = spec
        .window
        .as_deref()
        .and_then(window_secs)
        .unwrap_or(86400);
    let c = load(home, &spec.name, window);
    Some(Usage {
        used: c.count,
        limit,
        resets_at: c.window_start + window,
        exhausted: c.count >= limit,
    })
}

/// Count one completed call against the provider's window.
pub fn record(home: &Path, spec: &ProviderSpec) -> Result<()> {
    if spec.limit.is_none() {
        return Ok(());
    }
    let window = spec
        .window
        .as_deref()
        .and_then(window_secs)
        .unwrap_or(86400);
    let mut c = load(home, &spec.name, window);
    c.count += 1;
    let path = counter_file(home, &spec.name);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string(&c)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ProviderSpec;
    use std::collections::BTreeMap;

    fn spec(limit: Option<u64>, window: Option<&str>) -> ProviderSpec {
        ProviderSpec {
            name: "p".into(),
            kind: "command".into(),
            program: Some("cat".into()),
            args: vec![],
            env: BTreeMap::new(),
            model: None,
            cost: "free".into(),
            base_url: None,
            api_key_env: None,
            limit,
            window: window.map(Into::into),
        }
    }

    #[test]
    fn windows_parse() {
        assert_eq!(window_secs("5h"), Some(18000));
        assert_eq!(window_secs("7d"), Some(604800));
        assert_eq!(window_secs("1w"), Some(604800));
        assert_eq!(window_secs("30m"), None);
        assert_eq!(window_secs(""), None);
    }

    #[test]
    fn counts_until_limit_then_exhausts_and_resets() {
        let home = tempfile::tempdir().unwrap();
        let s = spec(Some(2), Some("1h"));
        assert_eq!(status(home.path(), &s).unwrap().used, 0);
        record(home.path(), &s).unwrap();
        record(home.path(), &s).unwrap();
        let u = status(home.path(), &s).unwrap();
        assert_eq!(u.used, 2);
        assert!(u.exhausted);
        // roll the window start into the past → fresh counter
        let path = counter_file(home.path(), "p");
        std::fs::write(&path, r#"{"window_start":1,"count":5}"#).unwrap();
        let u = status(home.path(), &s).unwrap();
        assert_eq!(u.used, 0);
        assert!(!u.exhausted);
    }

    #[test]
    fn no_limit_means_no_usage() {
        let home = tempfile::tempdir().unwrap();
        let s = spec(None, None);
        assert!(status(home.path(), &s).is_none());
        record(home.path(), &s).unwrap();
        assert!(!counter_file(home.path(), "p").exists());
    }
}
