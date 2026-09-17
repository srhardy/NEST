//! Usage — CodexBar-style per-provider limit windows.
//!
//! Each provider with a declared `limit`/`window` gets a counter file in
//! `~/.nest/usage/<name>.json`: `{window_start, count}` — a rolling window
//! anchored at the first call. The router skips providers over their
//! limit and the picker shows "used/limit · resets at …" per row.
//!
//! Reservations are atomic across processes: `try_reserve` takes the
//! check-and-increment under a lockfile before the provider is ever
//! called, so two `nest` processes can't both slip past `count = limit`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Provider names become filenames — anything else becomes `_`, so a
/// crafted `../name` can't escape the usage dir.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn counter_file(home: &Path, name: &str) -> PathBuf {
    home.join("usage").join(format!("{}.json", sanitize(name)))
}

fn lock_file(home: &Path, name: &str) -> PathBuf {
    home.join("usage").join(format!("{}.lock", sanitize(name)))
}

/// O_EXCL lockfile — atomic across processes on every platform we
/// support (a plain file, removed on drop).
struct Lock(PathBuf);
impl Lock {
    /// Spin briefly: reservations are milliseconds, never seconds.
    fn take(path: PathBuf) -> Result<Option<Lock>> {
        for _ in 0..300 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Some(Lock(path))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => return Err(anyhow::anyhow!("cannot lock {}: {e}", path.display())),
            }
        }
        Ok(None)
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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

/// Write-then-rename so a torn write can't corrupt the counter.
fn store(home: &Path, name: &str, c: &Counter) -> Result<()> {
    let path = counter_file(home, name);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string(c)?.as_bytes())?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
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

/// Reserve one call against the provider's window — the atomic
/// check-and-increment the router runs *before* calling the provider.
/// Returns `false` when the window is already spent. A failed provider
/// call still consumes its reservation: the counter tracks attempts,
/// which is the honest rate-limit model.
///
/// `Err` means the counter couldn't be trusted (lock contention or an
/// IO failure) — the caller treats it as a provider failure, never as
/// a free pass.
pub fn try_reserve(home: &Path, spec: &ProviderSpec) -> Result<bool> {
    let Some(limit) = spec.limit else {
        return Ok(true); // unlimited — nothing to reserve
    };
    let window = spec
        .window
        .as_deref()
        .and_then(window_secs)
        .unwrap_or(86400);
    std::fs::create_dir_all(lock_file(home, &spec.name).parent().unwrap())?;
    let Some(_lock) = Lock::take(lock_file(home, &spec.name))? else {
        anyhow::bail!("usage counter for '{}' is contended", spec.name);
    };
    let mut c = load(home, &spec.name, window);
    if c.count >= limit {
        return Ok(false);
    }
    c.count += 1;
    store(home, &spec.name, &c)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ProviderSpec;
    use std::collections::BTreeMap;

    fn spec(limit: Option<u64>, window: Option<&str>) -> ProviderSpec {
        spec_named("p", limit, window)
    }

    fn spec_named(name: &str, limit: Option<u64>, window: Option<&str>) -> ProviderSpec {
        ProviderSpec {
            name: name.into(),
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
    fn reserves_until_limit_then_refuses() {
        let home = tempfile::tempdir().unwrap();
        let s = spec(Some(2), Some("1h"));
        assert!(try_reserve(home.path(), &s).unwrap());
        assert!(try_reserve(home.path(), &s).unwrap());
        assert!(!try_reserve(home.path(), &s).unwrap());
        let u = status(home.path(), &s).unwrap();
        assert_eq!(u.used, 2);
        assert!(u.exhausted);
        // roll the window start into the past → fresh counter
        std::fs::write(
            counter_file(home.path(), "p"),
            r#"{"window_start":1,"count":5}"#,
        )
        .unwrap();
        assert!(try_reserve(home.path(), &s).unwrap());
        assert!(!status(home.path(), &s).unwrap().exhausted);
    }

    #[test]
    fn no_limit_means_no_tracking() {
        let home = tempfile::tempdir().unwrap();
        let s = spec(None, None);
        assert!(status(home.path(), &s).is_none());
        assert!(try_reserve(home.path(), &s).unwrap());
        assert!(!counter_file(home.path(), "p").exists());
    }

    #[test]
    fn sneaky_names_stay_inside_usage() {
        let home = tempfile::tempdir().unwrap();
        let s = spec_named("../../etc/evil", Some(1), None);
        try_reserve(home.path(), &s).unwrap();
        assert!(counter_file(home.path(), "../../etc/evil")
            .to_string_lossy()
            .contains("usage"));
        assert!(home.path().join("usage").join(".._.._etc_evil.json").exists());
    }
}
