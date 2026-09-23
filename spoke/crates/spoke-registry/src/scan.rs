use std::collections::HashSet;

/// Findings from a static scan of a package's files, before anything is installed.
#[derive(Debug, Clone, Default)]
pub struct ScanReport {
    pub scripts: Vec<String>,
    pub network_refs: usize,
    pub declares_hooks: bool,
    pub warnings: Vec<String>,
}

impl ScanReport {
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }
}

const SCRIPT_EXTENSIONS: &[&str] = &[
    ".sh", ".bash", ".zsh", ".js", ".mjs", ".cjs", ".ts", ".py", ".rb", ".pl", ".ps1", ".bat",
];

/// Heuristic scan. This is a warning system, not a sandbox: it reports what a
/// reviewer should look at, and never silently blocks.
pub fn scan(files: &[(String, String)]) -> ScanReport {
    let mut report = ScanReport::default();
    let mut network_refs = 0usize;

    for (rel, contents) in files {
        let lower = rel.to_ascii_lowercase();
        let is_script =
            SCRIPT_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) || contents.starts_with("#!");
        if is_script {
            report.scripts.push(rel.clone());
        }

        let body = contents.to_ascii_lowercase();
        for needle in ["http://", "https://", "curl ", "wget ", "nc -", "eval("] {
            network_refs += body.matches(needle).count();
        }

        if body.contains("hooks:") && rel.to_ascii_lowercase().contains("skill.md") {
            report.declares_hooks = true;
        }
    }

    report.network_refs = network_refs;

    if !report.scripts.is_empty() {
        report.warnings.push(format!(
            "contains {} script file(s) that will execute on the host: {}",
            report.scripts.len(),
            report.scripts.join(", ")
        ));
    }
    if report.network_refs > 0 {
        report.warnings.push(format!(
            "references network calls {} time(s) — review what it contacts",
            report.network_refs
        ));
    }
    if report.declares_hooks {
        report
            .warnings
            .push("declares hooks in SKILL.md frontmatter — hooks run shell commands".to_string());
    }

    report
}

/// Token-shingle Jaccard similarity over two texts (0.0 – 1.0).
pub fn similarity(a: &str, b: &str) -> f32 {
    let sa = shingles(a);
    let sb = shingles(b);
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let intersection = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn shingles(text: &str) -> HashSet<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .collect();
    if words.len() < 5 {
        return words.into_iter().collect();
    }
    words.windows(5).map(|w| w.join(" ")).collect()
}

/// Pairs of installed skills whose contents overlap meaningfully.
pub fn overlaps(skills: &[(String, String)], threshold: f32) -> Vec<(String, String, f32)> {
    let mut pairs = Vec::new();
    for i in 0..skills.len() {
        for j in (i + 1)..skills.len() {
            let score = similarity(&skills[i].1, &skills[j].1);
            if score >= threshold {
                pairs.push((skills[i].0.clone(), skills[j].0.clone(), score));
            }
        }
    }
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    pairs
}
