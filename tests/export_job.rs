// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editor's export as a job (`/api/export/job`): answered at once, its progress asked for until it ends, with an estimate
//! made before the tiles start that counts the tiles a past export keeps; and cancelled, it leaves nothing installed. Ignored by
//! default, as it needs the game's files: `NWN_ROOT=<the game> cargo test --test export_job -- --ignored`. It works in scratch
//! folders.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(root: &Path) -> Server {
    let game = std::env::var_os("NWN_ROOT").expect("NWN_ROOT: this test needs the game's files");
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/index.html"), "<!doctype html><title>t</title>").unwrap();
    for d in ["home/hak", "home/modules", "xdg"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = Command::new(env!("CARGO_BIN_EXE_landscaper"))
        .args(["serve", "--port", &port.to_string(), "--web"])
        .arg(root.join("web"))
        .env("NWN_ROOT", game)
        .env("NWN_HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("HOME", root)
        .env_remove("LANDSCAPER_MODULE_REPO")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = Server { child, port };
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(started.elapsed() < Duration::from_secs(300), "the server didn't start");
        std::thread::sleep(Duration::from_millis(200));
    }
    server
}

/// `method` `target` with `body`: the answer's status and JSON.
fn ask(s: &Server, method: &str, target: &str, body: &[u8]) -> (u16, serde_json::Value) {
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let host = format!("127.0.0.1:{}", s.port);
    write!(
        c,
        "{method} {target} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nX-Landscaper: 1\r\nConnection: close\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    c.write_all(body).unwrap();
    let mut out = String::new();
    c.read_to_string(&mut out).unwrap();
    let (head, json) = out.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, serde_json::from_str(json).unwrap_or_else(|_| json.into()))
}

/// Area `name`, `tiles` a side, rolling, painted in stripes of two textures (so every tile has a ground of its own to bake), as
/// the editor sends it: its query string and body.
fn area(name: &str, tiles: usize) -> (String, Vec<u8>) {
    let n = tiles * 16 + 1;
    let mut body: Vec<u8> = (0..n * n).flat_map(|k| (((k % n) as f32 * 0.21).sin() * 2.0 + 4.0).to_le_bytes()).collect();
    body.extend((0..n * n).map(|k| u8::from((k % n) / 5 % 2 == 1)));
    body.extend(serde_json::json!({ "palette": ["grassMedium", "dirt"] }).to_string().into_bytes());
    (format!("/api/export/job?name={name}&tiles={tiles}"), body)
}

/// Starts the export job of area `name` and asks for its progress until it ends: its last answer, and every progress seen.
fn follow(s: &Server, name: &str, tiles: usize, cancel: bool) -> (serde_json::Value, Vec<serde_json::Value>) {
    let (target, body) = area(name, tiles);
    let (status, job) = ask(s, "POST", &target, &body);
    assert_eq!(status, 202, "{job}");
    assert_eq!(job["running"], true, "{job}");
    if cancel {
        let (status, answer) = ask(s, "POST", "/api/export/job/cancel", b"");
        assert_eq!(status, 200, "{answer}");
    }
    let mut seen = Vec::new();
    let started = Instant::now();
    loop {
        let (status, job) = ask(s, "GET", "/api/export/job", b"");
        assert_eq!(status, 200, "{job}");
        seen.push(job["progress"].clone());
        if job["running"] == false {
            return (job, seen);
        }
        assert!(started.elapsed() < Duration::from_secs(900), "the export didn't end");
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test export_job -- --ignored"]
fn an_export_job_estimates_its_time_counts_kept_tiles_and_can_be_cancelled() {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("export_job-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let s = start(&root);
    // Nothing kept yet: an estimate as the tiles start, none of them cached, and the report at the end.
    let (job, seen) = follow(&s, "jobs", 4, false);
    assert_eq!(job["error"], serde_json::Value::Null, "{job}");
    assert!(job["result"]["routes"].is_object(), "{job}");
    let p = &job["progress"];
    assert_eq!(
        (p["step"].as_str(), p["done"].as_u64(), p["total"].as_u64(), p["cached"].as_u64()),
        (Some("done"), Some(16), Some(16), Some(0))
    );
    assert!(p["estimate"].as_f64().is_some_and(|e| e > 0.0), "{p}");
    assert!(seen.iter().any(|p| p["step"] == "tiles" && p["left"].is_number()), "{seen:?}");
    // What it measured is kept for the next estimate.
    let speed: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("xdg/landscaper/export-speed.json")).unwrap()).unwrap();
    assert!(speed["bake"].as_f64().is_some_and(|b| b > 0.0), "{speed}");
    // The same again: every tile's ground and shadows kept.
    let (job, _) = follow(&s, "jobs", 4, false);
    assert_eq!(job["progress"]["cached"], 16, "{job}");
    // Cancelled at once: it stops, says so, and installs nothing.
    let (job, _) = follow(&s, "jobc", 12, true);
    assert_eq!(job["cancelled"], true, "{job}");
    assert!(job["error"].as_str().is_some_and(|e| e.contains("cancelled")), "{job}");
    assert!(!root.join("home/modules/jobc.mod").exists());
    assert!(!root.join("home/hak/jobc.hak").exists());
    drop(s);
    let _ = std::fs::remove_dir_all(&root);
}
