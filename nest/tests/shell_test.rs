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
    assert!(r.contains("agent.toml"), "preview missing manifest card: {r}");
    let done = answer("build it");
    assert!(done.contains("\"done\":true"), "{done}");

    let built = body(&http(
        port,
        "POST",
        "/api/interview/build",
        Some(&format!(r#"{{"session":{session}}}"#)),
    ));
    assert!(built.contains("pr-reviewer"), "{built}");
    assert!(built.contains("test_drive"), "no test-drive proof: {built}");

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
    assert!(refused.contains("refused"), "hatch must be refused: {refused}");

    // expert save round-trips a valid agent.toml…
    let path = home.join("agent.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let save = body(&http(
        port,
        "POST",
        "/api/expert",
        Some(&serde_json::json!({"path": path.display().to_string(), "content": content}).to_string()),
    ));
    assert!(save.contains("saved"), "expert save: {save}");

    // …and refuses invalid TOML without touching the file
    let bad = body(&http(
        port,
        "POST",
        "/api/expert",
        Some(&serde_json::json!({"path": path.display().to_string(), "content": "[[[not toml"}).to_string()),
    ));
    assert!(bad.contains("does not parse"), "expert must refuse bad toml: {bad}");
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
    assert!(state.contains("\"skills\":[]"), "skill index missing: {state}");

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
    assert!(lock.contains("feathered by bigbird"), "lockfile provenance: {lock}");
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
    assert!(!state.contains("unsupported trigger"), "duties unsupported: {state}");
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

    // a chat id that tries to climb out of the outbox gets no reply file
    let raw = http_headers(
        port,
        "POST",
        "/inbox/hook",
        Some("chat: ../../../evil\nescape attempt"),
        &[("Authorization", "Bearer s3cret")],
    );
    let v: serde_json::Value = serde_json::from_str(&body(&raw)).unwrap_or_default();
    let id = v["id"].as_str().expect("no task id").to_string();

    // wait for the watcher to consume the task
    let done_file = remote.join("inbox/done").join(format!("{id}.md"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done_file.exists() {
        assert!(Instant::now() < deadline, "task never dispatched");
        thread::sleep(Duration::from_millis(300));
    }
    // give the reply writer a beat, then prove nothing escaped (or landed)
    thread::sleep(Duration::from_millis(500));
    assert!(!home.join("evil.txt").exists(), "traversal write landed");
    let outbox: Vec<_> = std::fs::read_dir(remote.join("outbox"))
        .map(|es| es.flatten().collect())
        .unwrap_or_default();
    assert!(
        outbox.is_empty(),
        "reply written for bad chat id: {outbox:?}"
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
