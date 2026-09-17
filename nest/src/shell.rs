//! The shell — NEST's one-window app, served locally.
//!
//! `nest shell` starts a tiny local server: a single-page app plus a JSON
//! API over the engine. Bots' peeps stream to the page over SSE (the
//! watch-it-think view); the roost backs the Activity tab. The Tauri
//! wrapper that adds native chrome (taskbar, desktop birdy) embeds this
//! same page later — nothing here assumes a browser.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::Read;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::brood::Brood;
use crate::hatchling::{self, Receipt, Status};
use crate::home::NestDirs;
use crate::peep::Peep;
use crate::perch;
use crate::roost;
use crate::spec::{self, AgentSpec};

/// The embedded single-page app.
const INDEX: &str = include_str!("web/index.html");

/// A run in progress (or finished) — the events the page subscribes to.
struct Run {
    id: String,
    /// Insertion order — used to bound the finished-run cache.
    order: u64,
    /// Serialized peep objects, in order.
    events: Mutex<Vec<Value>>,
    /// Bots with a live hatchling right now — Spawned inserts, terminal
    /// peeps remove. Exact names only; the roster never guesses.
    active: Mutex<BTreeSet<String>>,
    done: AtomicBool,
    receipt: Mutex<Option<Receipt>>,
}

/// Finished runs kept for late-joining SSE subscribers. Receipts persist
/// in the roost forever; the in-memory event buffers do not.
const MAX_DONE_RUNS: usize = 64;

impl Run {
    fn emit(&self, value: Value) {
        self.events.lock().unwrap().push(value);
    }
    fn emit_peep(&self, bot: &str, peep: &Peep) {
        match peep {
            Peep::Spawned { .. } => {
                self.active.lock().unwrap().insert(bot.to_string());
            }
            Peep::Done { .. } | Peep::Failed { .. } | Peep::BudgetExceeded { .. } => {
                self.active.lock().unwrap().remove(bot);
            }
            _ => {}
        }
        let v = serde_json::to_value(peep).unwrap_or_default();
        self.emit(json!({ "bot": bot, "peep": v }));
    }
    fn finish(&self, receipt: Receipt) {
        *self.receipt.lock().unwrap() = Some(receipt);
        self.done.store(true, Ordering::SeqCst);
    }
}

/// One eggbot interview in flight — the walk-through that builds a bot.
/// Deterministic Q&A (name → job → voice → permissions → preview); the
/// draft card shows exactly the file that will be written, so building is
/// never a surprise.
#[derive(Default)]
struct Interview {
    step: usize,
    name: String,
    title: String,
    job: String,
    voice: String,
    profile: String,
}

/// Lowercase kebab name, starting with a letter — e.g. `bigbird`, `pr-reviewer`.
fn kebab_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
}

impl Interview {
    fn question(&self) -> &'static str {
        match self.step {
            0 => "what should we call it? (one word, like pr-reviewer)",
            1 => "nice. what's its one job?",
            2 => "how should it talk? (friendly / precise / blunt — or describe it)",
            3 => "what can it touch? (reader / writer / full)",
            _ => "preview looks right? say 'build it' (or change anything first)",
        }
    }

    fn permissions(&self) -> BTreeMap<String, &'static str> {
        let mut p = BTreeMap::new();
        match self.profile.as_str() {
            "writer" => {
                p.insert("read".into(), "allow");
                p.insert("write".into(), "allow");
                p.insert("edit".into(), "allow");
                p.insert("bash".into(), "ask");
            }
            "full" => {
                p.insert("read".into(), "allow");
                p.insert("write".into(), "allow");
                p.insert("edit".into(), "allow");
                p.insert("bash".into(), "allow");
            }
            _ => {
                p.insert("read".into(), "allow");
                p.insert("write".into(), "deny");
                p.insert("edit".into(), "deny");
                p.insert("bash".into(), "deny");
            }
        }
        p
    }

    /// The exact file content — the preview card renders this verbatim.
    fn draft(&self) -> String {
        let mut fm = String::new();
        if !self.name.is_empty() {
            fm.push_str(&format!("name: {}\n", self.name));
        }
        if !self.title.is_empty() {
            fm.push_str(&format!("title: \"{}\"\n", self.title));
        }
        if !self.job.is_empty() {
            fm.push_str(&format!("job: \"{}\"\n", self.job.replace('"', "'")));
        }
        let perms = self.permissions();
        if !perms.is_empty() {
            fm.push_str("permissions:\n");
            for (tool, policy) in perms {
                fm.push_str(&format!("  {tool}: {policy}\n"));
            }
        }
        let body = if self.voice.is_empty() {
            String::new()
        } else {
            format!("You are {}. {}.", self.name, self.voice)
        };
        format!("---\n{fm}---\n\n{body}\n")
    }
}

/// Shared shell state.
pub struct Shell {
    dirs: NestDirs,
    cwd: PathBuf,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    interviews: Mutex<HashMap<u64, Interview>>,
    seq: Mutex<u64>,
}

impl Shell {
    pub fn new(dirs: NestDirs, cwd: PathBuf) -> Self {
        Self {
            dirs,
            cwd,
            runs: Mutex::new(HashMap::new()),
            interviews: Mutex::new(HashMap::new()),
            seq: Mutex::new(0),
        }
    }

    /// Seed the in-house bot: if the roster has no `eggbot`, write one to
    /// the personal agents dir. A project `.nest/` eggbot shadows this.
    fn ensure_eggbot(&self) {
        if self.agents().0.iter().any(|a| a.name == "eggbot") {
            return;
        }
        let dir = self.dirs.home.join("agents");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let _ = std::fs::write(
            dir.join("eggbot.md"),
            "---\nname: eggbot\ntitle: Eggbot\njob: Walks you through building a bot, then writes it\npermissions.read: allow\npermissions.write: allow\npermissions.edit: allow\npermissions.bash: deny\n---\n\nYou are eggbot, the friendly bot-builder. Ask one question at a time: name, then one job, then how it talks, then what it may touch. One bot, one job — if it wants two jobs, suggest splitting it.\n",
        );
    }

    fn interview_answer(&self, session: u64, answer: &str) -> Value {
        let answer = answer.trim();
        let mut interviews = self.interviews.lock().unwrap();
        let Some(iv) = interviews.get_mut(&session) else {
            return json!({"error": "no such interview"});
        };
        match iv.step {
            0 => {
                let name = answer.to_lowercase().replace(' ', "-");
                let ok = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    && name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
                if !ok {
                    return json!({"error": "names must be lowercase-kebab, e.g. pr-reviewer", "question": iv.question(), "draft": iv.draft()});
                }
                if self.agents().0.iter().any(|a| a.name == name) {
                    return json!({"error": format!("a bot named {name} already roosts here"), "question": iv.question(), "draft": iv.draft()});
                }
                iv.name = name;
                iv.title = answer.to_string();
            }
            1 => {
                if answer.is_empty() {
                    return json!({"error": "one job, in a sentence", "question": iv.question(), "draft": iv.draft()});
                }
                iv.job = answer.to_string();
            }
            2 => {
                iv.voice = match answer {
                    "friendly" => "Speak warmly and keep it short".into(),
                    "precise" => "Answer precisely; cite files and lines".into(),
                    "blunt" => "Be blunt. No filler".into(),
                    other => other.to_string(),
                };
            }
            3 => {
                iv.profile = match answer {
                    "writer" | "full" => answer.to_string(),
                    _ => "reader".to_string(),
                };
            }
            4 => {
                if answer == "build it" || answer == "build" || answer == "yes" {
                    return json!({"done": true, "draft": iv.draft()});
                }
                // anything else restarts the loop on that topic — keep it simple: re-ask
                return json!({"question": "almost — say 'build it' when the preview looks right", "draft": iv.draft()});
            }
            _ => {}
        }
        iv.step += 1;
        json!({"question": iv.question(), "draft": iv.draft(), "done": false})
    }

    fn interview_build(&self, session: u64) -> Result<String> {
        let iv = self
            .interviews
            .lock()
            .unwrap()
            .remove(&session)
            .context("no such interview")?;
        let dir = self.dirs.home.join("agents");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.md", iv.name));
        std::fs::write(&path, iv.draft())?;
        Ok(iv.name)
    }

    fn next_order(&self) -> u64 {
        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        *seq
    }

    /// The roster plus any agent files that failed to parse — one bad
    /// file never empties the sidebar.
    fn agents(&self) -> (Vec<AgentSpec>, Vec<String>) {
        spec::load_agents_lenient(&self.dirs.agent_dirs())
    }

    fn register_run(&self, run: Arc<Run>) {
        let mut runs = self.runs.lock().unwrap();
        runs.insert(run.id.clone(), run);
        // evict the oldest finished runs past the cap; receipts persist
        // in the roost, the streamed events do not need to
        let mut done: Vec<(u64, String)> = runs
            .values()
            .filter(|r| r.done.load(Ordering::SeqCst))
            .map(|r| (r.order, r.id.clone()))
            .collect();
        if done.len() > MAX_DONE_RUNS {
            done.sort();
            for (_, id) in done.drain(..done.len() - MAX_DONE_RUNS) {
                runs.remove(&id);
            }
        }
    }

    /// A bot's current display state for the roster: a live run means
    /// `working`; otherwise its latest receipt, else `idle`.
    fn agent_state(&self, name: &str, receipts: &[Receipt]) -> &'static str {
        let runs = self.runs.lock().unwrap();
        for run in runs.values() {
            if !run.done.load(Ordering::SeqCst) && run.active.lock().unwrap().contains(name) {
                return "working";
            }
        }
        drop(runs);
        match receipts.iter().find(|r| r.agent == name).map(|r| r.status) {
            Some(Status::Done) => "done",
            Some(Status::Failed | Status::SpawnFailed) => "stuck",
            Some(Status::BudgetExceeded) => "stuck",
            None => "idle",
        }
    }

    fn state_json(&self) -> Value {
        let receipts = roost::receipts(&self.dirs.roost()).unwrap_or_default();
        let (specs, warnings) = self.agents();
        let agents: Vec<Value> = specs
            .iter()
            .map(|a| {
                json!({
                    "name": a.name,
                    "title": a.title(),
                    "job": a.job,
                    "mode": a.mode.map(|m| format!("{m:?}").to_lowercase()),
                    "boss": a.boss,
                    "max_steps": a.max_steps,
                    "state": self.agent_state(&a.name, &receipts),
                })
            })
            .collect();
        json!({
            "agents": agents,
            "warnings": warnings,
            "perches": perch::detected_perches(),
            "providers": crate::forage::Router::from_dirs(&self.dirs).list(),
            "onboarding": self.onboarding_needed(),
        })
    }

    fn runs_json(&self) -> Value {
        json!({ "receipts": roost::receipts(&self.dirs.roost()).unwrap_or_default() })
    }

    /// Spawn a hatch run in the background; returns the run id.
    fn start_hatch(&self, bot: &str, task: &str, perch_name: Option<&str>) -> Result<String> {
        let (agents, _) = self.agents();
        let agent = agents
            .iter()
            .find(|a| a.name == bot)
            .with_context(|| format!("no bot named '{bot}'"))?
            .clone();
        let perch = match perch_name {
            Some(name) => {
                perch::perch_named(name).with_context(|| format!("unknown perch '{name}'"))?
            }
            None => perch::default_perch(),
        };
        let order = self.next_order();
        let run = Arc::new(Run {
            id: format!("run-{order}"),
            order,
            events: Mutex::new(Vec::new()),
            active: Mutex::new(BTreeSet::new()),
            done: AtomicBool::new(false),
            receipt: Mutex::new(None),
        });
        self.register_run(run.clone());
        let (run_id, cwd, roost_dir) = (run.id.clone(), self.cwd.clone(), self.dirs.roost());
        let task = task.to_string();
        thread::spawn(move || {
            let receipt = match hatchling::hatch(&agent, &task, perch.as_ref(), &cwd) {
                Ok(mut h) => {
                    let run2 = run.clone();
                    let name = agent.name.clone();
                    let r = hatchling::wait(
                        &mut h,
                        &task,
                        perch.as_ref(),
                        agent.max_steps,
                        move |peep| run2.emit_peep(&name, peep),
                    );
                    r
                }
                Err(e) => {
                    run.emit_peep(
                        &agent.name,
                        &Peep::Failed {
                            error: format!("{e:#}"),
                        },
                    );
                    Receipt::spawn_failed(&agent, &task, perch.as_ref(), &format!("{e:#}"))
                }
            };
            roost::record(&roost_dir, &receipt).ok();
            run.finish(receipt);
        });
        Ok(run_id)
    }

    /// One-click install: run the perch's install command, streaming its
    /// output as peeps so the page watches it like any other run.
    fn start_install(&self, name: &str) -> Result<String> {
        let cmd = perch::all_perches()
            .into_iter()
            .find(|p| p.name() == name)
            .and_then(|p| p.install_cmd().map(|c| c.to_string()))
            .with_context(|| format!("no one-click install for '{name}'"))?;
        let order = self.next_order();
        let run = Arc::new(Run {
            id: format!("run-{order}"),
            order,
            events: Mutex::new(Vec::new()),
            active: Mutex::new(BTreeSet::new()),
            done: AtomicBool::new(false),
            receipt: Mutex::new(None),
        });
        self.register_run(run.clone());
        let run_id = run.id.clone();
        let roost_dir = self.dirs.roost();
        let name = name.to_string();
        thread::spawn(move || {
            run.emit_peep(&name, &Peep::Spawned { pid: 0 });
            let status = std::process::Command::new("sh")
                .args(["-c", &cmd])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::{BufRead, BufReader};
                    let (tx, rx) = std::sync::mpsc::channel::<String>();
                    for pipe in [
                        child
                            .stdout
                            .take()
                            .map(|s| Box::new(s) as Box<dyn Read + Send>),
                        child
                            .stderr
                            .take()
                            .map(|s| Box::new(s) as Box<dyn Read + Send>),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let tx = tx.clone();
                        thread::spawn(move || {
                            for line in BufReader::new(pipe).lines() {
                                if tx.send(line.unwrap_or_default()).is_err() {
                                    return;
                                }
                            }
                        });
                    }
                    drop(tx);
                    while let Ok(line) = rx.recv() {
                        run.emit_peep(&name, &Peep::Output { text: line });
                    }
                    child.wait()
                });
            let tail: Vec<String> = run
                .events
                .lock()
                .unwrap()
                .iter()
                .rev()
                .filter_map(|e| e["peep"]["text"].as_str().map(|s| s.to_string()))
                .take(10)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let now = hatchling::now_ms();
            let receipt = match status {
                Ok(s) if s.success() => {
                    run.emit_peep(&name, &Peep::Done { code: 0 });
                    Receipt {
                        id: format!("{name}-{now}-install"),
                        agent: name.clone(),
                        perch: "install".into(),
                        task: cmd.clone(),
                        status: Status::Done,
                        steps: 0,
                        exit_code: Some(0),
                        started_ms: now,
                        finished_ms: now,
                        tail,
                    }
                }
                Ok(s) => {
                    let err = format!("install exited {}", s.code().unwrap_or(-1));
                    run.emit_peep(&name, &Peep::Failed { error: err.clone() });
                    Receipt {
                        id: format!("{name}-{now}-install"),
                        agent: name.clone(),
                        perch: "install".into(),
                        task: cmd.clone(),
                        status: Status::Failed,
                        steps: 0,
                        exit_code: s.code(),
                        started_ms: now,
                        finished_ms: now,
                        tail,
                    }
                }
                Err(e) => {
                    let err = format!("cannot run installer: {e}");
                    run.emit_peep(&name, &Peep::Failed { error: err.clone() });
                    Receipt {
                        id: format!("{name}-{now}-install"),
                        agent: name.clone(),
                        perch: "install".into(),
                        task: cmd.clone(),
                        status: Status::SpawnFailed,
                        steps: 0,
                        exit_code: None,
                        started_ms: now,
                        finished_ms: now,
                        tail: vec![err],
                    }
                }
            };
            roost::record(&roost_dir, &receipt).ok();
            run.finish(receipt);
        });
        Ok(run_id)
    }

    /// Does the first-run wizard need to show? Yes until a boss exists
    /// and `onboarded: true` is written into settings.md.
    fn onboarding_needed(&self) -> bool {
        let has_boss = self.agents().0.iter().any(|a| a.boss);
        if has_boss {
            return false;
        }
        let settings =
            std::fs::read_to_string(self.dirs.home.join("settings.md")).unwrap_or_default();
        !settings.lines().any(|l| l.trim() == "onboarded: true")
    }

    /// Write the user's named boss (Bigbird by default) with the
    /// abilities they picked, flip the onboarded flag, and kick off any
    /// engine installs they chose — the whole first-run flow in one call.
    fn onboarding_finish(&self, body: &Value) -> Result<Value> {
        let name = body["name"]
            .as_str()
            .unwrap_or("bigbird")
            .trim()
            .to_string();
        if !kebab_name(&name) {
            anyhow::bail!("name must be lowercase kebab (e.g. bigbird)");
        }
        let job = body["job"]
            .as_str()
            .unwrap_or("the boss — keeps an eye on everything and sends work to the right bot")
            .to_string();
        let abilities: Vec<&str> = body["abilities"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| vec!["read", "write"]);
        let can_read = abilities.contains(&"read");
        let can_write = abilities.contains(&"write");
        let can_run = abilities.contains(&"commands");
        let mut perms = BTreeMap::new();
        perms.insert("read", if can_read { "allow" } else { "deny" });
        perms.insert("write", if can_write { "allow" } else { "deny" });
        perms.insert("edit", if can_write { "allow" } else { "deny" });
        perms.insert(
            "bash",
            if can_run {
                "allow"
            } else if can_write {
                "ask"
            } else {
                "deny"
            },
        );
        let mut fm = format!("---\nname: {name}\ntitle: \"{job}\"\nboss: true\npermissions:\n");
        for (tool, policy) in &perms {
            fm.push_str(&format!("  {tool}: {policy}\n"));
        }
        fm.push_str("---\n\n");
        let mut job_sentence = job.clone();
        if let Some(c) = job_sentence.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        fm.push_str(&format!(
            "You are {name}, the boss of this nest. Keep an eye on everything, \
             answer what you can, and send work to the right bot when one fits better. \
             {job_sentence}.\n"
        ));
        let agents_dir = self.dirs.home.join("agents");
        std::fs::create_dir_all(&agents_dir)?;
        std::fs::write(agents_dir.join(format!("{name}.md")), fm)?;

        let settings_path = self.dirs.home.join("settings.md");
        let mut settings = std::fs::read_to_string(&settings_path).unwrap_or_default();
        if !settings.lines().any(|l| l.trim() == "onboarded: true") {
            if !settings.is_empty() && !settings.ends_with('\n') {
                settings.push('\n');
            }
            settings.push_str("onboarded: true\n");
            std::fs::write(&settings_path, settings)?;
        }

        let mut installs = Vec::new();
        if let Some(engines) = body["engines"].as_array() {
            for e in engines.iter().filter_map(|v| v.as_str()) {
                if let Ok(id) = self.start_install(e) {
                    installs.push(json!({"engine": e, "run": id}));
                }
            }
        }
        Ok(json!({"boss": name, "installs": installs}))
    }

    /// Spawn a brood dispatch in the background.
    fn start_brood(&self, task: &str, perch_name: Option<&str>) -> Result<String> {
        let (agents, _) = self.agents();
        let routes = spec::load_routes(&self.dirs.routing_files())?;
        let brood = Brood::new(agents, routes);
        let perch = match perch_name {
            Some(name) => {
                perch::perch_named(name).with_context(|| format!("unknown perch '{name}'"))?
            }
            None => perch::default_perch(),
        };
        let order = self.next_order();
        let run = Arc::new(Run {
            id: format!("run-{order}"),
            order,
            events: Mutex::new(Vec::new()),
            active: Mutex::new(BTreeSet::new()),
            done: AtomicBool::new(false),
            receipt: Mutex::new(None),
        });
        self.register_run(run.clone());
        let (run_id, cwd, roost_dir) = (run.id.clone(), self.cwd.clone(), self.dirs.roost());
        let task = task.to_string();
        thread::spawn(move || {
            let run2 = run.clone();
            match brood.dispatch(&task, perch.as_ref(), &cwd, move |bot, peep| {
                run2.emit_peep(bot, peep)
            }) {
                Ok(dispatch) => {
                    for receipt in dispatch.all_receipts() {
                        roost::record(&roost_dir, receipt).ok();
                    }
                    match dispatch.winner {
                        Some(receipt) => run.finish(receipt),
                        None => {
                            let last = dispatch
                                .attempts
                                .last()
                                .map(|(_, r)| r.clone())
                                .unwrap_or_else(|| {
                                    Receipt::spawn_failed(
                                        &AgentSpec {
                                            name: "brood".into(),
                                            title: None,
                                            job: String::new(),
                                            model: None,
                                            mode: None,
                                            boss: false,
                                            max_steps: None,
                                            permissions: Default::default(),
                                            skills: vec![],
                                            mcp: vec![],
                                            body: String::new(),
                                        },
                                        &task,
                                        perch.as_ref(),
                                        "no candidates",
                                    )
                                });
                            run.finish(last);
                        }
                    }
                }
                Err(e) => {
                    run.emit(json!({"bot": "brood", "peep": {"kind": "failed", "error": format!("{e:#}")}}));
                    let stub = AgentSpec {
                        name: "brood".into(),
                        title: None,
                        job: String::new(),
                        model: None,
                        mode: None,
                        boss: false,
                        max_steps: None,
                        permissions: Default::default(),
                        skills: vec![],
                        mcp: vec![],
                        body: String::new(),
                    };
                    run.finish(Receipt::spawn_failed(
                        &stub,
                        &task,
                        perch.as_ref(),
                        &format!("{e:#}"),
                    ));
                }
            }
        });
        Ok(run_id)
    }
}

/// Serve the shell until killed.
pub fn serve(dirs: NestDirs, cwd: PathBuf, port: u16) -> Result<()> {
    let shell = Arc::new(Shell::new(dirs, cwd));
    shell.ensure_eggbot();
    let addr = format!("127.0.0.1:{port}")
        .to_socket_addrs()?
        .next()
        .context("cannot resolve shell address")?;
    let server = Server::http(addr).map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}"))?;
    println!("nest shell listening on http://127.0.0.1:{port}");
    // each request gets its own thread — an open SSE stream must never
    // serialize the rest of the API behind it
    for request in server.incoming_requests() {
        let shell = shell.clone();
        thread::spawn(move || {
            let mut request = request;
            let method = request.method().clone();
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(&url).to_string();
            let response = route(&shell, &method, &path, &mut request);
            let _ = request.respond(response);
        });
    }
    Ok(())
}

/// Loopback-only writes: a page on another origin can fire a form/fetch
/// at this server without ever reading the response, so an Origin header
/// that isn't loopback is rejected. No Origin header means a non-browser
/// client (curl), which is allowed.
fn origin_ok(request: &tiny_http::Request) -> bool {
    let Some(origin) = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str())
    else {
        return true;
    };
    [
        "http://127.0.0.1",
        "https://127.0.0.1",
        "http://localhost",
        "https://localhost",
        "http://[::1]",
        "https://[::1]",
    ]
    .iter()
    .any(|p| origin.starts_with(p))
}

/// Fixed-length response over a boxed reader (the route return type).
fn respond<'a>(body: String, ctype: &str, status: u16) -> Response<Box<dyn Read + Send + 'a>> {
    let len = body.len();
    Response::new(
        StatusCode(status),
        vec![Header::from_bytes("Content-Type", ctype).unwrap()],
        Box::new(std::io::Cursor::new(body.into_bytes())) as Box<dyn Read + Send + 'a>,
        Some(len),
        None,
    )
}

fn json_response<'a>(value: Value) -> Response<Box<dyn Read + Send + 'a>> {
    respond(value.to_string(), "application/json", 200)
}

fn read_body(request: &mut tiny_http::Request) -> String {
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body).ok();
    body
}

/// Event stream reader — polls the run's event buffer and yields SSE
/// chunks until the run is done and flushed.
struct RunStream {
    run: Arc<Run>,
    cursor: usize,
    sent_receipt: bool,
    /// The serialized frame being written — a `Read` may get a buffer
    /// smaller than one event, so bytes wait here until fully flushed.
    pending: VecDeque<u8>,
}

impl Read for RunStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if !self.pending.is_empty() {
                let n = self.pending.len().min(buf.len());
                for (i, b) in self.pending.drain(..n).enumerate() {
                    buf[i] = b;
                }
                return Ok(n);
            }
            let events = self.run.events.lock().unwrap();
            if self.cursor < events.len() {
                let chunk = format!("data: {}\n\n", events[self.cursor]);
                self.cursor += 1;
                drop(events);
                self.pending.extend(chunk.bytes());
                continue;
            }
            if self.run.done.load(Ordering::SeqCst) {
                drop(events);
                if self.sent_receipt {
                    return Ok(0);
                }
                self.sent_receipt = true;
                let receipt = self.run.receipt.lock().unwrap();
                match receipt.as_ref() {
                    Some(receipt) => {
                        let v = serde_json::to_string(&json!({ "receipt": receipt }))
                            .unwrap_or_default();
                        self.pending
                            .extend(format!("event: receipt\ndata: {v}\n\n").bytes());
                        continue;
                    }
                    None => return Ok(0),
                }
            }
            drop(events);
            thread::sleep(Duration::from_millis(120));
        }
    }
}

fn route<'a>(
    shell: &Arc<Shell>,
    method: &Method,
    path: &str,
    request: &mut tiny_http::Request,
) -> Response<Box<dyn Read + Send + 'a>> {
    if !matches!(method, &Method::Get) && !origin_ok(request) {
        return respond("forbidden origin".into(), "text/plain", 403);
    }
    match (method, path) {
        (&Method::Get, "/") => respond(INDEX.to_string(), "text/html; charset=utf-8", 200),
        (&Method::Get, "/api/state") => json_response(shell.state_json()),
        (&Method::Get, "/api/runs") => json_response(shell.runs_json()),
        (&Method::Get, "/api/settings") => {
            let path = shell.dirs.home.join("settings.md");
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            json_response(json!({
                "path": path.display().to_string(),
                "content": content,
            }))
        }
        (&Method::Post, "/api/settings") => {
            let body = read_body(request);
            #[derive(serde::Deserialize)]
            struct In {
                content: String,
            }
            let res = serde_json::from_str::<In>(&body)
                .map_err(|e| e.to_string())
                .and_then(|in_| {
                    let path = shell.dirs.home.join("settings.md");
                    std::fs::create_dir_all(path.parent().unwrap()).ok();
                    std::fs::write(&path, &in_.content).map_err(|e| e.to_string())?;
                    Ok(path.display().to_string())
                });
            match res {
                Ok(p) => json_response(json!({"saved": p})),
                Err(e) => json_response(json!({"error": e})),
            }
        }
        (&Method::Post, "/api/interview/start") => {
            let session = shell.next_order();
            let iv = Interview::default();
            let question = iv.question();
            shell.interviews.lock().unwrap().insert(session, iv);
            json_response(json!({"session": session, "question": question}))
        }
        (&Method::Post, "/api/interview/answer") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            json_response(shell.interview_answer(
                v["session"].as_u64().unwrap_or_default(),
                v["answer"].as_str().unwrap_or_default(),
            ))
        }
        (&Method::Post, "/api/perches/install") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            match shell.start_install(v["name"].as_str().unwrap_or_default()) {
                Ok(id) => json_response(json!({"id": id})),
                Err(e) => json_response(json!({"error": format!("{e:#}")})),
            }
        }
        (&Method::Post, "/api/onboarding") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            match shell.onboarding_finish(&v) {
                Ok(out) => json_response(out),
                Err(e) => json_response(json!({"error": format!("{e:#}")})),
            }
        }
        (&Method::Post, "/api/interview/build") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            match shell.interview_build(v["session"].as_u64().unwrap_or_default()) {
                Ok(name) => json_response(json!({"built": name})),
                Err(e) => json_response(json!({"error": format!("{e:#}")})),
            }
        }
        (&Method::Post, "/api/hatch") | (&Method::Post, "/api/brood") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            let task = v["task"].as_str().unwrap_or_default().to_string();
            let perch = v["perch"].as_str().map(|s| s.to_string());
            let res = if path == "/api/hatch" {
                let bot = v["bot"].as_str().unwrap_or_default();
                shell.start_hatch(bot, &task, perch.as_deref())
            } else {
                shell.start_brood(&task, perch.as_deref())
            };
            match res {
                Ok(id) => json_response(json!({"id": id})),
                Err(e) => json_response(json!({"error": format!("{e:#}")})),
            }
        }
        (&Method::Get, p) if p.starts_with("/api/runs/") && p.ends_with("/events") => {
            let id = &p["/api/runs/".len()..p.len() - "/events".len()];
            let run = shell.runs.lock().unwrap().get(id).cloned();
            match run {
                Some(run) => Response::new(
                    StatusCode(200),
                    vec![
                        Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                        Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                    ],
                    Box::new(RunStream {
                        run,
                        cursor: 0,
                        sent_receipt: false,
                        pending: VecDeque::new(),
                    }) as Box<dyn Read + Send>,
                    None,
                    None,
                ),
                None => json_response(json!({"error": "no such run"})),
            }
        }
        _ => respond("not found".into(), "text/plain", 404),
    }
}
