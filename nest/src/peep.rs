//! Peeps — the lifecycle events a hatchling emits.
//!
//! Every perch translates its harness's native output format into this one
//! stream, so the shell (and later the watch-it-think dashboard) only ever
//! has to render `Peep`s.

use serde::Serialize;

/// One event from a running bot.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Peep {
    /// The harness process is up.
    Spawned { pid: u32 },
    /// A line of ordinary output.
    Output { text: String },
    /// A unit of work (a tool call, an assistant turn — whatever the
    /// harness counts as a step).
    Step { n: u32, label: String },
    /// A tool the bot wants needs approval; headless runs deny it.
    Ask { tool: String },
    /// The step budget ran out — the run was stopped.
    BudgetExceeded { steps: u32 },
    /// The wall-clock budget ran out — the run was stopped. Fires on
    /// every perch, including ones that never emit Step peeps.
    TimedOut { seconds: u64 },
    /// Finished cleanly.
    Done { code: i32 },
    /// Finished badly (non-zero exit, spawn failure, killed).
    Failed { error: String },
}
