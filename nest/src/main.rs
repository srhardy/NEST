//! `nest` — the engine's CLI and the Expert-mode passthrough.
//!
//! The shell (Phase 2) wraps this same library; everything here works
//! headless: `nest hatch doc-keeper "sync the readme"` is a complete run.

use std::env;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use nest::brood::Brood;
use nest::clutch;
use nest::forage::{ForageRequest, Router};
use nest::hatchling::{self, Status};
use nest::home::NestDirs;
use nest::peep::Peep;
use nest::perch;
use nest::roost;
use nest::spec;

#[derive(Parser)]
#[command(
    name = "nest",
    about = "hatch bots from the CLIs you already have",
    version
)]
struct Cli {
    /// Personal .nest/ dir (default: ~/.nest, env NEST_HOME).
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Working directory the run happens in (default: cwd).
    #[arg(long, global = true)]
    cwd: Option<PathBuf>,
    /// Perch to run on (default: first detected harness, else echo).
    #[arg(long, global = true)]
    perch: Option<String>,
    /// Approve every `ask`-permissioned tool for this run (no prompt).
    #[arg(long, global = true)]
    yes: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List known harness perches and whether each is installed.
    Perches,
    /// List the roster — every bot the engine can see.
    Agents,
    /// Run one bot on a task.
    Hatch {
        /// Bot name from .nest/agents/.
        bot: String,
        /// The task, in words.
        task: Vec<String>,
    },
    /// Let the brood pick the bot (routing → boss → fallback).
    Brood { task: Vec<String> },
    /// Run one bot on many tasks in parallel (the council primitive).
    /// Each argument after the bot name is one task — quote multi-word
    /// tasks. Receipts land in completion order (fastest first).
    Clutch {
        /// Bot name from .nest/agents/.
        bot: String,
        /// The tasks, one per argument.
        task: Vec<String>,
    },
    /// Show recent runs (receipts from the roost).
    Roost,
    /// A bot's first verified run — a bot that hasn't run isn't done.
    Imprint { bot: String, task: Vec<String> },
    /// Direct provider call — free-first router across providers.toml.
    Ask {
        /// Force one provider by name (default: cheapest available).
        #[arg(long)]
        provider: Option<String>,
        prompt: Vec<String>,
    },
    /// Search everything the nest writes — receipts, sessions, skills,
    /// agents, memory — one query over the file surfaces.
    Find {
        /// The query, in words.
        query: Vec<String>,
    },
    /// Run the task through a chain of bots — each stage's output feeds
    /// the next as context (`researcher|writer|reviewer` as one command).
    Pipe {
        /// Bot names separated by `|` or `,`.
        bots: String,
        /// The initial task.
        task: String,
    },
    /// Show the memory: profile and learned facts, personal then project.
    Memory,
    /// Digest the roost into memory/daily/ and prune stale facts
    /// (duplicates archive, never delete). Ships as a disabled
    /// `reflect` automation too — flip `enabled` for a daily run.
    Reflect,
    /// List declared providers in router order (free → cheap → paid).
    Providers,
    /// Open the shell — the one-window app over the engine.
    Shell {
        /// Port to listen on (default 4747).
        #[arg(long, default_value = "4747")]
        port: u16,
    },
    /// Headless runtime — patrol + remotes with no UI. For launchd/systemd.
    Daemon {
        /// Also serve the remotes webhook on <addr> (e.g. 0.0.0.0:8080 behind
        /// a tunnel): `POST /inbox/<remote>` with `Authorization: Bearer <t>`
        /// (token = ~/.nest/remotes/<remote>/token) drops a task file into the
        /// remote's inbox; the reply lands at `GET /inbox/<remote>/<id>/reply`
        /// where <id> is the body's `chat:`/`channel:` key or the returned task id.
        /// Bodies cap at 64KiB and a remote's inbox backlog at 64 tasks.
        #[arg(long)]
        webhook: Option<String>,
    },
    /// Distill a finished run into a reusable skill (~/.nest/skills/<name>/SKILL.md).
    Feather {
        /// Run id (or unique prefix) from `nest roost`.
        run_id: String,
        /// Skill name (default: derived from the bot + run).
        #[arg(long)]
        name: Option<String>,
        /// Replace an existing skill of the same name.
        #[arg(long)]
        force: bool,
    },
}

fn peep_line(bot: &str, peep: &Peep) -> Option<String> {
    match peep {
        Peep::Spawned { pid } => Some(format!("• {bot} hatched (pid {pid})")),
        Peep::Output { text } => Some(format!("  {text}")),
        Peep::Step { n, label } => Some(format!("  ⚙ step {n} ({label})")),
        Peep::Ask { tool } => Some(format!("  ? {tool} needs approval — skipped headless")),
        Peep::Session { id } => Some(format!("  ↔ session {id}")),
        Peep::BudgetExceeded { steps } => {
            Some(format!("  ✖ step budget exceeded ({steps}) — stopping"))
        }
        Peep::TimedOut { seconds } => {
            Some(format!("  ✖ wall clock exceeded ({seconds}s) — stopping"))
        }
        Peep::Done { .. } => Some(format!("✔ {bot} done")),
        Peep::Failed { error } => Some(format!("✖ {bot} failed: {error}")),
    }
}

fn resolve(cli: &Cli) -> Result<(NestDirs, PathBuf)> {
    let cwd = cli
        .cwd
        .clone()
        .or_else(|| env::current_dir().ok())
        .context("no working directory")?;
    let dirs = NestDirs::resolve(&cwd, cli.home.clone());
    Ok((dirs, cwd))
}

fn pick_perch(cli: &Cli) -> Result<Box<dyn nest::perch::Perch>> {
    match &cli.perch {
        Some(name) => perch::perch_named(name)
            .ok_or_else(|| anyhow::anyhow!("unknown perch '{name}' — try `nest perches`")),
        None => Ok(perch::default_perch()),
    }
}

fn load_roster(dirs: &NestDirs) -> Result<Vec<spec::AgentSpec>> {
    spec::load_agents(&dirs.agent_dirs())
}

fn find_agent<'a>(agents: &'a [spec::AgentSpec], name: &str) -> Result<&'a spec::AgentSpec> {
    agents
        .iter()
        .find(|a| a.name == name)
        .with_context(|| format!("no bot named '{name}' — `nest agents` lists the roster"))
}

/// Which `ask` tools get promoted for this run: `--yes` approves all;
/// a terminal gets a y/N prompt; anything else (pipes, daemons) denies —
/// the honest default when no human can answer.
fn approve_asks(asks: &[String], yes: bool) -> std::collections::BTreeSet<String> {
    use std::io::IsTerminal;
    if yes {
        return asks.iter().cloned().collect();
    }
    if asks.is_empty() || !std::io::stdin().is_terminal() {
        return Default::default();
    }
    eprintln!("asks for: {} — approve? [y/N] ", asks.join(", "));
    let mut line = String::new();
    let ok = std::io::stdin().read_line(&mut line).is_ok()
        && matches!(line.trim().to_lowercase().as_str(), "y" | "yes");
    if ok {
        asks.iter().cloned().collect()
    } else {
        Default::default()
    }
}

fn run_bot(
    agent: &spec::AgentSpec,
    task: &str,
    cli: &Cli,
    dirs: &NestDirs,
    cwd: &std::path::Path,
) -> Result<hatchling::Receipt> {
    let perch = pick_perch(cli)?;
    // the launch-time approval gate: promote what the human approved,
    // warn about what stays denied
    let asks = agent.ask_tools();
    let approved = approve_asks(&asks, cli.yes);
    let mut agent = agent.clone();
    agent.promote_approved_asks(&approved);
    for tool in asks.iter().filter(|t| !approved.contains(*t)) {
        println!("  ? {tool} needs approval — skipped (approve with --yes)");
    }
    let mut h = hatchling::hatch(&agent, task, perch.as_ref(), cwd, dirs, None)?;
    let name = agent.name.clone();
    let receipt = hatchling::wait(&mut h, task, perch.as_ref(), &agent, cwd, dirs, |peep| {
        if let Some(line) = peep_line(&name, peep) {
            println!("{line}");
        }
    });
    let path = roost::record(&dirs.roost(), &receipt)?;
    println!("  receipt: {}", path.display());
    nest::memory::extract_async(dirs.clone(), receipt.clone());
    Ok(receipt)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Cmd::Perches => {
            for p in perch::detected_perches() {
                let mark = if p.installed {
                    "installed".to_string()
                } else {
                    p.install
                        .map(|c| format!("not found — install: {c}"))
                        .unwrap_or_else(|| "not found".into())
                };
                println!("{:<14} {}", p.name, mark);
            }
        }
        Cmd::Agents => {
            let (dirs, _) = resolve(&cli)?;
            let agents = load_roster(&dirs)?;
            if agents.is_empty() {
                println!("no bots — .nest/agents/ is empty (spoke sync first)");
            }
            for a in &agents {
                let boss = if a.boss { " (boss)" } else { "" };
                let mode = a.mode.map(|m| format!("{m:?}").to_lowercase());
                println!(
                    "{:<16} {:<10} {}{}",
                    a.name,
                    mode.unwrap_or_default(),
                    a.job,
                    boss
                );
            }
        }
        Cmd::Hatch { bot, task } => {
            let (dirs, cwd) = resolve(&cli)?;
            let agents = load_roster(&dirs)?;
            let agent = find_agent(&agents, bot)?;
            let task = task.join(" ");
            let receipt = run_bot(agent, &task, &cli, &dirs, &cwd)?;
            if receipt.status != Status::Done {
                println!(
                    "🪶 that run struggled — `nest feather {}` records what went wrong",
                    receipt.id
                );
                std::process::exit(1);
            }
        }
        Cmd::Brood { task } => {
            let (dirs, cwd) = resolve(&cli)?;
            let agents = load_roster(&dirs)?;
            let routes = spec::load_routes(&dirs.routing_files())?;
            let brood = Brood::new(agents, routes);
            let perch = pick_perch(&cli)?;
            let task = task.join(" ");
            // chief first: a planner that can answer directly or delegate;
            // on no-provider/garbage it Undecides into the routing table
            let plan = match nest::chief::plan(&task, &[], &brood.agents, &Router::from_dirs(&dirs))
            {
                nest::chief::ChiefPlan::Answer(text) => {
                    println!("{text}");
                    return Ok(());
                }
                nest::chief::ChiefPlan::Delegations(delegates) => {
                    brood.plan_chief(&task, &delegates)
                }
                nest::chief::ChiefPlan::Undecided => brood.plan(&task),
            };
            for t in &plan {
                println!("→ {} ({:?})", t.agent.name, t.via);
            }
            let asks: Vec<String> = plan
                .iter()
                .flat_map(|t| t.agent.ask_tools())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let approved = approve_asks(&asks, cli.yes);
            let dispatch = brood.dispatch_full(
                &task,
                plan,
                perch.as_ref(),
                &cwd,
                &dirs,
                &approved,
                |bot, peep| {
                    if let Some(line) = peep_line(bot, peep) {
                        println!("{line}");
                    }
                },
            )?;
            for receipt in dispatch.all_receipts() {
                let path = roost::record(&dirs.roost(), receipt)?;
                println!("  receipt: {}", path.display());
                nest::memory::extract_async(dirs.clone(), receipt.clone());
            }
            match &dispatch.winner {
                Some(winner) if winner.hard_won => println!(
                    "🪶 hard-won run — `nest feather {}` distills what the fallback taught",
                    winner.id
                ),
                Some(_) => {}
                None => {
                    if let Some((_, receipt)) = dispatch.attempts.last() {
                        println!(
                            "🪶 every candidate failed — `nest feather {}` records what not to try",
                            receipt.id
                        );
                    }
                    bail!("every candidate failed — receipts are in the roost");
                }
            }
        }
        Cmd::Clutch { bot, task } => {
            let (dirs, cwd) = resolve(&cli)?;
            let agents = load_roster(&dirs)?;
            let agent = find_agent(&agents, bot)?;
            if task.is_empty() {
                bail!("clutch needs at least one task — `nest clutch {bot} \"task one\" \"task two\"`");
            }
            let perch = pick_perch(&cli)?;
            // same launch gate as `hatch` — asks approve or stay denied
            let approved = approve_asks(&agent.ask_tools(), cli.yes);
            let mut agent = agent.clone();
            agent.promote_approved_asks(&approved);
            let receipts = clutch::run(&agent, task, perch.as_ref(), &cwd, &dirs);
            let mut failed = 0;
            for receipt in &receipts {
                let path = roost::record(&dirs.roost(), receipt)?;
                nest::memory::extract_async(dirs.clone(), receipt.clone());
                println!(
                    "  {} → {:?} (steps {}) — receipt: {}",
                    receipt.task,
                    receipt.status,
                    receipt.steps,
                    path.display()
                );
                if receipt.status != Status::Done {
                    failed += 1;
                }
            }
            if failed > 0 {
                bail!(
                    "{failed} of {} clutch tasks did not finish Done",
                    receipts.len()
                );
            }
        }
        Cmd::Roost => {
            let (dirs, _) = resolve(&cli)?;
            for r in roost::receipts(&dirs.roost())? {
                let secs = (r.finished_ms - r.started_ms) as f64 / 1000.0;
                println!(
                    "{:<44} {:<16} {:<15} steps {:<3} {:.1}s",
                    r.id,
                    r.agent,
                    format!("{:?}", r.status).to_lowercase(),
                    r.steps,
                    secs
                );
            }
        }
        Cmd::Imprint { bot, task } => {
            let (dirs, cwd) = resolve(&cli)?;
            let agents = load_roster(&dirs)?;
            let agent = find_agent(&agents, bot)?;
            let task = task.join(" ");
            let receipt = run_bot(agent, &task, &cli, &dirs, &cwd)?;
            if receipt.status == Status::Done {
                println!("🐣 {} imprinted — first run verified", agent.name);
            } else {
                println!(
                    "🪶 that run struggled — `nest feather {}` records what went wrong",
                    receipt.id
                );
                bail!(
                    "{} did not imprint — a bot that hasn't run isn't done",
                    agent.name
                );
            }
        }
        Cmd::Find { query } => {
            let (dirs, _) = resolve(&cli)?;
            let q = query.join(" ");
            let hits = nest::find::find(&dirs, &q);
            if hits.is_empty() {
                println!("nothing found for '{q}'");
            }
            for hit in hits {
                println!("{:<8} {:<44} {}", hit.kind, hit.id, hit.title);
                if !hit.snippet.is_empty() {
                    println!("          {}", hit.snippet);
                }
            }
        }
        Cmd::Pipe { bots, task } => {
            let (dirs, cwd) = resolve(&cli)?;
            let agents = load_roster(&dirs)?;
            let names: Vec<&str> = bots
                .split(['|', ','])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if names.len() < 2 {
                bail!("pipe needs at least two bots — `nest pipe \"researcher|writer|reviewer\" \"task\"`");
            }
            let mut context = String::new();
            for (i, name) in names.iter().enumerate() {
                let agent = find_agent(&agents, name)?;
                let stage_task = if context.is_empty() {
                    task.clone()
                } else {
                    format!(
                        "{task}\n\n## Pipeline context — output from earlier stages\n\n{context}"
                    )
                };
                println!("—— stage {}: {name} ——", i + 1);
                let receipt = run_bot(agent, &stage_task, &cli, &dirs, &cwd)?;
                if receipt.status != hatchling::Status::Done {
                    bail!(
                        "stage '{name}' {:?} — the pipeline stops here",
                        receipt.status
                    );
                }
                context.push_str(&format!("\n### {name}\n\n{}\n", receipt.tail.join("\n")));
            }
        }
        Cmd::Memory => {
            let (dirs, _) = resolve(&cli)?;
            let mut any = false;
            for (label, path, content) in nest::memory::files(&dirs) {
                if content.trim().is_empty() {
                    continue;
                }
                any = true;
                println!("== {label} — {}", path.display());
                println!("{}", content.trim_end());
                println!();
            }
            if !any {
                println!(
                    "nothing remembered yet — facts land in ~/.nest/memory/facts.md after runs"
                );
            }
        }
        Cmd::Reflect => {
            let (dirs, _) = resolve(&cli)?;
            let report = nest::memory::reflect(&dirs)?;
            println!(
                "reflect: {} runs digested, {} facts kept, {} archived → {}",
                report.receipts_seen,
                report.facts_kept,
                report.facts_archived,
                report.digest.display()
            );
        }
        Cmd::Providers => {
            let (dirs, _) = resolve(&cli)?;
            let router = Router::from_dirs(&dirs);
            for p in router.list() {
                let mark = if p.exhausted {
                    "over limit"
                } else if p.available {
                    "ready"
                } else {
                    "not installed"
                };
                let model = p.model.as_deref().unwrap_or("-");
                let usage = p.usage.as_deref().unwrap_or("-");
                println!(
                    "{:<18} {:<8} {:<6} {:<14} {:<22} {}",
                    p.name, p.kind, p.cost, model, usage, mark
                );
            }
        }
        Cmd::Ask { provider, prompt } => {
            let (dirs, _) = resolve(&cli)?;
            let req = ForageRequest {
                prompt: prompt.join(" "),
                model: provider.clone(),
                tier: None,
            };
            let res = Router::from_dirs(&dirs).complete(&req)?;
            println!("{} [{}]", res.text, res.provider);
        }
        Cmd::Shell { port } => {
            let (dirs, cwd) = resolve(&cli)?;
            nest::shell::serve(dirs, cwd, *port)?;
        }
        Cmd::Feather {
            run_id,
            name,
            force,
        } => {
            let (dirs, _) = resolve(&cli)?;
            let dir = nest::feather::feather(&dirs, run_id, name.as_deref(), *force)?;
            println!("feathered {}", dir.display());
        }
        Cmd::Daemon { webhook } => {
            let (dirs, cwd) = resolve(&cli)?;
            nest::shell::daemon(dirs, cwd, webhook.as_deref())?;
        }
    }
    Ok(())
}
