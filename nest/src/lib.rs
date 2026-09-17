//! The NEST engine spine.
//!
//! NEST does not re-implement an agent runtime — it *hatches* bots by
//! wrapping the agent CLIs already installed on the machine (the
//! perch-per-harness pattern): launch, stream lifecycle events, collect
//! results, no control plane.
//!
//! The vocabulary is the nest theme:
//! - [`perch`] — a wrapper around one agent CLI (each harness gets a perch).
//! - [`hatchling`] — a running bot instance (an egg hatches into a bot).
//! - [`peep`] — lifecycle events streamed off a hatchling.
//! - [`roost`] — where finished runs rest: receipts under `.nest/roost/`.
//! - [`brood`] — the dispatch council: routes a task to the right bot and
//!   walks the fallback ladder when one fails.
//! - [`clutch`] — a batch of hatchlings run in parallel.
//! - [`forage`] — the provider layer: free-first failover across
//!   declared providers, any OpenAI-compatible endpoint included.
//! - [`usage`] — per-provider limit windows (CodexBar-style): the router
//!   skips a provider whose window is spent.
//! - an *imprint* is a bot's first verified run — a bot that hasn't run
//!   isn't done.

pub mod brood;
pub mod clutch;
pub mod forage;
pub mod hatchling;
pub mod home;
pub mod patrol;
pub mod peep;
pub mod perch;
pub mod roost;
pub mod shell;
pub mod spec;
pub mod usage;
