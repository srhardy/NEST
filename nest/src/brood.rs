//! The brood — NEST's dispatch council.
//!
//! Given a task, the brood decides which bot gets it: routes from
//! `.nest/routing.toml` (pattern match → target bot → its fallback
//! ladder), the boss bot for everything unmatched, and last-resort
//! order when neither exists. `dispatch` walks that candidate list,
//! hatching each until one finishes cleanly.

use std::path::Path;

use anyhow::{bail, Result};
use spoke_core::manifest::Route;

use crate::hatchling::{self, Receipt, Status};
use crate::peep::Peep;
use crate::perch::Perch;
use crate::spec::AgentSpec;

/// Why a bot was picked — shown on the receipt trail.
#[derive(Debug, Clone, PartialEq)]
pub enum Via {
    /// A routing pattern matched the task.
    Matched(String),
    /// Named in a route's fallback ladder.
    Fallback,
    /// The boss bot — default receiver for un-routed intents.
    Boss,
    /// No routes and no boss — only bot available.
    Only,
}

/// One dispatch candidate.
#[derive(Debug, Clone)]
pub struct Target {
    pub agent: AgentSpec,
    pub via: Via,
}

/// The council: the roster plus the routing table.
pub struct Brood {
    pub agents: Vec<AgentSpec>,
    pub routes: Vec<Route>,
}

impl Brood {
    pub fn new(agents: Vec<AgentSpec>, routes: Vec<Route>) -> Self {
        Self { agents, routes }
    }

    fn agent(&self, name: &str) -> Option<AgentSpec> {
        self.agents.iter().find(|a| a.name == name).cloned()
    }

    fn push_target(
        &self,
        out: &mut Vec<Target>,
        seen: &mut std::collections::BTreeSet<String>,
        name: &str,
        via: Via,
    ) {
        if seen.insert(name.to_string()) {
            if let Some(agent) = self.agent(name) {
                out.push(Target { agent, via });
            }
        }
    }

    /// Ordered candidates for a task. Every matching route contributes its
    /// bot + fallback chain (dedup'd); unmatched → boss; nothing → all.
    pub fn plan(&self, task: &str) -> Vec<Target> {
        let task_lc = task.to_lowercase();
        let mut out: Vec<Target> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for route in &self.routes {
            for pattern in &route.patterns {
                if task_lc.contains(&pattern.to_lowercase()) {
                    self.push_target(
                        &mut out,
                        &mut seen,
                        &route.bot,
                        Via::Matched(pattern.clone()),
                    );
                    for fallback in &route.fallback {
                        self.push_target(&mut out, &mut seen, fallback, Via::Fallback);
                    }
                    break; // first matching pattern per route is enough
                }
            }
        }
        if out.is_empty() {
            if let Some(boss) = self.agents.iter().find(|a| a.boss) {
                let name = boss.name.clone();
                self.push_target(&mut out, &mut seen, &name, Via::Boss);
            } else {
                // No routing, no boss — stable order over the whole roster.
                let names: Vec<String> = self.agents.iter().map(|a| a.name.clone()).collect();
                for name in names {
                    self.push_target(&mut out, &mut seen, &name, Via::Only);
                }
            }
        }
        out
    }

    /// Walk the candidate list: hatch each until one finishes `Done`.
    /// Every attempt's receipt is kept (the roost records failures too);
    /// `winner` is `None` when the whole ladder fails.
    pub fn dispatch(
        &self,
        task: &str,
        perch: &dyn Perch,
        cwd: &Path,
        dirs: &crate::home::NestDirs,
        on_peep: impl FnMut(&str, &Peep),
    ) -> Result<Dispatch> {
        let mut on_peep = on_peep;
        let candidates = self.plan(task);
        if candidates.is_empty() {
            bail!("the brood is empty — no bots declared in .nest/agents/");
        }
        let mut attempts = Vec::new();
        for target in candidates {
            let agent = &target.agent;
            let receipt = match hatchling::hatch(agent, task, perch, cwd, dirs) {
                Ok(mut h) => hatchling::wait(&mut h, task, perch, agent.max_steps, |peep| {
                    on_peep(&agent.name, peep)
                }),
                Err(e) => {
                    on_peep(
                        &agent.name,
                        &Peep::Failed {
                            error: format!("{e:#}"),
                        },
                    );
                    Receipt::spawn_failed(agent, task, perch, &format!("{e:#}"))
                }
            };
            let status = receipt.status;
            attempts.push((target.via.clone(), receipt));
            if status == Status::Done {
                let winner = attempts.pop().map(|(_, r)| r);
                return Ok(Dispatch { winner, attempts });
            }
        }
        Ok(Dispatch {
            winner: None,
            attempts,
        })
    }
}

/// The outcome of a brood dispatch: the run that finished `Done` (if any)
/// plus every attempt's receipt — the complete activity trail.
pub struct Dispatch {
    /// The candidate that succeeded; `None` when the whole ladder failed.
    pub winner: Option<Receipt>,
    /// Every attempted run's receipt, in order (excludes the winner).
    pub attempts: Vec<(Via, Receipt)>,
}

impl Dispatch {
    /// All receipts — attempts plus the winner — for recording.
    pub fn all_receipts(&self) -> impl Iterator<Item = &Receipt> {
        self.attempts
            .iter()
            .map(|(_, r)| r)
            .chain(self.winner.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn agent(name: &str, boss: bool) -> AgentSpec {
        AgentSpec {
            name: name.into(),
            title: None,
            job: String::new(),
            model: None,
            mode: None,
            boss,
            max_steps: None,
            permissions: BTreeMap::new(),
            skills: vec![],
            mcp: vec![],
            body: String::new(),
        }
    }

    fn route(patterns: &[&str], bot: &str, fallback: &[&str]) -> Route {
        Route {
            patterns: patterns.iter().map(|s| s.to_string()).collect(),
            bot: bot.into(),
            fallback: fallback.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn matched_route_plans_bot_then_fallback() {
        let brood = Brood::new(
            vec![
                agent("debugger", false),
                agent("council", false),
                agent("boss", true),
            ],
            vec![route(&["bug", "crash"], "debugger", &["council"])],
        );
        let plan = brood.plan("fix this bug please");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].agent.name, "debugger");
        assert_eq!(plan[0].via, Via::Matched("bug".into()));
        assert_eq!(plan[1].agent.name, "council");
        assert_eq!(plan[1].via, Via::Fallback);
    }

    #[test]
    fn unmatched_goes_to_boss() {
        let brood = Brood::new(
            vec![agent("boss", true), agent("other", false)],
            vec![route(&["bug"], "other", &[])],
        );
        let plan = brood.plan("unrelated task");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].agent.name, "boss");
        assert_eq!(plan[0].via, Via::Boss);
    }

    #[test]
    fn no_routes_no_boss_lists_all() {
        let brood = Brood::new(vec![agent("a", false), agent("b", false)], vec![]);
        assert_eq!(brood.plan("x").len(), 2);
    }
}
