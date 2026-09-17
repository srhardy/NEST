//! nest-desktop — the shell as a real app. One window, a taskbar icon,
//! a floating desktop birdy, and notifications when runs settle.
//!
//! The whole UI is the same web shell (`nest::shell::serve`) running
//! in-process on an ephemeral port — nothing here re-implements the app.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use notify_rust::Notification;
use tauri::{
    image::Image,
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

/// Free localhost port for the embedded shell. The listener stays bound
/// and is handed to `serve_on` — nobody can steal the port in between.
fn pick_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("cannot bind a port");
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

/// Where the running instance publishes its port — one NEST at a time.
fn lock_file(home: &std::path::Path) -> PathBuf {
    home.join("desktop.port")
}

/// If another instance already serves its recorded port, leave quietly.
fn already_running(dirs: &nest::home::NestDirs) -> bool {
    std::fs::read_to_string(lock_file(&dirs.home))
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .is_some_and(|port| get(port, "/api/state").is_some())
}

/// Wait until the shell is accepting connections.
fn wait_ready(port: u16) -> bool {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

/// A white-bird tray icon, drawn as RGBA — no asset files needed.
fn bird_icon() -> Image<'static> {
    const S: u32 = 32;
    let mut px = vec![0u8; (S * S * 4) as usize];
    let mut put = |x: i32, y: i32, a: u8| {
        if (0..S as i32).contains(&x) && (0..S as i32).contains(&y) {
            let i = ((y as u32 * S + x as u32) * 4) as usize;
            px[i] = 255;
            px[i + 1] = 255;
            px[i + 2] = 255;
            px[i + 3] = a;
        }
    };
    // body ellipse
    for y in 0..S as i32 {
        for x in 0..S as i32 {
            let dx = (x - 16) as f32 / 11.0;
            let dy = (y - 19) as f32 / 10.0;
            if dx * dx + dy * dy <= 1.0 {
                put(x, y, 255);
            }
        }
    }
    // crest triangle
    for y in 2..9 {
        for x in (16 - (y - 2) / 2)..=(16 + (y - 2) / 2) {
            put(x, y, 255);
        }
    }
    // face cutout (transparent ellipse)
    for y in 0..S as i32 {
        for x in 0..S as i32 {
            let dx = (x - 16) as f32 / 5.5;
            let dy = (y - 17) as f32 / 4.5;
            if dx * dx + dy * dy <= 1.0 {
                put(x, y, 0);
            }
        }
    }
    Image::new_owned(px, S, S)
}

/// GET `path` from the embedded shell — enough HTTP for a counter.
fn get(port: u16, path: &str) -> Option<String> {
    use std::io::{Read, Write};
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.write_all(format!("GET {path} HTTP/1.0\r\nHost: x\r\n\r\n").as_bytes())
        .ok()?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).ok()?;
    buf.split("\r\n\r\n").nth(1).map(|b| b.to_string())
}

/// Watch the roost; every new receipt ID is a desktop notification.
/// Receipts arrive newest-first, so unseen ones are replayed oldest→newest.
fn watch_roost(port: u16) {
    let mut seen: std::collections::HashSet<String> = get(port, "/api/runs")
        .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
        .and_then(|v| v["receipts"].as_array().cloned())
        .map(|rs| {
            rs.iter()
                .filter_map(|r| r["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    loop {
        thread::sleep(Duration::from_secs(5));
        let Some(body) = get(port, "/api/runs") else {
            continue;
        };
        let Some(receipts) = serde_json::from_str::<serde_json::Value>(&body).ok()
            .and_then(|v| v["receipts"].as_array().cloned())
        else {
            continue;
        };
        for r in receipts.iter().rev() {
            let Some(id) = r["id"].as_str() else { continue };
            if !seen.insert(id.to_string()) {
                continue;
            }
            let status = r["status"].as_str().unwrap_or("done");
            let task = r["task"].as_str().unwrap_or("a run");
            Notification::new()
                .summary("NEST")
                .body(&format!("{task} — {status}"))
                .show()
                .ok();
        }
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        w.show().ok();
        w.set_focus().ok();
    }
}

/// birdy's double-click calls home.
#[tauri::command]
fn reopen_main(app: AppHandle) {
    show_main(&app);
}

fn toggle_birdy(app: &AppHandle, port: u16) {
    if let Some(w) = app.get_webview_window("birdy") {
        if w.is_visible().unwrap_or(false) {
            w.hide().ok();
        } else {
            w.show().ok();
        }
        return;
    }
    WebviewWindowBuilder::new(
        app,
        "birdy",
        WebviewUrl::External(format!("http://127.0.0.1:{port}/birdy").parse().unwrap()),
    )
    .title("birdy")
    .inner_size(140.0, 150.0)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .build()
    .ok();
}

fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dirs = nest::home::NestDirs::resolve(&cwd, None);
    // one nest at a time — a second launch would run a second patrol and
    // remote watcher over the same home, duplicating every dispatch
    if already_running(&dirs) {
        return Ok(());
    }
    let (listener, port) = pick_port();
    let home = dirs.home.clone();
    thread::spawn(move || {
        let _ = nest::shell::serve_on(dirs, cwd, listener);
    });
    if !wait_ready(port) {
        anyhow::bail!("the nest shell did not start on port {port}");
    }
    std::fs::write(lock_file(&home), port.to_string()).ok();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![reopen_main])
        .setup(move |app| {
            let show = MenuItemBuilder::with_id("show", "Show NEST").build(app)?;
            let birdy = CheckMenuItemBuilder::with_id("birdy", "Desktop birdy")
                .checked(true)
                .build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show, &birdy, &quit])
                .build()?;
            TrayIconBuilder::with_id("nest")
                .icon(bird_icon())
                .menu(&menu)
                .tooltip("NEST — still nesting")
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => show_main(app),
                    "birdy" => toggle_birdy(app, port),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(format!("http://127.0.0.1:{port}").parse().unwrap()),
            )
            .title("NEST")
            .inner_size(1180.0, 780.0)
            .min_inner_size(860.0, 560.0)
            .build()?;

            toggle_birdy(app.handle(), port);
            thread::spawn(move || watch_roost(port));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    // minimize to the taskbar — the nest keeps working
                    api.prevent_close();
                    window.hide().ok();
                }
            }
        })
        .run(tauri::generate_context!())
        .context("tauri runtime failed")?;
    Ok(())
}
