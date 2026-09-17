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
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let req = match body {
        Some(b) => format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{b}",
            b.len()
        ),
        None => format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
    };
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap_or_default();
    let raw = String::from_utf8_lossy(&raw);
    raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
}

#[test]
fn shell_state_and_runs() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());

    let state = http(port, "GET", "/api/state", None);
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
    let s = http(port, "GET", "/api/settings", None);
    assert!(s.contains("# hi"));
}

#[test]
fn eggbot_interview_builds_a_bot() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let port = start(&home, t.path());

    // eggbot is seeded into the roster
    let state = http(port, "GET", "/api/state", None);
    assert!(state.contains("eggbot"), "eggbot not seeded: {state}");

    let start = http(port, "POST", "/api/interview/start", Some("{}"));
    let session = serde_json::from_str::<serde_json::Value>(&start).unwrap()["session"]
        .as_u64()
        .unwrap();

    let answer = |a: &str| -> String {
        http(
            port,
            "POST",
            "/api/interview/answer",
            Some(&format!(r#"{{"session":{session},"answer":"{a}"}}"#)),
        )
    };
    assert!(answer("Bad Name!!").contains("error")); // rejects non-kebab
    answer("pr-reviewer");
    answer("reviews pull requests");
    answer("precise");
    let r = answer("reader");
    assert!(r.contains("permissions"), "draft missing perms: {r}");
    let done = answer("build it");
    assert!(done.contains("\"done\":true"), "{done}");

    let built = http(
        port,
        "POST",
        "/api/interview/build",
        Some(&format!(r#"{{"session":{session}}}"#)),
    );
    assert!(built.contains("pr-reviewer"), "{built}");

    // the new bot is on disk, in manifest format the engine loads
    let file = home.join("agents/pr-reviewer.md");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("name: pr-reviewer") && text.contains("read: allow"),
        "{text}"
    );

    let state = http(port, "GET", "/api/state", None);
    assert!(state.contains("pr-reviewer"), "bot not in roster: {state}");
}

#[test]
fn shell_hatch_records_receipt() {
    let t = tempfile::tempdir().unwrap();
    let port = start(&t.path().join("home"), t.path());

    let res = http(
        port,
        "POST",
        "/api/hatch",
        Some(r#"{"bot":"bigbird","task":"say hi","perch":"echo"}"#),
    );
    assert!(res.contains("run-"), "hatch returned {res}");

    // poll the roost until the receipt lands
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let runs = http(port, "GET", "/api/runs", None);
        if runs.contains("bigbird") && runs.contains("done") {
            break;
        }
        assert!(Instant::now() < deadline, "receipt never landed: {runs}");
        thread::sleep(Duration::from_millis(100));
    }
}
