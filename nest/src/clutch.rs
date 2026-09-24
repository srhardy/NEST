//! A clutch — a batch of hatchlings run in parallel.
//!
//! Thin for now (the council pattern builds on it): spawn N bots on the
//! same task, collect every receipt. Receipts arrive in completion order —
//! each worker sends its result over a channel as it finishes. A spawn
//! failure doesn't abort the clutch: it lands as a `SpawnFailed` receipt.

use std::path::Path;
use std::sync::mpsc::channel;
use std::thread;

use crate::hatchling::{self, Receipt};
use crate::perch::Perch;
use crate::spec::AgentSpec;

/// Hatch `agent` once per entry in `tasks`, in parallel.
/// Returns receipts in completion order (fastest first).
pub fn run(
    agent: &AgentSpec,
    tasks: &[String],
    perch: &(dyn Perch + Sync),
    cwd: &Path,
    dirs: &crate::home::NestDirs,
) -> Vec<Receipt> {
    let (tx, rx) = channel::<Receipt>();
    thread::scope(|scope| {
        for task in tasks {
            let tx = tx.clone();
            scope.spawn(move || {
                let receipt = match hatchling::hatch(agent, task, perch, cwd, dirs) {
                    Ok(mut h) => hatchling::wait(&mut h, task, perch, agent, cwd, |_| {}),
                    Err(e) => Receipt::spawn_failed(agent, task, perch, &format!("{e:#}")),
                };
                tx.send(receipt).ok();
            });
        }
        drop(tx);
    });
    rx.iter().collect()
}
