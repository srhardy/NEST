//! Feather — the self-improving loop (the differentiator).
//!
//! A bot that finishes a run can leave a feather behind: `nest feather
//! <run-id>` sends the receipt through the forage router and asks it to
//! distill a reusable `SKILL.md` into `~/.nest/skills/<name>/`. The skill
//! carries its provenance in frontmatter — `authored-by: <bot>`,
//! `run: <id>` — so the roster literally gets smarter with use, and any
//! bot can pick the skill up by adding `skills: [name]` to its
//! frontmatter.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::forage::{ForageRequest, Router};
use crate::hatchling::Receipt;
use crate::home::NestDirs;
use crate::roost;

/// Find a receipt by id or unique prefix.
pub fn find_receipt(dirs: &NestDirs, id: &str) -> Result<Receipt> {
    let all = roost::receipts(&dirs.roost())?;
    if let Some(exact) = all.iter().find(|r| r.id == id) {
        return Ok(exact.clone());
    }
    let hits: Vec<_> = all.into_iter().filter(|r| r.id.starts_with(id)).collect();
    match hits.len() {
        0 => anyhow::bail!("no run '{id}' in the roost"),
        1 => Ok(hits.into_iter().next().unwrap()),
        _ => anyhow::bail!("'{id}' matches {} runs — give more of the id", hits.len()),
    }
}

/// A skill name is a directory name — lowercase slug, no path tricks.
fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "learned-skill".into()
    } else {
        s
    }
}

const DISTILL_PROMPT: &str = "You are distilling a finished task run into a reusable skill.\n\
Output ONLY a markdown document shaped like a SKILL.md:\n\
---\nname: <kebab-case name>\ndescription: <one line — when to use this skill>\n---\n\n\
Then concise instructions: what to do, what to avoid, pitfalls noticed during the run.\n\
Keep it under 40 lines. If the run shows nothing reusable, output a SKILL.md\n\
explaining what NOT to do. Here is the run:\n\n";

/// A learned skill's index row for the shell: name, who authored it,
/// from which run, and the one-line description from its frontmatter.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub authored_by: Option<String>,
    pub run: Option<String>,
    pub description: Option<String>,
}

fn frontmatter_field(body: &str, key: &str) -> Option<String> {
    let rest = body.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    rest[..end].lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if k.trim() == key {
            let v = v.trim();
            (!v.is_empty()).then(|| v.to_string())
        } else {
            None
        }
    })
}

/// Every skill under <home>/skills/*/SKILL.md, newest first.
pub fn list_skills(dirs: &NestDirs) -> Vec<SkillInfo> {
    let root = dirs.home.join("skills");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path().join("SKILL.md");
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mtime = e
            .path()
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push((
            mtime,
            SkillInfo {
                name,
                authored_by: frontmatter_field(&body, "authored-by"),
                run: frontmatter_field(&body, "run"),
                description: frontmatter_field(&body, "description"),
            },
        ));
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.0));
    out.into_iter().map(|(_, s)| s).collect()
}

/// Record a feathered skill in the nest's agent.lock: learned skills get
/// provenance (author + run) so the roster's memory is auditable.
pub fn record_learned_skill(dirs: &NestDirs, skill: &str, agent: &str, run: &str) {
    let mut lock = spoke_core::lock::Lock::load(&dirs.home).unwrap_or_default();
    let entry = spoke_core::lock::LockedPackage {
        name: format!("skill:{skill}"),
        source: Some(format!("feathered by {agent} from run {run}")),
        version: None,
        resolved_revision: None,
        license: None,
        integrity: None,
        tokens: 0,
        skills: vec![skill.to_string()],
        agents: Vec::new(),
        installed_paths: vec![dirs.home.join("skills").join(skill).display().to_string()],
    };
    lock.upsert(entry);
    let _ = lock.save(&dirs.home);
}

/// Should this receipt auto-feather? Opt-in via `auto_feather: true` in
/// settings.md; only hard-won or failed runs qualify (the ones worth
/// learning from), and only when a real provider exists.
pub fn auto_feather_due(dirs: &NestDirs, receipt: &Receipt) -> bool {
    let on = crate::spec::setting(dirs, "auto_feather")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1" || v.eq_ignore_ascii_case("on"))
        .unwrap_or(false);
    if !on {
        return false;
    }
    if receipt.status == crate::hatchling::Status::Done && !receipt.hard_won {
        return false;
    }
    let router = Router::from_dirs(dirs);
    router
        .list()
        .iter()
        .any(|p| p.name != "echo" && p.available && !p.exhausted)
}
/// Returns the skill's directory. `force` permits replacing an
/// existing skill — without it a name collision is a hard error, not
/// a silent overwrite. Records lockfile provenance on success.
pub fn feather(dirs: &NestDirs, run_id: &str, name: Option<&str>, force: bool) -> Result<PathBuf> {
    let receipt = find_receipt(dirs, run_id)?;
    let skill_name = name.map(slug).unwrap_or_else(|| {
        let id_prefix: String = receipt.id.chars().take(12).collect();
        slug(&format!("{}-learned-{id_prefix}", receipt.agent))
    });

    let prompt = format!(
        "{DISTILL_PROMPT}task: {}\nstatus: {:?}\nsteps: {}\noutput tail:\n{}",
        receipt.task,
        receipt.status,
        receipt.steps,
        receipt.tail.join("\n")
    );
    // Name collisions refuse early — before any provider call.
    let dir = dirs.home.join("skills").join(&skill_name);
    let path = dir.join("SKILL.md");
    if path.exists() && !force {
        anyhow::bail!("skill '{skill_name}' already exists — pass --force to replace it");
    }

    let res = Router::from_dirs(dirs).complete(&ForageRequest {
        prompt,
        model: None,
        tier: None,
    })?;
    // The echo fallback reflects the prompt itself — a skill written from it
    // is garbage, not a summary. Feathers need a real provider.
    if res.provider == "echo" {
        anyhow::bail!(
            "no real provider configured — declare one in providers.toml, then feather again"
        );
    }

    // strip a wrapping ```markdown fence if the model added one
    let mut body = res.text.trim().to_string();
    if let Some(rest) = body.strip_prefix("```") {
        body = rest
            .trim_start_matches(|c: char| c != '\n')
            .trim_start()
            .trim_end_matches("```")
            .trim()
            .to_string();
    }
    // provenance goes in the frontmatter — who authored this, from
    // which run. `name:` is canonicalized to the skill's directory so
    // the two identifiers can never disagree.
    let provenance = format!("authored-by: {}\nrun: {}\n", receipt.agent, receipt.id);
    let body = if let Some(rest) = body.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let yaml = rest[..end]
                .lines()
                .filter(|l| !l.trim_start().starts_with("name:"))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "---\nname: {skill_name}\n{provenance}{yaml}\n---{}",
                &rest[end + 4..]
            )
        } else {
            format!("---\nname: {skill_name}\n{provenance}---\n{body}")
        }
    } else {
        format!("---\nname: {skill_name}\n{provenance}---\n\n{body}")
    };

    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    // tmp + rename: a feather never half-writes over a live skill
    let tmp = dir.join("SKILL.md.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot install {}", path.display()))?;
    record_learned_skill(dirs, &skill_name, &receipt.agent, &receipt.id);
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hatchling::{Receipt, Status};

    fn receipt(id: &str) -> Receipt {
        Receipt {
            id: id.into(),
            agent: "doc-keeper".into(),
            perch: "echo".into(),
            task: "sync the docs".into(),
            status: Status::Done,
            steps: 3,
            exit_code: Some(0),
            started_ms: 1,
            finished_ms: 2,
            hard_won: false,
            failure_kind: None,
            peeps_path: None,
            tail: vec!["did the thing".into()],
        }
    }

    #[test]
    fn echo_provider_never_writes_a_skill() {
        // With no real provider declared, feather must refuse rather than
        // save the distill prompt itself as if it were a learned skill.
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(
            std::path::Path::new("/"),
            Some(home.path().to_path_buf()),
        );
        roost::record(&dirs.roost(), &receipt("run-abc123")).unwrap();
        let err = feather(&dirs, "run-abc123", Some("docs-sync"), false).unwrap_err();
        assert!(err.to_string().contains("no real provider"), "{err}");
        assert!(!dirs.home.join("skills/docs-sync/SKILL.md").exists());
    }

    #[test]
    fn id_prefix_resolves_and_ambiguity_errors() {
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(
            std::path::Path::new("/"),
            Some(home.path().to_path_buf()),
        );
        roost::record(&dirs.roost(), &receipt("run-aaa")).unwrap();
        roost::record(&dirs.roost(), &receipt("run-aab")).unwrap();
        assert!(find_receipt(&dirs, "run-aaa").is_ok());
        assert!(find_receipt(&dirs, "run-aa").is_err());
        assert!(find_receipt(&dirs, "nope").is_err());
    }

    #[test]
    fn existing_skills_refuse_without_force() {
        // The collision check runs before any provider call, so a taken
        // name errors even when nothing could distill anyway.
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(
            std::path::Path::new("/"),
            Some(home.path().to_path_buf()),
        );
        roost::record(&dirs.roost(), &receipt("run-abc123")).unwrap();
        let dir = dirs.home.join("skills/docs-sync");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "mine").unwrap();
        let err = feather(&dirs, "run-abc123", Some("docs-sync"), false).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[test]
    fn slugs_are_safe() {
        assert_eq!(slug("Fix OAuth Flow!"), "fix-oauth-flow");
        assert_eq!(slug("../escape"), "escape");
        assert_eq!(slug(""), "learned-skill");
    }

    #[test]
    fn list_skills_reads_provenance() {
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(
            std::path::Path::new("/"),
            Some(home.path().to_path_buf()),
        );
        assert!(list_skills(&dirs).is_empty());
        let dir = dirs.home.join("skills/docs-sync");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: docs-sync\nauthored-by: doc-keeper\nrun: run-abc123\ndescription: sync the docs\n---\n\nbody\n",
        )
        .unwrap();
        let skills = list_skills(&dirs);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "docs-sync");
        assert_eq!(skills[0].authored_by.as_deref(), Some("doc-keeper"));
        assert_eq!(skills[0].run.as_deref(), Some("run-abc123"));
        assert_eq!(skills[0].description.as_deref(), Some("sync the docs"));
    }

    #[test]
    fn auto_feather_due_needs_opt_in_and_hard_run() {
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(
            std::path::Path::new("/"),
            Some(home.path().to_path_buf()),
        );
        let mut easy = receipt("run-easy");
        easy.status = crate::hatchling::Status::Done;
        assert!(!auto_feather_due(&dirs, &easy), "off by default");
        std::fs::write(dirs.home.join("settings.md"), "auto_feather: true\n").unwrap();
        assert!(
            !auto_feather_due(&dirs, &easy),
            "easy Done runs never qualify"
        );
        let mut hard = receipt("run-hard");
        hard.status = crate::hatchling::Status::Done;
        hard.hard_won = true;
        // no real provider on PATH in tests → still false, but for the
        // provider reason, not the opt-in reason
        assert!(!auto_feather_due(&dirs, &hard));
        let mut failed = receipt("run-fail");
        failed.status = crate::hatchling::Status::Failed;
        assert!(!auto_feather_due(&dirs, &failed));
    }
}
