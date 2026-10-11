//! Shell e2e: real server on an ephemeral port, driven over real HTTP.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use nest::home::NestDirs;
use nest::shell::serve;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(home: &std::path::Path, cwd: &std::path::Path) -> u16 {
    // dispatches must use the echo stub even when a real agent CLI is
    // installed on the box (e.g. CI runner images ship several)
    std::env::set_var("NEST_PERCH", "echo");
    std::fs::create_dir_all(home.join("agents")).unwrap();
    std::fs::write(
        home.join("agents/bigbird.md"),
        "---\nname: bigbird\njob: the boss\nboss: true\n---\nYou answer things.\n",
    )
    .unwrap();
    std::fs::write(
        home.join("routing.toml"),
        "[[route]]\npatterns = [\"doc\"]\nbot = \"doc-keeper\"\n",
    )
    .unwrap();
    std::fs::write(
        home.join("agents/doc-keeper.md"),
        "---\nname: doc-keeper\njob: docs\n---\nI keep docs.\n",
    )
    .unwrap();
    let port = free_port();
    let (dirs, cwd) = (
        NestDirs::resolve(cwd, Some(home.to_path_buf())),
        cwd.to_path_buf(),
    );
    thread::spawn(move || serve(dirs, cwd, port));
    // wait for the listener
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return port;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("shell did not start");
}

fn http(port: u16, method: &str, path: &str, body: Option<&str>) -> String {
    http_headers(port, method, path, body, &[])
}

fn http_headers(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let extra: String = headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}\r\n"))
        .collect();
    let req = match body {
        Some(b) => format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nContent-Type: application/json\r\n{extra}Connection: close\r\n\r\n{b}",
            b.len()
        ),
        None => format!("{method} {path} HTTP/1.1\r\nHost: x\r\n{extra}Connection: close\r\n\r\n"),
    };
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap_or_default();
    let raw = String::from_utf8_lossy(&raw);
    raw.to_string()
}

fn body(raw: &str) -> String {
    raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
}

#[test]
fn shell_state_and_runs() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());

    let state = body(&http(port, "GET", "/api/state", None));
    let v: serde_json::Value = serde_json::from_str(&state).unwrap();
    let names: Vec<&str> = v["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["bigbird", "doc-keeper", "eggbot"]);
    assert!(v["perches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "echo"));

    // settings round-trip
    http(
        port,
        "POST",
        "/api/settings",
        Some(r##"{"content":"# hi\n"}"##),
    );
    let s = body(&http(port, "GET", "/api/settings", None));
    assert!(s.contains("# hi"));
}

#[test]
fn eggbot_interview_builds_a_bot() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());

    // eggbot is seeded into the roster
    let state = body(&http(port, "GET", "/api/state", None));
    assert!(state.contains("eggbot"), "eggbot not seeded: {state}");

    let start = body(&http(port, "POST", "/api/interview/start", Some("{}")));
    let session = serde_json::from_str::<serde_json::Value>(&start).unwrap()["session"]
        .as_u64()
        .unwrap();

    let answer = |a: &str| -> String {
        body(&http(
            port,
            "POST",
            "/api/interview/answer",
            Some(&format!(r#"{{"session":{session},"answer":"{a}"}}"#)),
        ))
    };
    assert!(answer("Bad Name!!").contains("error")); // rejects non-kebab
    answer("pr-reviewer");
    answer("reviews pull requests");
    answer("precise");
    answer("does not deploy anything");
    let r = answer("reader");
    assert!(r.contains("permissions"), "draft missing perms: {r}");
    answer("none");
    let r = answer("balanced");
    assert!(r.contains("spoke diff"), "preview is not a Spoke diff: {r}");
    assert!(
        r.contains("agent.toml"),
        "preview missing manifest path: {r}"
    );
    assert!(
        r.contains(".nest/agents/"),
        "preview missing rendered directory: {r}"
    );
    assert!(
        r.contains("name: pr-reviewer"),
        "preview missing rendered agent: {r}"
    );
    let premature = body(&http(
        port,
        "POST",
        "/api/interview/build",
        Some(&format!(r#"{{"session":{session}}}"#)),
    ));
    assert!(
        premature.contains("confirmed"),
        "unconfirmed build accepted: {premature}"
    );
    assert!(
        !home.join("agent.toml").exists(),
        "unconfirmed build wrote a manifest"
    );
    let done = answer("build it");
    assert!(done.contains("\"done\":true"), "{done}");

    let built = body(&http(
        port,
        "POST",
        "/api/interview/build",
        Some(&format!(r#"{{"session":{session}}}"#)),
    ));
    let built_json: serde_json::Value = serde_json::from_str(&built).unwrap();
    assert_eq!(built_json["built"], "pr-reviewer", "{built}");
    assert_eq!(built_json["test_drive"]["status"], "Done", "{built}");
    let receipt_path =
        std::path::PathBuf::from(built_json["test_drive"]["receipt"].as_str().unwrap());
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(receipt_path).unwrap()).unwrap();
    assert_eq!(receipt["agent"], "pr-reviewer");
    assert_eq!(receipt["perch"], "echo");

    // the new bot is declared in agent.toml (manifest-first)…
    // (cwd has no agent.toml here, so the personal home manifest is used)
    let manifest = std::fs::read_to_string(home.join("agent.toml")).unwrap();
    assert!(
        manifest.contains("name = \"pr-reviewer\"") && manifest.contains("anti_jobs"),
        "{manifest}"
    );
    // …and rendered flat into home/agents/ by the spoke sync path
    let file = home.join("agents/pr-reviewer.md");
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("rendered bot missing at {}: {e}", file.display()));
    assert!(
        text.contains("name: pr-reviewer") && text.contains("read"),
        "{text}"
    );

    let state = body(&http(port, "GET", "/api/state", None));
    assert!(state.contains("pr-reviewer"), "bot not in roster: {state}");
}

#[test]
fn eggbot_build_rolls_back_manifest_when_spoke_projection_fails() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let project = t.path();
    let project_manifest = project.join("agent.toml");
    let original_manifest = "version = 1\nharnesses = [\"nest\"]\n";
    std::fs::write(&project_manifest, original_manifest).unwrap();
    let port = start(&home, project);
    let agents_path = project.join(".nest/agents");
    std::fs::create_dir_all(project.join(".nest")).unwrap();
    std::fs::write(&agents_path, "block directory creation").unwrap();

    let start = body(&http(port, "POST", "/api/interview/start", Some("{}")));
    let session = serde_json::from_str::<serde_json::Value>(&start).unwrap()["session"]
        .as_u64()
        .unwrap();
    let answers = [
        "transaction-bot",
        "one safe job",
        "precise",
        "no deployment",
        "reader",
        "none",
        "balanced",
        "build it",
    ];
    for answer in answers {
        body(&http(
            port,
            "POST",
            "/api/interview/answer",
            Some(&format!(r#"{{"session":{session},"answer":"{answer}"}}"#)),
        ));
    }
    let failed = body(&http(
        port,
        "POST",
        "/api/interview/build",
        Some(&format!(r#"{{"session":{session}}}"#)),
    ));
    assert!(failed.contains("error"), "build should fail: {failed}");
    assert_eq!(
        std::fs::read_to_string(&project_manifest).unwrap(),
        original_manifest,
        "manifest was not rolled back"
    );
    assert!(agents_path.is_file());
}

#[test]
fn session_api_persists_messages_and_restores_transcript() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());
    let created = body(&http(
        port,
        "POST",
        "/api/sessions/send",
        Some(r#"{"bot":"bigbird","task":"persistent session test"}"#),
    ));
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    let session = created["session"].as_str().unwrap().to_string();
    let run = created["id"].as_str().unwrap().to_string();
    assert!(session.starts_with("session-"));

    let deadline = Instant::now() + Duration::from_secs(5);
    let transcript = loop {
        let raw = body(&http(
            port,
            "GET",
            &format!("/api/sessions/{session}"),
            None,
        ));
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        if value["session"]["messages"]
            .as_array()
            .is_some_and(|messages| messages.len() == 2 && messages[1]["state"] != "running")
        {
            break value["session"].clone();
        }
        assert!(Instant::now() < deadline, "session did not finish: {raw}");
        thread::sleep(Duration::from_millis(20));
    };
    let messages = transcript["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["text"], "persistent session test");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["run_id"], run);
    assert!(matches!(
        messages[1]["state"].as_str(),
        Some("done" | "failed" | "spawnfailed" | "budgetexceeded")
    ));
    let receipts = body(&http(port, "GET", "/api/runs", None));
    let receipts: serde_json::Value = serde_json::from_str(&receipts).unwrap();
    assert!(receipts["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| { r["agent"] == "bigbird" && r["task"] == "persistent session test" }));

    let listed = body(&http(port, "GET", "/api/sessions?bot=bigbird", None));
    let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed["sessions"][0]["id"], session);
}

/// Chat goes through the chief: a planner that returns `{"answer":…}`
/// answers directly — no bot hatches, and the transcript gets the text.
#[test]
fn session_send_chief_answers_directly() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // a cheap command provider whose reply the chief can parse
    std::fs::write(
        home.join("providers.toml"),
        "[[providers]]\nname = \"chief-llm\"\nkind = \"command\"\nprogram = \"sh\"\nargs = [\"-c\", \"printf '{\\\"answer\\\": \\\"hello from the chief\\\"}'\"]\ncost = \"free\"\n",
    )
    .unwrap();
    let port = start(&home, t.path());

    let created = body(&http(
        port,
        "POST",
        "/api/sessions/send",
        Some(r#"{"bot":"nest","task":"hi","approved_tools":["bash"]}"#),
    ));
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    let session = created["session"].as_str().unwrap().to_string();

    let deadline = Instant::now() + Duration::from_secs(10);
    let transcript = loop {
        let raw = body(&http(
            port,
            "GET",
            &format!("/api/sessions/{session}"),
            None,
        ));
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        if value["session"]["messages"]
            .as_array()
            .is_some_and(|messages| messages.len() == 2 && messages[1]["state"] != "running")
        {
            break value["session"].clone();
        }
        assert!(Instant::now() < deadline, "chief reply never landed: {raw}");
        thread::sleep(Duration::from_millis(20));
    };
    let messages = transcript["messages"].as_array().unwrap();
    assert_eq!(messages[1]["state"], "done");
    assert_eq!(messages[1]["text"], "hello from the chief");

    // the chief's answer is recorded in the roost like any other run
    let runs = body(&http(port, "GET", "/api/runs", None));
    let runs: serde_json::Value = serde_json::from_str(&runs).unwrap();
    assert!(runs["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| { r["agent"] == "chief" && r["status"] == "done" }));
}

/// The chief delegates: a planner returning `{"delegate":[…]}` hatches
/// the named bot; a planner that can't decide (no providers) leaves the
/// task to the deterministic routing table.
#[test]
fn session_send_chief_delegates_then_falls_back_to_routes() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("providers.toml"),
        "[[providers]]\nname = \"chief-llm\"\nkind = \"command\"\nprogram = \"sh\"\nargs = [\"-c\", \"printf '{\\\"delegate\\\": [{\\\"bot\\\": \\\"doc-keeper\\\"}]}'\"]\ncost = \"free\"\n",
    )
    .unwrap();
    let port = start(&home, t.path());

    let created = body(&http(
        port,
        "POST",
        "/api/sessions/send",
        Some(r#"{"bot":"nest","task":"doc work please"}"#),
    ));
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    assert!(created["id"].as_str().is_some(), "{created}");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let runs = body(&http(port, "GET", "/api/runs", None));
        let v: serde_json::Value = serde_json::from_str(&runs).unwrap_or_default();
        if v["receipts"].as_array().is_some_and(|rs| {
            rs.iter()
                .any(|r| r["agent"] == "doc-keeper" && r["status"] == "done")
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "delegation never ran: {runs}");
        thread::sleep(Duration::from_millis(100));
    }

    // no providers → the planner Undecides → routing.toml still wins
    std::fs::remove_file(home.join("providers.toml")).unwrap();
    let created = body(&http(
        port,
        "POST",
        "/api/sessions/send",
        Some(r#"{"bot":"nest","task":"fix the doc"}"#),
    ));
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    let session = created["session"].as_str().unwrap().to_string();
    let run = created["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    let transcript = loop {
        let raw = body(&http(
            port,
            "GET",
            &format!("/api/sessions/{session}"),
            None,
        ));
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        if value["session"]["messages"]
            .as_array()
            .is_some_and(|messages| {
                messages
                    .iter()
                    .any(|message| message["run_id"] == run && message["state"] != "running")
            })
        {
            break value["session"].clone();
        }
        assert!(
            Instant::now() < deadline,
            "routed fallback never ran: {raw}"
        );
        thread::sleep(Duration::from_millis(20));
    };
    let message = transcript["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["run_id"] == run)
        .unwrap();
    assert_eq!(message["state"], "done");
}

#[test]
fn shell_hatch_records_receipt() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());

    let res = body(&http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"bigbird","task":"say hi","perch":"echo"}"#),
    ));
    assert!(res.contains("run-"), "hatch returned {res}");

    // poll the roost until the receipt lands
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let runs = body(&http(port, "GET", "/api/runs", None));
        if runs.contains("bigbird") && runs.contains("done") {
            break;
        }
        assert!(Instant::now() < deadline, "receipt never landed: {runs}");
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn expert_cli_serves_read_only_commands() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // expert save path needs a manifest to validate against
    std::fs::write(
        home.join("agent.toml"),
        "version = 1\nharnesses = [\"nest\"]\n",
    )
    .unwrap();
    let port = start(&home, t.path());

    let agents = body(&http(
        port,
        "POST",
        "/api/cli",
        Some(r#"{"argv":["agents"]}"#),
    ));
    assert!(agents.contains("bigbird"), "agents output: {agents}");

    let perches = body(&http(
        port,
        "POST",
        "/api/cli",
        Some(r#"{"argv":["perches"]}"#),
    ));
    assert!(perches.contains("echo"), "perches output: {perches}");

    let refused = body(&http(
        port,
        "POST",
        "/api/cli",
        Some(r#"{"argv":["hatch"]}"#),
    ));
    assert!(
        refused.contains("refused"),
        "hatch must be refused: {refused}"
    );

    // expert save round-trips a valid agent.toml…
    let path = home.join("agent.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let save = body(&http(
        port,
        "POST",
        "/api/expert",
        Some(
            &serde_json::json!({"path": path.display().to_string(), "content": content})
                .to_string(),
        ),
    ));
    assert!(save.contains("saved"), "expert save: {save}");

    // …and refuses invalid TOML without touching the file
    let bad = body(&http(
        port,
        "POST",
        "/api/expert",
        Some(
            &serde_json::json!({"path": path.display().to_string(), "content": "[[[not toml"})
                .to_string(),
        ),
    ));
    assert!(
        bad.contains("does not parse"),
        "expert must refuse bad toml: {bad}"
    );
    assert!(std::fs::read_to_string(&path).unwrap() == content);
}

#[test]
fn auto_feather_opt_in_distills_hard_runs() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // a real provider (not echo) so feather can distill
    std::fs::write(
        home.join("providers.toml"),
        "[[providers]]\nname = \"fake-llm\"\nkind = \"command\"\nprogram = \"sh\"\nargs = [\"-c\", \"printf -- '---\\nname: x\\ndescription: y\\n---\\n\\nlearned\\n'\"]\ncost = \"free\"\n",
    )
    .unwrap();
    // opt in to auto-feather
    std::fs::write(home.join("settings.md"), "auto_feather: true\n").unwrap();
    let port = start(&home, t.path());

    // state carries the skill index (empty before any feather)
    let state = body(&http(port, "GET", "/api/state", None));
    assert!(
        state.contains("\"skills\":[]"),
        "skill index missing: {state}"
    );

    // fail a hatch (unknown bot → spawn-failed receipt is NOT auto-feathered;
    // only real receipts qualify — instead feather manually and check index)
    http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"bigbird","task":"say hi","perch":"echo"}"#),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let run_id = loop {
        let runs = body(&http(port, "GET", "/api/runs", None));
        let v: serde_json::Value = serde_json::from_str(&runs).unwrap_or_default();
        if let Some(id) = v["receipts"]
            .as_array()
            .and_then(|rs| rs.first())
            .and_then(|r| r["id"].as_str())
        {
            break id.to_string();
        }
        assert!(Instant::now() < deadline, "receipt never landed: {runs}");
        thread::sleep(Duration::from_millis(100));
    };
    let r = body(&http(
        port,
        "POST",
        "/api/feather",
        Some(&format!(r#"{{"run_id":"{run_id}"}}"#)),
    ));
    assert!(r.contains("dir"), "manual feather: {r}");

    // skill index now lists it, with lockfile provenance
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = body(&http(port, "GET", "/api/state", None));
        if state.contains("learned-") {
            break;
        }
        assert!(Instant::now() < deadline, "skill never indexed: {state}");
        thread::sleep(Duration::from_millis(200));
    }
    let lock = std::fs::read_to_string(home.join("agent.lock")).unwrap();
    assert!(
        lock.contains("feathered by bigbird"),
        "lockfile provenance: {lock}"
    );
}

#[test]
fn feather_endpoint_distills_a_run() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // a real provider (not the echo stub) — feather refuses without one
    std::fs::write(
        home.join("providers.toml"),
        "[[providers]]\nname = \"fake-llm\"\nkind = \"command\"\nprogram = \"sh\"\nargs = [\"-c\", \"printf -- '---\\nsummary: distilled\\n---\\n\\nlearned steps\\n'\"]\ncost = \"free\"\n",
    )
    .unwrap();
    let port = start(&home, t.path());

    http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"bigbird","task":"say hi","perch":"echo"}"#),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let run_id = loop {
        let runs = body(&http(port, "GET", "/api/runs", None));
        let v: serde_json::Value = serde_json::from_str(&runs).unwrap_or_default();
        if let Some(id) = v["receipts"]
            .as_array()
            .and_then(|rs| rs.first())
            .and_then(|r| r["id"].as_str())
        {
            break id.to_string();
        }
        assert!(Instant::now() < deadline, "receipt never landed: {runs}");
        thread::sleep(Duration::from_millis(100));
    };

    let r = body(&http(
        port,
        "POST",
        "/api/feather",
        Some(&format!(r#"{{"run_id":"{run_id}"}}"#)),
    ));
    assert!(r.contains("dir"), "{r}");

    let skills = home.join("skills");
    let skill_md = std::fs::read_dir(&skills)
        .unwrap()
        .flatten()
        .map(|e| e.path().join("SKILL.md"))
        .find(|p| p.is_file())
        .expect("no SKILL.md written");
    let body = std::fs::read_to_string(&skill_md).unwrap();
    assert!(body.contains("authored-by: bigbird"), "{body}");
}

#[test]
fn share_endpoint_exports_a_bot_template() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());

    let r = body(&http(
        port,
        "POST",
        "/api/share",
        Some(r#"{"bot":"bigbird"}"#),
    ));
    assert!(r.contains("shared"), "share: {r}");
    assert!(r.contains("spoke add"), "share hint: {r}");
    let dir = home.join("shared/bigbird-bot");
    assert!(dir.join("package.toml").is_file(), "no package.toml");
    let md = std::fs::read_to_string(dir.join("agents/bigbird.md")).unwrap();
    assert!(md.contains("name: bigbird"), "{md}");

    let bad = body(&http(
        port,
        "POST",
        "/api/share",
        Some(r#"{"bot":"ghost"}"#),
    ));
    assert!(bad.contains("no bot named"), "share must refuse: {bad}");
}

#[test]
fn patrol_triggers_cover_git_push_and_app_events() {
    // unit-covered in patrol tests; shell state must surface git-push
    // duties as armed triggers (not "unsupported")
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("automations.toml"),
        "[[automation]]\nname = \"push-watch\"\nevent = \"git-push\"\nprompt = \"summarize the push\"\nbot = \"bigbird\"\n\n[[automation]]\nname = \"deploy-hook\"\nevent = \"app:deploy\"\nprompt = \"deploy finished\"\naction = \"brood\"\nreply = \"quiet\"\n",
    )
    .unwrap();
    let port = start(&home, t.path());
    let state = body(&http(port, "GET", "/api/state", None));
    assert!(state.contains("git-push"), "git-push duty missing: {state}");
    assert!(state.contains("app:deploy"), "app duty missing: {state}");
    assert!(
        !state.contains("unsupported trigger"),
        "duties unsupported: {state}"
    );
}

#[test]
fn webhook_inbox_drop_auth_and_reply() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(home.join("agents")).unwrap();
    std::fs::write(
        home.join("agents/bigbird.md"),
        "---\nname: bigbird\njob: the boss\nboss: true\n---\nYou answer things.\n",
    )
    .unwrap();
    // a remote opts in by owning a token file
    let remote = home.join("remotes/hook");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("token"), "s3cret\n").unwrap();

    let port = free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let dirs = NestDirs::resolve(t.path(), Some(home.clone()));
    thread::spawn(move || nest::shell::webhook_serve(dirs, t.path().to_path_buf(), listener));
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    // no token → 403
    let denied = http(port, "POST", "/inbox/hook", Some("chat: 42\nhello"));
    assert!(denied.contains("403"), "{denied}");
    // wrong token → 403
    let wrong = http_headers(
        port,
        "POST",
        "/inbox/hook",
        Some("chat: 42\nhello"),
        &[("Authorization", "Bearer nope")],
    );
    assert!(wrong.contains("403"), "{wrong}");
    // a remote with no token file never accepts inbound
    let ghost = http_headers(
        port,
        "POST",
        "/inbox/ghost",
        Some("chat: 1\nhi"),
        &[("Authorization", "Bearer s3cret")],
    );
    assert!(ghost.contains("403"), "{ghost}");
    // traversal in remote/id segments can't escape the remotes dir
    let escape = http_headers(
        port,
        "POST",
        "/inbox/..%2Fevil",
        Some("x"),
        &[("Authorization", "Bearer s3cret")],
    );
    assert!(!escape.contains("\"id\""), "{escape}");

    // oversized bodies are refused before hitting disk
    let fat = http_headers(
        port,
        "POST",
        "/inbox/hook",
        Some(&"x".repeat(70 * 1024)),
        &[("Authorization", "Bearer s3cret")],
    );
    assert!(fat.contains("413"), "{fat}");

    // good token → task file lands in the remote's inbox
    let ok = http_headers(
        port,
        "POST",
        "/inbox/hook",
        Some("chat: 42\nwhat's up"),
        &[("Authorization", "Bearer s3cret")],
    );
    assert!(ok.contains("\"remote\":\"hook\""), "{ok}");
    let inbox = remote.join("inbox");
    let drop = std::fs::read_dir(&inbox)
        .unwrap()
        .flatten()
        .next()
        .expect("no inbox file");
    assert!(std::fs::read_to_string(drop.path())
        .unwrap()
        .contains("what's up"));

    // replies are the existing outbox contract: outbox/<chat>.txt
    let pending = http_headers(
        port,
        "GET",
        "/inbox/hook/42/reply",
        None,
        &[("Authorization", "Bearer s3cret")],
    );
    assert!(pending.contains("404"), "{pending}");
    std::fs::create_dir_all(remote.join("outbox")).unwrap();
    std::fs::write(remote.join("outbox/42.txt"), "the nest says hi").unwrap();
    let reply = http_headers(
        port,
        "GET",
        "/inbox/hook/42/reply",
        None,
        &[("Authorization", "Bearer s3cret")],
    );
    assert!(body(&reply).contains("the nest says hi"), "{reply}");

    // a full backlog refuses new drops
    for i in 0..64 {
        std::fs::write(inbox.join(format!("seed-{i}.md")), "x").unwrap();
    }
    let flooded = http_headers(
        port,
        "POST",
        "/inbox/hook",
        Some("chat: 9\nmore"),
        &[("Authorization", "Bearer s3cret")],
    );
    assert!(flooded.contains("429"), "{flooded}");
}

#[test]
fn webhook_generic_task_replies_under_task_id() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let _shell_port = start(&home, t.path()); // watchers own dispatch + outbox
    let remote = home.join("remotes/hook");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("token"), "s3cret\n").unwrap();

    let port = free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let dirs = NestDirs::resolve(t.path(), Some(home.clone()));
    thread::spawn(move || nest::shell::webhook_serve(dirs, t.path().to_path_buf(), listener));
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    // no chat:/channel: line — the reply still lands, keyed on the task id
    let raw = http_headers(
        port,
        "POST",
        "/inbox/hook",
        Some("generic ping for nobody"),
        &[("Authorization", "Bearer s3cret")],
    );
    let v: serde_json::Value = serde_json::from_str(&body(&raw)).unwrap_or_default();
    let id = v["id"].as_str().expect("no task id").to_string();

    let reply_file = remote.join("outbox").join(format!("{id}.txt"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !reply_file.exists() {
        assert!(
            Instant::now() < deadline,
            "no reply at {}",
            reply_file.display()
        );
        thread::sleep(Duration::from_millis(300));
    }
    assert!(std::fs::read_to_string(&reply_file)
        .unwrap()
        .contains("generic ping"));
}

#[test]
fn webhook_traversal_chat_id_writes_no_outbox() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let _shell_port = start(&home, t.path()); // watchers own dispatch + outbox
    let remote = home.join("remotes/hook");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("token"), "s3cret\n").unwrap();

    let port = free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let dirs = NestDirs::resolve(t.path(), Some(home.clone()));
    thread::spawn(move || nest::shell::webhook_serve(dirs, t.path().to_path_buf(), listener));
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    // chat ids that try to climb out of the outbox get no reply file —
    // traversal, absolute paths, windows backslashes, subdirs, and dotted
    // names all fail name_ok and are answered nowhere
    let mut ids = Vec::new();
    for bad in [
        "chat: ../../../evil",
        "chat: /abs/evil",
        "chat: ..\\..\\evil",
        "chat: sub/dir",
        "chat: name.with.dots",
    ] {
        let raw = http_headers(
            port,
            "POST",
            "/inbox/hook",
            Some(&format!("{bad}\nescape attempt")),
            &[("Authorization", "Bearer s3cret")],
        );
        let v: serde_json::Value = serde_json::from_str(&body(&raw)).unwrap_or_default();
        ids.push(v["id"].as_str().expect("no task id").to_string());
    }

    // wait for the watcher to consume every task
    let deadline = Instant::now() + Duration::from_secs(30);
    for id in &ids {
        let done_file = remote.join("inbox/done").join(format!("{id}.md"));
        while !done_file.exists() {
            assert!(Instant::now() < deadline, "task never dispatched");
            thread::sleep(Duration::from_millis(300));
        }
    }
    // give the reply writer a beat, then prove nothing escaped (or landed)
    thread::sleep(Duration::from_millis(500));
    assert!(!home.join("evil.txt").exists(), "traversal write landed");
    let outbox = remote.join("outbox");
    assert!(
        !outbox.join("..\\..\\evil.txt").exists(),
        "backslash-named write landed"
    );
    let entries: Vec<_> = std::fs::read_dir(&outbox)
        .map(|es| es.flatten().collect())
        .unwrap_or_default();
    assert!(
        entries.is_empty(),
        "reply written for bad chat id: {entries:?}"
    );
}

#[test]
fn cross_origin_writes_are_blocked() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());

    // a page on another origin can't mutate state
    let evil = http_headers(
        port,
        "POST",
        "/api/settings",
        Some(r#"{"content":"pwned"}"#),
        &[("Origin", "https://evil.example")],
    );
    assert!(evil.contains("403"), "expected 403: {evil}");

    // loopback origins and non-browser clients (no Origin) still work
    let ok = http_headers(
        port,
        "POST",
        "/api/settings",
        Some(r#"{"content":"fine"}"#),
        &[("Origin", "http://127.0.0.1:1")],
    );
    assert!(ok.contains("saved"), "{ok}");
    let curl = http(
        port,
        "POST",
        "/api/settings",
        Some(r#"{"content":"also fine"}"#),
    );
    assert!(curl.contains("saved"), "{curl}");

    // GETs are unaffected — a browser can always fetch, it just can't read cross-origin
    let state = http_headers(
        port,
        "GET",
        "/api/state",
        None,
        &[("Origin", "https://evil.example")],
    );
    assert!(body(&state).contains("agents"), "{state}");
}

#[test]
fn bad_agent_file_warns_but_keeps_roster() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());
    std::fs::write(home.join("agents/broken.md"), "---\n[bad toml\n---\n").unwrap();

    let state = body(&http(port, "GET", "/api/state", None));
    let v: serde_json::Value = serde_json::from_str(&state).unwrap();
    let names: Vec<&str> = v["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["bigbird", "doc-keeper", "eggbot"]);
    assert_eq!(v["warnings"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn sse_stream_does_not_block_api() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());

    http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"bigbird","task":"go","perch":"echo"}"#),
    );

    // hold the SSE stream open in the background, then hit the API —
    // a serialized request loop would stall until the stream closed
    let stream = thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).ok();
        s.write_all(b"GET /api/runs/run-1/events HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap_or_default();
        String::from_utf8_lossy(&raw).to_string()
    });
    thread::sleep(Duration::from_millis(200));
    let state = body(&http(port, "GET", "/api/state", None));
    assert!(state.contains("agents"), "api stalled behind sse: {state}");

    let raw = stream.join().unwrap();
    assert!(raw.contains("data:"), "no sse frames: {raw}");
    assert!(raw.contains("event: receipt"), "no receipt frame: {raw}");
}

#[test]
fn onboarding_writes_boss_and_marks_done() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());

    // fixture writes a boss already — remove it so onboarding is needed
    std::fs::remove_file(home.join("agents/bigbird.md")).unwrap();
    let state = body(&http(port, "GET", "/api/state", None));
    assert!(state.contains(r#""onboarding":true"#), "{state}");

    let r = body(&http(
        port,
        "POST",
        "/api/onboarding",
        Some(
            r#"{"name":"bigbird","job":"the boss","abilities":["read","commands"],"engines":["echo"]}"#,
        ),
    ));
    assert!(r.contains(r#""boss":"bigbird""#), "{r}");
    // echo is a builtin stub — nothing to install, so it gets no run
    assert!(!r.contains(r#""run":"#), "{r}");

    let boss = std::fs::read_to_string(home.join("agents/bigbird.md")).unwrap();
    assert!(boss.contains("boss: true"), "{boss}");
    assert!(boss.contains("bash: allow"), "{boss}");
    assert!(boss.contains("write: deny"), "{boss}");

    let settings = std::fs::read_to_string(home.join("settings.md")).unwrap();
    assert!(settings.contains("onboarded: true"), "{settings}");

    let state = body(&http(port, "GET", "/api/state", None));
    assert!(state.contains(r#""onboarding":false"#), "{state}");

    // a named boss counts even without the flag
    std::fs::remove_file(home.join("settings.md")).unwrap();
    let state = body(&http(port, "GET", "/api/state", None));
    assert!(state.contains(r#""onboarding":false"#), "{state}");
}

#[test]
fn perch_install_streams_a_run() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());

    // echo has no installer
    let bad = body(&http(
        port,
        "POST",
        "/api/perches/install",
        Some(r#"{"name":"echo"}"#),
    ));
    assert!(bad.contains("error"), "{bad}");

    // claude-code has one — the run streams whatever the installer does
    let r = body(&http(
        port,
        "POST",
        "/api/perches/install",
        Some(r#"{"name":"claude-code"}"#),
    ));
    assert!(r.contains("run-"), "{r}");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let runs = body(&http(port, "GET", "/api/runs", None));
        if runs.contains(r#""perch":"install""#) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "install receipt never landed: {runs}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn remote_inbox_dispatches_and_replies() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let _port = start(&home, t.path());
    let inbox = home.join("remotes/telegram/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(
        inbox.join("42-1001.md"),
        "---\nremote: telegram\nchat: 1001\nfrom: steven\n---\n\nhi nest\n",
    )
    .unwrap();

    // the watcher picks it up, dispatches to a bot (echo perch prints the
    // prompt back — the reply file lands in outbox/1001.txt)
    let deadline = Instant::now() + Duration::from_secs(20);
    let outbox = home.join("remotes/telegram/outbox/1001.txt");
    loop {
        if outbox.exists() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no outbox reply; inbox done dir: {:?}",
            std::fs::read_dir(inbox.join("done")).map(|d| d.count())
        );
        thread::sleep(Duration::from_millis(300));
    }
    assert!(
        inbox.join("done/42-1001.md").exists(),
        "inbox file not archived"
    );
    assert!(std::fs::read_to_string(&outbox)
        .unwrap()
        .contains("hi nest"));
}

#[test]
fn patrol_file_change_runs_a_routine() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let cwd = t.path().join("work");
    std::fs::create_dir_all(cwd.join("src")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("automations.toml"),
        "[[automation]]\nname = \"src-watch\"\nevent = \"file-change:src/**\"\nprompt = \"look at the tree\"\n",
    )
    .unwrap();
    let _port = start(&home, &cwd);

    // baseline scan happens on first tick — write after it settles
    thread::sleep(Duration::from_millis(400));
    std::fs::write(cwd.join("src/new.rs"), "fn n(){}").unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let roost = home.join("roost");
    loop {
        let hit = std::fs::read_dir(&roost)
            .map(|d| {
                d.flatten().any(|e| {
                    std::fs::read_to_string(e.path())
                        .map(|s| s.contains("src-watch"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if hit {
            break;
        }
        assert!(Instant::now() < deadline, "patrol never fired the routine");
        thread::sleep(Duration::from_millis(500));
    }
}

/// An agent that asks before running bash — the approval-gate fixture.
fn seed_gated_bot(home: &std::path::Path) {
    std::fs::create_dir_all(home.join("agents")).unwrap();
    std::fs::write(
        home.join("agents/gated.md"),
        "---\nname: gated\njob: gated work\npermissions:\n  bash: ask\n---\nI ask first.\n",
    )
    .unwrap();
}

fn inbox_items(port: u16) -> Vec<serde_json::Value> {
    let raw = body(&http(port, "GET", "/api/inbox", None));
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    v["items"].as_array().unwrap().clone()
}

/// Wait for an inbox item matching `pred` (10s), returning its id.
fn wait_inbox_item(port: u16, pred: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(item) = inbox_items(port).iter().find(|i| pred(i)) {
            return item.clone();
        }
        assert!(Instant::now() < deadline, "inbox item never landed");
        thread::sleep(Duration::from_millis(100));
    }
}

/// Hatching a bot with unapproved `ask` tools leaves a pending approval
/// card; approving it resolves the card and re-runs the task promoted.
#[test]
fn approval_card_approve_re_runs() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    seed_gated_bot(&home);
    let port = start(&home, t.path());

    http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"gated","task":"touch prod","perch":"echo"}"#),
    );

    let card = wait_inbox_item(port, |i| {
        i["kind"] == "approval" && !i["resolved"].as_bool().unwrap()
    });
    assert!(
        card["title"].as_str().unwrap().contains("bash"),
        "card title should name the withheld tool: {}",
        card["title"]
    );
    assert_eq!(card["approval"]["bot"], "gated");
    let id = card["id"].as_str().unwrap().to_string();

    // the badge number rides /api/state
    let state = body(&http(port, "GET", "/api/state", None));
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    assert!(state["inbox"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["id"] == id));

    // hostile ids never reach the filesystem; junk actions are rejected
    for bad in ["..", "inbox-x%2Fy", "../roost/x"] {
        let r = body(&http(
            port,
            "POST",
            &format!("/api/approvals/{bad}"),
            Some(r#"{"action":"dismiss"}"#),
        ));
        assert!(r.contains("invalid inbox id"), "{bad}: {r}");
    }
    let r = body(&http(
        port,
        "POST",
        &format!("/api/approvals/{id}"),
        Some(r#"{"action":"explode"}"#),
    ));
    assert!(r.contains("unknown action"), "{r}");

    // approve → resolved + a re-run is spawned
    let ok = body(&http(
        port,
        "POST",
        &format!("/api/approvals/{id}"),
        Some(r#"{"action":"approve"}"#),
    ));
    let ok: serde_json::Value = serde_json::from_str(&ok).unwrap();
    assert!(ok["run"].as_str().is_some(), "{ok}");

    // resolved items stay in history but refuse a second action
    let again = body(&http(
        port,
        "POST",
        &format!("/api/approvals/{id}"),
        Some(r#"{"action":"approve"}"#),
    ));
    assert!(again.contains("already resolved"), "{again}");

    // the re-run leaves a second 'touch prod' receipt in the roost
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let raw = body(&http(port, "GET", "/api/runs", None));
        let runs: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let n = runs["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["task"] == "touch prod")
            .count();
        if n >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "re-run never produced a receipt: {raw}"
        );
        thread::sleep(Duration::from_millis(150));
    }
}

/// Dismiss resolves a card without re-running; notes dismiss the same way.
#[test]
fn approval_card_dismiss_resolves() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    seed_gated_bot(&home);
    let port = start(&home, t.path());

    http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"gated","task":"touch prod","perch":"echo"}"#),
    );
    let card = wait_inbox_item(port, |i| i["kind"] == "approval");
    let id = card["id"].as_str().unwrap();

    let r = body(&http(
        port,
        "POST",
        &format!("/api/approvals/{id}"),
        Some(r#"{"action":"dismiss"}"#),
    ));
    assert!(r.contains("\"resolved\""), "{r}");

    // dismissed = resolved in the listing, history preserved
    let items = inbox_items(port);
    let item = items.iter().find(|i| i["id"] == id).unwrap();
    assert!(item["resolved"].as_bool().unwrap());

    // and no re-run happened — only the original receipt
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let raw = body(&http(port, "GET", "/api/runs", None));
        let runs: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let receipts = runs["receipts"].as_array().unwrap();
        let n = receipts
            .iter()
            .filter(|r| r["task"] == "touch prod")
            .count();
        if n == 1 {
            return; // original receipt landed, no second one coming
        }
        assert!(n == 0, "dismiss spawned a re-run: {raw}");
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(150));
    }
}

/// A routine firing drops a note in the inbox — the triage queue for
/// things that happened while you were away.
#[test]
fn routine_run_leaves_inbox_note() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("automations.toml"),
        "[[automation]]\nname = \"nightly\"\nbot = \"doc-keeper\"\nevent = \"app:nightly\"\nprompt = \"sweep the docs\"\n",
    )
    .unwrap();
    let port = start(&home, t.path());

    let r = body(&http(port, "POST", "/api/duties/nightly/fire", None));
    assert!(
        r.contains("\"fired\":\"nightly\"") || r.contains("\"fired\": \"nightly\""),
        "{r}"
    );

    let note = wait_inbox_item(port, |i| {
        i["kind"] == "note" && i["title"].as_str().is_some_and(|t| t.contains("nightly"))
    });
    assert!(note["title"].as_str().unwrap().contains("done"), "{note}");
}

/// `nest find` over HTTP — one query hits receipts, agents, and files.
#[test]
fn find_returns_cross_type_hits() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());

    // a run whose task carries a unique needle → a receipt hit
    http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"doc-keeper","task":"document the zebra-cake","perch":"echo"}"#),
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let raw = body(&http(port, "GET", "/api/find?q=zebra-cake", None));
        let hits: serde_json::Value = serde_json::from_str(&raw).unwrap();
        if hits["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["kind"] == "receipt")
        {
            break;
        }
        assert!(Instant::now() < deadline, "receipt never found: {raw}");
        thread::sleep(Duration::from_millis(150));
    }

    // agent files are searchable by name and body
    let raw = body(&http(port, "GET", "/api/find?q=doc-keeper", None));
    let hits: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(hits["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["kind"] == "agent" && h["id"] == "doc-keeper"));

    // percent-encoded queries decode; a miss is an empty list, not an error
    let raw = body(&http(port, "GET", "/api/find?q=keep%20docs", None));
    let hits: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(!hits["results"].as_array().unwrap().is_empty());
    let raw = body(&http(port, "GET", "/api/find?q=zzz-nothing", None));
    let hits: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(hits["results"].as_array().unwrap().is_empty());
}

/// `/api/memory` is a file editor over `~/.nest/memory/`; `/api/reflect`
/// runs the digest + prune on demand.
#[test]
fn memory_endpoints_edit_and_reflect() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());

    // files exist even before any run — profile + facts, empty content
    let raw = body(&http(port, "GET", "/api/memory", None));
    let mem: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let labels: Vec<&str> = mem["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"profile") && labels.contains(&"facts"),
        "{labels:?}"
    );

    // write a profile + a fact; unknown labels are refused
    let r = body(&http(
        port,
        "POST",
        "/api/memory",
        Some(r#"{"label":"profile","content":"Sam — terse answers\n"}"#),
    ));
    assert!(r.contains("saved"), "{r}");
    let r = body(&http(
        port,
        "POST",
        "/api/memory",
        Some(r#"{"label":"facts","content":"likes rust\n"}"#),
    ));
    assert!(r.contains("saved"), "{r}");
    let r = body(&http(
        port,
        "POST",
        "/api/memory",
        Some(r#"{"label":"../../roost/x","content":"nope"}"#),
    ));
    assert!(r.contains("unknown memory file"), "{r}");

    // the written profile now rides every prompt via /api/memory content
    let raw = body(&http(port, "GET", "/api/memory", None));
    assert!(raw.contains("terse answers"), "{raw}");

    // reflect over HTTP — digest + inbox note even with zero runs
    let r = body(&http(port, "POST", "/api/reflect", Some("{}")));
    let r: serde_json::Value = serde_json::from_str(&r).unwrap();
    assert!(r["digest"].as_str().is_some(), "{r}");
    assert_eq!(r["facts_kept"], 1);
}

#[test]
fn expert_write_rejects_hostile_input() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());
    let agent_toml = t.path().join("agent.toml");

    // a path that isn't agent.toml — traversal, absolute, sibling file
    for evil in [
        "../../etc/passwd",
        "/tmp/evil.toml",
        "agent.toml.bak",
        "../agent.toml",
    ] {
        let req = format!(r#"{{"path":{:?},"content":"x = 1"}}"#, evil.to_string());
        let r = body(&http(port, "POST", "/api/expert", Some(&req)));
        assert!(r.contains("error"), "path {evil} should fail: {r}");
    }
    assert!(!t.path().join("evil.toml").exists());

    // toml that doesn't parse
    let req = format!(
        r#"{{"path":{:?},"content":"not [valid toml"}}"#,
        agent_toml.display().to_string()
    );
    let r = body(&http(port, "POST", "/api/expert", Some(&req)));
    assert!(r.contains("does not parse"), "{r}");

    // toml that parses but fails manifest validation — a bot named ".."
    // is not a legal filename and must never be written out
    let bad_manifest = "version = 1\n[[bots]]\nname = \"..\"\njob = \"x\"\n";
    let req = format!(
        r#"{{"path":{:?},"content":{:?}}}"#,
        agent_toml.display().to_string(),
        bad_manifest
    );
    let r = body(&http(port, "POST", "/api/expert", Some(&req)));
    assert!(r.contains("invalid"), "{r}");
    assert!(!agent_toml.exists(), "invalid manifest must not be saved");

    // a valid manifest saves
    let good = "version = 1\n[[bots]]\nname = \"writer\"\njob = \"write things\"\n";
    let req = format!(
        r#"{{"path":{:?},"content":{:?}}}"#,
        agent_toml.display().to_string(),
        good
    );
    let r = body(&http(port, "POST", "/api/expert", Some(&req)));
    assert!(r.contains("saved"), "{r}");
    assert_eq!(std::fs::read_to_string(&agent_toml).unwrap(), good);
}
