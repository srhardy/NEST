//! `nest` — the engine's CLI and the Expert-mode passthrough.
//!
//! The shell (Phase 2) wraps this same library; everything here works
//! headless: `nest hatch doc-keeper "sync the readme"` is a complete run.

use std::env;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use nest::brood::Brood;
use nest::forage::{EchoForage, Forage, ForageRequest};
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
    /// Show recent runs (receipts from the roost).
    Roost,
    /// A bot's first verified run — a bot that hasn't run isn't done.
    Imprint { bot: String, task: Vec<String> },
    /// Direct provider call (the forage layer, echo stub for now).
    Ask { prompt: Vec<String> },
    /// Open the shell — the one-window app over the engine.
    Shell {
        /// Port to listen on (default 4747).
        #[arg(long, default_value = "4747")]
        port: u16,
    },
}

fn peep_line(bot: &str, peep: &Peep) -> Option<String> {
    match peep {
        Peep::Spawned { pid } => Some(format!("• {bot} hatched (pid {pid})")),
        Peep::Output { text } => Some(format!("  {text}")),
        Peep::Step { n, label } => Some(format!("  ⚙ step {n} ({label})")),
        Peep::Ask { tool } => Some(format!("  ? {tool} needs approval — skipped headless")),
        Peep::BudgetExceeded { steps } => {
            Some(format!("  ✖ step budget exceeded ({steps}) — stopping"))
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

fn run_bot(
    agent: &spec::AgentSpec,
    task: &str,
    cli: &Cli,
    dirs: &NestDirs,
    cwd: &std::path::Path,
) -> Result<hatchling::Receipt> {
    let perch = pick_perch(cli)?;
    let mut h = hatchling::hatch(agent, task, perch.as_ref(), cwd)?;
    let name = agent.name.clone();
    let receipt = hatchling::wait(&mut h, task, perch.as_ref(), agent.max_steps, |peep| {
        if let Some(line) = peep_line(&name, peep) {
            println!("{line}");
        }
    });
    let path = roost::record(&dirs.roost(), &receipt)?;
    println!("  receipt: {}", path.display());
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
            let plan = brood.plan(&task);
            for t in &plan {
                println!("→ {} ({:?})", t.agent.name, t.via);
            }
            let dispatch = brood.dispatch(&task, perch.as_ref(), &cwd, |bot, peep| {
                if let Some(line) = peep_line(bot, peep) {
                    println!("{line}");
                }
            })?;
            for receipt in dispatch.all_receipts() {
                let path = roost::record(&dirs.roost(), receipt)?;
                println!("  receipt: {}", path.display());
            }
            if dispatch.winner.is_none() {
                bail!("every candidate failed — receipts are in the roost");
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
                bail!(
                    "{} did not imprint — a bot that hasn't run isn't done",
                    agent.name
                );
            }
        }
        Cmd::Ask { prompt } => {
            let req = ForageRequest {
                prompt: prompt.join(" "),
                model: None,
                tier: None,
            };
            let res = EchoForage.complete(&req)?;
            println!("{} [{}]", res.text, res.provider);
        }
        Cmd::Shell { port } => {
            let (dirs, cwd) = resolve(&cli)?;
            nest::shell::serve(dirs, cwd, *port)?;
        }
    }
    Ok(())
}
