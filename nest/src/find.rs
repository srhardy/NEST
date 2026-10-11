//! `nest find` — one query over everything the nest writes.
//!
//! No index, no daemon: the surfaces are all files, so a capped scan of
//! receipts, sessions, skills, agents, and memory is fast enough and
//! always honest about what's actually on disk.

use std::path::PathBuf;

use serde::Serialize;

use crate::home::NestDirs;

/// One match — the kind tells the UI where to open it.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    /// `"receipt"` | `"session"` | `"skill"` | `"agent"` | `"memory"`.
    pub kind: &'static str,
    /// Receipt id, session id, skill/agent name, or filename.
    pub id: String,
    /// The thing to scan for the title — task, bot, name.
    pub title: String,
    /// The first line that matched, trimmed.
    pub snippet: String,
    #[serde(skip)]
    pub path: PathBuf,
}

struct Scored(Hit, u32);

/// Search every file surface for `query` (case-insensitive substring).
/// Score: name/id match > title match > body match; unresolved order is
/// score-desc then kind. Capped at 50 hits.
pub fn find(dirs: &NestDirs, query: &str) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Scored> = Vec::new();
    receipts(dirs, &needle, &mut hits);
    sessions(dirs, &needle, &mut hits);
    markdown_dirs(dirs, &needle, &mut hits);
    hits.sort_by_key(|s| std::cmp::Reverse(s.1));
    hits.into_iter().take(50).map(|s| s.0).collect()
}

fn contains(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(needle)
}

/// The first line containing the needle, trimmed to ~140 chars.
fn snippet(text: &str, needle: &str) -> String {
    let line = text
        .lines()
        .find(|l| contains(l, needle))
        .unwrap_or_default()
        .trim();
    line.chars().take(140).collect()
}

fn push(
    hits: &mut Vec<Scored>,
    kind: &'static str,
    id: String,
    title: String,
    text: &str,
    needle: &str,
    path: PathBuf,
) {
    let score = if contains(&id, needle) {
        3
    } else if contains(&title, needle) {
        2
    } else if contains(text, needle) {
        1
    } else {
        return;
    };
    hits.push(Scored(
        Hit {
            kind,
            id,
            title,
            snippet: snippet(text, needle),
            path,
        },
        score,
    ));
}

/// Run receipts — the roost is the activity log.
fn receipts(dirs: &NestDirs, needle: &str, hits: &mut Vec<Scored>) {
    for receipt in crate::roost::receipts(&dirs.roost()).unwrap_or_default() {
        let text = receipt.tail.join("\n");
        push(
            hits,
            "receipt",
            receipt.id.clone(),
            format!("{} — {}", receipt.agent, receipt.task),
            &text,
            needle,
            dirs.roost().join(format!("{}.json", receipt.id)),
        );
    }
}

/// Chat transcripts — user + assistant message text.
fn sessions(dirs: &NestDirs, needle: &str, hits: &mut Vec<Scored>) {
    let dir = dirs.home.join("sessions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.extension().is_some_and(|x| x == "json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let id = v["id"].as_str().unwrap_or_default().to_string();
        let title = format!("{} chat", v["bot"].as_str().unwrap_or("nest"));
        let text = v["messages"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| m["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        push(hits, "session", id, title, &text, needle, path);
    }
}

/// Agent files, skills, and memory notes — all markdown, all greppable.
fn markdown_dirs(dirs: &NestDirs, needle: &str, hits: &mut Vec<Scored>) {
    // (directory, kind, title-from-filename)
    let mut surfaces: Vec<(PathBuf, &'static str)> = Vec::new();
    for agents in dirs.agent_dirs() {
        surfaces.push((agents, "agent"));
    }
    for root in [&dirs.project, &Some(dirs.home.clone())]
        .into_iter()
        .flatten()
    {
        surfaces.push((root.join("skills"), "skill"));
        surfaces.push((root.join("memory"), "memory"));
    }
    for (dir, kind) in surfaces {
        let mut stack = vec![dir];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|x| x == "md") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                push(hits, kind, id.clone(), id, &text, needle, path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hatchling::Receipt;
    use std::fs;

    fn dirs(tmp: &tempfile::TempDir) -> NestDirs {
        NestDirs::resolve(tmp.path(), Some(tmp.path().join("h")))
    }

    fn receipt() -> Receipt {
        Receipt {
            id: "bot-1-1".into(),
            agent: "writer".into(),
            perch: "echo".into(),
            task: "refactor the parser".into(),
            status: crate::hatchling::Status::Done,
            steps: 1,
            exit_code: Some(0),
            started_ms: 1,
            finished_ms: 2,
            hard_won: false,
            failure_kind: None,
            peeps_path: None,
            session: None,
            tail: vec!["rewrote parse.rs".into()],
        }
    }

    #[test]
    fn find_covers_receipts_sessions_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        crate::roost::record(&dirs.roost(), &receipt()).unwrap();
        fs::create_dir_all(dirs.home.join("sessions")).unwrap();
        fs::write(
            dirs.home.join("sessions/s-1.json"),
            r#"{"schema":1,"id":"s-1","bot":"writer","messages":[{"text":"refactor plan"}]}"#,
        )
        .unwrap();
        fs::create_dir_all(dirs.home.join("skills/refactorer")).unwrap();
        fs::write(
            dirs.home.join("skills/refactorer/SKILL.md"),
            "---\nname: refactorer\n---\nrefactors things\n",
        )
        .unwrap();
        fs::create_dir_all(dirs.home.join("agents")).unwrap();
        fs::write(
            dirs.home.join("agents/writer.md"),
            "---\nname: writer\njob: refactor code\n---\nYou write.\n",
        )
        .unwrap();
        fs::create_dir_all(dirs.home.join("memory")).unwrap();
        fs::write(
            dirs.home.join("memory/facts.md"),
            "prefers small refactors\n",
        )
        .unwrap();

        let hits = find(&dirs, "refactor");
        let kinds: Vec<&str> = hits.iter().map(|h| h.kind).collect();
        for kind in ["receipt", "session", "skill", "agent", "memory"] {
            assert!(kinds.contains(&kind), "missing {kind} hit: {kinds:?}");
        }

        // a miss returns nothing, an empty query returns nothing
        assert!(find(&dirs, "zzz-nothing").is_empty());
        assert!(find(&dirs, "  ").is_empty());
    }

    #[test]
    fn name_matches_rank_above_body_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        fs::create_dir_all(dirs.home.join("agents")).unwrap();
        fs::write(
            dirs.home.join("agents/parser.md"),
            "---\nname: parser\njob: x\n---\nYou parse.\n",
        )
        .unwrap();
        fs::write(
            dirs.home.join("agents/talker.md"),
            "---\nname: talker\njob: mentions the parser\n---\nYou talk.\n",
        )
        .unwrap();
        let hits = find(&dirs, "parser");
        assert_eq!(hits[0].id, "parser");
    }
}
