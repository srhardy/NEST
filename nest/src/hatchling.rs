//! Hatchlings — running bot instances.
//!
//! `hatch` spawns the perch's command; `wait` drives the run: reader
//! threads translate output lines into peeps through a scoped borrow of
//! the perch, peeps stream to the caller live, the step budget is
//! enforced, and the run ends as a `Receipt` for the roost.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// Why a run ended badly — the brood's ladder consults this: spawn/exit
/// failures are worth retrying on the next candidate; budget kills and
/// denials are not (another bot would be a different action, not a retry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The perch command couldn't even start.
    Spawn,
    /// The harness exited non-zero for an ordinary reason.
    Exit,
    /// A budget — steps or wall-clock — stopped the run.
    Budget,
    /// The output tail smells like a permission/sandbox denial.
    Denied,
}

/// Wall-clock budget for a run when the bot declares no `max_seconds` —
/// a headless hatch shouldn't outlive ten silent minutes.
pub const DEFAULT_MAX_SECONDS: u64 = 600;

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
    /// True when this run finished `Done` only after earlier dispatch
    /// candidates failed — a hard-won run worth feathering.
    #[serde(default)]
    pub hard_won: bool,
    /// Why the run ended badly, when it did — `None` on `Done`.
    #[serde(default)]
    pub failure_kind: Option<FailureKind>,
    /// The full peep stream for this run, `<roost>/<id>.peeps.jsonl` —
    /// the receipt is the summary; that file is the tape.
    #[serde(default)]
    pub peeps_path: Option<PathBuf>,
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
            hard_won: false,
            failure_kind: Some(FailureKind::Spawn),
            peeps_path: None,
            tail: vec![error.to_string()],
        }
    }
}

/// Spawn `task` on `perch` as `agent`, in working directory `cwd`.
pub fn hatch(
    agent: &AgentSpec,
    task: &str,
    perch: &dyn Perch,
    cwd: &Path,
    dirs: &crate::home::NestDirs,
) -> Result<Hatchling> {
    let launch = perch.launch(agent, task, dirs);
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args)
        .envs(&launch.env)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // the child leads its own process group so `kill_tree` can take the
    // whole tree down — a grandchild holding our pipes open would
    // otherwise stall the drain long after the run was stopped
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
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

/// Stop a runaway hatchling. On unix the child leads its own process
/// group (see `hatch`) — kill the whole group so grandchildren can't
/// hold the output pipes open and stall the drain. Windows kills the
/// direct child only (job-object tree kill is a follow-up).
#[cfg(unix)]
fn kill_tree(child: &mut Child) {
    unsafe {
        // negative pid = the child's entire process group
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    child.kill().ok();
}

#[cfg(not(unix))]
fn kill_tree(child: &mut Child) {
    child.kill().ok();
}

/// How long the peep drain waits for readers to notice a dead child
/// before giving up — post-kill output is residual anyway.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// Drive a hatchling to completion. `on_peep` sees every event live
/// (spawn → steps/output → terminal) and every event is appended to
/// `<roost>/<id>.peeps.jsonl` — the tape behind the receipt's summary.
/// Steps are counted against the agent's `max_steps` budget (a step is a
/// harness work unit; output-only perches emit none) and the wall clock
/// against `max_seconds` (default `DEFAULT_MAX_SECONDS`) — the universal
/// guardrail that also catches runs whose perch never emits a Step.
/// Either cap kills the child and records `BudgetExceeded`. Subagent
/// runs get a scoped spool dir (cwd/.nest/spool/<id>) that is removed
/// before the receipt lands — council runs leave no litter.
pub fn wait(
    hatchling: &mut Hatchling,
    task: &str,
    perch: &dyn Perch,
    agent: &AgentSpec,
    cwd: &Path,
    dirs: &crate::home::NestDirs,
    mut on_peep: impl FnMut(&Peep),
) -> Receipt {
    let child = &mut hatchling.child;
    let pid = child.id();

    // the peep tape: one JSON object per line, `ms` beside the peep kind.
    // Best-effort — a run isn't failed because its log couldn't open.
    let peeps_path = dirs.roost().join(format!("{}.peeps.jsonl", hatchling.id));
    let mut log = std::fs::create_dir_all(dirs.roost())
        .and_then(|_| std::fs::File::create(&peeps_path))
        .ok()
        .map(std::io::BufWriter::new);
    let mut emit = |peep: &Peep| {
        if let Some(w) = log.as_mut() {
            let mut v = serde_json::to_value(peep).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("ms".into(), now_ms().into());
            }
            writeln!(w, "{v}").ok();
        }
        on_peep(peep);
    };
    emit(&Peep::Spawned { pid });

    let max_seconds = agent.max_seconds.unwrap_or(DEFAULT_MAX_SECONDS);
    let deadline = Instant::now() + Duration::from_secs(max_seconds);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = channel::<Peep>();

    let (steps, tail, killed) = thread::scope(|scope| {
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
        // Some(kind) once a budget has killed the child — the loop then
        // just drains whatever the readers still have to say.
        let mut killed: Option<FailureKind> = None;
        loop {
            if killed.is_none() && Instant::now() >= deadline {
                killed = Some(FailureKind::Budget);
                emit(&Peep::TimedOut {
                    seconds: max_seconds,
                });
                kill_tree(child);
            }
            let next = match killed {
                // after a kill the drain is bounded — orphaned writers
                // that somehow survive can't stall the run
                Some(_) => match rx.recv_timeout(DRAIN_GRACE) {
                    Ok(peep) => Some(peep),
                    Err(_) => None,
                },
                None => match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(peep) => Some(peep),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => None,
                },
            };
            let Some(peep) = next else { break };
            match &peep {
                Peep::Step { label, .. } => {
                    steps += 1;
                    emit(&Peep::Step {
                        n: steps,
                        label: label.clone(),
                    });
                }
                Peep::Output { text } => {
                    tail.push(text.clone());
                    if tail.len() > TAIL_LINES {
                        tail.remove(0);
                    }
                    emit(&peep);
                }
                _ => emit(&peep),
            }
            if killed.is_none() {
                if let Some(max) = agent.max_steps {
                    if steps >= max {
                        killed = Some(FailureKind::Budget);
                        emit(&Peep::BudgetExceeded { steps: max });
                        kill_tree(child);
                    }
                }
            }
        }
        (steps, tail, killed)
    });

    let exit = child.wait().ok();
    let code = exit.and_then(|s| s.code());
    // a subagent's spool directory (if any) is cleaned up before the
    // receipt lands — council runs leave no workspace litter
    if agent.mode == Some(spoke_core::manifest::BotMode::Subagent) {
        let spool = cwd.join(".nest").join("spool").join(&hatchling.id);
        std::fs::remove_dir_all(&spool).ok();
    }
    let (status, failure_kind) = match killed {
        Some(kind) => (Status::BudgetExceeded, Some(kind)),
        None => match code {
            Some(0) => (Status::Done, None),
            _ => (Status::Failed, Some(perch.failure_kind(&tail))),
        },
    };
    match status {
        Status::Done => emit(&Peep::Done { code: 0 }),
        Status::Failed => emit(&Peep::Failed {
            error: format!(
                "exit code {}",
                code.map(|c| c.to_string()).unwrap_or("?".into())
            ),
        }),
        // the BudgetExceeded/TimedOut peep already landed at kill time
        Status::BudgetExceeded | Status::SpawnFailed => {}
    }
    if let Some(w) = log.as_mut() {
        w.flush().ok();
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
        hard_won: false,
        failure_kind,
        peeps_path: log.is_some().then_some(peeps_path),
        tail,
    }
}
