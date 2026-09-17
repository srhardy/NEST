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

/// Distill a finished run into `<home>/skills/<name>/SKILL.md`.
/// Returns the skill's directory. `force` permits replacing an
/// existing skill — without it a name collision is a hard error, not
/// a silent overwrite.
pub fn feather(dirs: &NestDirs, run_id: &str, name: Option<&str>, force: bool) -> Result<PathBuf> {
    let receipt = find_receipt(dirs, run_id)?;
    let skill_name = name
        .map(slug)
        .unwrap_or_else(|| {
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
    let res = Router::from_dirs(dirs).complete(&ForageRequest {
        prompt,
        model: None,
        tier: None,
    })?;

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
            format!("---\nname: {skill_name}\n{provenance}{yaml}\n---{}", &rest[end + 4..])
        } else {
            format!("---\nname: {skill_name}\n{provenance}---\n{body}")
        }
    } else {
        format!("---\nname: {skill_name}\n{provenance}---\n\n{body}")
    };

    let dir = dirs.home.join("skills").join(&skill_name);
    let path = dir.join("SKILL.md");
    if path.exists() && !force {
        anyhow::bail!("skill '{skill_name}' already exists — pass --force to replace it");
    }
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    // tmp + rename: a feather never half-writes over a live skill
    let tmp = dir.join("SKILL.md.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot install {}", path.display()))?;
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
            tail: vec!["did the thing".into()],
        }
    }

    #[test]
    fn feathers_a_run_into_skills() {
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(&std::path::Path::new("/"), Some(home.path().to_path_buf()));
        roost::record(&dirs.roost(), &receipt("run-abc123")).unwrap();
        let dir = feather(&dirs, "run-abc123", Some("docs-sync"), false).unwrap();
        let body = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(body.contains("authored-by: doc-keeper"), "{body}");
        assert!(body.contains("run: run-abc123"), "{body}");
        // echo router just reflects the distill prompt — file still exists
        assert!(dir.ends_with("skills/docs-sync"));
    }

    #[test]
    fn id_prefix_resolves_and_ambiguity_errors() {
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(&std::path::Path::new("/"), Some(home.path().to_path_buf()));
        roost::record(&dirs.roost(), &receipt("run-aaa")).unwrap();
        roost::record(&dirs.roost(), &receipt("run-aab")).unwrap();
        assert!(find_receipt(&dirs, "run-aaa").is_ok());
        assert!(find_receipt(&dirs, "run-aa").is_err());
        assert!(find_receipt(&dirs, "nope").is_err());
    }

    #[test]
    fn existing_skills_need_force() {
        let home = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(&std::path::Path::new("/"), Some(home.path().to_path_buf()));
        roost::record(&dirs.roost(), &receipt("run-abc123")).unwrap();
        feather(&dirs, "run-abc123", Some("docs-sync"), false).unwrap();
        // second feather with the same name refuses rather than clobbering
        assert!(feather(&dirs, "run-abc123", Some("docs-sync"), false).is_err());
        feather(&dirs, "run-abc123", Some("docs-sync"), true).unwrap();
    }

    #[test]
    fn slugs_are_safe() {
        assert_eq!(slug("Fix OAuth Flow!"), "fix-oauth-flow");
        assert_eq!(slug("../escape"), "escape");
        assert_eq!(slug(""), "learned-skill");
    }
}
