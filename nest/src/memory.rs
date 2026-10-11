//! The memory — files, not a database.
//!
//! `~/.nest/memory/` is the personal store; `.nest/memory/` is the
//! project's (project shadows personal, same precedence as agents).
//! Inside each: `facts.md` is one learned fact per line with `# <run-id>`
//! provenance trailing; `user_profile.md` is hand-editable; `archive.md`
//! receives whatever reflect retires; `daily/` holds reflect digests.
//!
//! Writers: post-run extraction (a forage distill of each receipt),
//! hand edits, `nest reflect`. Reader: `build_prompt`'s `## Memory`
//! block — every hatch sees what the nest knows.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::forage::{ForageRequest, Router};
use crate::hatchling::{Receipt, Status};
use crate::home::NestDirs;

/// Hard cap on the memory block injected into prompts — memory must
/// never swallow the task.
pub const PROMPT_CAP: usize = 4096;

/// The extraction prompt — durable facts only. The model returns one
/// fact per line or the word NONE; anything else is ignored.
const EXTRACT_PROMPT: &str = "You are extracting durable facts worth remembering from a finished \
task run. A fact is a stable preference, decision, convention, or piece \
of knowledge that would help future runs — NOT a log of what happened. \
Good: \"the user prefers terse commit messages\", \"the API lives in \
src/server.rs\", \"tests run via cargo nextest\". Bad: \"the run \
finished\", \"three files were edited\".\n\
Reply with one fact per line, plain text, no bullets or numbers. If \
nothing is worth remembering, reply with the single word NONE.\n\n";

/// Memory directories, project first then personal — the precedence
/// every other nest file follows.
fn memory_dirs(dirs: &NestDirs) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = &dirs.project {
        out.push(p.join("memory"));
    }
    out.push(dirs.home.join("memory"));
    out
}

/// The personal memory dir — where new facts and digests always land.
fn home_memory(dirs: &NestDirs) -> PathBuf {
    dirs.home.join("memory")
}

/// Fact lines from one `facts.md` — blanks and comments skipped.
fn read_facts(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        // fact lines never start with '#' — those are comments
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Normalized key for dedup: the fact text without its provenance tail,
/// lowercase, whitespace collapsed.
fn normalized(fact: &str) -> String {
    let body = fact
        .rsplit_once('#')
        .map(|(body, _)| body)
        .unwrap_or(fact)
        .trim();
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The `## Memory` block for `build_prompt`: the hand-editable profile
/// plus deduped facts, project first. `None` when there's nothing to say.
pub fn prompt_block(dirs: &NestDirs) -> Option<String> {
    let profile =
        std::fs::read_to_string(home_memory(dirs).join("user_profile.md")).unwrap_or_default();
    let profile = profile.trim();

    let mut seen = std::collections::BTreeSet::new();
    let mut facts = Vec::new();
    for dir in memory_dirs(dirs) {
        for fact in read_facts(&dir.join("facts.md")) {
            if seen.insert(normalized(&fact)) {
                facts.push(fact);
            }
        }
    }

    if profile.is_empty() && facts.is_empty() {
        return None;
    }
    let mut block = String::new();
    if !profile.is_empty() {
        block.push_str("### About the user\n\n");
        block.push_str(profile);
        block.push('\n');
    }
    if !facts.is_empty() {
        block.push_str("\n### What the nest has learned\n\n");
        for fact in facts {
            block.push_str(&format!("- {fact}\n"));
        }
    }
    if block.len() > PROMPT_CAP {
        block.truncate(PROMPT_CAP);
        block.push_str("\n…memory truncated\n");
    }
    Some(block)
}

/// Append facts to the personal `facts.md`, each stamped `# <run-id>`.
/// Deduped against what's already known (normalized text); returns how
/// many actually landed.
pub fn append_facts(dirs: &NestDirs, facts: &[String], run_id: &str) -> Result<usize> {
    let dir = home_memory(dirs);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("facts.md");
    let mut known: std::collections::BTreeSet<String> =
        read_facts(&path).iter().map(|f| normalized(f)).collect();
    let mut lines = String::new();
    let mut added = 0;
    for fact in facts {
        let fact = fact.trim().trim_start_matches(['-', '*', '•', ' ']).trim();
        if fact.is_empty() || !known.insert(normalized(fact)) {
            continue;
        }
        lines.push_str(&format!("{fact} # {run_id}\n"));
        added += 1;
    }
    if added == 0 {
        return Ok(0);
    }
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(&lines);
    std::fs::write(&path, existing)?;
    Ok(added)
}

/// Distill one receipt into candidate facts via the cheapest provider
/// and append what survives dedup. Echo-provider homes skip silently —
/// a reflected prompt is garbage, not knowledge. `Ok(0)` when nothing
/// was worth remembering.
pub fn extract(dirs: &NestDirs, receipt: &Receipt) -> Result<usize> {
    let router = Router::from_dirs(dirs);
    if !router
        .list()
        .iter()
        .any(|p| p.name != "echo" && p.available && !p.exhausted)
    {
        return Ok(0);
    }
    let prompt = format!(
        "{EXTRACT_PROMPT}task: {}\nbot: {}\nstatus: {:?}\noutput tail:\n{}",
        receipt.task,
        receipt.agent,
        receipt.status,
        receipt.tail.join("\n")
    );
    let res = router.complete(&ForageRequest {
        prompt,
        model: None,
        tier: None,
    })?;
    if res.provider == "echo" {
        return Ok(0);
    }
    let text = res.text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") {
        return Ok(0);
    }
    let facts: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("none"))
        .map(str::to_string)
        .collect();
    append_facts(dirs, &facts, &receipt.id)
}

/// Fire-and-forget extraction for the post-receipt call sites — memory
/// work never blocks a run. Skips receipts that weren't Done: a failure
/// is a skill's job (feather), not a fact's.
pub fn extract_async(dirs: NestDirs, receipt: Receipt) {
    if receipt.status != Status::Done {
        return;
    }
    std::thread::spawn(move || {
        let _ = extract(&dirs, &receipt);
    });
}

/// What a reflect pass did.
#[derive(Debug)]
pub struct ReflectReport {
    /// The digest file written under `memory/daily/`.
    pub digest: PathBuf,
    pub receipts_seen: usize,
    pub facts_kept: usize,
    /// Duplicates retired to `archive.md`.
    pub facts_archived: usize,
}

/// `nest reflect` — a digest of recent runs plus fact housekeeping.
/// Fully deterministic: receipts are summarized by status and bot, and
/// `facts.md` is deduped by normalized text with the losers archived,
/// never deleted. The digest lands as an inbox note too — it exists to
/// be seen.
pub fn reflect(dirs: &NestDirs) -> Result<ReflectReport> {
    let dir = home_memory(dirs);
    std::fs::create_dir_all(dir.join("daily"))?;

    // --- prune: dedup facts.md, losers to archive.md ---
    let facts_path = dir.join("facts.md");
    let facts = read_facts(&facts_path);
    let mut seen = std::collections::BTreeSet::new();
    let mut kept = Vec::new();
    let mut archived = Vec::new();
    for fact in facts {
        if seen.insert(normalized(&fact)) {
            kept.push(fact);
        } else {
            archived.push(fact);
        }
    }
    if !archived.is_empty() {
        let mut archive = std::fs::read_to_string(dir.join("archive.md")).unwrap_or_default();
        if !archive.is_empty() && !archive.ends_with('\n') {
            archive.push('\n');
        }
        for fact in &archived {
            archive.push_str(&format!("{fact}\n"));
        }
        std::fs::write(dir.join("archive.md"), archive)?;
        std::fs::write(
            &facts_path,
            kept.iter().map(|f| format!("{f}\n")).collect::<String>(),
        )?;
    }

    // --- digest: the roost's recent story, by status and bot ---
    let receipts = crate::roost::receipts(&dirs.roost()).unwrap_or_default();
    let recent: Vec<&Receipt> = receipts.iter().take(50).collect();
    let mut by_status: std::collections::BTreeMap<String, usize> = Default::default();
    let mut by_bot: std::collections::BTreeMap<String, usize> = Default::default();
    let mut failures = Vec::new();
    for r in &recent {
        let status = serde_json::to_value(&r.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", r.status));
        *by_status.entry(status).or_default() += 1;
        *by_bot.entry(r.agent.clone()).or_default() += 1;
        if let Some(kind) = &r.failure_kind {
            failures.push(format!("- {} ({kind:?}): {}", r.agent, r.task));
        }
    }
    let stamp = crate::hatchling::now_ms();
    let mut md = format!("# reflect — {stamp}\n\n");
    md.push_str(&format!("{} recent runs\n", recent.len()));
    for (status, n) in &by_status {
        md.push_str(&format!("- {status}: {n}\n"));
    }
    md.push_str("\n## by bot\n");
    for (bot, n) in &by_bot {
        md.push_str(&format!("- {bot}: {n} runs\n"));
    }
    if !failures.is_empty() {
        md.push_str("\n## failures worth a look\n");
        for f in &failures {
            md.push_str(&format!("{f}\n"));
        }
    }
    md.push_str(&format!(
        "\n## memory\n\n- {} facts kept, {} archived\n",
        kept.len(),
        archived.len()
    ));
    let digest = dir.join("daily").join(format!("{stamp}.md"));
    std::fs::write(&digest, &md).context("cannot write reflect digest")?;

    // the digest exists to be seen — drop it in the inbox
    let summary = format!(
        "{} recent runs · {} facts kept · {} archived",
        recent.len(),
        kept.len(),
        archived.len()
    );
    let item = crate::inbox::Item::note(
        "reflect digest",
        &format!("{summary}\nfile: {}", digest.display()),
    );
    crate::inbox::add(dirs, &item).ok();

    Ok(ReflectReport {
        digest,
        receipts_seen: recent.len(),
        facts_kept: kept.len(),
        facts_archived: archived.len(),
    })
}

/// All memory files, for the shell's editor — (label, path, content).
pub fn files(dirs: &NestDirs) -> Vec<(String, PathBuf, String)> {
    let mut out = Vec::new();
    for (label, name) in [("profile", "user_profile.md"), ("facts", "facts.md")] {
        let path = home_memory(dirs).join(name);
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        out.push((label.to_string(), path, content));
    }
    if let Some(project) = &dirs.project {
        let path = project.join("memory").join("facts.md");
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        out.push(("project facts".to_string(), path, content));
    }
    out
}

/// Write one memory file back — the labels are a closed set, never a
/// path from the wire.
pub fn save_file(dirs: &NestDirs, label: &str, content: &str) -> Result<PathBuf> {
    let path = match label {
        "profile" => home_memory(dirs).join("user_profile.md"),
        "facts" => home_memory(dirs).join("facts.md"),
        "project" => dirs
            .project
            .as_ref()
            .map(|p| p.join("memory").join("facts.md"))
            .context("no project memory — edit from inside a project")?,
        other => anyhow::bail!("unknown memory file '{other}'"),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hatchling::{Receipt, Status};
    use std::fs;

    fn dirs(tmp: &tempfile::TempDir) -> NestDirs {
        NestDirs::resolve(tmp.path(), Some(tmp.path().join("h")))
    }

    fn receipt(task: &str) -> Receipt {
        Receipt {
            id: "bot-1-1".into(),
            agent: "writer".into(),
            perch: "echo".into(),
            task: task.into(),
            status: Status::Done,
            steps: 1,
            exit_code: Some(0),
            started_ms: 1,
            finished_ms: 2,
            hard_won: false,
            failure_kind: None,
            peeps_path: None,
            session: None,
            tail: vec!["done it".into()],
        }
    }

    #[test]
    fn append_dedups_and_stamps_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        let facts = vec![
            "prefers small commits".to_string(),
            "- prefers small commits".to_string(), // bullet-stripped twin
            "uses tabs".to_string(),
        ];
        let added = append_facts(&dirs, &facts, "run-1").unwrap();
        assert_eq!(added, 2);
        let text = fs::read_to_string(dirs.home.join("memory/facts.md")).unwrap();
        assert!(text.contains("prefers small commits # run-1"));
        assert!(text.contains("uses tabs # run-1"));
        assert_eq!(text.matches("# run-1").count(), 2);

        // a second pass adds nothing
        assert_eq!(append_facts(&dirs, &facts, "run-2").unwrap(), 0);
    }

    #[test]
    fn prompt_block_merges_project_and_personal() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("w");
        fs::create_dir_all(work.join(".nest/memory")).unwrap();
        fs::create_dir_all(tmp.path().join("h/memory")).unwrap();
        let dirs = NestDirs::resolve(&work, Some(tmp.path().join("h")));
        fs::write(
            dirs.home.join("memory/user_profile.md"),
            "Sam — terse answers please\n",
        )
        .unwrap();
        fs::write(dirs.home.join("memory/facts.md"), "likes rust\n").unwrap();
        fs::write(
            work.join(".nest/memory/facts.md"),
            "project uses zig cc\nlikes rust # older\n",
        )
        .unwrap();

        let block = prompt_block(&dirs).unwrap();
        assert!(block.contains("Sam — terse answers please"));
        assert!(block.contains("project uses zig cc"));
        // personal fact deduped against the project's copy
        assert_eq!(block.matches("likes rust").count(), 1);

        // an empty nest produces no block
        let empty = tempfile::tempdir().unwrap();
        let empty_dirs = NestDirs::resolve(empty.path(), Some(empty.path().join("h")));
        assert!(prompt_block(&empty_dirs).is_none());
    }

    #[test]
    fn prompt_block_respects_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        let huge = "x".repeat(PROMPT_CAP * 3);
        fs::create_dir_all(dirs.home.join("memory")).unwrap();
        fs::write(dirs.home.join("memory/facts.md"), format!("{huge}\n")).unwrap();
        let block = prompt_block(&dirs).unwrap();
        assert!(block.len() <= PROMPT_CAP + 32);
    }

    #[test]
    fn extract_skips_echo_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        // no providers.toml → echo fallback → nothing lands
        assert_eq!(extract(&dirs, &receipt("do x")).unwrap(), 0);
        assert!(!dirs.home.join("memory/facts.md").exists());
    }

    #[test]
    fn extract_lands_facts_from_a_scripted_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        std::fs::create_dir_all(&dirs.home).unwrap();
        std::fs::write(
            dirs.home.join("providers.toml"),
            "[[providers]]\nname = \"mem-llm\"\nkind = \"command\"\nprogram = \"sh\"\nargs = [\"-c\", \"printf 'the project uses zig cc for linking\\n- terse commits\\n'\"]\ncost = \"free\"\n",
        )
        .unwrap();
        let added = extract(&dirs, &receipt("link the binary")).unwrap();
        assert_eq!(added, 2);
        let facts = std::fs::read_to_string(dirs.home.join("memory/facts.md")).unwrap();
        assert!(facts.contains("the project uses zig cc for linking # bot-1-1"));
        assert!(facts.contains("terse commits"));
    }

    #[test]
    fn reflect_digests_and_archives() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        crate::roost::record(&dirs.roost(), &receipt("write docs")).unwrap();
        fs::create_dir_all(dirs.home.join("memory")).unwrap();
        fs::write(
            dirs.home.join("memory/facts.md"),
            "likes rust # run-1\nLikes   Rust # run-2\nuses tabs # run-1\n",
        )
        .unwrap();

        let report = reflect(&dirs).unwrap();
        assert!(report.digest.is_file());
        assert_eq!(report.facts_kept, 2);
        assert_eq!(report.facts_archived, 1);
        assert_eq!(report.receipts_seen, 1);

        let digest = fs::read_to_string(&report.digest).unwrap();
        assert!(digest.contains("write") || digest.contains("writer"));
        let archive = fs::read_to_string(dirs.home.join("memory/archive.md")).unwrap();
        assert!(archive.contains("Rust # run-2"));

        // the digest announces itself in the inbox
        let items = crate::inbox::list(&dirs);
        assert!(items.iter().any(|i| i.title.contains("reflect")));
    }

    #[test]
    fn save_file_only_writes_known_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(&tmp);
        save_file(&dirs, "facts", "one fact\n").unwrap();
        assert!(dirs.home.join("memory/facts.md").is_file());
        for bad in ["../x", "archive", "roost", ""] {
            assert!(save_file(&dirs, bad, "x").is_err(), "{bad:?} must fail");
        }
    }
}
