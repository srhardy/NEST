//! End-to-end spine: load a roster from a .nest/ dir, hatch on the builtin
//! perches, dispatch through the brood, land a receipt in the roost.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use nest::brood::{Brood, Via};
use nest::clutch;
use nest::hatchling::{self, Status};
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
        permissions: BTreeMap::new(),
        skills: vec![],
        mcp: vec![],
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
    let receipt = hatchling::wait(&mut h, "say hi", &perch, None, |p| {
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
    let receipt = hatchling::wait(&mut h, "try", &perch, None, |_| {});
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
    let receipt = hatchling::wait(&mut h, "loop", &perch, agent.max_steps, |p| {
        budget_hit |= matches!(p, Peep::BudgetExceeded { .. });
    });
    assert!(budget_hit);
    assert_eq!(receipt.status, Status::BudgetExceeded);
    assert!(receipt.steps >= 2);
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
            let r = hatchling::wait(&mut h, "t", perch, None, |_| {});
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
    let tasks = vec!["first".to_string(), "second".to_string(), "third".to_string()];
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
