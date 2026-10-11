//! The inbox — a persisted triage queue for things that happened
//! while you were away.
//!
//! Items are files, `~/.nest/inbox/<id>.json` — same file-first rule as
//! everything else. Today the writers are the approval gate (a bot
//! asked for tools nobody approved) and routine results; readers are
//! the shell's inbox pane and `GET /api/inbox`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::hatchling::now_ms;
use crate::home::NestDirs;

/// Monotonic suffix so rapid adds in one process can't collide on ms.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// What an approval item re-runs when the user approves — enough to
/// reconstruct the dispatch that was gated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    /// `Some(bot)` re-hatches one bot; `None` re-dispatches the task.
    pub bot: Option<String>,
    pub task: String,
    /// The `ask`-permissioned tools that were withheld.
    pub asks: Vec<String>,
    /// Whether the re-run plans through the chief (chat) or the routes.
    pub chief: bool,
}

/// One inbox item — a file under `<home>/inbox/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// `"approval"` or `"note"`.
    pub kind: String,
    pub title: String,
    pub body: String,
    pub created_ms: u64,
    /// Read/acted-on — resolved items stay in the inbox history but
    /// stop demanding attention.
    #[serde(default)]
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
}

impl Item {
    /// A pending approval: `bot` asked for `asks` and nobody approved —
    /// the run went ahead with them denied, and this item offers the
    /// re-run with them allowed.
    pub fn approval(bot: Option<String>, task: &str, asks: Vec<String>, chief: bool) -> Self {
        let who = bot.clone().unwrap_or_else(|| "the brood".into());
        Self {
            id: new_id(),
            kind: "approval".into(),
            title: format!("{who} asks for: {}", asks.join(", ")),
            body: format!("task: {task}\nunapproved asks stayed denied for this run"),
            created_ms: now_ms(),
            resolved: false,
            approval: Some(Approval {
                bot,
                task: task.to_string(),
                asks,
                chief,
            }),
        }
    }

    /// A plain note — routine results, digests, things worth surfacing
    /// without an action attached.
    pub fn note(title: &str, body: &str) -> Self {
        Self {
            id: new_id(),
            kind: "note".into(),
            title: title.to_string(),
            body: body.to_string(),
            created_ms: now_ms(),
            resolved: false,
            approval: None,
        }
    }
}

fn new_id() -> String {
    format!(
        "inbox-{}-{}-{}",
        now_ms(),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    )
}

/// Filename-safe check — same rule as session ids and webhook names.
/// Every path that lands an id-derived filename goes through this.
pub fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn item_path(dirs: &NestDirs, id: &str) -> Result<PathBuf> {
    anyhow::ensure!(id_ok(id), "invalid inbox id");
    Ok(dirs.home.join("inbox").join(format!("{id}.json")))
}

/// Write an item to the inbox; returns its path.
pub fn add(dirs: &NestDirs, item: &Item) -> Result<PathBuf> {
    let dir = dirs.home.join("inbox");
    std::fs::create_dir_all(&dir)?;
    let path = item_path(dirs, &item.id)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(item)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Every inbox item, unresolved first then newest — the queue order.
pub fn list(dirs: &NestDirs) -> Vec<Item> {
    let dir = dirs.home.join("inbox");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut items: Vec<Item> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<Item>(&b).ok())
        .collect();
    items.sort_by_key(|i| (i.resolved, std::cmp::Reverse(i.created_ms)));
    items
}

/// Fetch one item by id.
pub fn get(dirs: &NestDirs, id: &str) -> Result<Item> {
    let path = item_path(dirs, id)?;
    let bytes = std::fs::read(&path).with_context(|| format!("no inbox item '{id}'"))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Mark an item resolved — it stays in history, stops demanding action.
pub fn resolve(dirs: &NestDirs, id: &str) -> Result<()> {
    let mut item = get(dirs, id)?;
    item.resolved = true;
    let path = item_path(dirs, id)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&item)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Count of unresolved items — the badge number.
pub fn pending(dirs: &NestDirs) -> usize {
    list(dirs).iter().filter(|i| !i.resolved).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(tmp: &tempfile::TempDir) -> NestDirs {
        NestDirs::resolve(tmp.path(), Some(tmp.path().join("h")))
    }

    #[test]
    fn add_list_resolve_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        let item = Item::approval(
            Some("writer".into()),
            "deploy the thing",
            vec!["bash".into()],
            false,
        );
        let path = add(&dirs, &item).unwrap();
        assert!(path.is_file());

        let items = list(&dirs);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "approval");
        assert_eq!(items[0].approval.as_ref().unwrap().asks, ["bash"]);
        assert!(!items[0].resolved);
        assert_eq!(pending(&dirs), 1);

        resolve(&dirs, &item.id).unwrap();
        assert_eq!(pending(&dirs), 0);
        // resolved items stay in the listing — history, not deletion
        assert_eq!(list(&dirs).len(), 1);
    }

    #[test]
    fn ids_are_path_safe_and_unique() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        let (a, b) = (Item::note("a", ""), Item::note("b", ""));
        assert_ne!(a.id, b.id);
        add(&dirs, &a).unwrap();
        add(&dirs, &b).unwrap();
        assert_eq!(list(&dirs).len(), 2);

        // hostile ids never become paths
        for bad in ["../x", "/abs", "a\\b", "sub/dir", "..", ".hidden", ""] {
            assert!(!id_ok(bad), "{bad:?} must be rejected");
            assert!(get(&dirs, bad).is_err());
            assert!(resolve(&dirs, bad).is_err());
        }
    }

    #[test]
    fn unresolved_sorts_first() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        let mut old = Item::note("old", "");
        old.created_ms = 1;
        let new = Item::note("new", "");
        add(&dirs, &old).unwrap();
        resolve(&dirs, &old.id).unwrap();
        add(&dirs, &new).unwrap();
        let items = list(&dirs);
        assert_eq!(items[0].title, "new");
        assert_eq!(items[1].title, "old");
    }
}
