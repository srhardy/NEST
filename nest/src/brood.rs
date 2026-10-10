//! The brood — NEST's dispatch council.
//!
//! Given a task, the brood decides which bot gets it: routes from
//! `.nest/routing.toml` (pattern match → target bot → its fallback
//! ladder), the boss bot for everything unmatched, and last-resort
//! order when neither exists. `dispatch` walks that candidate list,
//! hatching each until one finishes cleanly.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Result};
use spoke_core::manifest::{Route, ToolPolicy};

use crate::chief::Delegate;
use crate::hatchling::{self, FailureKind, Receipt, Status};
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
    /// The chief delegated to this bot directly.
    Chief,
}

/// One dispatch candidate.
#[derive(Debug, Clone)]
pub struct Target {
    pub agent: AgentSpec,
    pub via: Via,
    /// Per-candidate task override — chief delegations carry sub-tasks;
    /// `None` runs the dispatch's task verbatim.
    pub task: Option<String>,
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
        seen: &mut BTreeSet<String>,
        name: &str,
        via: Via,
    ) {
        if seen.insert(name.to_string()) {
            if let Some(agent) = self.agent(name) {
                out.push(Target {
                    agent,
                    via,
                    task: None,
                });
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

    /// The chief's candidate order: its delegations first (dedup'd,
    /// unknown bots skipped), then everything `plan` would have offered —
    /// routes and the boss remain the deterministic fallback, so a bad
    /// delegation degrades to keyword routing rather than failing.
    pub fn plan_chief(&self, task: &str, delegates: &[Delegate]) -> Vec<Target> {
        let mut out: Vec<Target> = Vec::new();
        let mut seen = BTreeSet::new();
        for d in delegates {
            if seen.insert(d.bot.clone()) {
                if let Some(agent) = self.agent(&d.bot) {
                    out.push(Target {
                        agent,
                        via: Via::Chief,
                        task: d.task.clone(),
                    });
                }
            }
        }
        for target in self.plan(task) {
            if seen.insert(target.agent.name.clone()) {
                out.push(target);
            }
        }
        out
    }

    /// Dispatch with the deterministic `plan` candidate list and no
    /// approvals — the classic walk-the-ladder behavior.
    pub fn dispatch(
        &self,
        task: &str,
        perch: &dyn Perch,
        cwd: &Path,
        dirs: &crate::home::NestDirs,
        on_peep: impl FnMut(&str, &Peep),
    ) -> Result<Dispatch> {
        self.dispatch_full(
            task,
            self.plan(task),
            perch,
            cwd,
            dirs,
            &BTreeSet::new(),
            on_peep,
        )
    }

    /// Walk an explicit candidate list: hatch each until one finishes
    /// `Done`. `approved` names `ask`-policy tools the caller pre-approved
    /// — they're promoted to `allow` for this dispatch only; tools still
    /// denied emit an `Ask` peep so the tape shows what was withheld.
    /// Every attempt's receipt is kept (the roost records failures too);
    /// `winner` is `None` when the whole ladder fails.
    pub fn dispatch_full(
        &self,
        task: &str,
        candidates: Vec<Target>,
        perch: &dyn Perch,
        cwd: &Path,
        dirs: &crate::home::NestDirs,
        approved: &BTreeSet<String>,
        on_peep: impl FnMut(&str, &Peep),
    ) -> Result<Dispatch> {
        let mut on_peep = on_peep;
        if candidates.is_empty() {
            bail!("the brood is empty — no bots declared in .nest/agents/");
        }
        let mut attempts = Vec::new();
        for target in candidates {
            // promote approved asks for this run only; unapproved asks
            // stay denied (the perch's deny side) and get an Ask peep
            let mut agent = target.agent.clone();
            let asks = agent.ask_tools();
            for tool in &asks {
                if approved.contains(tool) {
                    agent.permissions.insert(tool.clone(), ToolPolicy::Allow);
                } else {
                    on_peep(&agent.name, &Peep::Ask { tool: tool.clone() });
                }
            }
            let agent = &agent;
            let task = target.task.as_deref().unwrap_or(task);
            let receipt = match hatchling::hatch(agent, task, perch, cwd, dirs, None) {
                Ok(mut h) => hatchling::wait(&mut h, task, perch, agent, cwd, dirs, |peep| {
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
            // a budget kill or a denial stops the ladder: the run happened
            // and was stopped — handing the same task to the next bot
            // wouldn't retry the failure, it'd be a different action.
            let stop = matches!(
                receipt.failure_kind,
                Some(FailureKind::Budget) | Some(FailureKind::Denied)
            );
            attempts.push((target.via.clone(), receipt));
            if status == Status::Done {
                let mut winner = attempts.pop().map(|(_, r)| r).unwrap();
                // attempts left over are the failures the ladder climbed past
                winner.hard_won = !attempts.is_empty();
                return Ok(Dispatch {
                    winner: Some(winner),
                    attempts,
                });
            }
            if stop {
                break;
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
            max_seconds: None,
            permissions: BTreeMap::new(),
            skills: vec![],
            mcp: vec![],
            budget: None,
            routines: Vec::new(),
            egress: None,
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

    /// Test perch: agents named "fails" exit 1, everyone else echoes.
    struct ScriptedPerch;
    impl Perch for ScriptedPerch {
        fn name(&self) -> &'static str {
            "scripted"
        }
        fn binary(&self) -> &'static str {
            ""
        }
        fn launch(
            &self,
            agent: &AgentSpec,
            _task: &str,
            _dirs: &crate::home::NestDirs,
            _session: Option<&str>,
        ) -> crate::perch::Launch {
            crate::perch::Launch {
                program: "sh".into(),
                args: if agent.name == "fails" {
                    vec!["-c".into(), "exit 1".into()]
                } else {
                    vec!["-c".into(), "echo ok".into()]
                },
                env: Default::default(),
            }
        }
        fn parse(&self, line: &str) -> Option<Peep> {
            Some(Peep::Output { text: line.into() })
        }
    }

    #[test]
    fn winner_after_failures_is_hard_won() {
        let brood = Brood::new(
            vec![agent("fails", false), agent("works", false)],
            vec![route(&["task"], "fails", &["works"])],
        );
        let tmp = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(tmp.path(), Some(tmp.path().join("h")));
        let dispatch = brood
            .dispatch("task", &ScriptedPerch, tmp.path(), &dirs, |_, _| {})
            .unwrap();
        let winner = dispatch.winner.as_ref().unwrap();
        assert_eq!(winner.agent, "works");
        assert!(winner.hard_won);
        assert_eq!(dispatch.attempts.len(), 1);
    }

    #[test]
    fn first_callee_winner_is_not_hard_won() {
        let brood = Brood::new(vec![agent("works", false)], vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(tmp.path(), Some(tmp.path().join("h")));
        let dispatch = brood
            .dispatch("task", &ScriptedPerch, tmp.path(), &dirs, |_, _| {})
            .unwrap();
        assert!(!dispatch.winner.as_ref().unwrap().hard_won);
    }

    #[test]
    fn chief_delegations_come_first_then_the_routes() {
        let brood = Brood::new(
            vec![
                agent("debugger", false),
                agent("doc-keeper", false),
                agent("boss", true),
            ],
            vec![route(&["bug"], "debugger", &[])],
        );
        let delegates = vec![
            Delegate {
                bot: "doc-keeper".into(),
                task: Some("log the incident".into()),
            },
            // unknown bots are skipped, not fatal
            Delegate {
                bot: "ghost".into(),
                task: None,
            },
            // duplicates don't double-hatch
            Delegate {
                bot: "doc-keeper".into(),
                task: Some("again".into()),
            },
        ];
        let plan = brood.plan_chief("fix the bug", &delegates);
        assert_eq!(plan[0].agent.name, "doc-keeper");
        assert_eq!(plan[0].via, Via::Chief);
        assert_eq!(plan[0].task.as_deref(), Some("log the incident"));
        // the deterministic plan still follows — a bad delegation
        // degrades to routing, not to nothing
        assert_eq!(plan[1].agent.name, "debugger");
        assert_eq!(plan[1].via, Via::Matched("bug".into()));
        // a matched route has no trailing boss — the boss only joins
        // when the deterministic plan comes back empty
        assert_eq!(plan.len(), 2);

        // unmatched task: the boss still trails the delegation
        let plan = brood.plan_chief("nobody routes this", &delegates);
        assert_eq!(plan[0].via, Via::Chief);
        assert_eq!(plan[1].agent.name, "boss");
        assert_eq!(plan[1].via, Via::Boss);
    }

    #[test]
    fn empty_delegations_match_plain_plan() {
        let brood = Brood::new(vec![agent("boss", true)], vec![]);
        let delegated = brood.plan_chief("x", &[]);
        let plain = brood.plan("x");
        assert_eq!(delegated.len(), plain.len());
        assert_eq!(delegated[0].agent.name, plain[0].agent.name);
    }

    #[test]
    fn unapproved_asks_emit_an_ask_peep_and_stay_denied() {
        use spoke_core::manifest::ToolPolicy;
        let mut asking = agent("asking", false);
        asking.permissions.insert("bash".into(), ToolPolicy::Ask);
        let brood = Brood::new(vec![asking], vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(tmp.path(), Some(tmp.path().join("h")));
        let mut peeps: Vec<Peep> = Vec::new();
        let dispatch = brood
            .dispatch_full(
                "t",
                brood.plan("t"),
                &ScriptedPerch,
                tmp.path(),
                &dirs,
                &Default::default(),
                |_, p| peeps.push(p.clone()),
            )
            .unwrap();
        assert!(dispatch.winner.is_some());
        assert!(peeps
            .iter()
            .any(|p| matches!(p, Peep::Ask { tool } if tool == "bash")));
    }

    #[test]
    fn approved_asks_promote_without_an_ask_peep() {
        use spoke_core::manifest::ToolPolicy;
        let mut asking = agent("asking", false);
        asking.permissions.insert("bash".into(), ToolPolicy::Ask);
        let brood = Brood::new(vec![asking], vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let dirs = crate::home::NestDirs::resolve(tmp.path(), Some(tmp.path().join("h")));
        let approved: std::collections::BTreeSet<String> = ["bash".to_string()].into();
        let mut peeps: Vec<Peep> = Vec::new();
        let dispatch = brood
            .dispatch_full(
                "t",
                brood.plan("t"),
                &ScriptedPerch,
                tmp.path(),
                &dirs,
                &approved,
                |_, p| peeps.push(p.clone()),
            )
            .unwrap();
        assert!(dispatch.winner.is_some());
        assert!(!peeps.iter().any(|p| matches!(p, Peep::Ask { .. })));
    }
}
