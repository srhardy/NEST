//! patrol — routines that watch the nest. Declared automations
//! (`[[automation]]` in `automations.toml`) fire on a schedule (5-field
//! cron, UTC) or on an event (`file-change:<glob>`); each firing is a
//! normal brood dispatch, so receipts land in the roost like any run.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::home::NestDirs;
use crate::spec::{self, AutomationSpec};

/// How the patrol decides an automation is due.
#[derive(Debug)]
enum Trigger {
    /// Fires when the UTC minute matches; deduped by minute-stamp.
    Cron { spec: [Field; 5] },
    /// Fires when the newest mtime under the glob changes.
    FileChange { root: PathBuf, glob: String },
    /// Declared but unhandled — surfaced as armed-but-waiting.
    Unknown(String),
}

#[derive(Debug)]
struct Field {
    /// `x/step` resolves here; otherwise `values` holds the set.
    steps: Vec<u32>,
    values: Vec<u32>,
}

impl Field {
    fn parse(s: &str, lo: u32, hi: u32) -> Option<Field> {
        let (base, step) = match s.split_once('/') {
            Some((b, s)) => (b, s.parse::<u32>().ok()?.max(1)),
            None => (s, 0),
        };
        let mut values = Vec::new();
        if base == "*" {
            values.extend(lo..=hi);
        } else {
            for part in base.split(',') {
                if let Some((a, b)) = part.split_once('-') {
                    let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
                    if a > b || b > hi {
                        return None;
                    }
                    values.extend(a..=b);
                } else {
                    let v: u32 = part.parse().ok()?;
                    if v < lo || v > hi {
                        return None;
                    }
                    values.push(v);
                }
            }
        }
        let field = Field {
            steps: if step > 0 {
                values.iter().filter(|v| *v % step == 0).cloned().collect()
            } else {
                Vec::new()
            },
            values,
        };
        Some(field)
    }

    fn has(&self, v: u32) -> bool {
        if !self.steps.is_empty() {
            return self.steps.contains(&v);
        }
        self.values.contains(&v)
    }
}

/// Parse "min hour dom month dow" into fields. None if malformed.
fn parse_cron(expr: &str) -> Option<[Field; 5]> {
    let mut it = expr.split_whitespace();
    let m = Field::parse(it.next()?, 0, 59)?;
    let h = Field::parse(it.next()?, 0, 23)?;
    let dom = Field::parse(it.next()?, 1, 31)?;
    let mon = Field::parse(it.next()?, 1, 12)?;
    let dow = Field::parse(it.next()?, 0, 6)?;
    if it.next().is_some() {
        return None;
    }
    Some([m, h, dom, mon, dow])
}

/// (min, hour, dom, month, dow, epoch-minutes) in UTC for a unix time.
fn utc_parts(t: u64) -> (u32, u32, u32, u32, u32, u64) {
    let days = t / 86_400;
    let secs = t % 86_400;
    let (min, hour) = ((secs / 60) as u32 % 60, (secs / 3600) as u32);
    // civil-from-days (Howard Hinnant's algorithm)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let _y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dom = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mon = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let dow = ((days + 4) % 7) as u32; // 1970-01-01 was Thursday(4)
    (min, hour, dom, mon, dow, t / 60)
}

/// Newest mtime (secs) under `root` matching `glob` segments
/// (`*` = one level, `**` = any depth). None if nothing matches.
fn newest_under(root: &Path, glob: &str) -> Option<u64> {
    let segs: Vec<&str> = glob.split('/').collect();
    let mut best: Option<u64> = None;
    fn walk(dir: &Path, segs: &[&str], best: &mut Option<u64>) {
        let Some((head, rest)) = segs.split_first() else {
            return;
        };
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let ft = e.file_type().ok();
            if *head == "**" {
                if rest.is_empty() {
                    touch(&e.path(), best);
                }
                walk(&e.path(), rest, best);
                if ft.map(|f| f.is_dir()).unwrap_or(false) {
                    walk(&e.path(), segs, best);
                }
            } else if glob_match(head, &name) {
                if rest.is_empty() {
                    touch(&e.path(), best);
                } else if ft.map(|f| f.is_dir()).unwrap_or(false) {
                    walk(&e.path(), rest, best);
                }
            }
        }
    }
    fn touch(p: &Path, best: &mut Option<u64>) {
        if let Ok(m) = p.metadata().and_then(|m| m.modified()) {
            if let Ok(d) = m.duration_since(UNIX_EPOCH) {
                *best = Some(best.map_or(d.as_secs(), |b| b.max(d.as_secs())));
            }
        }
    }
    fn glob_match(pat: &str, name: &str) -> bool {
        if pat == "*" {
            return true;
        }
        if let Some(suf) = pat.strip_prefix('*') {
            return name.ends_with(suf);
        }
        pat == name
    }
    walk(root, &segs, &mut best);
    best
}

/// A live automation the patrol watches.
pub struct Duty {
    pub spec: AutomationSpec,
    trigger: Trigger,
    last_fired: AtomicU64,
    /// For FileChange: newest mtime seen (baseline set on first scan).
    seen: AtomicU64,
}

impl Duty {
    pub fn new(spec: AutomationSpec, cwd: &Path) -> Duty {
        let trigger = if let Some(cron) = &spec.schedule {
            match parse_cron(cron) {
                Some(spec) => Trigger::Cron { spec },
                None => Trigger::Unknown(format!("bad cron '{cron}'")),
            }
        } else if let Some(ev) = &spec.event {
            if let Some(glob) = ev.strip_prefix("file-change:") {
                Trigger::FileChange {
                    root: cwd.to_path_buf(),
                    glob: glob.to_string(),
                }
            } else {
                Trigger::Unknown(ev.clone())
            }
        } else {
            Trigger::Unknown("no trigger".into())
        };
        Duty {
            spec,
            trigger,
            last_fired: AtomicU64::new(0),
            seen: AtomicU64::new(u64::MAX),
        }
    }

    /// Is this duty due right now? (updates dedup state when yes)
    pub fn due(&self, now: u64) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        match &self.trigger {
            Trigger::Cron { spec } => {
                let (min, hour, dom, mon, dow, stamp) = utc_parts(now);
                let hit = spec[0].has(min)
                    && spec[1].has(hour)
                    && spec[2].has(dom)
                    && spec[3].has(mon)
                    && spec[4].has(dow);
                hit && self.last_fired.swap(stamp, SeqCst) != stamp
            }
            Trigger::FileChange { root, glob } => {
                let newest = newest_under(root, glob).unwrap_or(0);
                let seen = self.seen.load(SeqCst);
                if seen == u64::MAX {
                    self.seen.store(newest, SeqCst);
                    return false; // first scan is baseline
                }
                newest != seen && {
                    self.seen.store(newest, SeqCst);
                    self.last_fired.store(now, SeqCst);
                    true
                }
            }
            Trigger::Unknown(_) => false,
        }
    }

    /// Manual firing stamps the duty so its "last fired" is honest.
    pub fn stamp(&self, now: u64) {
        self.last_fired.store(now, std::sync::atomic::Ordering::SeqCst);
    }

    /// For the UI: what kind of trigger this is waiting on.
    pub fn kind(&self) -> String {
        match &self.trigger {
            Trigger::Cron { .. } => format!("cron: {}", self.spec.schedule.clone().unwrap()),
            Trigger::FileChange { glob, .. } => format!("file-change: {glob}"),
            Trigger::Unknown(e) => format!("unsupported trigger: {e}"),
        }
    }
}

/// Everything the patrol is watching.
pub fn load_duties(dirs: &NestDirs, cwd: &Path) -> (Vec<Duty>, Vec<String>) {
    let (autos, mut warns) = spec::load_automations(&dirs.automation_files());
    let duties = autos
        .into_iter()
        .filter(|a| {
            if !a.enabled {
                warns.push(format!("automation '{}' is disabled", a.name));
                return false;
            }
            if a.prompt.trim().is_empty() {
                warns.push(format!("automation '{}' has an empty prompt", a.name));
                return false;
            }
            true
        })
        .map(|a| Duty::new(a, cwd))
        .collect();
    (duties, warns)
}

/// Poll interval for the patrol loop.
pub const TICK: Duration = Duration::from_secs(15);

/// Seconds since epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Serde view for /api/state.
#[derive(serde::Serialize)]
pub struct DutyInfo {
    pub name: String,
    pub trigger: String,
    pub bot: Option<String>,
    pub last_fired: u64,
}

pub fn describe(duties: &[Duty]) -> Vec<DutyInfo> {
    duties
        .iter()
        .map(|d| DutyInfo {
            name: d.spec.name.clone(),
            trigger: d.kind(),
            bot: d.spec.bot.clone(),
            last_fired: d.last_fired.load(std::sync::atomic::Ordering::SeqCst),
        })
        .collect()
}

/// The patrol loop: `fire(duty)` is called for each due automation.
pub fn run(duties: &[Duty], mut fire: impl FnMut(&Duty)) {
    loop {
        let t = now();
        for d in duties {
            if d.due(t) {
                fire(d);
            }
        }
        std::thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    #[test]
    fn cron_matches_fields() {
        let c = parse_cron("*/15 9-17 * * 1-5").unwrap();
        let d = Duty {
            spec: AutomationSpec {
                name: "t".into(),
                schedule: Some("x".into()),
                event: None,
                prompt: "p".into(),
                bot: None,
                enabled: true,
            },
            trigger: Trigger::Cron { spec: c },
            last_fired: AtomicU64::new(0),
            seen: AtomicU64::new(0),
        };
        // 2026-09-17 is a Thursday (dow 4), 04:00 UTC
        let t = 1_789_617_600;
        let (min, hour, dom, mon, dow, _stamp) = utc_parts(t);
        assert_eq!((min, hour, dow), (0, 4, 4));
        // wed 2026-09-16 09:30 UTC — inside */15, 9-17, 1-5
        let t2 = 1_789_551_000;
        let (m2, h2, _d2, mo2, w2, _s2) = utc_parts(t2);
        assert_eq!((m2, h2, w2, mo2), (30, 9, 3, 9));
        assert!(d.due(t2));
        assert!(!d.due(t2), "same minute must not fire twice");
        assert!(d.due(t2 + 15 * 60), "next matching minute fires");
        let _ = (dom, mon);
    }

    #[test]
    fn bad_cron_becomes_unknown() {
        let mut s = AutomationSpec {
            name: "x".into(),
            schedule: Some("not a cron".into()),
            event: None,
            prompt: "p".into(),
            bot: None,
            enabled: true,
        };
        let d = Duty::new(s.clone(), Path::new("."));
        assert!(matches!(d.trigger, Trigger::Unknown(_)));
        s.schedule = Some("* * * * *".into());
        let d = Duty::new(s, Path::new("."));
        assert!(matches!(d.trigger, Trigger::Cron { .. }));
    }

    #[test]
    fn file_change_fires_on_new_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.rs"), "fn a(){}").unwrap();
        let mut s = AutomationSpec {
            name: "w".into(),
            schedule: None,
            event: Some("file-change:src/**".into()),
            prompt: "p".into(),
            bot: None,
            enabled: true,
        };
        let d = Duty::new(s.clone(), dir.path());
        assert!(!d.due(100), "first scan is baseline");
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(src.join("b.rs"), "fn b(){}").unwrap();
        assert!(d.due(now()), "new file fires");
        assert!(!d.due(now() + 1), "unchanged doesn't refire");
        s.event = Some("reboot".into());
        let d = Duty::new(s, dir.path());
        assert!(!d.due(now()), "unknown event never fires");
        let _ = SeqCst;
    }
}
