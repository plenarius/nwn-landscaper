// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Try on dev through `landscaper serve` against a stand-in review service: `NWN_ROOT=<the game> cargo test --test dev_trial -- --ignored`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
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

/// What the stand-in service was sent: method, path and body of each request.
type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

/// A review service on a free port: it records each request and answers with `answer(method, path)` (status, JSON).
fn service(answer: fn(&str, &str) -> (u16, serde_json::Value)) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let kept = seen.clone();
    std::thread::spawn(move || {
        for mut c in listener.incoming().flatten() {
            let mut r = BufReader::new(c.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
            let mut len = 0;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':')
                    && k.eq_ignore_ascii_case("content-length")
                {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let (status, json) = answer(&method, &path);
            kept.lock().unwrap().push((method, path, body));
            let text = json.to_string();
            let _ = write!(
                c,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    (port, seen)
}

/// A server on your own machine that uses the review service on `review`.
fn start(root: &Path, review: u16) -> Server {
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
        .env("LANDSCAPER_REVIEW_URL", format!("http://127.0.0.1:{review}"))
        .env("LANDSCAPER_REVIEW_KEY", "k")
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

/// `method target` with `body`, as the editor sends it; the status line and the JSON answer.
fn request(s: &Server, method: &str, target: &str, body: &[u8]) -> (String, serde_json::Value) {
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(900))).unwrap();
    let host = format!("127.0.0.1:{}", s.port);
    write!(
        c,
        "{method} {target} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nX-Landscaper: 1\r\nConnection: close\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    c.write_all(body).unwrap();
    let mut out = Vec::new();
    c.read_to_end(&mut out).unwrap();
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&out[..split]).into_owned();
    let mut body = &out[split + 4..];
    // (A chunked answer: one chunk is enough for these.)
    if head.lines().any(|l| l.eq_ignore_ascii_case("transfer-encoding: chunked")) {
        let eol = body.windows(2).position(|w| w == b"\r\n").unwrap();
        body = &body[eol + 2..];
    }
    let json = serde_json::Deserializer::from_slice(body).into_iter().next().and_then(Result::ok).unwrap_or_default();
    (head.lines().next().unwrap_or("").to_string(), json)
}

/// The parts of a multipart body: (field name, bytes).
fn parts(body: &[u8]) -> Vec<(String, Vec<u8>)> {
    let eol = body.windows(2).position(|w| w == b"\r\n").unwrap();
    let boundary = [b"\r\n".as_slice(), &body[..eol]].concat();
    let mut out = Vec::new();
    let mut rest = &body[eol..];
    while let Some(start) = rest.windows(4).position(|w| w == b"\r\n\r\n") {
        let head = String::from_utf8_lossy(&rest[..start]).into_owned();
        let data = &rest[start + 4..];
        let end = data.windows(boundary.len()).position(|w| w == boundary.as_slice()).unwrap();
        let name = head.split("name=\"").nth(1).and_then(|n| n.split('"').next()).unwrap_or("").to_string();
        out.push((name, data[..end].to_vec()));
        rest = &data[end + boundary.len()..];
    }
    out
}

/// Whether file name `f` ends in extension `e`, in any case.
fn ext(f: &str, e: &str) -> bool {
    Path::new(f).extension().is_some_and(|x| x.eq_ignore_ascii_case(e))
}

/// The file names in a zip.
fn zip_names(data: &[u8]) -> Vec<String> {
    zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap().file_names().map(str::to_string).collect()
}

fn answer(method: &str, path: &str) -> (u16, serde_json::Value) {
    match (method, path.split('?').next().unwrap_or("")) {
        ("POST", "/api/v1/dev/trial") => (
            200,
            serde_json::json!({ "user": "local", "revs": { "trl": 1 }, "reconnect": true, "commands": { "trl": ";forge trl" },
                                "errors": [], "warnings": [], "expires_in": 3600 }),
        ),
        ("GET", "/api/v1/dev/trial") => (
            200,
            serde_json::json!({ "enabled": true, "trials": { "local": { "akeys": ["trl"], "expires_in": 3500 },
                                                             "bob": { "akeys": ["town"], "expires_in": 9 } } }),
        ),
        ("POST", "/api/v1/dev/trial/release") => (404, serde_json::json!({ "detail": "local has no trial on dev." })),
        _ => (404, serde_json::json!({ "detail": "not here" })),
    }
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test dev_trial -- --ignored"]
fn a_trial_sends_the_publish_form_with_the_texture_library() {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("dev_trial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (review, seen) = service(answer);
    let s = start(&root, review);
    let n = 3 * 16 + 1;
    let mut body: Vec<u8> = (0..n * n).flat_map(|_| 1.0f32.to_le_bytes()).collect();
    body.extend(vec![0u8; n * n]);
    body.extend(br#"{"palette":["grassMedium"]}"#);
    let (status, a) = request(&s, "POST", "/api/export?name=trl&tiles=3", &body);
    assert!(status.contains(" 200"), "export: {status}\n{a}");

    let (status, a) = request(&s, "POST", "/api/dev-trial?name=trl&whole=0&note=first", b"");
    assert!(status.contains(" 200"), "trial: {status}\n{a}");
    assert_eq!(a["commands"]["trl"], ";forge trl");
    assert!(a["findings"].is_array());
    let sent = seen.lock().unwrap().iter().find(|(m, p, _)| m == "POST" && p == "/api/v1/dev/trial").cloned().expect("a trial sent");
    let form = parts(&sent.2);
    let field = |k: &str| form.iter().find(|(n, _)| n == k).map_or_else(|| panic!("no {k}"), |(_, d)| d.clone());
    assert_eq!(field("user"), b"local");
    assert_eq!(field("note"), b"first");
    assert!(form.iter().all(|(n, _)| n != "dry_run"), "a trial is no dry run");
    let areas: serde_json::Value = serde_json::from_slice(&field("areas")).unwrap();
    assert_eq!(areas[0]["name"], "trl");
    assert_eq!(areas[0]["new"], true);
    assert!(zip_names(&field("module_trl")).iter().any(|f| f.contains(".are.") && ext(f, "json")));
    assert!(zip_names(&field("hak_trl")).iter().any(|f| ext(f, "set")));
    let library = zip_names(&field("library_files"));
    assert_eq!(library.iter().filter(|f| ext(f, "shd")).count(), 2, "both shaders: {library:?}");
    assert!(library.iter().any(|f| ext(f, "dds")), "no textures in {library:?}");

    let (status, a) = request(&s, "GET", "/api/dev-trial", b"");
    assert!(status.contains(" 200"), "status: {status}\n{a}");
    assert_eq!(a["enabled"], true);
    assert_eq!(a["trial"]["commands"]["trl"], ";forge trl");
    assert!(!a.to_string().contains("town"), "only the builder's own trial: {a}");
    assert!(seen.lock().unwrap().iter().any(|(_, p, _)| p == "/api/v1/dev/trial?user=local&touch=false"));
    request(&s, "GET", "/api/dev-trial?touch=1", b"");
    assert!(seen.lock().unwrap().iter().any(|(_, p, _)| p == "/api/v1/dev/trial?user=local&touch=true"));

    let (status, a) = request(&s, "POST", "/api/dev-trial/release", b"");
    assert!(status.contains(" 400"), "release: {status}\n{a}");
    assert_eq!(a["error"], "releasing the trial: local has no trial on dev.");
    drop(s);
    let _ = std::fs::remove_dir_all(&root);
}
