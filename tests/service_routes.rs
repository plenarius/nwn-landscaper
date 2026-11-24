// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The read-only service routes (`/api/service/*`, docs/REVIEW-SERVICE.md "Fetching exports") through a real team server on a
//! scratch data folder. The exports are written by hand (no game files needed); the area files of a real export, and the `export`
//! id Publish sends, are checked by the ignored test at the end: `NWN_ROOT=<the game> cargo test --test service_routes -- --ignored`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::Digest;

const KEY: &str = "test-service-key-0123456789abcdef";

struct Server {
    child: Child,
    port: u16,
    root: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// The scratch folders of a test called `tag`.
fn scratch(tag: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("service_routes-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("xdg/landscaper/textures")).unwrap();
    std::fs::write(root.join("xdg/landscaper/textures/catalog.json"), "{}").unwrap();
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/index.html"), "<!doctype html><title>t</title>").unwrap();
    std::fs::create_dir_all(root.join("home")).unwrap();
    root
}

/// The team server's command on `root`, signing in by `X-Forwarded-User` with no proxy secret (`LANDSCAPER_INSECURE`).
fn command(root: &Path, env: &[(&str, &str)]) -> Command {
    let game = std::env::var_os("NWN_ROOT").unwrap_or_else(|| root.join("nogame").into_os_string());
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_landscaper"));
    cmd.args(["serve", "--team", "--port", "0", "--web"])
        .arg(root.join("web"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("NWN_ROOT", game)
        .env("LANDSCAPER_AUTH", "headers")
        .env("LANDSCAPER_ALLOWED_GROUPS", "*")
        .env("LANDSCAPER_ADMIN_GROUPS", "none")
        .env("LANDSCAPER_ADMIN_USERS", "alice")
        .env("LANDSCAPER_PUBLIC_HOST", "landscaper.test")
        .env("LANDSCAPER_INSECURE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

fn start(root: PathBuf, env: &[(&str, &str)]) -> Server {
    let mut child = command(&root, env).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("start landscaper serve");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let port = lines
        .by_ref()
        .map_while(Result::ok)
        .find_map(|l| l.split_once(" on ").and_then(|(_, r)| r.split_whitespace().next()?.rsplit_once(':')?.1.parse::<u16>().ok()))
        .expect("the server printed no address");
    std::thread::spawn(move || lines.for_each(drop));
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(started.elapsed() < Duration::from_secs(120), "the server didn't start");
        std::thread::sleep(Duration::from_millis(50));
    }
    Server { child, port, root }
}

/// An answer: status, headers (lower case) and body.
struct Got {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Got {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("not JSON ({}): {}", self.status, String::from_utf8_lossy(&self.body)))
    }

    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|l| l.strip_prefix(&format!("{name}: ")).map(str::to_string))
    }
}

/// `method target` with only `headers` (no Host, no `X-Landscaper`: a service sends neither), the body read to its length.
fn send(s: &Server, method: &str, target: &str, headers: &[(&str, &str)]) -> Got {
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let mut req = format!("{method} {target} HTTP/1.1\r\nConnection: close\r\nContent-Length: 0\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    c.write_all(req.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").expect("an answer");
    let head = String::from_utf8_lossy(&out[..split]).to_ascii_lowercase();
    let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Got { status, head, body: out[split + 4..].to_vec() }
}

/// A GET with the service key.
fn service(s: &Server, target: &str) -> Got {
    send(s, "GET", target, &[("Authorization", &format!("Bearer {KEY}"))])
}

fn sha(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes an export of `name` into `user`'s workspace as an export leaves it: its header and its hak. `from`: the area it reworks.
fn export(data: &Path, user: &str, name: &str, from: Option<&str>, hak: &[u8]) {
    let ws = data.join("workspaces").join(user);
    std::fs::create_dir_all(ws.join("exports")).unwrap();
    std::fs::create_dir_all(ws.join("nwn/hak")).unwrap();
    std::fs::write(ws.join(format!("exports/{name}.json")), format!(r#"{{"name":"{name}","tiles":3,"module":"mod_{name}"}}"#)).unwrap();
    std::fs::write(ws.join(format!("nwn/hak/{name}.hak")), hak).unwrap();
    if let Some(area) = from {
        std::fs::create_dir_all(ws.join(format!("exports/{name}.import"))).unwrap();
        std::fs::write(ws.join(format!("exports/{name}.import/source.json")), format!(r#"{{"area":"{area}"}}"#)).unwrap();
    }
}

#[test]
fn a_service_lists_and_downloads_a_builders_own_exports_and_nothing_else() {
    let root = scratch("routes");
    let data = root.join("xdg/landscaper");
    // bob reworks town (assigned to him) and keep (not assigned); alice has an export of her own.
    let hak: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    export(&data, "bob", "rwk", Some("town"), &hak);
    export(&data, "bob", "old", Some("keep"), b"old hak");
    export(&data, "alice", "cave", None, b"alice's hak");
    std::fs::write(data.join("workspaces/bob/nwn/hak/ls_textures.hak"), b"bob's library").unwrap();
    let grant = serde_json::json!([{ "user": "bob", "area": "town", "granted_by": "alice", "granted_at": 1 }]);
    std::fs::write(data.join("assignments.json"), grant.to_string()).unwrap();
    let s = start(root, &[("LANDSCAPER_SERVICE_KEY", KEY)]);

    // The list: bob's fetchable export only, with its hak's and library's sizes and SHAs, and an id of the header and the hak.
    let got = service(&s, "/api/service/exports?user=bob");
    assert_eq!(got.status, 200, "{}", String::from_utf8_lossy(&got.body));
    let list = got.json();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    let e = &list[0];
    assert_eq!(
        (e["name"].as_str(), e["area"].as_str(), e["new"].as_bool(), e["module"].as_str()),
        (Some("rwk"), Some("town"), Some(false), Some("mod_rwk"))
    );
    assert_eq!(e["hak"], serde_json::json!({ "bytes": hak.len(), "sha256": sha(&hak) }));
    assert_eq!(e["library"], serde_json::json!({ "bytes": 13, "sha256": sha(b"bob's library") }));
    assert!(e["exported_at"].as_u64().is_some_and(|t| t > 1_700_000_000), "{e}");
    let header = std::fs::read(data.join("workspaces/bob/exports/rwk.json")).unwrap();
    let id = sha(&[header.as_slice(), sha(&hak).as_bytes()].concat());
    assert_eq!(e["export"], id.as_str());

    // The hak, streamed whole, its ETag the SHA-256; 304 when the service has it already.
    let got = service(&s, "/api/service/hak?user=bob&name=rwk");
    assert_eq!(got.status, 200);
    assert!(got.body == hak, "the hak as it is on disk ({} bytes of {})", got.body.len(), hak.len());
    assert_eq!(got.header("content-length"), Some(hak.len().to_string()), "a length, not chunks");
    let etag = got.header("etag").unwrap();
    assert_eq!(etag, format!("\"{}\"", sha(&hak)));
    assert_eq!(got.header("cache-control").as_deref(), Some("private, no-store"));
    let again =
        send(&s, "GET", "/api/service/hak?user=bob&name=rwk", &[("Authorization", &format!("Bearer {KEY}")), ("If-None-Match", &etag)]);
    assert_eq!((again.status, again.body.len()), (304, 0));
    let lib = service(&s, "/api/service/library?user=bob");
    assert_eq!((lib.status, lib.body.as_slice()), (200, b"bob's library".as_slice()));

    // An ended (here: never given) assignment is refused as Publish refuses it; another builder's export isn't bob's.
    let refused = service(&s, "/api/service/hak?user=bob&name=old");
    assert_eq!(refused.status, 403, "{}", String::from_utf8_lossy(&refused.body));
    assert!(refused.json()["error"].as_str().unwrap().contains("keep"));
    assert_eq!(service(&s, "/api/service/hak?user=bob&name=cave").status, 404);
    assert_eq!(service(&s, "/api/service/export?user=bob&name=cave").status, 404);
    // alice is an admin by name: her new area is listed without a module's area list to check its name against.
    assert_eq!(service(&s, "/api/service/exports?user=alice").json()[0]["name"], "cave");
    assert_eq!(service(&s, "/api/service/library?user=alice").status, 404, "no library in her workspace");

    // Names that would leave the workspace, a user with none, and no user at all.
    for bad in ["..%2Falice", "%2Fetc", "a%5Cb", "rwk%00", "RWK", "abcdefghij"] {
        assert_eq!(service(&s, &format!("/api/service/hak?user=bob&name={bad}")).status, 400, "{bad}");
    }
    for user in ["..%2Fbob", "bob%2F..", "%2E%2E"] {
        assert_eq!(service(&s, &format!("/api/service/exports?user={user}")).status, 400, "{user}");
    }
    assert_eq!(service(&s, "/api/service/exports?user=carol").status, 404);
    assert_eq!(service(&s, "/api/service/exports").status, 400);
    assert_eq!(service(&s, "/api/service/nothing?user=bob").status, 404);

    // The key: required, exact, and GET only.
    for auth in [None, Some("Bearer wrong"), Some(&format!("Bearer {KEY}x")[..]), Some(&format!("Basic {KEY}")[..]), Some(KEY)] {
        let h: Vec<(&str, &str)> = auth.iter().map(|a| ("Authorization", *a)).collect();
        assert_eq!(send(&s, "GET", "/api/service/exports?user=bob", &h).status, 401, "{auth:?}");
    }
    assert_eq!(send(&s, "POST", "/api/service/exports?user=bob", &[("Authorization", &format!("Bearer {KEY}"))]).status, 405);
    // The key opens nothing else: the editor's API still wants the sign-in proxy's user.
    let other =
        send(&s, "GET", "/api/areas", &[("Host", "landscaper.test"), ("X-Landscaper", "1"), ("Authorization", &format!("Bearer {KEY}"))]);
    assert_eq!(other.status, 401, "{}", String::from_utf8_lossy(&other.body));

    // A new export: a new id and ETag.
    let hak2 = [hak.as_slice(), b"more"].concat();
    std::fs::write(s.root.join("xdg/landscaper/workspaces/bob/nwn/hak/rwk.hak"), &hak2).unwrap();
    let e2 = service(&s, "/api/service/exports?user=bob").json()[0].clone();
    assert_ne!(e2["export"], id.as_str());
    assert_eq!(e2["hak"]["sha256"], sha(&hak2).as_str());
    let got =
        send(&s, "GET", "/api/service/hak?user=bob&name=rwk", &[("Authorization", &format!("Bearer {KEY}")), ("If-None-Match", &etag)]);
    assert_eq!((got.status, got.body.len()), (200, hak2.len()));

    // Every call is in the activity log as service-export, as the builder.
    let log: String = std::fs::read_dir(s.root.join("xdg/landscaper"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("audit"))
        .flat_map(|e| {
            if e.path().is_dir() { std::fs::read_dir(e.path()).unwrap().flatten().map(|f| f.path()).collect() } else { vec![e.path()] }
        })
        .map(|p| std::fs::read_to_string(p).unwrap_or_default())
        .collect();
    assert!(
        log.lines().any(|l| l.contains(r#""action":"service-export""#) && l.contains(r#""user":"bob""#) && l.contains(r#""area":"rwk""#)),
        "{log}"
    );
}

#[test]
fn without_the_key_the_routes_are_not_there_and_a_short_key_stops_the_server() {
    let s = start(scratch("off"), &[]);
    std::fs::create_dir_all(s.root.join("xdg/landscaper/workspaces/bob")).unwrap();
    assert_eq!(service(&s, "/api/service/exports?user=bob").status, 404);
    drop(s);
    let root = scratch("short");
    let out = command(&root, &[("LANDSCAPER_SERVICE_KEY", "short")]).stdout(Stdio::null()).stderr(Stdio::piped()).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("LANDSCAPER_SERVICE_KEY is 5 characters"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// What the stand-in review service was sent: path and body of each POST.
type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// A review service with an empty module that takes every publish.
fn review() -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let kept = seen.clone();
    std::thread::spawn(move || {
        for mut c in listener.incoming().flatten() {
            let mut r = BufReader::new(c.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
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
            let json = match path.split('?').next().unwrap_or("") {
                "/api/v1/areas/index" => serde_json::json!({ "areas": [{ "area": "town" }] }),
                "/api/v1/areas/publish" => serde_json::json!({ "plan": { "area": "fga", "areas": ["fga"], "new": true, "module_files": [],
                                                                         "hak_files": 1, "library": true, "warnings": [], "review": "" },
                                                               "pr_urls": null }),
                _ => serde_json::json!({ "detail": "not here" }),
            };
            let status = if json.get("detail").is_some() { 404 } else { 200 };
            if len > 0 {
                kept.lock().unwrap().push((path, body));
            }
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

/// The text field `name` of a multipart body.
fn field(body: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let at = text.find(&format!("name=\"{name}\"\r\n\r\n"))? + name.len() + 11;
    Some(text[at..].split("\r\n").next()?.to_string())
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test service_routes -- --ignored"]
fn a_real_export_is_fetched_with_its_area_files_and_publish_sends_the_same_export_id() {
    let (port, seen) = review();
    let url = format!("http://127.0.0.1:{port}");
    let root = scratch("real");
    // (With the game, the server builds its starter textures: the empty stand-in would leave the export none.)
    std::fs::remove_dir_all(root.join("xdg/landscaper/textures")).unwrap();
    let s = start(root, &[("LANDSCAPER_SERVICE_KEY", KEY), ("LANDSCAPER_REVIEW_URL", &url), ("LANDSCAPER_REVIEW_KEY", "k")]);
    let n = 3 * 16 + 1;
    let mut body: Vec<u8> = (0..n * n).flat_map(|_| 1.0f32.to_le_bytes()).collect();
    body.extend(vec![0u8; n * n]);
    body.extend(br#"{"palette":["grassMedium"]}"#);
    let editor = |method: &str, target: &str, body: &[u8]| {
        let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(900))).unwrap();
        write!(c, "{method} {target} HTTP/1.1\r\nHost: landscaper.test\r\nX-Landscaper: 1\r\nX-Forwarded-User: bob\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
        c.write_all(body).unwrap();
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        out
    };
    let out = editor("POST", "/api/export?name=fga&tiles=3", &body);
    assert!(out.starts_with("HTTP/1.1 200"), "export: {out:.600}");

    let list = service(&s, "/api/service/exports?user=bob").json();
    let e = &list[0];
    assert_eq!((e["name"].as_str(), e["area"].as_str(), e["new"].as_bool()), (Some("fga"), Some("fga"), Some(true)), "{list}");
    let got = service(&s, "/api/service/export?user=bob&name=fga");
    assert_eq!(got.status, 200, "{}", String::from_utf8_lossy(&got.body));
    let x = got.json();
    assert_eq!(x["export"], e["export"]);
    assert_eq!(x["docs"]["are"]["ResRef"]["value"], "fga");
    assert_eq!(x["docs"]["are"]["Tileset"]["value"], "fga");
    assert!(x["docs"]["git"].is_object() && x["blueprints"].is_object() && x["findings"].is_array(), "{x:.400}");
    assert!(!x.to_string().contains("ls_ref"), "the builder's mark is taken out");
    let hak = service(&s, "/api/service/hak?user=bob&name=fga");
    assert_eq!((hak.status, hak.body.len() as u64), (200, e["hak"]["bytes"].as_u64().unwrap()));
    assert_eq!(sha(&hak.body), e["hak"]["sha256"].as_str().unwrap());

    // Publish sends the id the service listed.
    let out = editor("POST", "/api/publish?name=fga&dry=0&note=go", b"");
    assert!(out.starts_with("HTTP/1.1 200"), "publish: {out:.600}");
    let sent = seen.lock().unwrap().iter().rev().find(|(p, _)| p == "/api/v1/areas/publish").cloned().expect("a publish");
    assert_eq!(field(&sent.1, "export").as_deref(), e["export"].as_str());
}
