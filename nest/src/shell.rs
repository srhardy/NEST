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
use crate::patrol;
use crate::peep::Peep;
use crate::perch;
use crate::roost;
use crate::spec::{self, AgentSpec};

use spoke_adapters::nest::render_bot_md;
use spoke_core::manifest::{Bot, BotMode, Manifest, ToolPolicy};

/// The embedded single-page app.
const INDEX: &str = include_str!("web/index.html");
/// The desktop-birdy page for the wrapper's float window.
const BIRDY: &str = include_str!("web/birdy.html");

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
/// Eight questions (name → job → voice → not-job → touch → skills →
/// model → test-drive); the draft card shows exactly the rendered agent
/// file Spoke would write, so building is never a surprise.
#[derive(Default)]
struct Interview {
    step: usize,
    name: String,
    title: String,
    job: String,
    voice: String,
    anti_jobs: String,
    profile: String,
    skills: String,
    model: String,
    /// extra frontmatter the test-drive hatch used (imprint proof)
    test_drove: bool,
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
            3 => "what is explicitly NOT its job? (one line, or 'none')",
            4 => "what can it touch? (reader / writer / full)",
            5 => "any skills it starts with? (comma names from ~/.nest/skills, or 'none')",
            6 => "which model tier? (concise / balanced / full — or 'none')",
            _ => "preview looks right? say 'build it' (or change anything first)",
        }
    }

    fn permissions(&self) -> BTreeMap<String, ToolPolicy> {
        let mut p = BTreeMap::new();
        match self.profile.as_str() {
            "writer" => {
                p.insert("read".into(), ToolPolicy::Allow);
                p.insert("write".into(), ToolPolicy::Allow);
                p.insert("edit".into(), ToolPolicy::Allow);
                p.insert("bash".into(), ToolPolicy::Ask);
            }
            "full" => {
                p.insert("read".into(), ToolPolicy::Allow);
                p.insert("write".into(), ToolPolicy::Allow);
                p.insert("edit".into(), ToolPolicy::Allow);
                p.insert("bash".into(), ToolPolicy::Allow);
            }
            _ => {
                p.insert("read".into(), ToolPolicy::Allow);
                p.insert("write".into(), ToolPolicy::Deny);
                p.insert("edit".into(), ToolPolicy::Deny);
                p.insert("bash".into(), ToolPolicy::Deny);
            }
        }
        p
    }

    fn skill_list(&self) -> Vec<String> {
        self.skills
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.to_lowercase() != "none")
            .collect()
    }

    /// The manifest bot — the single truth `interview_build` appends to
    /// `agent.toml`. Rendering goes through the same `render_bot_md` Spoke
    /// uses, so the preview card and the file on disk can never disagree.
    fn bot(&self) -> Bot {
        let voice_line = if self.voice.is_empty() {
            format!("You are {}.", self.name)
        } else {
            format!("You are {}. {}.", self.name, self.voice)
        };
        let anti: Vec<String> = {
            let a = self.anti_jobs.trim();
            if a.is_empty() || a.eq_ignore_ascii_case("none") {
                Vec::new()
            } else {
                vec![a.to_string()]
            }
        };
        Bot {
            name: self.name.clone(),
            title: Some(self.title.clone()),
            job: self.job.clone(),
            voice: vec![voice_line],
            anti_jobs: anti,
            skills: self.skill_list(),
            mcp: Vec::new(),
            model: {
                let m = self.model.trim().to_lowercase();
                if ["concise", "balanced", "full"].contains(&m.as_str()) {
                    Some(m)
                } else {
                    None
                }
            },
            memory: Vec::new(),
            mode: Some(BotMode::Primary),
            permissions: self.permissions(),
            boss: false,
            max_steps: Some(30),
        }
    }

    /// The exact file content — the Spoke render of `bot()`.
    fn draft(&self) -> String {
        render_bot_md(&self.bot())
    }

    /// A spoke-diff-style preview card: the manifest entry it will append
    /// plus the rendered agent file. Persona + files + undo, not just text.
    fn preview(&self) -> String {
        let b = self.bot();
        let mut card = String::from("agent.toml — will append:\n\n[[bots]]\n");
        card.push_str(&format!("name = \"{}\"\n", b.name));
        card.push_str(&format!(
            "title = \"{}\"\n",
            b.title.clone().unwrap_or_default()
        ));
        card.push_str(&format!("job = \"{}\"\n", b.job));
        if !b.voice.is_empty() {
            card.push_str(&format!("voice = [\"{}\"]\n", b.voice.join(" ")));
        }
        if !b.anti_jobs.is_empty() {
            card.push_str(&format!(
                "anti_jobs = [{}]\n",
                b.anti_jobs
                    .iter()
                    .map(|a| format!("\"{a}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !b.skills.is_empty() {
            card.push_str(&format!("skills = [{}]\n", b.skills.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ")));
        }
        if let Some(m) = &b.model {
            card.push_str(&format!("model = \"{m}\"\n"));
        }
        card.push_str("mode = \"primary\"\nmax_steps = 30\npermissions = { ");
        card.push_str(
            &b.permissions
                .iter()
                .map(|(t, p)| {
                    let w = match p {
                        ToolPolicy::Allow => "allow",
                        ToolPolicy::Ask => "ask",
                        ToolPolicy::Deny => "deny",
                    };
                    format!("{t} = \"{w}\"")
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        card.push_str(" }\n");
        card.push_str("\nrenders .nest/agents/<name>.md via spoke sync:\n\n");
        card.push_str(&self.draft());
        card.push_str("\nundo: delete the [[bots]] entry + `spoke sync` (backups in ~/.spoke/backups/)");
        if !self.test_drove {
            card.push_str("\ntest-drive: pending — 'build it' runs the bot once (echo perch in tests)");
        }
        card
    }
}

/// Shared shell state.
pub struct Shell {
    dirs: NestDirs,
    cwd: PathBuf,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    interviews: Mutex<HashMap<u64, Interview>>,
    /// Scheduled/event automations the patrol is watching.
    duties: Vec<patrol::Duty>,
    patrol_warnings: Vec<String>,
    seq: Mutex<u64>,
}

impl Shell {
    pub fn new(dirs: NestDirs, cwd: PathBuf) -> Self {
        let (duties, patrol_warnings) = patrol::load_duties(&dirs, &cwd);
        Self {
            dirs,
            cwd,
            runs: Mutex::new(HashMap::new()),
            interviews: Mutex::new(HashMap::new()),
            duties,
            patrol_warnings,
            seq: Mutex::new(0),
        }
    }

    /// One dispatch path for routines: the patrol's timer and the
    /// `/api/duties/<name>/fire` webhook both land here.
    fn fire_duty(&self, d: &patrol::Duty) {
        let prompt = format!("[routine '{}'] {}", d.spec.name, d.spec.prompt);
        let _ = match &d.spec.bot {
            Some(bot) => self.start_hatch(bot, &prompt, None),
            None => self.start_brood(&prompt, None),
        };
    }

    /// First run plants a commented providers.toml — the format is
    /// learned by example, not by reading docs.
    fn ensure_providers(&self) {
        let path = self.dirs.home.join("providers.toml");
        if path.exists() {
            return;
        }
        let _ = std::fs::write(
            path,
            "# providers — the free-first router tries these top-to-bottom, first answer wins.\n#\n# [[providers]]            # any local CLI: prompt on stdin, answer on stdout\n# name = \"local-helper\"\n# kind = \"command\"\n# program = \"ollama\"       # or llm, claude -p, …\n# args = [\"run\", \"qwen3\"]\n# cost = \"free\"            # free | cheap | paid — free always answers first\n#\n# [[providers]]            # ollama shorthand — the free-forever local\n# name = \"ollama-qwen\"\n# kind = \"ollama\"\n# model = \"qwen3\"\n# cost = \"free\"\n#\n# [[providers]]            # any OpenAI-compatible endpoint — free tiers included\n# name = \"openrouter-free\"\n# kind = \"openai\"\n# base_url = \"https://openrouter.ai/api/v1\"\n# model = \"deepseek/deepseek-chat-v3-0324:free\"\n# api_key_env = \"OPENROUTER_KEY\"   # env var name, never the key itself\n# cost = \"free\"\n# limit = 50               # CodexBar-style cap…\n# window = \"24h\"           # …per rolling window (5h, 24h, 7d); spent providers sit out\n",
        );
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
            "---\nname: eggbot\ntitle: Eggbot\njob: Walks you through building a bot, then writes it\npermissions:\n  read: allow\n  write: allow\n  edit: allow\n  bash: deny\n---\n\nYou are eggbot, the friendly bot-builder. Ask one question at a time: name, then one job, then how it talks, then what it may touch. One bot, one job — if it wants two jobs, suggest splitting it.\n",
        );
    }

    fn interview_answer(&self, session: u64, answer: &str) -> Value {
        let answer = answer.trim();
        let mut interviews = self.interviews.lock().unwrap();
        let Some(iv) = interviews.get_mut(&session) else {
            return json!({"error": "no such interview"});
        };
        // NOTE: `iv` borrows `interviews` mutably — compute draft/question
        // from `iv` itself, never via a second lookup (borrow checker).
        match iv.step {
            0 => {
                let name = answer.to_lowercase().replace(' ', "-");
                let ok = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    && name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
                if !ok {
                    let out = json!({"error": "names must be lowercase-kebab, e.g. pr-reviewer", "question": iv.question(), "draft": iv.preview()});
                    return out;
                }
                if self.agents().0.iter().any(|a| a.name == name) {
                    let out = json!({"error": format!("a bot named {name} already roosts here"), "question": iv.question(), "draft": iv.preview()});
                    return out;
                }
                iv.name = name;
                iv.title = answer.to_string();
            }
            1 => {
                if answer.is_empty() {
                    let out = json!({"error": "one job, in a sentence", "question": iv.question(), "draft": iv.preview()});
                    return out;
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
                iv.anti_jobs = answer.to_string();
            }
            4 => {
                iv.profile = match answer {
                    "writer" | "full" => answer.to_string(),
                    _ => "reader".to_string(),
                };
            }
            5 => {
                iv.skills = answer.to_string();
            }
            6 => {
                iv.model = match answer.to_lowercase().as_str() {
                    "concise" | "balanced" | "full" => answer.to_lowercase(),
                    _ => String::new(),
                };
            }
            7 => {
                if answer == "build it" || answer == "build" || answer == "yes" {
                    let out = json!({"done": true, "draft": iv.preview()});
                    return out;
                }
                // anything else restarts the loop on that topic — keep it simple: re-ask
                let out = json!({"question": "almost — say 'build it' when the preview looks right", "draft": iv.preview()});
                return out;
            }
            _ => {}
        }
        iv.step += 1;
        let out = json!({"question": iv.question(), "draft": iv.preview(), "done": false});
        out
    }

    /// Where eggbot writes new bots: the project's agent.toml when the cwd
    /// has one, else the personal home. Manifest-first — the rendered
    /// .nest/agents/<name>.md follows via a Spoke sync of the nest harness.
    fn interview_manifest_dir(&self) -> PathBuf {
        if self.cwd.join("agent.toml").is_file() {
            self.cwd.clone()
        } else {
            self.dirs.home.clone()
        }
    }

    fn interview_build(&self, session: u64) -> Result<Value> {
        let iv = self
            .interviews
            .lock()
            .unwrap()
            .remove(&session)
            .context("no such interview")?;
        let bot = iv.bot();
        bot.validate().context("interview produced an invalid bot")?;
        let project = self.interview_manifest_dir();
        let manifest_path = project.join(spoke_core::manifest::MANIFEST_FILE);
        // load or seed a manifest — never clobber comments, append raw TOML
        let mut manifest = if manifest_path.is_file() {
            Manifest::load(&project).context("reading agent.toml")?
        } else {
            Manifest::default()
        };
        if manifest.bots.iter().any(|b| b.name == bot.name) {
            anyhow::bail!("bot '{}' is already declared in {}", bot.name, manifest_path.display());
        }
        if !manifest_path.is_file() {
            std::fs::write(&manifest_path, Manifest::template())
                .with_context(|| format!("seeding {}", manifest_path.display()))?;
            manifest = Manifest::load(&project).context("reading seeded agent.toml")?;
        }
        let mut body = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str("\n[[bots]]\n");
        body.push_str(&format!("name = \"{}\"\n", bot.name.replace('"', "")));
        if let Some(title) = &bot.title {
            body.push_str(&format!("title = \"{}\"\n", title.replace('"', "'")));
        }
        body.push_str(&format!("job = \"{}\"\n", bot.job.replace('"', "'")));
        if !bot.voice.is_empty() {
            body.push_str(&format!(
                "voice = [{}]\n",
                bot.voice
                    .iter()
                    .map(|v| format!("\"{}\"", v.replace('"', "'")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !bot.anti_jobs.is_empty() {
            body.push_str(&format!(
                "anti_jobs = [{}]\n",
                bot.anti_jobs
                    .iter()
                    .map(|a| format!("\"{}\"", a.replace('"', "'")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !bot.skills.is_empty() {
            body.push_str(&format!(
                "skills = [{}]\n",
                bot.skills
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(m) = &bot.model {
            body.push_str(&format!("model = \"{m}\"\n"));
        }
        body.push_str("mode = \"primary\"\nmax_steps = 30\npermissions = { ");
        body.push_str(
            &bot.permissions
                .iter()
                .map(|(t, p)| {
                    let w = match p {
                        ToolPolicy::Allow => "allow",
                        ToolPolicy::Ask => "ask",
                        ToolPolicy::Deny => "deny",
                    };
                    format!("{t} = \"{w}\"")
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        body.push_str(" }\n");
        std::fs::write(&manifest_path, &body)
            .with_context(|| format!("writing {}", manifest_path.display()))?;
        // validate the merged manifest before rendering
        let manifest = Manifest::load(&project).context("re-reading agent.toml")?;
        manifest.validate().context("merged agent.toml is invalid")?;
        // render via the nest adapter with backups — same path as `spoke sync`
        let harness = spoke_adapters::nest::Nest;
        let ctx = spoke_adapters::Ctx::new(&self.dirs.home, &project, &manifest);
        let plan = spoke_adapters::Harness::plan(&harness, &ctx);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let changes = spoke_adapters::apply::apply(&plan, &self.dirs.home, &project, now)
            .context("rendering .nest/ from agent.toml")?;
        let mut rendered: Vec<String> = changes
            .iter()
            .filter(|c| c.changed)
            .map(|c| c.path.display().to_string())
            .collect();
        // personal bots live flat in home/agents/ (the engine's personal
        // dir), not under the adapter's project .nest/ — write the same
        // Spoke render there too so the roster picks it up
        if project == self.dirs.home {
            let dir = self.dirs.home.join("agents");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{}.md", bot.name));
            std::fs::write(&path, render_bot_md(&bot))?;
            rendered.push(path.display().to_string());
        }
        // a bot that hasn't run isn't done — hatch it once on the echo
        // stub and record the receipt. Echo is always available, so the
        // interview can never fail for lack of a paid CLI on PATH.
        let home_dirs = NestDirs::resolve(&self.cwd, Some(self.dirs.home.clone()));
        let agents = spec::load_agents(&home_dirs.agent_dirs())?;
        let agent = agents
            .iter()
            .find(|a| a.name == bot.name)
            .context("rendered bot not found in roster")?;
        let perch = perch::perch_named("echo").unwrap();
        let mut h = hatchling::hatch(agent, "introduce yourself in one line", perch.as_ref(), &self.cwd, &self.dirs)?;
        let receipt = hatchling::wait(&mut h, "introduce yourself", perch.as_ref(), agent.max_steps, |_| {});
        let receipt_path = roost::record(&self.dirs.roost(), &receipt)?;
        Ok(json!({
            "built": bot.name,
            "manifest": manifest_path.display().to_string(),
            "rendered": rendered,
            "test_drive": {
                "status": format!("{:?}", receipt.status),
                "receipt": receipt_path.display().to_string(),
            },
        }))
    }

    fn next_order(&self) -> u64 {
        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        *seq
    }

    /// The roster plus any agent files that failed to parse — one bad
    /// file never empties the sidebar. Re-resolved per call so a `.nest/`
    /// created mid-run (eggbot's first render) shows up immediately.
    fn agents(&self) -> (Vec<AgentSpec>, Vec<String>) {
        let dirs = NestDirs::resolve(&self.cwd, Some(self.dirs.home.clone()));
        spec::load_agents_lenient(&dirs.agent_dirs())
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
    /// `working` (output/steps flowing) or `thinking` (hatched, no output
    /// yet — the watch-it-think beat); otherwise its latest receipt, else
    /// `idle`.
    fn agent_state(&self, name: &str, receipts: &[Receipt]) -> &'static str {
        let runs = self.runs.lock().unwrap();
        for run in runs.values() {
            if run.done.load(Ordering::SeqCst) {
                continue;
            }
            let active = run.active.lock().unwrap();
            if !active.contains(name) {
                continue;
            }
            let events = run.events.lock().unwrap();
            let saw_output = events.iter().any(|e| {
                e.get("bot").and_then(|b| b.as_str()) == Some(name)
                    && e
                        .get("peep")
                        .and_then(|p| p.get("kind"))
                        .and_then(|k| k.as_str())
                        .is_some_and(|k| k == "output" || k == "step")
            });
            return if saw_output { "working" } else { "thinking" };
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
        let mut all_warnings = warnings;
        all_warnings.extend(self.patrol_warnings.iter().cloned());
        json!({
            "agents": agents,
            "warnings": all_warnings,
            "perches": perch::detected_perches(),
            "providers": crate::forage::Router::from_dirs(&self.dirs).list(),
            "duties": patrol::describe(&self.duties),
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
        let (task, dirs) = (task.to_string(), self.dirs.clone());
        thread::spawn(move || {
            let receipt = match hatchling::hatch(&agent, &task, perch.as_ref(), &cwd, &dirs) {
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
                        hard_won: false,
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
                        hard_won: false,
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
                        hard_won: false,
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
        let (task, dirs) = (task.to_string(), self.dirs.clone());
        thread::spawn(move || {
            let run2 = run.clone();
            match brood.dispatch(&task, perch.as_ref(), &cwd, &dirs, move |bot, peep| {
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

/// The remotes loop: every remote bridge drops task files into
/// `~/.nest/remotes/<name>/inbox/`; we dispatch each through the brood
/// and write the answer back to that remote's outbox. The bridges stay
/// dumb — all the thinking happens here.
fn remotes_watch(shell: Arc<Shell>) {
    let remotes = shell.dirs.home.join("remotes");
    loop {
        let Ok(entries) = std::fs::read_dir(&remotes) else {
            thread::sleep(Duration::from_secs(5));
            continue;
        };
        for remote in entries.filter_map(|e| e.ok()) {
            let inbox = remote.path().join("inbox");
            let Ok(tasks) = std::fs::read_dir(&inbox) else {
                continue;
            };
            for task in tasks.filter_map(|e| e.ok()) {
                let path = task.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let Ok(body) = std::fs::read_to_string(&path) else {
                    continue;
                };
                // where does the reply go? the bridge's frontmatter says
                // (telegram: chat: <id>, slack: channel: <id>)
                let chat_id = body
                    .lines()
                    .find(|l| l.starts_with("chat:") || l.starts_with("channel:"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|s| s.trim().to_string());
                let reply_to = match chat_id {
                    // the id becomes an outbox filename — a malformed one
                    // (../, absolute paths) gets no reply at all
                    Some(id) if name_ok(&id) => Some(id),
                    Some(_) => None,
                    // generic webhooks carry no chat id — answer under the
                    // task's own filename so GET /inbox/<remote>/<id>/reply works
                    None => path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string()),
                };
                let remote_name = remote.file_name().to_string_lossy().to_string();
                let prompt = format!(
                    "A message arrived on the {remote_name} remote. Reply to it \
                     helpfully and briefly — your whole reply is sent back.\n\n{body}"
                );
                if let Ok(id) = shell.start_brood(&prompt, None) {
                    // wait for the run, then answer in the outbox
                    let (shell2, outbox) = (shell.clone(), remote.path().join("outbox"));
                    let reply_to2 = reply_to.clone();
                    thread::spawn(move || {
                        for _ in 0..1200 {
                            let done = shell2
                                .runs
                                .lock()
                                .unwrap()
                                .get(&id)
                                .map(|r| r.done.load(Ordering::SeqCst))
                                .unwrap_or(true);
                            if done {
                                break;
                            }
                            thread::sleep(Duration::from_millis(250));
                        }
                        let answer = shell2
                            .runs
                            .lock()
                            .unwrap()
                            .get(&id)
                            .and_then(|r| r.receipt.lock().unwrap().clone())
                            .map(|r| r.tail.join("\n"))
                            .unwrap_or_else(|| "(the nest had nothing to say)".into());
                        if let Some(to) = reply_to2 {
                            std::fs::create_dir_all(&outbox).ok();
                            std::fs::write(outbox.join(format!("{to}.txt")), answer).ok();
                        }
                    });
                }
                // handled either way — move it to done/ so it isn't re-read
                let done_dir = inbox.join("done");
                std::fs::create_dir_all(&done_dir).ok();
                if let Some(name) = path.file_name() {
                    std::fs::rename(&path, done_dir.join(name)).ok();
                }
            }
        }
        thread::sleep(Duration::from_secs(3));
    }
}

/// Serve the shell until killed.
pub fn serve(dirs: NestDirs, cwd: PathBuf, port: u16) -> Result<()> {
    let addr = format!("127.0.0.1:{port}")
        .to_socket_addrs()?
        .next()
        .context("cannot resolve shell address")?;
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}"))?;
    println!("nest shell listening on http://127.0.0.1:{port}");
    serve_on(dirs, cwd, listener)
}

/// The background loops every NEST runtime owns: remote bridges' inbox
/// spool + the routine patrol. `serve_on` runs them alongside HTTP;
/// `daemon` runs them with no UI at all.
fn spawn_watchers(shell: &Arc<Shell>) {
    {
        let shell = shell.clone();
        thread::spawn(move || remotes_watch(shell));
    }
    {
        let shell = shell.clone();
        thread::spawn(move || {
            patrol::run(&shell.duties, |d| shell.fire_duty(d));
        });
    }
}

/// Serve on an already-bound listener — the caller owns the port
/// reservation, so there's no bind-time race with another process.
pub fn serve_on(dirs: NestDirs, cwd: PathBuf, listener: std::net::TcpListener) -> Result<()> {
    let shell = Arc::new(Shell::new(dirs, cwd));
    shell.ensure_eggbot();
    shell.ensure_providers();
    spawn_watchers(&shell);
    let server = Server::from_listener(listener, None)
        .map_err(|e| anyhow::anyhow!("cannot serve on listener: {e}"))?;
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

/// Headless runtime — the watchers with no HTTP at all. This is the
/// "bots that work while the laptop lid is closed" unit: launchd or
/// systemd keeps one process alive, routines fire on schedule, remote
/// bridges get their replies in outbox/.
pub fn daemon(dirs: NestDirs, cwd: PathBuf, webhook: Option<&str>) -> Result<()> {
    let shell = Arc::new(Shell::new(dirs, cwd));
    shell.ensure_eggbot();
    shell.ensure_providers();
    spawn_watchers(&shell);
    match webhook {
        Some(addr) => {
            let listener = std::net::TcpListener::bind(addr)
                .with_context(|| format!("webhook cannot bind {addr}"))?;
            let (d2, c2) = (shell.dirs.clone(), shell.cwd.clone());
            thread::spawn(move || webhook_serve(d2, c2, listener));
            println!(
                "nest daemon — {} routine(s) armed, watching remotes; webhook on {addr}; ctrl-c to stop",
                shell.duties.len()
            );
        }
        None => {
            println!(
                "nest daemon — {} routine(s) armed, watching remotes; ctrl-c to stop",
                shell.duties.len()
            );
        }
    }
    loop {
        thread::park();
    }
}

/// The external-trigger bridge — the inbox contract over HTTP instead of a
/// filesystem drop. `POST /inbox/<remote>` writes the request body to
/// `~/.nest/remotes/<remote>/inbox/<ms>-<pid>.md`; the existing remotes loop
/// dispatches it like any bridge drop, so a body starting with `chat: <id>`
/// gets its reply at `GET /inbox/<remote>/<id>/reply` (outbox/<id>.txt).
///
/// Auth: `Authorization: Bearer <token>` against
/// `~/.nest/remotes/<remote>/token`. A remote without a token file never
/// accepts inbound traffic — opting a remote in is creating that file.
/// HTTPS is the tunnel's job (cloudflared/tailscale), not this server's.
pub fn webhook_serve(dirs: NestDirs, cwd: PathBuf, listener: std::net::TcpListener) {
    // the bridge only touches files — its own Shell shares the same
    // ~/.nest layout as the daemon's, and remotes_watch does the dispatch
    let shell = Arc::new(Shell::new(dirs, cwd));
    let Ok(server) = Server::from_listener(listener, None) else {
        return;
    };
    for request in server.incoming_requests() {
        let shell = shell.clone();
        thread::spawn(move || {
            let mut request = request;
            let method = request.method().clone();
            let path = request
                .url()
                .split('?')
                .next()
                .unwrap_or_default()
                .to_string();
            let response = webhook_route(&shell, &method, &path, &mut request);
            let _ = request.respond(response);
        });
    }
}

/// names that can safely appear in a URL segment or become a filename —
/// anything else (dots, slashes, NUL) would escape the outbox
fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn webhook_route<'a>(
    shell: &Arc<Shell>,
    method: &Method,
    path: &str,
    request: &mut tiny_http::Request,
) -> Response<Box<dyn Read + Send + 'a>> {
    let err = |status: u16, msg: &str| {
        respond(
            json!({"error": msg}).to_string(),
            "application/json",
            status,
        )
    };
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    if parts.first() != Some(&"inbox") || parts.len() < 2 || !name_ok(parts[1]) {
        return err(404, "not found");
    }
    let remote = parts[1];
    let dir = shell.dirs.home.join("remotes").join(remote);
    let token = std::fs::read_to_string(dir.join("token")).unwrap_or_default();
    let token = token.trim();
    let bearer = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default();
    if token.is_empty() || bearer != format!("Bearer {token}") {
        return err(403, "unauthorized — no token match for this remote");
    }
    match (method, parts.as_slice()) {
        (&Method::Post, [_, _]) => {
            // bounded read — a token holder still can't fill the disk
            let mut body = String::new();
            request
                .as_reader()
                .take(64 * 1024 + 1)
                .read_to_string(&mut body)
                .ok();
            if body.len() > 64 * 1024 {
                return err(413, "body too large (64KiB max)");
            }
            if body.trim().is_empty() {
                return err(400, "empty body");
            }
            let inbox = dir.join("inbox");
            let pending = std::fs::read_dir(&inbox)
                .map(|es| {
                    es.flatten()
                        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
                        .count()
                })
                .unwrap_or(0);
            if pending >= 64 {
                return err(429, "inbox backlog — try again once tasks drain");
            }
            if let Err(e) = std::fs::create_dir_all(&inbox) {
                return err(500, &format!("cannot create inbox: {e}"));
            }
            let id = format!("{}-{}", crate::hatchling::now_ms(), std::process::id());
            match std::fs::write(inbox.join(format!("{id}.md")), &body) {
                Ok(()) => json_response(json!({"id": id, "remote": remote})),
                Err(e) => err(500, &format!("cannot write task: {e}")),
            }
        }
        (&Method::Get, [_, _, id, "reply"]) if name_ok(id) => {
            match std::fs::read_to_string(dir.join("outbox").join(format!("{id}.txt"))) {
                Ok(body) => respond(body, "text/plain; charset=utf-8", 200),
                Err(_) => err(404, "pending"),
            }
        }
        _ => err(404, "not found"),
    }
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
        (&Method::Get, "/birdy") => respond(BIRDY.to_string(), "text/html; charset=utf-8", 200),
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
        // Expert mode's raw view: the real config files, verbatim, and
        // whether each exists — "expert" must mean showing the truth.
        // POST /api/expert saves agent.toml back (validated first).
        // POST /api/cli runs read-only nest commands (agents, perches,
        // providers, roost) and returns their output.
        (&Method::Get, "/api/expert") => {
            let mut files: Vec<PathBuf> =
                vec![shell.cwd.join("agent.toml"), shell.cwd.join("AGENTS.md")];
            if let Some(p) = &shell.dirs.project {
                files.extend(
                    [
                        "routing.toml",
                        "automations.toml",
                        "services.toml",
                        "mcp.json",
                    ]
                    .iter()
                    .map(|f| p.join(f)),
                );
            }
            files.push(shell.dirs.home.join("providers.toml"));
            let note = |p: &PathBuf| match p.file_name().and_then(|f| f.to_str()) {
                Some("mcp.json") => "projected config — spoke writes it, nest doesn't read it yet",
                Some("services.toml") => "declared services — boot on start not wired yet",
                _ => "",
            };
            let files: Vec<Value> = files
                .iter()
                .map(|p| {
                    json!({
                        "path": p.display().to_string(),
                        "exists": p.is_file(),
                        "note": note(p),
                        "content": std::fs::read_to_string(p).unwrap_or_default(),
                    })
                })
                .collect();
            json_response(json!({"files": files}))
        }
        (&Method::Post, "/api/expert") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            let path = v["path"].as_str().unwrap_or_default().to_string();
            let content = v["content"].as_str().unwrap_or_default().to_string();
            // only agent.toml is writable — it is validated before it lands
            let allowed = shell.cwd.join("agent.toml") == PathBuf::from(&path)
                || shell.dirs.home.join("agent.toml") == PathBuf::from(&path);
            if !allowed {
                return json_response(json!({"error": "only agent.toml is editable in expert mode"}));
            }
            let parsed: Result<Manifest, _> = toml::from_str(&content);
            let manifest = match parsed {
                Ok(m) => m,
                Err(e) => return json_response(json!({"error": format!("agent.toml does not parse: {e}")})),
            };
            if let Err(e) = manifest.validate() {
                return json_response(json!({"error": format!("agent.toml invalid: {e:#}")}));
            }
            match std::fs::write(&path, &content) {
                Ok(()) => json_response(json!({"saved": path})),
                Err(e) => json_response(json!({"error": format!("cannot write: {e}")})),
            }
        }
        (&Method::Post, "/api/cli") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            let argv: Vec<String> = v["argv"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let Some((cmd, _rest)) = argv.split_first() else {
                return json_response(json!({"error": "empty command"}));
            };
            // read-only allowlist — hatches stay in the terminal for now
            let out = match cmd.as_str() {
                "agents" => {
                    let (agents, warnings) = shell.agents();
                    let mut lines: Vec<String> = agents
                        .iter()
                        .map(|a| {
                            format!(
                                "{:<16} {:<10} {}{}",
                                a.name,
                                a.mode.map(|m| format!("{m:?}").to_lowercase()).unwrap_or_default(),
                                a.job,
                                if a.boss { " (boss)" } else { "" }
                            )
                        })
                        .collect();
                    lines.extend(warnings.iter().map(|w| format!("warning: {w}")));
                    lines.join("\n")
                }
                "perches" => perch::detected_perches()
                    .iter()
                    .map(|p| {
                        format!(
                            "{:<14} {}",
                            p.name,
                            if p.installed {
                                "installed".to_string()
                            } else {
                                p.install
                                    .map(|c| format!("not found — install: {c}"))
                                    .unwrap_or_else(|| "not found".into())
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                "providers" => crate::forage::Router::from_dirs(&shell.dirs)
                    .list()
                    .iter()
                    .map(|p| {
                        format!(
                            "{:<18} {:<8} {:<6} {}",
                            p.name,
                            p.kind,
                            p.cost,
                            if p.exhausted {
                                "over limit"
                            } else if p.available {
                                "ready"
                            } else {
                                "not installed"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                "roost" => roost::receipts(&shell.dirs.roost())
                    .unwrap_or_default()
                    .iter()
                    .take(20)
                    .map(|r| {
                        format!(
                            "{:<44} {:<16} {:?} steps {}",
                            r.id, r.agent, r.status, r.steps
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                other => {
                    return json_response(json!({"error": format!("refused — '{other}' is not read-only (agents, perches, providers, roost)")}));
                }
            };
            json_response(json!({"output": out}))
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
                Ok(out) => json_response(out),
                Err(e) => json_response(json!({"error": format!("{e:#}")})),
            }
        }
        // distill a finished run into ~/.nest/skills/<name>/SKILL.md — the
        // activity tab's feather button shares this path with `nest feather`
        (&Method::Post, "/api/feather") => {
            let body = read_body(request);
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            match crate::feather::feather(
                &shell.dirs,
                v["run_id"].as_str().unwrap_or_default(),
                v["name"].as_str(),
                v["force"].as_bool().unwrap_or(false),
            ) {
                Ok(dir) => json_response(json!({"dir": dir.display().to_string()})),
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
        // routine webhook — POST /api/duties/<name>/fire runs the routine
        // now (the settings "fire" button and external triggers share it)
        (&Method::Post, p) if p.starts_with("/api/duties/") && p.ends_with("/fire") => {
            let name = &p["/api/duties/".len()..p.len() - "/fire".len()];
            match shell.duties.iter().find(|d| d.spec.name == name) {
                Some(d) => {
                    d.stamp(patrol::now());
                    shell.fire_duty(d);
                    json_response(json!({"fired": name}))
                }
                None => json_response(json!({"error": "no such routine"})),
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
