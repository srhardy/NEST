//! A clutch — a batch of hatchlings run in parallel.
//!
//! Thin for now (the council pattern builds on it): spawn N bots on the
//! same task, collect every receipt. Ordering is finish-order.

use std::path::Path;
use std::thread;

use anyhow::Result;

use crate::hatchling::{self, Receipt};
use crate::perch::Perch;
use crate::spec::AgentSpec;

/// Hatch `agent` once per entry in `tasks` (or `n` copies of `task`),
/// in parallel. Returns receipts in completion order.
pub fn run(
    agent: &AgentSpec,
    tasks: &[String],
    perch: &(dyn Perch + Sync),
    cwd: &Path,
) -> Result<Vec<Receipt>> {
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for task in tasks {
            handles.push(scope.spawn(move || {
                let mut h = hatchling::hatch(agent, task, perch, cwd)?;
                Ok(hatchling::wait(
                    &mut h,
                    task,
                    perch,
                    agent.max_steps,
                    |_| {},
                ))
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("clutch thread panicked"))
            .collect()
    })
}
