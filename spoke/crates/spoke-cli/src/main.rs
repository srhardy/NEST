use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};

use spoke_adapters::{agent_install_op, find, registry, skill_install_op, Ctx};
use spoke_core::ir::{Op, Plan};
use spoke_core::lock::{Lock, LockedPackage};
use spoke_core::{paths, Manifest};
use spoke_registry::scan;
use spoke_registry::Store;

#[derive(Parser)]
#[command(name = "spoke", version, about = "One hub for every coding agent")]
struct Cli {
    /// Home directory to operate on (defaults to $HOME)
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Project directory (defaults to the current directory)
    #[arg(long, global = true)]
    project: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Detect harnesses, write agent.toml, prepare the Spoke store
    Init {
        /// Overwrite an existing agent.toml
        #[arg(long)]
        force: bool,
    },
    /// Project the manifest into every enabled harness
    Sync {
        /// Show what would change without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Show what sync would change (same as `sync --dry-run`)
    Diff,
    /// Harness detection, capabilities, drift and warnings
    Doctor,
    /// Install a package: name-in-index, package dir/path, .md bot file, skill dir (or ~/.nest/skills/<name>), or https?:// / file:// URL
    Add {
        spec: String,
        /// Show what would be installed without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove an installed package and its skills
    Remove { name: String },
    /// List installed packages
    List,
    /// Explain a package: source, integrity, skills, installed paths
    Why { name: String },
    /// Report overlapping skills between installed packages
    Dedupe {
        /// Similarity threshold (0.0 – 1.0)
        #[arg(long, default_value_t = 0.4)]
        threshold: f32,
    },
    /// Record an event in the Spoke log (called by harness hooks)
    Event {
        kind: String,
        #[arg(long)]
        detail: Option<String>,
    },
    /// Install Spoke's harness hooks (budget accounting); --uninstall removes them
    Hooks {
        #[arg(long)]
        uninstall: bool,
    },
    /// Context cost and spend receipts for installed skills
    Budget {
        #[arg(long)]
        receipts: bool,
    },
    /// Manage the bot roster declared in agent.toml
    Bot {
        #[command(subcommand)]
        command: BotCommand,
    },
    /// Manage the shared package registry (~/.spoke/registry)
    Registry {
        #[command(subcommand)]
        command: RegistryCommand,
    },
}

#[derive(Subcommand)]
enum RegistryCommand {
    /// Clone or update the registry index repo (git URL; omit to pull the
    /// checkout already at ~/.spoke/registry)
    Sync { url: Option<String> },
    /// List the packages in the synced index
    Show,
}

#[derive(Subcommand)]
enum BotCommand {
    /// List declared bots and their jobs
    List,
    /// Append a [[bots]] entry to agent.toml
    Add {
        /// kebab-case name; becomes .nest/agents/<name>.md
        name: String,
        /// The one job this bot is for (required — one bot, one job)
        #[arg(long)]
        job: String,
        /// Display name in the roster
        #[arg(long)]
        title: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("spoke: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let home = cli.home.unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
    });
    let project = match cli.project {
        Some(path) => path,
        None => std::env::current_dir().context("resolving the current directory")?,
    };

    match cli.command {
        Command::Init { force } => init(&home, &project, force),
        Command::Sync { dry_run } => sync(&home, &project, dry_run),
        Command::Diff => sync(&home, &project, true),
        Command::Doctor => doctor(&home, &project),
        Command::Add { spec, dry_run } => add(&home, &project, &spec, dry_run),
        Command::Remove { name } => remove(&home, &project, &name),
        Command::List => list(&project),
        Command::Why { name } => why(&home, &project, &name),
        Command::Dedupe { threshold } => dedupe(&home, &project, threshold),
        Command::Event { kind, detail } => event(&home, &kind, detail.as_deref()),
        Command::Hooks { uninstall } => hooks(&home, &project, uninstall),
        Command::Budget { receipts } => budget(&home, &project, receipts),
        Command::Bot { command } => match command {
            BotCommand::List => bot_list(&project),
            BotCommand::Add { name, job, title } => {
                bot_add(&project, &name, &job, title.as_deref())
            }
        },
        Command::Registry { command } => match command {
            RegistryCommand::Sync { url } => registry_sync(&home, url.as_deref()),
            RegistryCommand::Show => registry_show(&home),
        },
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn init(home: &Path, project: &Path, force: bool) -> Result<()> {
    println!("Spoke init");
    println!("  project: {}", project.display());

    let detected: Vec<&str> = registry()
        .iter()
        .filter(|h| h.detect(home))
        .map(|h| h.display())
        .collect();
    if detected.is_empty() {
        println!("  harnesses detected: none yet — install Claude Code or Codex first");
    } else {
        println!("  harnesses detected: {}", detected.join(", "));
    }

    let manifest_path = project.join(spoke_core::MANIFEST_FILE);
    if manifest_path.exists() && !force {
        println!("  kept existing {}", manifest_path.display());
    } else {
        std::fs::write(&manifest_path, Manifest::template())
            .with_context(|| format!("writing {}", manifest_path.display()))?;
        println!("  wrote {}", manifest_path.display());
    }

    let store = paths::spoke_home(home);
    std::fs::create_dir_all(store.join("backups"))
        .with_context(|| format!("creating {}", store.display()))?;
    println!("  store: {}", store.display());

    println!();
    println!("Next:");
    println!("  spoke diff     # see exactly what Spoke would change");
    println!("  spoke sync     # apply it (files are backed up first)");
    println!("  spoke add <pkg>  # install a skill package");
    Ok(())
}

fn sync(home: &Path, project: &Path, dry_run: bool) -> Result<()> {
    let manifest = Manifest::load(project)?;
    manifest.validate()?;
    let ts = unix_secs();

    println!(
        "Spoke {} — {}",
        if dry_run { "diff" } else { "sync" },
        project.display()
    );

    let mut files_changed = 0usize;
    let mut files_unchanged = 0usize;
    let mut warnings = 0usize;

    for id in &manifest.harnesses {
        let Some(harness) = find(id) else {
            println!("  ! unknown harness '{id}' in agent.toml — skipped");
            warnings += 1;
            continue;
        };
        if !harness.detect(home) {
            println!(
                "  - {} is enabled but not detected on this machine — skipped",
                harness.display()
            );
            warnings += 1;
            continue;
        }

        let ctx = Ctx::new(home, project, &manifest);
        let plan = harness.plan(&ctx);
        let changes = if dry_run {
            spoke_adapters::apply::preview(&plan)?
        } else {
            spoke_adapters::apply::apply(&plan, home, project, ts)?
        };

        let mut by_file: BTreeMap<PathBuf, Vec<&str>> = BTreeMap::new();
        let mut unchanged = 0usize;
        for change in &changes {
            if change.changed {
                let reasons = by_file.entry(change.path.clone()).or_default();
                if !reasons.contains(&change.why.as_str()) {
                    reasons.push(change.why.as_str());
                }
            } else {
                unchanged += 1;
            }
        }

        files_changed += by_file.len();
        files_unchanged += unchanged;

        println!("  {}: {} file(s)", harness.display(), by_file.len());
        for (path, reasons) in &by_file {
            println!(
                "    {} {} — {}",
                if dry_run { "would update" } else { "updated" },
                path.display(),
                reasons.join("; ")
            );
        }
        for warning in &plan.warnings {
            println!("    note: {warning}");
            warnings += 1;
        }
    }

    println!(
        "  total: {} file(s) {}, {} operation(s) already in sync, {} warning(s)",
        files_changed,
        if dry_run { "would change" } else { "written" },
        files_unchanged,
        warnings
    );
    if !dry_run && files_changed > 0 {
        println!("  backups: {}", paths::backups_dir(home, ts).display());
    }
    Ok(())
}

fn doctor(home: &Path, project: &Path) -> Result<()> {
    println!("Spoke doctor");
    println!("  project: {}", project.display());

    let manifest_path = project.join(spoke_core::MANIFEST_FILE);
    let manifest = if manifest_path.exists() {
        println!("  manifest: present");
        Some(Manifest::load(project)?)
    } else {
        println!("  manifest: missing (run `spoke init`)");
        None
    };

    if let Some(manifest) = &manifest {
        println!("  budget preset: {}", manifest.budget.preset);
        println!(
            "  bots declared: {}   automations declared: {}   routes: {}   services: {}",
            manifest.bots.len(),
            manifest.automations.len(),
            manifest.routing.len(),
            manifest.services.len()
        );
    }
    let lock = Lock::load(project)?;
    println!("  packages installed: {}", lock.packages.len());
    println!();

    for harness in registry() {
        let detected = harness.detect(home);
        let enabled = manifest
            .as_ref()
            .map(|m| m.enabled(harness.id()))
            .unwrap_or(false);

        println!("  {} ({})", harness.display(), harness.id());
        println!(
            "    detected: {}   enabled: {}   skills root: {}",
            yes_no(detected),
            yes_no(enabled),
            harness.skills_root(home).display()
        );
        println!("    capabilities: {}", harness.capabilities().describe());

        if let Some(manifest) = &manifest {
            if enabled && detected {
                let ctx = Ctx::new(home, project, manifest);
                let plan = harness.plan(&ctx);
                let changes = spoke_adapters::apply::preview(&plan)?;
                let pending = changes.iter().filter(|c| c.changed).count();
                println!("    drift: {pending} file(s) would change");
                for warning in &plan.warnings {
                    println!("    note: {warning}");
                }
            }
        }
    }

    println!();
    println!("  store: {}", paths::spoke_home(home).display());
    Ok(())
}

/// Turn a spec into something `Store::resolve` can read:
/// an `https?://` or `file://` URL is fetched to disk first, and a bare
/// `.md` file (fetched or local) is wrapped as a one-bot package.
fn materialize_spec(home: &Path, spec: &str) -> Result<PathBuf> {
    let path = if spec.starts_with("http://") || spec.starts_with("https://") {
        fetch_url(home, spec)?
    } else if let Some(rest) = spec.strip_prefix("file://") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(spec)
    };
    if path.extension().is_some_and(|e| e == "md") && path.is_file() {
        return wrap_md_as_package(home, &path);
    }
    // a bare skill dir (a feather under ~/.nest/skills/, say) is a
    // bot-authored artifact — wrap it as a one-skill package
    if path.is_dir() && !path.join("package.toml").exists() && path.join("SKILL.md").is_file() {
        return wrap_skill_dir_as_package(home, &path);
    }
    // `spoke add <skill>` — the name of a bot-authored skill already in ~/.nest/skills/
    if !path.exists()
        && spec
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        let nest_skill = home.join(".nest").join("skills").join(spec);
        if nest_skill.join("SKILL.md").is_file() {
            return wrap_skill_dir_as_package(home, &nest_skill);
        }
    }
    Ok(path)
}

/// `curl` the URL into the Spoke tmp dir — curl is on every machine
/// NEST targets, and it keeps this crate dependency-free.
fn fetch_url(home: &Path, url: &str) -> Result<PathBuf> {
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("download")
        .split('?')
        .next()
        .unwrap_or("download");
    let dir = paths::spoke_home(home).join("tmp");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let dest = dir.join(name);
    let status = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "30", url, "-o"])
        .arg(&dest)
        .status()
        .context("curl is required for `spoke add <url>`")?;
    if !status.success() {
        anyhow::bail!("fetching {url} failed (curl exit {status})");
    }
    Ok(dest)
}

/// A lone `.md` bot file installs as a one-agent package — the name
/// comes from frontmatter if present, else the filename.
fn wrap_md_as_package(home: &Path, file: &Path) -> Result<PathBuf> {
    let body =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let name = frontmatter_name(&body).unwrap_or_else(|| {
        file.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });
    let dir = paths::spoke_home(home)
        .join("tmp")
        .join(format!("{name}-pkg"));
    let agents = dir.join("agents");
    std::fs::create_dir_all(&agents).with_context(|| format!("creating {}", agents.display()))?;
    std::fs::copy(file, agents.join(format!("{name}.md")))
        .with_context(|| format!("copying {}", file.display()))?;
    std::fs::write(
        dir.join("package.toml"),
        format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"imported from {}\"\ntier = \"imported\"\n\n[source]\npath = \".\"\n\n[[agents]]\nname = \"{name}\"\npath = \"agents/{name}.md\"\n",
            file.display()
        ),
    )?;
    Ok(dir)
}

/// The `name:` line of a `---`-fenced frontmatter block, if there is one.
fn frontmatter_name(body: &str) -> Option<String> {
    body.lines()
        .skip_while(|l| *l != "---")
        .skip(1)
        .take_while(|l| *l != "---")
        .find_map(|l| l.strip_prefix("name:").map(|n| n.trim().to_string()))
        .filter(|n| !n.is_empty())
}

/// Directory names are slugs — lowercase, dashes for everything else.
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

fn copy_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            std::fs::create_dir_all(&dest)
                .with_context(|| format!("creating {}", dest.display()))?;
            copy_dir_contents(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).with_context(|| format!("copying {}", path.display()))?;
        }
    }
    Ok(())
}

/// A directory holding SKILL.md — a feather a bot authored — wraps as a
/// one-skill package so `spoke add` can install it into every harness.
fn wrap_skill_dir_as_package(home: &Path, skill_dir: &Path) -> Result<PathBuf> {
    let body = std::fs::read_to_string(skill_dir.join("SKILL.md"))
        .with_context(|| format!("reading {}", skill_dir.join("SKILL.md").display()))?;
    let name = frontmatter_name(&body).unwrap_or_else(|| {
        skill_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });
    let name = slug(&name);
    let dir = paths::spoke_home(home)
        .join("tmp")
        .join(format!("{name}-skillpkg"));
    let src = dir.join("src");
    std::fs::create_dir_all(&src).with_context(|| format!("creating {}", src.display()))?;
    copy_dir_contents(skill_dir, &src)?;
    std::fs::write(
        dir.join("package.toml"),
        format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"bot-authored skill (nest feather)\"\ntier = \"bot-authored\"\n\n[source]\npath = \"src\"\n\n[[skills]]\nname = \"{name}\"\npath = \".\"\n"
        ),
    )?;
    Ok(dir)
}

fn add(home: &Path, project: &Path, spec: &str, dry_run: bool) -> Result<()> {
    let manifest = Manifest::load(project)?;
    manifest.validate()?;

    let store = Store::new(home);
    let spec_path = materialize_spec(home, spec)?;
    let package = store.resolve(&spec_path.to_string_lossy())?;
    package.validate()?;

    println!(
        "Resolved {}@{} ({})",
        package.name,
        package.version,
        package.license.as_deref().unwrap_or("license unknown")
    );
    if !package.description.is_empty() {
        println!("  {}", package.description);
    }
    println!("  source: {}", package.source.describe());
    if !package.requires.mcp.is_empty() {
        println!(
            "  note: requires MCP server(s) {} — not wired yet (Spoke's MCP surface lands in M2+)",
            package.requires.mcp.join(", ")
        );
    }

    let materialized = store.fetch(&package)?;
    let skills = store.skill_files(&package, &materialized.dir)?;
    let agents = store.agent_files(&package, &materialized.dir)?;

    let mut total_tokens = 0u64;
    let mut all_files: Vec<(String, String)> = Vec::new();
    for (skill, files) in &skills {
        let report = scan::scan(files);
        let tokens = spoke_registry::token_estimate(files);
        total_tokens += tokens;
        println!(
            "  skill '{skill}': {} file(s), ~{tokens} tokens (estimate)",
            files.len()
        );
        for warning in &report.warnings {
            println!("    scan: {warning}");
        }
        all_files.extend(files.clone());
    }
    for (agent, files) in &agents {
        let report = scan::scan(files);
        let tokens = spoke_registry::token_estimate(files);
        total_tokens += tokens;
        println!(
            "  agent '{agent}': {} file(s), ~{tokens} tokens (estimate)",
            files.len()
        );
        for warning in &report.warnings {
            println!("    scan: {warning}");
        }
        all_files.extend(files.clone());
    }
    let integrity = Store::integrity(&all_files);

    // Overlap check against what is already installed.
    let existing = installed_skill_texts(project)?;
    for (skill, files) in &skills {
        let body = files
            .iter()
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for (other_name, other_body) in &existing {
            if other_name == skill {
                continue;
            }
            let score = scan::similarity(&body, other_body);
            if score >= 0.4 {
                println!(
                    "  overlap: '{skill}' and '{other_name}' are {:.0}% similar",
                    score * 100.0
                );
            }
        }
    }

    // Plan the install across every enabled + detected harness.
    let mut plan = Plan::default();
    let mut installed_paths: BTreeSet<String> = BTreeSet::new();
    let mut targets: Vec<String> = Vec::new();
    for id in &manifest.harnesses {
        let Some(harness) = find(id) else {
            println!("  ! unknown harness '{id}' — skipped");
            continue;
        };
        if !harness.detect(home) {
            println!("  - {} not detected — skipped", harness.display());
            continue;
        }
        targets.push(harness.display().to_string());
        for (skill, files) in &skills {
            plan.push(skill_install_op(
                harness.as_ref(),
                home,
                skill,
                files.clone(),
                &materialized.source_description,
            ));
            installed_paths.insert(
                harness
                    .skills_root(home)
                    .join(skill)
                    .to_string_lossy()
                    .to_string(),
            );
        }
        for (agent, files) in &agents {
            plan.push(agent_install_op(
                harness.as_ref(),
                home,
                agent,
                files.clone(),
                &materialized.source_description,
            ));
            installed_paths.insert(
                harness
                    .agents_root(home)
                    .join(agent)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }

    if targets.is_empty() {
        anyhow::bail!("no enabled harness is installed on this machine — nothing to install into");
    }

    let changes = if dry_run {
        spoke_adapters::apply::preview(&plan)?
    } else {
        spoke_adapters::apply::apply(&plan, home, project, unix_secs())?
    };
    let changed = changes.iter().filter(|c| c.changed).count();

    if !dry_run {
        let mut lock = Lock::load(project)?;
        lock.upsert(LockedPackage {
            name: package.name.clone(),
            source: Some(materialized.source_description.clone()),
            version: Some(package.version.clone()),
            license: package.license.clone(),
            integrity: Some(integrity.clone()),
            tokens: total_tokens,
            skills: skills.iter().map(|(name, _)| name.clone()).collect(),
            agents: agents.iter().map(|(name, _)| name.clone()).collect(),
            installed_paths: installed_paths.iter().cloned().collect(),
        });
        lock.save(project)?;
    }

    println!(
        "  {} {} director(ies) into {} — {}",
        if dry_run {
            "would install"
        } else {
            "installed"
        },
        changed,
        targets.join(", "),
        if dry_run {
            "dry run, nothing written".to_string()
        } else {
            format!("lockfile updated ({})", spoke_core::lock::LOCK_FILE)
        }
    );
    if !dry_run && changed > 0 {
        println!(
            "  backups: {}",
            paths::backups_dir(home, unix_secs()).display()
        );
    }
    println!("  integrity: {integrity}");
    Ok(())
}

fn remove(home: &Path, project: &Path, name: &str) -> Result<()> {
    let mut lock = Lock::load(project)?;
    let package = lock
        .get(name)
        .cloned()
        .ok_or_else(|| anyhow!("package '{name}' is not installed"))?;

    let mut paths: BTreeSet<PathBuf> = package.installed_paths.iter().map(PathBuf::from).collect();
    if let Ok(manifest) = Manifest::load(project) {
        for id in &manifest.harnesses {
            if let Some(harness) = find(id) {
                for skill in &package.skills {
                    paths.insert(harness.skills_root(home).join(skill));
                }
                for agent in &package.agents {
                    paths.insert(harness.agents_root(home).join(agent));
                }
            }
        }
    }

    let mut plan = Plan::default();
    for path in &paths {
        plan.push(Op::DeleteDir {
            path: path.clone(),
            why: format!("remove skill directory for '{name}'"),
        });
    }

    let changes = spoke_adapters::apply::apply(&plan, home, project, unix_secs())?;
    let removed = changes.iter().filter(|c| c.changed).count();

    lock.remove(name);
    lock.save(project)?;

    println!("Removed {name}: {removed} director(ies) moved to backups, lockfile updated");
    Ok(())
}

fn list(project: &Path) -> Result<()> {
    let lock = Lock::load(project)?;
    if lock.packages.is_empty() {
        println!("No packages installed (add one with `spoke add <name-or-path>`)");
        return Ok(());
    }
    println!("Installed packages ({}):", lock.packages.len());
    for package in &lock.packages {
        let mut parts = vec![format!("skills: {}", package.skills.join(", "))];
        if !package.agents.is_empty() {
            parts.push(format!("agents: {}", package.agents.join(", ")));
        }
        println!(
            "  {}@{}  ~{} tokens  {}  {}",
            package.name,
            package.version.as_deref().unwrap_or("?"),
            package.tokens,
            package.license.as_deref().unwrap_or("license unknown"),
            parts.join("  ")
        );
    }
    Ok(())
}

fn why(home: &Path, project: &Path, name: &str) -> Result<()> {
    let lock = Lock::load(project)?;
    let package = lock
        .get(name)
        .ok_or_else(|| anyhow!("package '{name}' is not installed"))?;

    println!(
        "{}@{}",
        package.name,
        package.version.as_deref().unwrap_or("?")
    );
    println!(
        "  source:    {}",
        package.source.as_deref().unwrap_or("unknown")
    );
    println!(
        "  license:   {}",
        package.license.as_deref().unwrap_or("unknown")
    );
    println!("  tokens:    ~{} (estimate)", package.tokens);
    println!("  skills:    {}", package.skills.join(", "));
    if !package.agents.is_empty() {
        println!("  agents:    {}", package.agents.join(", "));
    }
    for path in &package.installed_paths {
        let exists = Path::new(path).exists();
        println!(
            "  installed: {path} {}",
            if exists { "" } else { "(missing!)" }
        );
    }

    // Verify the cached package content still matches the recorded integrity.
    if let (Some(expected), Some(version)) = (&package.integrity, &package.version) {
        let store = Store::new(home);
        let cache = store.packages_dir().join(&package.name).join(version);
        if cache.exists() {
            let files = spoke_registry::collect_files(&cache)?;
            let actual = Store::integrity(&files);
            let verdict = if &actual == expected {
                "ok".to_string()
            } else {
                "DRIFT — cached content no longer matches the lockfile".to_string()
            };
            println!("  integrity: {verdict}");
        }
    }
    Ok(())
}

fn dedupe(home: &Path, project: &Path, threshold: f32) -> Result<()> {
    let skills = installed_skill_texts(project)?;
    if skills.len() < 2 {
        println!("Fewer than two installed skills — nothing to compare");
        return Ok(());
    }
    let pairs = scan::overlaps(&skills, threshold);
    let _ = home;
    if pairs.is_empty() {
        println!(
            "No overlaps above {:.0}% among {} installed skill(s)",
            threshold * 100.0,
            skills.len()
        );
        return Ok(());
    }
    println!(
        "Overlapping skills ({} pair(s) above {:.0}%):",
        pairs.len(),
        threshold * 100.0
    );
    for (a, b, score) in pairs {
        println!("  {:.0}%  {a}  <->  {b}", score * 100.0);
    }
    println!("Consider removing one of each pair: `spoke remove <package>`");
    Ok(())
}

/// Text of every installed skill, keyed by skill directory name and deduped
/// across harnesses (the same skill is installed once per harness).
fn installed_skill_texts(project: &Path) -> Result<Vec<(String, String)>> {
    let lock = Lock::load(project)?;
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for package in &lock.packages {
        for path in &package.installed_paths {
            let dir = Path::new(path);
            if !dir.exists() {
                continue;
            }
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| package.name.clone());
            if !seen.insert(name.clone()) {
                continue;
            }
            let files = spoke_registry::collect_files(dir).unwrap_or_default();
            let body = files
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            out.push((name, body));
        }
    }
    Ok(out)
}

/// `spoke registry sync` — the registry index is a git repo. First run
/// clones it into `~/.spoke/registry/`; later runs pull the same checkout.
/// After sync, `spoke add <name>` resolves through the index and `fetch`
/// clones each package's own `source.git`.
fn registry_sync(home: &Path, url: Option<&str>) -> Result<()> {
    let store = Store::new(home);
    let dir = store.registry_dir();
    if dir.join(".git").is_dir() {
        if url.is_some() {
            println!("registry already checked out — ignoring url, pulling");
        }
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["pull", "--ff-only", "--quiet"])
            .status()
            .context("running git pull")?;
        anyhow::ensure!(
            status.success(),
            "git pull --ff-only failed in {} — resolve it manually",
            dir.display()
        );
        println!("pulled {}", dir.display());
    } else {
        let url = url.ok_or_else(|| {
            anyhow!(
                "no registry at {} — pass a git url: `spoke registry sync <git-url>`",
                dir.display()
            )
        })?;
        if dir.exists() && dir.read_dir()?.next().is_some() {
            anyhow::bail!(
                "{} exists but isn't a git checkout — move it aside first",
                dir.display()
            );
        }
        let status = std::process::Command::new("git")
            .args(["clone", "--quiet"])
            .arg(url)
            .arg(&dir)
            .status()
            .context("running git clone")?;
        anyhow::ensure!(status.success(), "git clone of '{url}' failed");
        println!("cloned {url} -> {}", dir.display());
    }
    let index = spoke_registry::package::Index::load(&store.index_path())
        .context("registry synced but has no readable index.toml")?;
    println!("{} package(s) in the index", index.packages.len());
    Ok(())
}

fn registry_show(home: &Path) -> Result<()> {
    let store = Store::new(home);
    let index = spoke_registry::package::Index::load(&store.index_path())
        .context("no registry synced — run `spoke registry sync <git-url>` first")?;
    if index.packages.is_empty() {
        println!("index is empty");
    }
    for entry in &index.packages {
        println!("  {:<24} {}", entry.name, entry.manifest);
    }
    Ok(())
}

fn bot_list(project: &Path) -> Result<()> {
    let manifest = Manifest::load(project)?;
    if manifest.bots.is_empty() {
        println!("No bots declared (add one with `spoke bot add <name> --job \"...\"`)");
        return Ok(());
    }
    println!("Bots declared in agent.toml ({}):", manifest.bots.len());
    for bot in &manifest.bots {
        println!(
            "  {}{}  — {}",
            bot.name,
            bot.title
                .as_ref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default(),
            bot.job
        );
    }
    if !manifest.automations.is_empty() {
        println!();
        println!("Automations ({}):", manifest.automations.len());
        for automation in &manifest.automations {
            let trigger = automation
                .schedule
                .as_ref()
                .map(|s| format!("schedule '{s}'"))
                .or_else(|| automation.event.as_ref().map(|e| format!("event '{e}'")))
                .unwrap_or_default();
            println!(
                "  {}  {}{}",
                automation.name,
                trigger,
                automation
                    .bot
                    .as_ref()
                    .map(|b| format!("  -> bot {b}"))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn bot_add(project: &Path, name: &str, job: &str, title: Option<&str>) -> Result<()> {
    let manifest = Manifest::load(project)?;
    if manifest.bots.iter().any(|b| b.name == name) {
        anyhow::bail!("bot '{name}' is already declared in agent.toml");
    }
    let candidate = spoke_core::manifest::Bot {
        name: name.to_string(),
        title: title.map(|t| t.to_string()),
        job: job.to_string(),
        voice: Vec::new(),
        anti_jobs: Vec::new(),
        skills: Vec::new(),
        mcp: Vec::new(),
        model: None,
        memory: Vec::new(),
        mode: None,
        permissions: Default::default(),
        boss: false,
        max_steps: None,
    };
    candidate.validate()?;

    // Append as raw TOML so the file's comments and formatting are preserved.
    let manifest_path = project.join(spoke_core::MANIFEST_FILE);
    let mut body = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    body.push_str("\n[[bots]]\n");
    body.push_str(&format!("name = {}\n", toml_quote(name)));
    if let Some(title) = title {
        body.push_str(&format!("title = {}\n", toml_quote(title)));
    }
    body.push_str(&format!("job = {}\n", toml_quote(job)));
    std::fs::write(&manifest_path, body)
        .with_context(|| format!("writing {}", manifest_path.display()))?;

    println!(
        "Added bot '{name}' to {} — run `spoke sync` to render .nest/agents/{name}.md",
        manifest_path.display()
    );
    Ok(())
}

fn toml_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn events_path(home: &Path) -> PathBuf {
    paths::spoke_home(home).join("events.jsonl")
}

/// Append one event to the Spoke log. Harness hooks call this.
fn event(home: &Path, kind: &str, detail: Option<&str>) -> Result<()> {
    use std::io::Write;

    let path = events_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let record = serde_json::json!({
        "ts": unix_secs(),
        "kind": kind,
        "detail": detail,
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let mut line = serde_json::to_string(&record).context("serializing event")?;
    line.push('\n');

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    Ok(())
}

fn read_events(home: &Path) -> Result<Vec<serde_json::Value>> {
    let path = events_path(home);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

fn hooks(home: &Path, project: &Path, uninstall: bool) -> Result<()> {
    let manifest = Manifest::load(project)?;
    let bin = std::env::current_exe()
        .context("resolving the spoke binary path")?
        .to_string_lossy()
        .to_string();

    let mut plan = Plan::default();
    let mut targets: Vec<String> = Vec::new();
    let mut unsupported: Vec<String> = Vec::new();

    for id in &manifest.harnesses {
        let Some(harness) = find(id) else { continue };
        if !harness.detect(home) {
            continue;
        }
        if harness.id() != "claude-code" {
            unsupported.push(harness.display().to_string());
            continue;
        }
        targets.push(harness.display().to_string());
        let settings = project.join(".claude").join("settings.json");
        for (event_name, kind) in [
            ("SessionStart", "session_start"),
            ("SessionEnd", "session_end"),
            ("PreCompact", "pre_compact"),
        ] {
            if uninstall {
                plan.push(Op::RemoveJsonArrayItems {
                    path: settings.clone(),
                    pointer: vec!["hooks".to_string(), event_name.to_string()],
                    marker: format!("{bin} event"),
                    why: format!("remove Spoke's {event_name} hook"),
                });
            } else {
                plan.push(Op::MergeJsonArrayAppend {
                    path: settings.clone(),
                    pointer: vec!["hooks".to_string(), event_name.to_string()],
                    value: serde_json::json!([
                        { "hooks": [ { "type": "command", "command": format!("{bin} event {kind}") } ] }
                    ]),
                    why: format!("log {event_name} events for budget accounting"),
                });
            }
        }
    }

    for name in &unsupported {
        println!("  note: {name} uses a different hook model — not wired yet");
    }
    if targets.is_empty() {
        println!("No supported harness detected — nothing to do");
        return Ok(());
    }

    let changes = spoke_adapters::apply::apply(&plan, home, project, unix_secs())?;
    let changed = changes.iter().filter(|c| c.changed).count();
    println!(
        "{} {} file(s): {}",
        if uninstall {
            "Removed Spoke hooks from"
        } else {
            "Installed Spoke hooks into"
        },
        changed,
        targets.join(", ")
    );
    if changed > 0 {
        println!(
            "  backups: {}",
            paths::backups_dir(home, unix_secs()).display()
        );
    }
    Ok(())
}

fn budget(home: &Path, project: &Path, receipts: bool) -> Result<()> {
    let lock = Lock::load(project)?;
    let total: u64 = lock.packages.iter().map(|p| p.tokens).sum();

    println!("Spoke budget — {}", project.display());
    if lock.packages.is_empty() {
        println!("  no packages installed");
    } else {
        println!(
            "  {} package(s), ~{total} tokens of skill context per session (estimate)",
            lock.packages.len()
        );
        for package in &lock.packages {
            println!(
                "    {:<20} ~{:>6} tokens   {}",
                package.name,
                package.tokens,
                package.license.as_deref().unwrap_or("license unknown")
            );
        }
    }

    let skills = installed_skill_texts(project)?;
    let pairs = scan::overlaps(&skills, 0.4);
    if !pairs.is_empty() {
        let mut saving = 0u64;
        for (a, _, _) in &pairs {
            saving += lock
                .packages
                .iter()
                .find(|p| &p.name == a)
                .map(|p| p.tokens)
                .unwrap_or(0);
        }
        println!(
            "  overlap: {} pair(s); removing one of each would save ~{saving} tokens/session (estimate)",
            pairs.len()
        );
        for (a, b, score) in pairs.iter().take(3) {
            println!("    {:.0}%  {a} <-> {b}", score * 100.0);
        }
    }

    let events = read_events(home)?;
    let sessions = events
        .iter()
        .filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("session_start"))
        .count();
    println!(
        "  events: {} recorded, {} session(s) started",
        events.len(),
        sessions
    );
    println!("  log: {}", events_path(home).display());

    if receipts {
        println!(
            "  receipts: measurement only so far — context compression and quality benchmarking are not wired yet"
        );
        println!("            all token figures are estimates (bytes/4 heuristic)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoke_registry::package::PackageManifest;

    fn write_skill(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: learned\n---\n\nDo the learned thing.\n"),
        )
        .unwrap();
    }

    #[test]
    fn bare_skill_name_wraps_a_nest_skill_dir() {
        let home = tempfile::tempdir().unwrap();
        write_skill(&home.path().join(".nest/skills/learned-foo"), "learned-foo");
        let pkg = materialize_spec(home.path(), "learned-foo").unwrap();
        let manifest = PackageManifest::load(&pkg.join("package.toml")).unwrap();
        assert_eq!(manifest.skills.len(), 1);
        assert_eq!(manifest.skills[0].name, "learned-foo");
        assert_eq!(manifest.skills[0].path, ".");
        assert!(pkg.join("src/SKILL.md").is_file());
    }

    #[test]
    fn skill_dir_by_path_wraps_without_package_toml() {
        let home = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        write_skill(&src.path().join("learned-bar"), "learned-bar");
        let pkg = materialize_spec(
            home.path(),
            src.path().join("learned-bar").to_str().unwrap(),
        )
        .unwrap();
        let manifest = PackageManifest::load(&pkg.join("package.toml")).unwrap();
        assert_eq!(manifest.tier.as_deref(), Some("bot-authored"));
        assert_eq!(manifest.skills[0].name, "learned-bar");
    }

    #[test]
    fn dir_with_package_toml_resolves_normally() {
        let home = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let pkg_dir = src.path().join("real-pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("package.toml"), "x").unwrap();
        std::fs::write(pkg_dir.join("SKILL.md"), "ignored").unwrap();
        let path = materialize_spec(home.path(), pkg_dir.to_str().unwrap()).unwrap();
        assert_eq!(path, pkg_dir);
    }

    #[test]
    fn unknown_bare_name_passes_through() {
        let home = tempfile::tempdir().unwrap();
        let path = materialize_spec(home.path(), "not-a-skill").unwrap();
        assert_eq!(path, PathBuf::from("not-a-skill"));
    }

    #[test]
    fn traversal_names_never_escape_the_skills_dir() {
        let home = tempfile::tempdir().unwrap();
        let sneaky = home.path().join(".nest/secret");
        std::fs::create_dir_all(&sneaky).unwrap();
        std::fs::write(sneaky.join("SKILL.md"), "x").unwrap();
        let path = materialize_spec(home.path(), "../secret").unwrap();
        assert_eq!(path, PathBuf::from("../secret"));
    }

    /// A registry repo: index.toml + one manifest per package, committed.
    fn git_registry(dir: &Path, names: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(dir)
            .status()
            .unwrap();
        assert!(status.success());
        let mut index = String::new();
        for name in names {
            index += &format!(
                "[[package]]\nname = \"{name}\"\nmanifest = \"pkg/{name}/package.toml\"\n\n"
            );
            let pkg = dir.join("pkg").join(name);
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            std::fs::write(
                pkg.join("package.toml"),
                format!("name = \"{name}\"\nversion = \"0.1.0\"\n\n[source]\npath = \"src\"\n"),
            )
            .unwrap();
            std::fs::write(pkg.join("src/demo.txt"), "x").unwrap();
        }
        std::fs::write(dir.join("index.toml"), index).unwrap();
        git(&["add", "-A"]);
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(["commit", "-qm", "x"])
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn commit_into(dir: &Path) {
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        };
        run(&["add", "-A"]);
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(["commit", "-qm", "x"])
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn registry_sync_clones_and_resolves() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        git_registry(repo.path(), &["demo"]);
        registry_sync(home.path(), Some(repo.path().to_str().unwrap())).unwrap();
        let manifest = Store::new(home.path()).resolve("demo").unwrap();
        assert_eq!(manifest.name, "demo");
    }

    #[test]
    fn registry_sync_pulls_new_packages() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        git_registry(repo.path(), &["one"]);
        registry_sync(home.path(), Some(repo.path().to_str().unwrap())).unwrap();

        let pkg = repo.path().join("pkg/two");
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::write(
            pkg.join("package.toml"),
            "name = \"two\"\nversion = \"0.1.0\"\n\n[source]\npath = \"src\"\n",
        )
        .unwrap();
        std::fs::write(pkg.join("src/demo.txt"), "x").unwrap();
        let mut index = std::fs::read_to_string(repo.path().join("index.toml")).unwrap();
        index += "[[package]]\nname = \"two\"\nmanifest = \"pkg/two/package.toml\"\n";
        std::fs::write(repo.path().join("index.toml"), index).unwrap();
        commit_into(repo.path());

        registry_sync(home.path(), None).unwrap();
        assert!(Store::new(home.path()).resolve("two").is_ok());
    }

    #[test]
    fn registry_sync_without_url_needs_a_checkout() {
        let home = tempfile::tempdir().unwrap();
        let err = registry_sync(home.path(), None).unwrap_err().to_string();
        assert!(err.contains("pass a git url"), "{err}");
    }

    #[test]
    fn registry_sync_rejects_a_non_git_dir() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".spoke/registry");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.toml"), "").unwrap();
        let err = registry_sync(home.path(), Some("x"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("isn't a git checkout"), "{err}");
    }

    #[test]
    fn registry_show_reads_the_index() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        git_registry(repo.path(), &["demo"]);
        registry_sync(home.path(), Some(repo.path().to_str().unwrap())).unwrap();
        registry_show(home.path()).unwrap();
    }
}
