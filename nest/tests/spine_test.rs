//! End-to-end spine: load a roster from a .nest/ dir, hatch on the builtin
//! perches, dispatch through the brood, land a receipt in the roost.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use nest::brood::{Brood, Via};
use nest::clutch;
use nest::hatchling::{self, FailureKind, Status};
use nest::home::NestDirs;
use nest::peep::Peep;
use nest::perch::{perch_named, Echo, Fail, Perch};
use nest::roost;
use nest::spec::{self, AgentSpec};

fn write_agent(dir: &Path, name: &str, extra_frontmatter: &str) {
    let agents = dir.join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join(format!("{name}.md")),
        format!("---\nname: {name}\njob: test job\n{extra_frontmatter}---\n\nYou are {name}.\n"),
    )
    .unwrap();
}

fn spec_with(name: &str, max_steps: Option<u32>) -> AgentSpec {
    AgentSpec {
        name: name.into(),
        title: None,
        job: "j".into(),
        model: None,
        mode: None,
        boss: false,
        max_steps,
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

#[test]
fn echo_hatch_runs_to_done_and_leaves_a_receipt() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    let dirs = NestDirs::resolve(cwd, Some(cwd.to_path_buf()));
    let agent = spec_with("echobot", None);
    let perch = Echo;
    let mut h = hatchling::hatch(&agent, "say hi", &perch, cwd, &dirs).unwrap();
    let mut saw_spawned = false;
    let mut saw_output = false;
    let receipt = hatchling::wait(&mut h, "say hi", &perch, &agent, cwd, &dirs, |p| {
        saw_spawned |= matches!(p, Peep::Spawned { .. });
        saw_output |= matches!(p, Peep::Output { .. });
    });
    assert_eq!(receipt.status, Status::Done);
    assert!(saw_spawned && saw_output);
    assert!(receipt.tail.iter().any(|l| l.contains("say hi")));

    let roost_dir = cwd.join(".nest").join("roost");
    roost::record(&roost_dir, &receipt).unwrap();
    assert_eq!(roost::receipts(&roost_dir).unwrap().len(), 1);
}

#[test]
fn fail_perch_records_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let agent = spec_with("doomed", None);
    let perch = Fail;
    let mut h = hatchling::hatch(&agent, "try", &perch, tmp.path(), &dirs).unwrap();
    let receipt = hatchling::wait(&mut h, "try", &perch, &agent, tmp.path(), &dirs, |_| {});
    assert_eq!(receipt.status, Status::Failed);
}

/// A test perch whose every line is a Step — proves the budget kills runs.
struct Stepper;
impl Perch for Stepper {
    fn name(&self) -> &'static str {
        "stepper"
    }
    fn binary(&self) -> &'static str {
        ""
    }
    fn launch(&self, _a: &AgentSpec, _t: &str, _d: &NestDirs) -> nest::perch::Launch {
        nest::perch::Launch {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "for i in 1 2 3 4 5; do echo step; sleep 0.05; done".into(),
            ],
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        Some(Peep::Step {
            n: 0,
            label: line.into(),
        })
    }
}

#[test]
fn step_budget_kills_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let agent = spec_with("loopy", Some(2));
    let perch = Stepper;
    let mut h = hatchling::hatch(&agent, "loop", &perch, tmp.path(), &dirs).unwrap();
    let mut budget_hit = false;
    let receipt = hatchling::wait(&mut h, "loop", &perch, &agent, tmp.path(), &dirs, |p| {
        budget_hit |= matches!(p, Peep::BudgetExceeded { .. });
    });
    assert!(budget_hit);
    assert_eq!(receipt.status, Status::BudgetExceeded);
    assert_eq!(receipt.failure_kind, Some(FailureKind::Budget));
    assert!(receipt.steps >= 2);
}

/// A perch that sleeps forever — the wall-clock budget must kill runs
/// even when no Step peeps ever arrive (output-only perches included).
struct Sleeper;
impl Perch for Sleeper {
    fn name(&self) -> &'static str {
        "sleeper"
    }
    fn binary(&self) -> &'static str {
        ""
    }
    fn launch(&self, _a: &AgentSpec, _t: &str, _d: &NestDirs) -> nest::perch::Launch {
        nest::perch::Launch {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 60".into()],
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        Some(Peep::Output { text: line.into() })
    }
}

#[test]
fn wall_clock_budget_kills_silent_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let mut agent = spec_with("sleeper", None);
    agent.max_seconds = Some(1);
    let perch = Sleeper;
    let mut h = hatchling::hatch(&agent, "wait", &perch, tmp.path(), &dirs).unwrap();
    let mut timed_out = false;
    let receipt = hatchling::wait(&mut h, "wait", &perch, &agent, tmp.path(), &dirs, |p| {
        timed_out |= matches!(p, Peep::TimedOut { seconds: 1 });
    });
    assert!(timed_out, "a TimedOut peep must be emitted");
    assert_eq!(receipt.status, Status::BudgetExceeded);
    assert_eq!(receipt.failure_kind, Some(FailureKind::Budget));
    // killed near the 1s cap, not after the child's 60s sleep
    assert!(receipt.finished_ms - receipt.started_ms < 10_000);
}

#[test]
fn wait_writes_the_peep_tape_next_to_the_receipt() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    let dirs = NestDirs::resolve(cwd, Some(cwd.to_path_buf()));
    let agent = spec_with("echobot", None);
    let perch = Echo;
    let mut h = hatchling::hatch(&agent, "say hi", &perch, cwd, &dirs).unwrap();
    let receipt = hatchling::wait(&mut h, "say hi", &perch, &agent, cwd, &dirs, |_| {});
    assert_eq!(receipt.status, Status::Done);

    let tape = receipt.peeps_path.expect("receipt names its peep tape");
    assert_eq!(
        tape,
        dirs.roost().join(format!("{}.peeps.jsonl", receipt.id))
    );
    let text = fs::read_to_string(&tape).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(lines.len() >= 3, "spawned + output + done at minimum");
    assert!(lines.iter().all(|l| l["ms"].as_u64().is_some()));
    assert_eq!(lines[0]["kind"], "spawned");
    assert_eq!(lines.last().unwrap()["kind"], "done");
    assert!(lines.iter().any(|l| l["kind"] == "output"));
}

/// A perch whose stderr looks like a sandbox denial — the classifier
/// must read the tail, not just the exit code.
struct Denied;
impl Perch for Denied {
    fn name(&self) -> &'static str {
        "denied"
    }
    fn binary(&self) -> &'static str {
        ""
    }
    fn launch(&self, _a: &AgentSpec, _t: &str, _d: &NestDirs) -> nest::perch::Launch {
        nest::perch::Launch {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "echo 'bash: permission denied by sandbox' >&2; exit 1".into(),
            ],
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        Some(Peep::Output { text: line.into() })
    }
}

#[test]
fn denial_markers_classify_the_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let agent = spec_with("sandboxed", None);
    let perch = Denied;
    let mut h = hatchling::hatch(&agent, "try", &perch, tmp.path(), &dirs).unwrap();
    let receipt = hatchling::wait(&mut h, "try", &perch, &agent, tmp.path(), &dirs, |_| {});
    assert_eq!(receipt.status, Status::Failed);
    assert_eq!(receipt.failure_kind, Some(FailureKind::Denied));
}

/// Dispatch runs one perch across candidates, so the scripted perch
/// picks its behavior off the agent name — same trick as brood.rs tests.
struct ScriptedGate;
impl Perch for ScriptedGate {
    fn name(&self) -> &'static str {
        "gate"
    }
    fn binary(&self) -> &'static str {
        ""
    }
    fn launch(&self, agent: &AgentSpec, _t: &str, _d: &NestDirs) -> nest::perch::Launch {
        nest::perch::Launch {
            program: "sh".into(),
            args: if agent.name == "denied-bot" {
                vec![
                    "-c".into(),
                    "echo 'operation not permitted' >&2; exit 1".into(),
                ]
            } else {
                vec!["-c".into(), "echo ok".into()]
            },
            env: BTreeMap::new(),
        }
    }
    fn parse(&self, line: &str) -> Option<Peep> {
        Some(Peep::Output { text: line.into() })
    }
}

#[test]
fn brood_stops_the_ladder_on_denied() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let agents = vec![spec_with("denied-bot", None), spec_with("backup", None)];
    let routes = vec![spoke_core::manifest::Route {
        patterns: vec!["task".into()],
        bot: "denied-bot".into(),
        fallback: vec!["backup".into()],
    }];
    let brood = Brood::new(agents, routes);
    let dispatch = brood
        .dispatch("do the task", &ScriptedGate, tmp.path(), &dirs, |_, _| {})
        .unwrap();
    // the denial stops the ladder — backup was never even hatched
    assert!(dispatch.winner.is_none());
    assert_eq!(dispatch.attempts.len(), 1);
    assert_eq!(
        dispatch.attempts[0].1.failure_kind,
        Some(FailureKind::Denied)
    );
}

#[test]
fn brood_falls_back_when_first_candidate_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let agents = vec![spec_with("primary", None), spec_with("backup", None)];
    let routes = vec![spoke_core::manifest::Route {
        patterns: vec!["task".into()],
        bot: "primary".into(),
        fallback: vec!["backup".into()],
    }];
    let brood = Brood::new(agents, routes);
    let plan = brood.plan("do the task");
    assert_eq!(plan[0].via, Via::Matched("task".into()));
    // primary on `fail`, backup on `echo` — dispatch walks the ladder.
    let (receipt, attempts) = {
        let fail = Fail;
        let echo = Echo;
        // first candidate fails → second wins (dispatch uses one perch; emulate
        // by running candidates manually through the same plan)
        let mut attempts = Vec::new();
        let mut winner = None;
        for target in &plan {
            let perch: &dyn Perch = if target.agent.name == "primary" {
                &fail
            } else {
                &echo
            };
            let mut h = hatchling::hatch(&target.agent, "t", perch, tmp.path(), &dirs).unwrap();
            let r = hatchling::wait(&mut h, "t", perch, &target.agent, tmp.path(), &dirs, |_| {});
            attempts.push((target.agent.name.clone(), r.status));
            if r.status == Status::Done {
                winner = Some(r);
                break;
            }
        }
        (winner.unwrap(), attempts)
    };
    assert_eq!(receipt.agent, "backup");
    assert_eq!(attempts[0].1, Status::Failed);
    assert_eq!(attempts[1].1, Status::Done);
}

#[test]
fn egress_advisory_does_not_treat_url_text_as_a_network_breach() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let mut agent = spec_with("offline", None);
    agent.egress = Some("none".into());
    let perch = Echo;
    // Egress is advisory until a real OS/network boundary exists. Merely
    // printing a URL must not be reported as a blocked network attempt.
    let mut h = hatchling::hatch(
        &agent,
        "fetch https://example.com now",
        &perch,
        tmp.path(),
        &dirs,
    )
    .unwrap();
    let receipt = hatchling::wait(&mut h, "fetch", &perch, &agent, tmp.path(), &dirs, |_| {});
    assert_eq!(receipt.status, Status::Done);
}

#[test]
fn subagent_spool_is_cleaned_up() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let mut agent = spec_with("council-member", None);
    agent.mode = Some(spoke_core::manifest::BotMode::Subagent);
    let perch = Echo;
    let mut h = hatchling::hatch(&agent, "deliberate", &perch, tmp.path(), &dirs).unwrap();
    // plant spool litter the run should remove
    let spool = tmp.path().join(".nest").join("spool").join(&h.id);
    std::fs::create_dir_all(&spool).unwrap();
    std::fs::write(spool.join("scratch.txt"), "notes").unwrap();
    let receipt = hatchling::wait(
        &mut h,
        "deliberate",
        &perch,
        &agent,
        tmp.path(),
        &dirs,
        |_| {},
    );
    assert_eq!(receipt.status, Status::Done);
    assert!(!spool.exists(), "subagent spool must be cleaned");
}

#[test]
fn loads_roster_and_routes_from_nest_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home/.nest");
    let project = tmp.path().join("proj");
    fs::create_dir_all(project.join(".nest")).unwrap();
    write_agent(&project.join(".nest"), "boss", "boss: true\n");
    write_agent(&home, "pack-bot", "");
    fs::write(
        project.join(".nest/routing.toml"),
        "# generated\n[[route]]\npatterns = [\"docs\"]\nbot = \"pack-bot\"\nfallback = [\"boss\"]\n",
    )
    .unwrap();
    let dirs = NestDirs::resolve(&project, Some(home.clone()));
    let agents = spec::load_agents(&dirs.agent_dirs()).unwrap();
    assert_eq!(agents.len(), 2);
    let routes = spec::load_routes(&dirs.routing_files()).unwrap();
    assert_eq!(routes.len(), 1);
    let brood = Brood::new(agents, routes);
    let plan = brood.plan("update the docs");
    assert_eq!(plan[0].agent.name, "pack-bot");
    assert_eq!(plan[1].agent.name, "boss");
}

#[test]
fn clutch_runs_tasks_in_parallel_and_returns_all_receipts() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = NestDirs::resolve(tmp.path(), Some(tmp.path().to_path_buf()));
    let agent = spec_with("worker", None);
    let perch = Echo;
    let tasks = vec![
        "first".to_string(),
        "second".to_string(),
        "third".to_string(),
    ];
    let receipts = clutch::run(&agent, &tasks, &perch, tmp.path(), &dirs);
    assert_eq!(receipts.len(), 3);
    assert!(receipts.iter().all(|r| r.status == Status::Done));
    let mut got: Vec<&str> = receipts.iter().map(|r| r.task.as_str()).collect();
    got.sort();
    assert_eq!(got, vec!["first", "second", "third"]);
}

#[test]
fn perch_lookup_by_name() {
    assert!(perch_named("echo").is_some());
    assert!(perch_named("fail").is_some());
    assert!(perch_named("nonexistent").is_none());
}
