//! Hatchlings — running bot instances.
//!
//! `hatch` spawns the perch's command; `wait` drives the run: reader
//! threads translate output lines into peeps through a scoped borrow of
//! the perch, peeps stream to the caller live, the step budget is
//! enforced, and the run ends as a `Receipt` for the roost.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::peep::Peep;
use crate::perch::Perch;
use crate::spec::AgentSpec;

/// Unix epoch millis — receipts and hatchling ids stamp with this.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Terminal state of a run — what the receipt records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Done,
    Failed,
    BudgetExceeded,
    SpawnFailed,
}

/// A spawned bot. Call `wait` to stream peeps and collect the receipt.
pub struct Hatchling {
    /// `bot-<epoch ms>-<pid>` — also the receipt filename.
    pub id: String,
    pub agent: String,
    pub perch: String,
    child: Child,
    started_ms: u64,
}

/// The record of a finished run — written into the roost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub agent: String,
    pub perch: String,
    pub task: String,
    pub status: Status,
    pub steps: u32,
    pub exit_code: Option<i32>,
    pub started_ms: u64,
    pub finished_ms: u64,
    /// Last lines of output (capped), for receipts and triage.
    pub tail: Vec<String>,
}

const TAIL_LINES: usize = 40;

impl Receipt {
    /// A receipt for a run that never got off the ground — the roost keeps
    /// spawn failures too, so no attempt vanishes from the activity log.
    pub fn spawn_failed(agent: &AgentSpec, task: &str, perch: &dyn Perch, error: &str) -> Self {
        let now = now_ms();
        Self {
            id: format!("{}-{}-spawnfail", agent.name, now),
            agent: agent.name.clone(),
            perch: perch.name().to_string(),
            task: task.to_string(),
            status: Status::SpawnFailed,
            steps: 0,
            exit_code: None,
            started_ms: now,
            finished_ms: now,
            tail: vec![error.to_string()],
        }
    }
}

/// Spawn `task` on `perch` as `agent`, in working directory `cwd`.
pub fn hatch(agent: &AgentSpec, task: &str, perch: &dyn Perch, cwd: &Path, dirs: &crate::home::NestDirs) -> Result<Hatchling> {
    let launch = perch.launch(agent, task, dirs);
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args)
        .envs(&launch.env)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .with_context(|| format!("cannot spawn perch '{}'", perch.name()))?;
    Ok(Hatchling {
        id: format!("{}-{}-{}", agent.name, now_ms(), child.id()),
        agent: agent.name.clone(),
        perch: perch.name().to_string(),
        child,
        started_ms: now_ms(),
    })
}

/// Drive a hatchling to completion. `on_peep` sees every event live
/// (spawn → steps/output → terminal). Steps are counted; when they pass
/// `max_steps` the child is killed and the run records `BudgetExceeded`.
pub fn wait(
    hatchling: &mut Hatchling,
    task: &str,
    perch: &dyn Perch,
    max_steps: Option<u32>,
    mut on_peep: impl FnMut(&Peep),
) -> Receipt {
    let child = &mut hatchling.child;
    let pid = child.id();
    on_peep(&Peep::Spawned { pid });

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = channel::<Peep>();

    let (steps, tail, killed_budget) = thread::scope(|scope| {
        let streams: Vec<Box<dyn std::io::Read + Send>> = [
            stdout.map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            stderr.map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        .collect();
        for stream in streams {
            let tx = tx.clone();
            scope.spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Some(peep) = perch.parse(&line) {
                        if tx.send(peep).is_err() {
                            return;
                        }
                    }
                }
            });
        }
        drop(tx);

        let mut steps = 0u32;
        let mut tail: Vec<String> = Vec::new();
        let mut killed_budget = false;
        while let Ok(peep) = rx.recv() {
            match &peep {
                Peep::Step { label, .. } => {
                    steps += 1;
                    on_peep(&Peep::Step {
                        n: steps,
                        label: label.clone(),
                    });
                }
                Peep::Output { text } => {
                    tail.push(text.clone());
                    if tail.len() > TAIL_LINES {
                        tail.remove(0);
                    }
                    on_peep(&peep);
                }
                _ => on_peep(&peep),
            }
            if let Some(max) = max_steps {
                if steps >= max && !killed_budget {
                    killed_budget = true;
                    on_peep(&Peep::BudgetExceeded { steps: max });
                    child.kill().ok();
                }
            }
        }
        (steps, tail, killed_budget)
    });

    let exit = child.wait().ok();
    let code = exit.and_then(|s| s.code());
    let status = if killed_budget {
        Status::BudgetExceeded
    } else {
        match code {
            Some(0) => Status::Done,
            _ => Status::Failed,
        }
    };
    match status {
        Status::Done => on_peep(&Peep::Done { code: 0 }),
        Status::Failed => on_peep(&Peep::Failed {
            error: format!(
                "exit code {}",
                code.map(|c| c.to_string()).unwrap_or("?".into())
            ),
        }),
        Status::BudgetExceeded => {}
        Status::SpawnFailed => {}
    }

    Receipt {
        id: hatchling.id.clone(),
        agent: hatchling.agent.clone(),
        perch: hatchling.perch.clone(),
        task: task.to_string(),
        status,
        steps,
        exit_code: code,
        started_ms: hatchling.started_ms,
        finished_ms: now_ms(),
        tail,
    }
}
