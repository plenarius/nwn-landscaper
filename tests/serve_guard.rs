// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `landscaper serve` from the outside: a real server on a scratch data folder (no game files, so routes that need them
//! fail, which is fine: what is checked is what happens before that).
//!
//! - Every route that takes a name (`name`, `resref`, `file`, `module`, `user`, `area`, `ext`, or a piece of the path)
//!   gets `../`, an absolute path, a backslash, a NUL byte and an over-long name, and nothing may be written outside the
//!   data folder, nor anything named after the probe anywhere.
//! - The request gate: the `X-Landscaper` header, `Origin`, `Sec-Fetch-Site` and `Host`.
//! - A team server: one builder can't write into another's workspace, and with `LANDSCAPER_ADMIN_GROUPS=none` a groups
//!   header makes nobody an admin.
//! - A team server's records and names: an import can't take a saved area's name, a revoke keeps what another assignment
//!   covers, a damaged `assignments.json` is never written over, and a hung review service costs one call, not one a request.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// What a Windows program can't start without, kept through `env_clear` (Winsock fails without `SystemRoot`; temporary files
/// would go to the Windows folder without `TEMP`). Nothing elsewhere.
fn windows_basics() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let wanted = ["SystemRoot", "SystemDrive", "TEMP", "TMP"];
    std::env::vars_os().filter(|(k, _)| cfg!(windows) && wanted.iter().any(|w| k.eq_ignore_ascii_case(w))).collect()
}

/// A server and its scratch folder; stopped when dropped.
struct Server {
    child: Child,
    port: u16,
    root: PathBuf,
}

/// The port a server started with `--port 0` says it listens on (`http://127.0.0.1:41234/`, or `on 0.0.0.0:41234` for a team
/// server); the rest of what it prints is read and dropped, so it never blocks on a full pipe.
fn printed_port(out: std::process::ChildStdout) -> u16 {
    use std::io::BufRead;
    let mut lines = std::io::BufReader::new(out).lines();
    let port = lines
        .by_ref()
        .map_while(Result::ok)
        .find_map(|line| {
            line.split_whitespace().find_map(|w| {
                let w = w.trim_start_matches("http://").trim_end_matches('/');
                w.rsplit_once(':').filter(|(host, _)| host.contains('.') || host.contains(':')).and_then(|(_, p)| p.parse::<u16>().ok())
            })
        })
        .expect("the server printed no address");
    std::thread::spawn(move || lines.for_each(drop));
    port
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

impl Server {
    fn start(tag: &str, env: &[(&str, &str)], team: bool) -> Server {
        Server::start_with(tag, env, team, &[])
    }

    /// As [`Server::start`], with more arguments for `landscaper serve`.
    fn start_with(tag: &str, env: &[(&str, &str)], team: bool, args: &[&str]) -> Server {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serve_guard-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Server::spawn(root, env, team, args)
    }

    /// Stops the server and starts another on the same data folder, with `env`: a restart.
    fn restart(mut self, env: &[(&str, &str)], team: bool) -> Server {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let root = self.root.clone();
        // (Dropping `self` would remove the folder.)
        std::mem::forget(self);
        Server::spawn(root, env, team, &[])
    }

    /// Starts `landscaper serve` on the scratch folder `root`, making what it needs there.
    fn spawn(root: PathBuf, env: &[(&str, &str)], team: bool, args: &[&str]) -> Server {
        let data = root.join("xdg/landscaper");
        // A texture library already there, so the server doesn't try to build one from game files it hasn't got.
        std::fs::create_dir_all(data.join("textures")).unwrap();
        std::fs::write(data.join("textures/catalog.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::write(root.join("web/index.html"), "<!doctype html><title>t</title>").unwrap();
        for d in ["home", "nogame", "outside"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_landscaper"));
        // `--port 0`: the server takes a free port and prints it. (Picking one here and handing it over raced: another
        // test's server could take it in between, and this test then talked to that one.)
        cmd.args(["serve", "--port", "0", "--web"])
            .arg(root.join("web"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .envs(windows_basics())
            .env("HOME", root.join("home"))
            .env("XDG_DATA_HOME", root.join("xdg"))
            .env("NWN_ROOT", root.join("nogame"))
            .env("NWN_HOME", root.join("nogame"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if team {
            cmd.arg("--team");
        }
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("start landscaper serve");
        let port = printed_port(child.stdout.take().unwrap());
        let server = Server { child, port, root };
        let started = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(started.elapsed() < Duration::from_secs(60), "the server didn't start");
            std::thread::sleep(Duration::from_millis(50));
        }
        server
    }

    fn data(&self) -> PathBuf {
        self.root.join("xdg/landscaper")
    }

    /// Sends a request with `headers` (Host and the like are not added: give them); its status and body.
    fn raw(&self, method: &str, target: &str, headers: &[(&str, &str)], body: &[u8]) -> (u16, String) {
        let (status, _, body) = self.exchange(method, target, headers, body);
        (status, body)
    }

    /// As [`Server::raw`], with the answer's headers too (in lower case).
    fn exchange(&self, method: &str, target: &str, headers: &[(&str, &str)], body: &[u8]) -> (u16, String, String) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut req = format!("{method} {target} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).unwrap();
        s.write_all(body).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out).into_owned();
        let status = text.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        (status, head.to_ascii_lowercase(), body.to_string())
    }

    /// What the editor sends: its own Host and Origin, and the header.
    fn editor(&self, method: &str, target: &str, extra: &[(&str, &str)], body: &[u8]) -> (u16, String) {
        let host = format!("127.0.0.1:{}", self.port);
        let origin = format!("http://{host}");
        let mut h = vec![("Host", host.as_str()), ("X-Landscaper", "1"), ("Origin", origin.as_str())];
        h.extend_from_slice(extra);
        self.raw(method, target, &h, body)
    }
}

#[test]
fn every_answer_carries_the_pages_policy_and_refuses_frames_and_sniffing() {
    let s = Server::start("headers", &[], false);
    let policy = "default-src 'self'; script-src 'self' 'sha256-TCQjodja3ktH01cTB7LwVQgTmMjeLTzCsuSoGtZ5I3U='; object-src 'none'";
    std::fs::write(
        s.root.join("web/index.html"),
        format!(r#"<!doctype html><meta http-equiv="Content-Security-Policy" content="{policy}"><title>t</title>"#),
    )
    .unwrap();
    // The page, an API answer and a refusal: each with every header (read from the raw answer: `raw` keeps only the body).
    let host = format!("127.0.0.1:{}", s.port);
    for (target, api) in [("/", false), ("/api/me", true), ("/api/nothing-here", true), ("/../etc/passwd", false)] {
        let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let extra = if api { "X-Landscaper: 1\r\n" } else { "" };
        write!(c, "GET {target} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        let head = out.split("\r\n\r\n").next().unwrap_or_default().to_ascii_lowercase();
        for want in [
            format!("content-security-policy: {policy}; frame-ancestors 'none'").to_ascii_lowercase(),
            "x-content-type-options: nosniff".into(),
            "x-frame-options: deny".into(),
        ] {
            assert!(head.contains(&want), "{target}: no `{want}` in\n{head}");
        }
    }
}

/// Every file and folder under `dir`.
fn tree(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        out.push(e.path());
        if e.path().is_dir() {
            tree(&e.path(), out);
        }
    }
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// An export's body for a 3 by 3 tile area, as the editor sends it: `heights` (f32 bytes), the paint (all the palette's first
/// texture), then `json` (the palette, water, entrances, atmosphere, objects...).
fn export_body(heights: &[u8], json: &serde_json::Value) -> Vec<u8> {
    let n = 3 * 16 + 1;
    let mut body = heights.to_vec();
    body.extend(vec![0u8; n * n]);
    body.extend(json.to_string().into_bytes());
    body
}

#[test]
fn an_export_never_gives_a_taken_out_entrances_id_again() {
    // The last export of `cave` had entrances 1 and 3 (2 was taken out earlier) and counted to 4.
    let s = Server::start("entrance-ids", &[], false);
    let exports = s.data().join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let e = |id: u32, y: u32| format!(r#"{{"id":{id},"x":19,"y":{y},"dir":0,"width":4,"depth":4,"floor":0}}"#);
    let n = 3 * 16 + 1;
    let header = format!(
        r#"{{"name":"cave","tiles":3,"spacing":0.625,"width":{n},"height":{n},"data":"cave.f32","entrances":[{},{}],"entranceNextId":4}}"#,
        e(1, 5),
        e(3, 25)
    );
    std::fs::write(exports.join("cave.json"), &header).unwrap();
    let heights = vec![0u8; n * n * 4];
    // A new entrance that takes 2 again: refused before anything is written.
    let reused: serde_json::Value = serde_json::from_str(&format!("[{},{},{}]", e(1, 5), e(3, 25), e(2, 15))).unwrap();
    let body = export_body(&heights, &serde_json::json!({ "palette": ["grassMedium"], "entrances": reused }));
    let (st, b) = s.editor("POST", "/api/export?name=cave&tiles=3", &[], &body);
    assert_eq!(st, 400, "{b}");
    assert!(b.contains("entrance id 2") && b.contains("4 or more"), "{b}");
    assert_eq!(std::fs::read_to_string(exports.join("cave.json")).unwrap(), header);
}

/// Names a request may try, each with "probe" in it.
fn bad_names(root: &Path) -> Vec<String> {
    vec![
        "../probe".into(),
        "../../probe".into(),
        "../../../probe".into(),
        "../../../../probe".into(),
        root.join("outside/probe").display().to_string(),
        "..\\..\\probe".into(),
        "pro\0be".into(),
        format!("{}probe", "x".repeat(300)),
    ]
}

/// Every route that makes a path from a name in the request: (method, target with `{}` for the name, body with `@NAME@` for it
/// in a JSON string).
fn routes() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let heights = vec![0u8; 49 * 49 * 4];
    let p = |m, t| (m, t, Vec::new());
    vec![
        ("POST", "/api/export?name={}&tiles=3", heights.clone()),
        ("POST", "/api/check?name={}&tiles=3", heights.clone()),
        ("POST", "/api/export?name=okarea&tiles=3", export_body(&heights, &serde_json::json!({ "palette": ["@import:@NAME@"] }))),
        ("POST", "/api/save?name={}", br#"{"name":"x","tiles":3,"heights":""}"#.to_vec()),
        ("PUT", "/api/save?name={}", br#"{"name":"x","tiles":3,"heights":""}"#.to_vec()),
        p("POST", "/api/import?name={}&upload=a.mod&area=a"),
        p("POST", "/api/import?name=okname&upload={}&area=a"),
        p("POST", "/api/import?name=okname&upload={}.mod&area=a"),
        p("POST", "/api/import?name={}&source={}"),
        p("POST", "/api/reimport?name={}"),
        p("POST", "/api/publish?name={}&dry=1"),
        p("POST", "/api/publish-module?module={}&dry=1"),
        p("POST", "/api/package?name={}&whole=1"),
        p("POST", "/api/package?name={}"),
        p("POST", "/api/version?name={}"),
        p("POST", "/api/pick-texture?name={}&x=1&y=1"),
        p("POST", "/api/textures/packs/install?id={}"),
        p("POST", "/api/textures/packs/remove?id={}"),
        ("POST", "/api/upload?file={}", b"MOD V1.0".to_vec()),
        ("POST", "/api/upload?file={}.mod", b"MOD V1.0".to_vec()),
        p("POST", "/api/upload-delete?file={}"),
        p("POST", "/api/upload-delete?file={}.mod"),
        p("POST", "/api/new-blueprint?ext=uti&resref={}&from=nw_it_gem001&name=x"),
        p("POST", "/api/new-blueprint?ext={}&resref=okbp&from=nw_it_gem001&name=x"),
        ("POST", "/api/new-blueprint-edit?ext=uti&resref={}", b"{}".to_vec()),
        ("POST", "/api/new-blueprint-edit?ext={}&resref=okbp", b"{}".to_vec()),
        p("POST", "/api/new-blueprint-delete?ext=uti&resref={}"),
        p("POST", "/api/new-blueprint-delete?ext={}&resref=okbp"),
        ("POST", "/api/new-blueprint-gear?resref={}", br#"{"inventory":[]}"#.to_vec()),
        ("POST", "/api/new-blueprint-gear?ext={}&resref=okbp", br#"{"inventory":[]}"#.to_vec()),
        p("POST", "/api/script?resref={}&from=&kind=action"),
        ("POST", "/api/script-save?resref={}", b"void main() {}".to_vec()),
        p("POST", "/api/script-delete?resref={}"),
        p("POST", "/api/dialog?resref={}&from="),
        ("POST", "/api/dialog-save?resref={}", b"{}".to_vec()),
        p("POST", "/api/dialog-delete?resref={}"),
        ("POST", "/api/itemprops-add?resref={}", br#"{"property":1}"#.to_vec()),
        p("POST", "/api/itemprops-remove?resref={}&index=0"),
        p("POST", "/api/assignments?user={}&area=a"),
        p("POST", "/api/assignments?user=bob&area={}"),
        p("POST", "/api/assignments/revoke?user={}&area={}"),
        p("POST", "/api/view-as?user={}"),
        p("GET", "/api/area?name={}"),
        p("DELETE", "/api/area?name={}&base=none"),
        p("GET", "/api/originals?name={}"),
        ("PUT", "/api/underlay?name={}", b"\x89PNG\r\n\x1a\n".to_vec()),
        p("GET", "/api/underlay?name={}"),
        p("DELETE", "/api/underlay?name={}"),
        p("GET", "/api/package-download?name={}&whole=1"),
        p("GET", "/api/import/{}"),
        p("GET", "/api/import-file/{}/decal_a.png"),
        p("GET", "/api/import-file/okname/{}"),
        p("GET", "/api/new-blueprint?ext=uti&resref={}"),
        p("GET", "/api/script?resref={}&any=1"),
        p("GET", "/api/dialog?resref={}&any=1"),
        p("GET", "/api/itemprops?resref={}"),
        p("GET", "/api/blueprint-fields?kind=door&resref={}"),
        p("GET", "/api/doors?module={}"),
        p("GET", "/api/links?module={}"),
        p("GET", "/api/model/{}"),
        p("GET", "/api/texture/{}"),
        p("GET", "/api/waypoints?area={}"),
        p("GET", "/textures/{}"),
        p("GET", "/{}"),
    ]
}

/// An ERF holding one 3 by 3 area, `ttr01`, as a builder uploads one.
fn area_erf() -> Vec<u8> {
    let mut are = mg_gff::Gff::new(*b"ARE ");
    for (k, v) in [("Width", 3), ("Height", 3)] {
        are.root.set(k, mg_gff::Value::Int(v));
    }
    are.root.set("Tileset", mg_gff::Value::ResRef(b"ttr01".to_vec()));
    let mut erf = mg_erf::ErfWriter::new(*b"ERF ");
    let data = are.to_bytes().unwrap();
    erf.add(mg_core::ResRef::from_str("cave1").unwrap(), mg_core::ResType::ARE, data.as_slice()).unwrap();
    erf.to_bytes().unwrap()
}

#[test]
fn uploads_take_turns_and_stay_within_a_workspaces_limits() {
    let env = [("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_INSECURE", "1")];
    let s = Server::start("uploads", &env, true);
    let bob = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "bob")];
    let erf = area_erf();
    let (st, b) = s.raw("POST", "/api/upload?file=cave.erf", &bob, &erf);
    assert_eq!(st, 200, "{b}");
    assert!(b.contains(r#""area":"cave1""#), "{b}");
    let (_, list) = s.raw("GET", "/api/uploads", &bob, b"");
    assert!(list.contains(r#""file":"cave.erf""#) && list.contains("cave1"), "{list}");
    // One upload at a time: while one is still arriving, a second is refused.
    let mut slow = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    write!(
        slow,
        "POST /api/upload?file=slow.mod HTTP/1.1\r\nHost: landscaper.test\r\nX-Landscaper: 1\r\nX-Forwarded-User: bob\r\n\
         Connection: close\r\nContent-Length: 1000000\r\n\r\nMOD V1.0"
    )
    .unwrap();
    let uploads = s.data().join("workspaces/bob/uploads");
    let part = || std::fs::read_dir(&uploads).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part"));
    let wait = |until: &dyn Fn() -> bool, what: &str| {
        let started = Instant::now();
        while !until() {
            assert!(started.elapsed() < Duration::from_secs(30), "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait(&part, "the slow upload started");
    let (st, b) = s.raw("POST", "/api/upload?file=other.erf", &bob, &erf);
    assert_eq!(st, 429, "{b}");
    // Another builder isn't held up.
    let carol = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "carol")];
    assert_eq!(s.raw("POST", "/api/upload?file=cave.erf", &carol, &erf).0, 200);
    drop(slow);
    wait(&|| !part(), "the slow upload ended");
    // (Its turn ends a moment after its file goes.)
    wait(&|| s.raw("POST", "/api/upload?file=other.erf", &bob, &erf).0 != 429, "bob's turn came back");
    assert!(uploads.join("other.erf").exists());
    // At most 20 files a workspace (a file of the same name is replaced, so it doesn't count).
    for k in 0..18 {
        std::fs::write(uploads.join(format!("old{k}.mod")), b"MOD ").unwrap();
    }
    let (st, b) = s.raw("POST", "/api/upload?file=one-more.erf", &bob, &erf);
    assert_eq!(st, 413, "{b}");
    assert!(b.contains("at their limit"), "{b}");
    assert_eq!(s.raw("POST", "/api/upload?file=cave.erf", &bob, &erf).0, 200, "replacing one is fine");
    let mut names: Vec<String> =
        std::fs::read_dir(&uploads).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.retain(|n| n.starts_with('.'));
    assert!(names.is_empty(), "no part files left: {names:?}");
}

/// Sends every bad name to every route; nothing may appear outside the data folder or carry the probe's name.
fn nothing_escapes(s: &Server, extra: &[(&str, &str)]) {
    let mut before = Vec::new();
    tree(&s.root, &mut before);
    for name in bad_names(&s.root) {
        for (method, target, body) in routes() {
            let target = target.replace("{}", &enc(&name));
            let in_json = serde_json::to_string(&name).unwrap();
            let body = match body.windows(6).position(|w| w == b"@NAME@") {
                Some(at) => [&body[..at], &in_json.as_bytes()[1..in_json.len() - 1], &body[at + 6..]].concat(),
                None => body.clone(),
            };
            let (status, answer) = s.editor(method, &target, extra, &body);
            // Answered by the route, not refused by the gate (that would test nothing).
            assert!(status != 0 && !answer.contains("\"error\":\"refused"), "{method} {target}: {status} {answer}");
        }
    }
    let mut after = Vec::new();
    tree(&s.root, &mut after);
    let data = s.data();
    for p in after.iter().filter(|p| !before.contains(p)) {
        assert!(p.starts_with(&data), "written outside the data folder: {}", p.display());
        assert!(!p.to_string_lossy().contains("probe"), "written: {}", p.display());
    }
}

#[test]
fn no_route_writes_outside_its_folder() {
    let s = Server::start("local", &[], false);
    nothing_escapes(&s, &[]);
    // And the files the reviewer found are not there.
    for p in ["xdg/probe.f32", "xdg/probe.json", "probe.f32", "probe.json"] {
        assert!(!s.root.join(p).exists(), "{p}");
    }
}

#[test]
fn a_failed_export_leaves_the_last_exports_files_as_they_were() {
    // The last export of `okarea`: its header (with the fog the installed module has), heights and paint.
    let s = Server::start("export-rollback", &[], false);
    let exports = s.data().join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let n = 3 * 16 + 1;
    let header = format!(
        concat!(
            r#"{{"name":"okarea","tiles":3,"tilesY":3,"spacing":0.625,"width":{n},"height":{n},"data":"okarea.f32","#,
            r##""atmosphere":{{"sunFog":"#111111"}}}}"##
        ),
        n = n
    );
    let was = [("okarea.json", header.into_bytes()), ("okarea.f32", vec![0u8; n * n * 4])];
    for (f, d) in &was {
        std::fs::write(exports.join(f), d).unwrap();
    }
    // An export with new fog and new ground that fails (here: no game to build it with; in the review, a missing model).
    let heights: Vec<u8> = std::iter::repeat_n(2.5f32.to_le_bytes(), n * n).flatten().collect();
    let sent = export_body(&heights, &serde_json::json!({ "palette": ["grassMedium"], "atmosphere": { "sunFog": "#ff0000" } }));
    let (status, body) = s.editor("POST", "/api/export?name=okarea&tiles=3", &[], &sent);
    assert_eq!(status, 400, "{body}");
    for (f, d) in &was {
        assert_eq!(&std::fs::read(exports.join(f)).unwrap(), d, "{f}: the failed export's version was left as the last export");
    }
    assert!(!exports.join("okarea.materials").exists(), "a file the last export didn't have isn't left either");
}

#[test]
fn the_gate_refuses_other_sites_and_names_and_lets_the_editor_in() {
    let s = Server::start("gate", &[], false);
    let host = format!("127.0.0.1:{}", s.port);
    let area = br#"{"name":"good","tiles":3,"heights":"","palette":[]}"#;
    // The editor's request passes: a save lands in the areas folder.
    let (status, body) = s.editor("POST", "/api/save?name=good", &[], area);
    assert_eq!(status, 200, "{body}");
    assert!(s.data().join("areas/good.json").is_file());
    assert_eq!(s.editor("GET", "/api/me", &[], b"").0, 200);
    assert_eq!(s.editor("GET", "/api/area?name=good", &[], b"").0, 200);
    assert_eq!(s.raw("GET", "/", &[("Host", &host)], b"").0, 200, "the page needs no header");
    // A dot file of the editor's folder (a clone's `.git`) is not served.
    std::fs::create_dir_all(s.root.join("web/.git")).unwrap();
    std::fs::write(s.root.join("web/.git/config"), "[core]").unwrap();
    std::fs::write(s.root.join("web/.env"), "X=1").unwrap();
    for dot in ["/.git/config", "/.env"] {
        assert_eq!(s.raw("GET", dot, &[("Host", &host)], b"").0, 403, "{dot}");
    }
    for host in ["localhost", "[::1]"] {
        assert_eq!(s.raw("GET", "/api/me", &[("Host", &format!("{host}:{}", s.port)), ("X-Landscaper", "1")], b"").0, 200, "{host}");
    }

    let refused = |status: u16, body: &str, what: &str| {
        assert_eq!(status, 403, "{what}: {body}");
        assert!(body.starts_with("{\"error\":\"refused"), "{what}: {body}");
    };
    // No header: the reviewer's cross-site export, sent the way a page can (text/plain, its own Origin).
    let (st, b) = s.raw(
        "POST",
        "/api/export?name=..%2F..%2F..%2Ftraversal_probe&tiles=3",
        &[("Host", &host), ("Content-Type", "text/plain"), ("Origin", "https://evil.example")],
        &[0u8; 49 * 49 * 4],
    );
    refused(st, &b, "cross-site export");
    let (st, b) = s.raw("POST", "/api/save?name=good", &[("Host", &host)], area);
    refused(st, &b, "no header");
    let (st, b) = s.raw("GET", "/api/script?resref=x&any=1", &[("Host", &host)], b"");
    refused(st, &b, "a read without the header");
    // A foreign Origin, even with the header.
    let (st, b) = s.raw("POST", "/api/save?name=good", &[("Host", &host), ("X-Landscaper", "1"), ("Origin", "https://evil.example")], area);
    refused(st, &b, "foreign Origin");
    let (st, b) = s.raw("POST", "/api/save?name=good", &[("Host", &host), ("X-Landscaper", "1"), ("Sec-Fetch-Site", "cross-site")], area);
    refused(st, &b, "Sec-Fetch-Site: cross-site");
    // A foreign Host (DNS rebinding), for the API and the page.
    let evil = format!("evil.example:{}", s.port);
    let (st, b) = s.raw("GET", "/api/me", &[("Host", &evil), ("X-Landscaper", "1")], b"");
    refused(st, &b, "foreign Host");
    let (st, b) = s.raw("GET", "/", &[("Host", &evil)], b"");
    refused(st, &b, "foreign Host, the page");
    // Pictures load without the header (a GET only: 404 here, there is no such picture, but not refused).
    let (st, b) = s.raw("GET", "/api/import/good", &[("Host", &host)], b"");
    assert_eq!(st, 404, "{b}");
    let (st, b) = s.raw("POST", "/api/import/good", &[("Host", &host)], b"");
    refused(st, &b, "a POST to a picture's path");
    let (st, b) = s.raw("GET", "/api/areas", &[("Host", &host)], b"");
    refused(st, &b, "a GET of another API route");
}

#[test]
fn a_team_builder_cant_write_into_another_builders_workspace_or_make_themselves_admin() {
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_ADMIN_USERS", "alice"),
        ("LANDSCAPER_ADMIN_GROUPS", "none"),
        ("LANDSCAPER_PUBLIC_HOST", "landscaper.test"),
        // (No proxy secret here, to keep the requests short; a_team_server_needs_a_proxy_secret tests it.)
        ("LANDSCAPER_INSECURE", "1"),
    ];
    let s = Server::start("team", &env, true);
    let alice = s.data().join("workspaces/alice/areas");
    std::fs::create_dir_all(&alice).unwrap();
    let bob = |method: &str, target: &str, extra: &[(&str, &str)], body: &[u8]| {
        let mut h = vec![("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "bob")];
        h.extend_from_slice(extra);
        s.raw(method, target, &h, body)
    };
    // The reviewer's reproduction: bob exports into alice's areas.
    let (st, b) = bob("POST", "/api/export?name=..%2F..%2F..%2Falice%2Fareas%2Fpwn&tiles=3", &[], &[0u8; 49 * 49 * 4]);
    assert_eq!(st, 400, "{b}");
    assert_eq!(std::fs::read_dir(&alice).unwrap().count(), 0, "nothing in alice's areas");
    nothing_escapes(&s, &[("X-Forwarded-User", "bob")]);
    // A forged groups header: not an admin, and no activity log.
    let (st, b) = bob("GET", "/api/me", &[("X-Forwarded-Groups", "Administrator")], b"");
    assert_eq!(st, 200, "{b}");
    assert!(b.contains(r#""admin":false"#), "{b}");
    assert_eq!(bob("GET", "/api/audit", &[("X-Forwarded-Groups", "Administrator")], b"").0, 403);
    // alice is an admin by name.
    let (_, b) = s.raw("GET", "/api/me", &[("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "alice")], b"");
    assert!(b.contains(r#""admin":true"#), "{b}");
    // Another name for the server is refused once LANDSCAPER_PUBLIC_HOST is set.
    let (st, b) = s.raw("GET", "/api/me", &[("Host", "evil.example"), ("X-Landscaper", "1"), ("X-Forwarded-User", "bob")], b"");
    assert_eq!(st, 403, "{b}");
}

/// A stand-in for the review service (docs/REVIEW-SERVICE.md) on a port of its own: it answers the area list after `delay`,
/// with `areas` (or 503 when there are none), and 404 to the rest. Counts the area list's fetches.
struct Review {
    url: String,
    fetches: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Review {
    fn start(areas: &'static str, delay: Duration) -> Review {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let fetches = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = fetches.clone();
        std::thread::spawn(move || {
            for mut c in listener.incoming().flatten() {
                let counted = counted.clone();
                std::thread::spawn(move || {
                    let mut head = Vec::new();
                    let mut byte = [0u8];
                    while !head.ends_with(b"\r\n\r\n") && c.read(&mut byte).is_ok_and(|n| n == 1) {
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head);
                    let path = head.split(' ').nth(1).unwrap_or_default();
                    let (status, body) = if path == "/api/v1/areas/index" {
                        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        std::thread::sleep(delay);
                        if areas.is_empty() { ("503 Service Unavailable", "{}".to_string()) } else { ("200 OK", areas.to_string()) }
                    } else {
                        ("404 Not Found", "{}".to_string())
                    };
                    let _ = write!(
                        c,
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                });
            }
        });
        Review { url, fetches }
    }

    fn fetches(&self) -> usize {
        self.fetches.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[test]
fn an_import_never_takes_the_name_of_a_saved_working_copy() {
    let env = [("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_INSECURE", "1")];
    let s = Server::start("import-name", &env, true);
    let carol = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "carol")];
    // The review's reproduction: carol saved `town` and never exported it, then imports an area of her own file as `town`.
    let (st, b) = s.raw("PUT", "/api/save?name=town&new=1", &carol, &area_json("town", 16, Some("carol-1")));
    assert_eq!(st, 200, "{b}");
    assert_eq!(s.raw("POST", "/api/upload?file=cave.erf", &carol, &area_erf()).0, 200);
    let (st, b) = s.raw("POST", "/api/import?upload=cave.erf&area=cave1&name=town", &carol, b"");
    assert_eq!(st, 409, "{b}");
    assert!(b.contains(r#""exists":"town""#) && b.contains("already an area called town"), "{b}");
    let exports = s.data().join("workspaces/carol/exports");
    assert!(!exports.join("town.import").exists() && !exports.join("town.json").exists(), "nothing imported");
}

#[test]
fn revoking_an_area_keeps_the_imports_another_assignment_still_covers() {
    let review = Review::start(r#"{"areas":[{"area":"chess","tag":"CHESS"}]}"#, Duration::ZERO);
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_ADMIN_USERS", "alice"),
        ("LANDSCAPER_INSECURE", "1"),
        ("LANDSCAPER_REVIEW_URL", review.url.as_str()),
        ("LANDSCAPER_REVIEW_KEY", "k"),
    ];
    let s = Server::start("revoke", &env, true);
    let alice = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "alice")];
    let admin = |target: &str| {
        let (st, b) = s.raw("POST", target, &alice, b"");
        assert_eq!(st, 200, "{target}: {b}");
        serde_json::from_str::<serde_json::Value>(&b).unwrap()
    };
    // erin's import of chess as chs: its kept original and its note of where it came from.
    let import = s.data().join("workspaces/erin/exports/chs.import");
    std::fs::create_dir_all(&import).unwrap();
    std::fs::write(import.join("source.json"), r#"{"area":"chess"}"#).unwrap();
    std::fs::write(import.join("area.are"), b"ARE V3.2").unwrap();
    let kept = || import.join("area.are").exists();
    let removed = |v: &serde_json::Value| v["removed_imports"].as_array().unwrap().len();
    admin("/api/assignments?user=erin&area=*");
    admin("/api/assignments?user=erin&area=chess");
    // The review's reproduction: chess revoked while `*` still covers it.
    let v = admin("/api/assignments/revoke?user=erin&area=chess");
    assert_eq!((v["ended"].as_u64(), removed(&v)), (Some(1), 0), "{v}");
    assert!(kept(), "`*` still covers chess");
    // A revoke that ends nothing removes nothing.
    let v = admin("/api/assignments/revoke?user=erin&area=chess");
    assert_eq!((v["ended"].as_u64(), removed(&v)), (Some(0), 0), "{v}");
    assert!(kept());
    // `*` revoked while chess is held again: kept.
    admin("/api/assignments?user=erin&area=chess");
    let v = admin("/api/assignments/revoke?user=erin&area=*");
    assert_eq!((v["ended"].as_u64(), removed(&v)), (Some(1), 0), "{v}");
    assert!(kept(), "chess still covers it");
    // The last assignment that covered it ends: it goes, all but its note.
    let v = admin("/api/assignments/revoke?user=erin&area=chess");
    assert_eq!(v["removed_imports"], serde_json::json!(["chs"]), "{v}");
    assert!(!kept() && import.join("source.json").exists());
}

#[test]
fn a_hung_review_service_costs_one_call_and_new_names_are_refused_without_its_list() {
    // A service that answers the area list 503 after 3 s: a new area's name can't be checked.
    let review = Review::start("", Duration::from_secs(3));
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_INSECURE", "1"),
        ("LANDSCAPER_REVIEW_URL", review.url.as_str()),
        ("LANDSCAPER_REVIEW_KEY", "k"),
    ];
    let s = Server::start("review-down", &env, true);
    // The review's reproduction: three saves of new areas at once. Before, they answered after one, two and three calls'
    // time, and each was let through.
    let started = Instant::now();
    let answers: Vec<(u16, String, Duration)> = std::thread::scope(|sc| {
        let calls: Vec<_> = (0..3)
            .map(|k| {
                let s = &s;
                sc.spawn(move || {
                    let h = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "ivy")];
                    let (st, b) = s.raw("PUT", &format!("/api/save?name=ar{k}&new=1"), &h, &area_json(&format!("ar{k}"), 16, None));
                    (st, b, started.elapsed())
                })
            })
            .collect();
        calls.into_iter().map(|c| c.join().unwrap()).collect()
    });
    for (st, b, took) in &answers {
        assert_eq!(*st, 400, "{b}");
        assert!(b.contains("area list isn't available"), "{b}");
        assert!(*took < Duration::from_secs(5), "answered after {took:?}: one call's time at most");
    }
    // A failure is kept: the next ask doesn't call again, and nothing was written.
    let h = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "ivy")];
    let t = Instant::now();
    let (st, b) = s.raw("GET", "/api/import-sources?q=forest", &h, b"");
    assert_eq!(st, 503, "{b}");
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(review.fetches(), 1, "the start's fetch, and no other");
    assert!(!s.data().join("workspaces/ivy/areas/ar0.json").exists());
}

#[test]
fn a_damaged_assignments_file_is_never_read_as_empty_or_written_over() {
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_ADMIN_USERS", "alice"),
        ("LANDSCAPER_INSECURE", "1"),
    ];
    let s = Server::start("records", &env, true);
    let alice = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "alice")];
    assert_eq!(s.raw("POST", "/api/assignments?user=erin&area=*", &alice, b"").0, 200);
    assert_eq!(s.raw("POST", "/api/assignments?user=gil&area=*", &alice, b"").0, 200);
    // The review's reproduction: a trailing comma, then a grant.
    let file = s.data().join("assignments.json");
    let damaged = std::fs::read_to_string(&file).unwrap().trim_end().trim_end_matches(']').trim_end().to_string() + ",]";
    std::fs::write(&file, &damaged).unwrap();
    let (st, b) = s.raw("GET", "/api/assignments", &alice, b"");
    assert_eq!(st, 500, "{b}");
    assert!(b.contains("assignments.json doesn't parse"), "{b}");
    let (st, b) = s.raw("POST", "/api/assignments?user=hal&area=*", &alice, b"");
    assert_eq!(st, 400, "{b}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), damaged, "left as it was");
    // A team server doesn't start on it, nor on a damaged markers.json.
    let start = |what: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_landscaper"))
            .args(["serve", "--team", "--port", "0", "--web"])
            .arg(s.root.join("web"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .envs(windows_basics())
            .env("HOME", s.root.join("home"))
            .env("XDG_DATA_HOME", s.root.join("xdg"))
            .env("NWN_ROOT", s.root.join("nogame"))
            .env("NWN_HOME", s.root.join("nogame"))
            .envs(env)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && err.contains(&format!("{what} doesn't parse")), "{what}: {err}");
    };
    start("assignments.json");
    std::fs::write(&file, "[]").unwrap();
    std::fs::write(s.data().join("markers.json"), "{\"erin/chess\": ").unwrap();
    start("markers.json");
}

/// An area as the editor saves it (`Area.toJSON`), `pad` bytes of heights long, with `created` if given.
fn area_json(name: &str, pad: usize, created: Option<&str>) -> Vec<u8> {
    let mut a = serde_json::json!({ "name": name, "tiles": 3, "heights": "A".repeat(pad) });
    if let Some(c) = created {
        a["created"] = c.into();
    }
    a.to_string().into_bytes()
}

#[test]
fn overlapping_saves_of_one_area_never_lose_it() {
    let env = [("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_INSECURE", "1")];
    let s = Server::start("save-race", &env, true);
    let areas = s.data().join("workspaces/bob/areas");
    // The review's reproduction: eight saves of one area at once, five times over (2 MB each here, 6 MB there).
    let bodies: Vec<Vec<u8>> = (0..8).map(|k| area_json("race", (2 << 20) + k, None)).collect();
    for round in 0..5 {
        std::thread::scope(|sc| {
            for b in &bodies {
                let s = &s;
                sc.spawn(move || {
                    let h = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "bob")];
                    let (st, body) = s.raw("PUT", "/api/save?name=race", &h, b);
                    assert_eq!(st, 200, "round {round}: {body}");
                });
            }
        });
        let now = std::fs::read(areas.join("race.json")).unwrap_or_else(|e| panic!("round {round}: race.json is gone ({e})"));
        assert!(bodies.contains(&now), "round {round}: race.json is one save's, whole");
    }
    let mut names: Vec<String> =
        std::fs::read_dir(&areas).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["race.json", "race.json.bak"], "no temporary files left");
}

#[test]
fn a_new_area_never_replaces_one_of_the_same_name_unless_asked() {
    let s = Server::start("new-area", &[], false);
    let areas = s.data().join("areas");
    let save = |q: &str, body: &[u8]| s.editor("PUT", &format!("/api/save?name=alpha{q}"), &[], body);
    let alpha = area_json("alpha", 10, Some("one"));
    assert_eq!(save("&new=1", &alpha).0, 200, "a new name is free");
    // The editor's New with the open area's name: refused, and nothing changes.
    let (st, b) = save("&new=1", &area_json("alpha", 20, Some("two")));
    assert_eq!(st, 409, "{b}");
    assert!(b.contains(r#""exists":"alpha""#) && b.contains("already an area called alpha"), "{b}");
    assert_eq!(std::fs::read(areas.join("alpha.json")).unwrap(), alpha);
    // A later autosave of that other area (its `created` differs) is refused too.
    assert_eq!(save("", &area_json("alpha", 30, Some("two"))).0, 409);
    assert_eq!(std::fs::read(areas.join("alpha.json")).unwrap(), alpha);
    // The same area's autosaves (same `created`, or none from an older editor) go through, the one before kept as .bak.
    let alpha2 = area_json("alpha", 11, Some("one"));
    assert_eq!(save("", &alpha2).0, 200);
    assert_eq!(save("", &area_json("alpha", 12, None)).0, 200);
    assert_eq!(std::fs::read(areas.join("alpha.json.bak")).unwrap(), alpha2);
    // Asked to replace: it does, and the replaced area is kept where autosaves don't reach.
    let two = area_json("alpha", 20, Some("two"));
    assert_eq!(save("&new=1&replace=1", &two).0, 200);
    assert_eq!(std::fs::read(areas.join("alpha.json")).unwrap(), two);
    let replaced: Vec<_> = std::fs::read_dir(&areas)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("alpha.json.replaced-"))
        .collect();
    assert_eq!(replaced.len(), 1);
    assert_eq!(std::fs::read(replaced[0].path()).unwrap(), area_json("alpha", 12, None));
    // An area that exists only as an export or an import counts too.
    let exports = s.data().join("exports");
    std::fs::create_dir_all(exports.join("beta.import")).unwrap();
    std::fs::write(exports.join("gamma.json"), "{}").unwrap();
    for n in ["beta", "gamma"] {
        let (st, b) = s.editor("PUT", &format!("/api/save?name={n}&new=1"), &[], &area_json(n, 1, None));
        assert_eq!(st, 409, "{n}: {b}");
        assert!(!areas.join(format!("{n}.json")).exists());
    }
    // (The listing shows the area once, none of its kept copies.)
    let (_, list) = s.editor("GET", "/api/areas", &[], b"");
    assert_eq!(list.matches(r#""name":"alpha""#).count(), 1, "{list}");
}

/// The most the server takes in a body that holds an area (serve.rs `body_max::AREA`).
const AREA_MAX: usize = 8 << 20;

#[test]
fn an_oversized_body_is_refused_with_413_and_nothing_is_written() {
    let s = Server::start("bodies", &[], false);
    let areas = s.data().join("areas");
    let big = vec![b' '; AREA_MAX + 1];
    for target in ["/api/save?name=big", "/api/export?name=big&tiles=3", "/api/check?name=big&tiles=3"] {
        let (st, b) = s.editor("POST", target, &[], &big);
        assert_eq!(st, 413, "{target}: {b}");
    }
    // Scripts, conversations and a blueprint's fields have smaller limits (and need no game to be refused).
    for (target, size) in [
        ("/api/script-save?resref=x", (1 << 20) + 1),
        ("/api/dialog-save?resref=x", (4 << 20) + 1),
        ("/api/new-blueprint-edit?ext=utc&resref=x", (256 << 10) + 1),
    ] {
        let (st, b) = s.editor("POST", target, &[], &vec![b' '; size]);
        assert_eq!(st, 413, "{target}: {b}");
    }
    // A body that doesn't say its length (chunked) is cut off at the limit too.
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let host = format!("127.0.0.1:{}", s.port);
    write!(
        c,
        "PUT /api/save?name=big HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nX-Landscaper: 1\r\nConnection: close\r\n\
         Transfer-Encoding: chunked\r\n\r\n"
    )
    .unwrap();
    let chunk = vec![b' '; 1 << 20];
    for _ in 0..(AREA_MAX >> 20) + 1 {
        if write!(c, "{:x}\r\n", chunk.len()).and_then(|()| c.write_all(&chunk)).and_then(|()| c.write_all(b"\r\n")).is_err() {
            break; // the server answered and closed
        }
    }
    let _ = c.write_all(b"0\r\n\r\n");
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out);
    // (On Windows the server's close with the rest of the body unread resets the connection, and the reset throws the 413
    // away before it is read: refused all the same, and below, nothing written.)
    assert!(text.starts_with("HTTP/1.1 413") || (cfg!(windows) && out.is_empty()), "{}", text.lines().next().unwrap_or(""));
    // Nothing of "big" anywhere.
    let mut files = Vec::new();
    tree(&s.root, &mut files);
    assert!(!files.iter().any(|p| p.to_string_lossy().contains("big")), "{files:?}");
    assert!(!areas.join("big.json").exists());
    // A body under the limit still gets to the route (which then finds it isn't an area).
    let (st, b) = s.editor("PUT", "/api/save?name=big", &[], b"{}");
    assert_eq!(st, 400, "{b}");
}

/// Starts a server (a team server when `team`) that should refuse to start; what it said.
fn refuses_to_start(team: bool, env: &[(&str, &str)], args: &[&str]) -> String {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serve_guard-nostart-{}", std::process::id()));
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/index.html"), "<!doctype html><title>t</title>").unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_landscaper"));
    cmd.args(["serve", "--port", &port.to_string(), "--web"])
        .arg(root.join("web"))
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .envs(windows_basics())
        .env("HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("NWN_ROOT", root.join("nogame"))
        .env("NWN_HOME", root.join("nogame"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if team {
        cmd.arg("--team");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            panic!("the server started with {env:?} {args:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut err = String::new();
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(!child.wait().unwrap().success());
    err
}

#[test]
fn a_team_server_needs_a_proxy_secret() {
    for (env, why) in [
        (vec![], "--insecure"),
        (vec![("LANDSCAPER_PROXY_SECRET", "change-me")], "change-me"),
        (vec![("LANDSCAPER_PROXY_SECRET", "short-secret")], "at least 16"),
        // --insecure allows no secret, never a weak one.
        (vec![("LANDSCAPER_PROXY_SECRET", "short-secret"), ("LANDSCAPER_INSECURE", "1")], "at least 16"),
    ] {
        let said = refuses_to_start(true, &env, &[]);
        assert!(said.contains("LANDSCAPER_PROXY_SECRET") && said.contains(why), "{env:?}: {said}");
    }
    // With a good secret: a request must carry it.
    // (Made at run time: a literal one this long reads as a leaked key to the secret scanners.)
    let secret = "s".repeat(20);
    let secret = secret.as_str();
    let wrong = format!("{}X", &secret[1..]);
    let env = [("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_PROXY_SECRET", secret)];
    let s = Server::start("secret", &env, true);
    let me = |proxy: Option<&str>| {
        let host = format!("127.0.0.1:{}", s.port);
        let mut h = vec![("Host", host.as_str()), ("X-Landscaper", "1"), ("X-Forwarded-User", "bob")];
        h.extend(proxy.map(|p| ("X-Landscaper-Proxy", p)));
        s.raw("GET", "/api/me", &h, b"").0
    };
    assert_eq!(me(None), 403);
    assert_eq!(me(Some(&wrong)), 403);
    assert_eq!(me(Some(secret)), 200);
    // And --insecure (the flag, not only the variable) starts one without a secret.
    let s = Server::start_with("insecure", &[("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*")], true, &["--insecure"]);
    let host = format!("127.0.0.1:{}", s.port);
    assert_eq!(s.raw("GET", "/api/me", &[("Host", &host), ("X-Landscaper", "1"), ("X-Forwarded-User", "bob")], b"").0, 200);
}

#[test]
fn without_team_only_this_machine_may_reach_the_server_unless_allowed() {
    for bind in ["0.0.0.0", "::", "192.0.2.1"] {
        let said = refuses_to_start(false, &[], &["--bind", bind]);
        assert!(said.contains("--team") && said.contains("--allow-remote"), "{bind}: {said}");
    }
    // Allowed: it starts, and says what that means.
    let s = Server::start_with("allow-remote", &[], false, &["--bind", "0.0.0.0", "--allow-remote"]);
    assert_eq!(s.editor("GET", "/api/me", &[], b"").0, 200);
}

/// The refusal lines in a server's activity log.
fn refusals(s: &Server) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(s.data().join("audit")).into_iter().flatten().flatten() {
        for line in std::fs::read_to_string(e.path()).unwrap().lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v["action"] == "refused" {
                out.push(v);
            }
        }
    }
    out
}

#[test]
fn a_request_that_didnt_come_through_the_proxy_chooses_neither_its_address_nor_its_name_in_the_log() {
    let secret = "s".repeat(20);
    let env = [("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "builders"), ("LANDSCAPER_PROXY_SECRET", secret.as_str())];
    let s = Server::start("forwarded", &env, true);
    let host = format!("127.0.0.1:{}", s.port);
    // The review's reproduction: 300 requests without the secret, each with a forged address (40 here).
    for k in 0..40 {
        let (forged, user) = (format!("198.51.100.{k}"), format!("mallory{k}"));
        let h = [("Host", host.as_str()), ("X-Landscaper", "1"), ("X-Forwarded-For", &forged), ("X-Forwarded-User", &user)];
        assert_eq!(s.raw("GET", "/api/me", &h, b"").0, 403);
    }
    let lines = refusals(&s);
    assert_eq!(lines.len(), 1, "one line a minute from the real address: {lines:?}");
    assert_eq!((lines[0]["ip"].as_str(), lines[0]["user"].is_null()), (Some("127.0.0.1"), true), "{lines:?}");
    // Through the proxy (it has the secret), its address and user are the proxy's word: recorded.
    let h = [
        ("Host", host.as_str()),
        ("X-Landscaper", "1"),
        ("X-Landscaper-Proxy", secret.as_str()),
        ("X-Forwarded-For", "203.0.113.7"),
        ("X-Forwarded-User", "carol"),
    ];
    assert_eq!(s.raw("GET", "/api/me", &h, b"").0, 403, "carol isn't in the builders group");
    let lines = refusals(&s);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!((lines[1]["ip"].as_str(), lines[1]["user"].as_str()), (Some("203.0.113.7"), Some("carol")), "{lines:?}");
}

/// The `X-Landscaper-Version` an answer's headers (lower case, see [`Server::exchange`]) carry.
fn version_in(head: &str) -> String {
    head.lines().find_map(|l| l.strip_prefix("x-landscaper-version: ")).expect("no version").trim().to_string()
}

/// The `version` of a save's JSON answer.
fn saved_version(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body).unwrap()["version"].as_str().expect("no version").to_string()
}

#[test]
fn two_tabs_on_one_area_no_longer_save_over_each_other() {
    let s = Server::start("stale-save", &[], false);
    let areas = s.data().join("areas");
    std::fs::create_dir_all(&areas).unwrap();
    // An area saved before the editor gave areas a `created` id.
    std::fs::write(areas.join("old.json"), area_json("old", 5, None)).unwrap();
    let host = format!("127.0.0.1:{}", s.port);
    let origin = format!("http://{host}");
    let editor = [("Host", host.as_str()), ("X-Landscaper", "1"), ("Origin", origin.as_str())];
    let open = || {
        let (st, head, b) = s.exchange("GET", "/api/area?name=old", &editor, b"");
        assert_eq!(st, 200, "{b}");
        version_in(&head)
    };
    let save = |q: &str, body: &[u8]| s.editor("PUT", &format!("/api/save?name=old{q}"), &[], body);
    // Two tabs open it.
    let (first, second) = (open(), open());
    assert_eq!(first, second);
    // The first tab autosaves fast, many times in a row (within one second): it never refuses itself.
    let mut base = first.clone();
    for k in 0..20 {
        let (st, b) = save(&format!("&base={base}"), &area_json("old", 10 + k, None));
        assert_eq!(st, 200, "save {k}: {b}");
        base = saved_version(&b);
    }
    assert_eq!(open(), base, "the version a save answers is the one an open gives");
    // The second tab's save started from the copy before all that: refused, with what the editor shows, and nothing written.
    let mine = std::fs::read(areas.join("old.json")).unwrap();
    let (st, b) = save(&format!("&base={second}"), &area_json("old", 99, None));
    assert_eq!(st, 409, "{b}");
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert!(v["moved"] == true && v["exists"] == "old" && v["saved"].as_u64().is_some() && v["version"] == base.as_str(), "{b}");
    assert!(v["error"].as_str().unwrap().contains("saved from another tab or computer"), "{b}");
    assert_eq!(std::fs::read(areas.join("old.json")).unwrap(), mine);
    // Asked to keep its own: it saves, and the copy it replaced is kept where autosaves don't reach.
    assert_eq!(save(&format!("&base={second}&replace=1"), &area_json("old", 99, None)).0, 200);
    let replaced = std::fs::read_dir(&areas).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("old.json.replaced-"));
    assert_eq!(std::fs::read(replaced.expect("the replaced copy is kept").path()).unwrap(), mine);
    // A save with no base (an older editor) isn't checked.
    assert_eq!(save("", &area_json("old", 7, None)).0, 200);
    // An area with only an export: no working copy yet. Two tabs open it, the first saves, the second is refused.
    let exports = s.data().join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    std::fs::write(exports.join("ex.json"), r#"{"name":"ex","tiles":3}"#).unwrap();
    std::fs::write(exports.join("ex.f32"), [0u8; 16]).unwrap();
    let (st, head, b) = s.exchange("GET", "/api/area?name=ex", &editor, b"");
    assert_eq!(st, 200, "{b}");
    let from_export = version_in(&head);
    let put = |body: &[u8]| s.editor("PUT", &format!("/api/save?name=ex&base={from_export}"), &[], body);
    assert_eq!(put(&area_json("ex", 1, None)).0, 200);
    assert_eq!(put(&area_json("ex", 2, None)).0, 409, "the first tab's save made a working copy since");
}

#[test]
fn the_read_only_view_refuses_every_write_whatever_sends_it() {
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_ADMIN_USERS", "alice"),
        ("LANDSCAPER_ADMIN_GROUPS", "none"),
        ("LANDSCAPER_INSECURE", "1"),
    ];
    let s = Server::start("view-as", &env, true);
    let as_ =
        |user: &'static str| [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", user), ("Cookie", "ls_as=bob")];
    std::fs::create_dir_all(s.data().join("workspaces/bob/areas")).unwrap();
    std::fs::write(s.data().join("workspaces/bob/areas/keep.json"), area_json("keep", 1, None)).unwrap();
    // alice (an admin) looks at bob's workspace: reads go, and so do going back and checking paths.
    let alice = as_("alice");
    assert_eq!(s.raw("GET", "/api/areas", &alice, b"").0, 200);
    assert_ne!(s.raw("POST", "/api/check?name=x&tiles=3", &alice, b"").0, 403);
    // Anything else, sent by any client (the editor's own guard aside), is refused, and nothing is written anywhere.
    for (method, target) in [
        ("PUT", "/api/save?name=town&new=1"),
        ("POST", "/api/save?name=town"),
        ("POST", "/api/export?name=town&tiles=3"),
        ("POST", "/api/assignments?user=erin&area=*"),
        ("DELETE", "/api/script?name=my_spawn"),
        ("DELETE", "/api/area?name=keep&base=none"),
    ] {
        let (st, b) = s.raw(method, target, &alice, &area_json("town", 1, None));
        assert_eq!(st, 403, "{method} {target}: {b}");
        assert!(b.contains("bob's workspace is read-only here"), "{b}");
    }
    let mut written = Vec::new();
    tree(&s.data().join("workspaces"), &mut written);
    assert!(written.iter().all(|p| !p.to_string_lossy().contains("town")), "{written:?}");
    assert!(!s.data().join("assignments.json").exists());
    assert!(s.data().join("workspaces/bob/areas/keep.json").exists(), "bob's area isn't deleted");
    assert_eq!(s.raw("POST", "/api/view-as?user=", &alice, b"").0, 200, "back to her own");
    // The cookie means nothing to a builder who isn't an admin: carol saves into her own workspace.
    let (st, b) = s.raw("PUT", "/api/save?name=town&new=1", &as_("carol"), &area_json("town", 1, None));
    assert_eq!(st, 200, "{b}");
    assert!(s.data().join("workspaces/carol/areas/town.json").exists());
}

#[test]
fn the_review_services_area_list_outlives_a_restart_while_it_is_down() {
    let review = Review::start(r#"{"areas":[{"area":"chess","tag":"CHESS"}]}"#, Duration::ZERO);
    let env = |url| {
        [
            ("LANDSCAPER_AUTH", "headers"),
            ("LANDSCAPER_ALLOWED_GROUPS", "*"),
            ("LANDSCAPER_INSECURE", "1"),
            ("LANDSCAPER_REVIEW_URL", url),
            ("LANDSCAPER_REVIEW_KEY", "k"),
        ]
    };
    let s = Server::start("review-restart", &env(review.url.as_str()), true);
    let ivy = [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", "ivy")];
    let save = |s: &Server, name: &str| s.raw("PUT", &format!("/api/save?name={name}&new=1"), &ivy, &area_json(name, 1, None));
    assert_eq!(save(&s, "glade").0, 200, "the list came, and glade isn't in it");
    assert!(s.data().join("review-areas.json").exists(), "the list is kept on disk");
    // The server restarts while the service is down (it answers 503).
    let down = Review::start("", Duration::ZERO);
    let s = s.restart(&env(down.url.as_str()), true);
    // Before: every new name was refused ("the module's area list isn't available") until the service came back.
    let (st, b) = save(&s, "meadow");
    assert_eq!(st, 200, "{b}");
    let (st, b) = save(&s, "chess");
    assert_eq!(st, 400, "{b}");
    assert!(b.contains("chess is the name of an existing area"), "{b}");
    let (st, b) = s.raw("GET", "/api/import-sources?q=chess", &ivy, b"");
    assert_eq!(st, 200, "{b}");
    assert!(b.contains(r#""area":"chess""#), "{b}");
    // The failure memory still holds: one call to the service that is down, not one a request.
    assert_eq!(down.fetches(), 1);
}

#[test]
fn the_export_takes_its_lists_in_its_body_and_refuses_them_in_the_query_string() {
    let s = Server::start("export-body", &[], false);
    let exports = s.data().join("exports");
    let heights = vec![0u8; 49 * 49 * 4];
    // An editor older than this engine: each list in the query string is refused, by name, before anything is written.
    for (key, value) in [("palette", "grassMedium"), ("water", "[]"), ("entrances", "[]"), ("atmosphere", "{}")] {
        let (st, b) = s.editor("POST", &format!("/api/export?name=old&tiles=3&{key}={}", enc(value)), &[], &heights);
        assert_eq!(st, 400, "{key}: {b}");
        assert!(b.contains(&format!("export's {key} goes in the request's JSON body")) && b.contains("reload the page"), "{key}: {b}");
        let (st, b) = s.editor("POST", &format!("/api/check?name=old&tiles=3&{key}={}", enc(value)), &[], &heights);
        assert_eq!(st, 400, "check {key}: {b}");
    }
    let mut written = Vec::new();
    tree(&exports, &mut written);
    assert!(written.iter().all(|p| !p.to_string_lossy().contains("old")), "{written:?}");
    // In the body, they are read (here the check gets as far as the game files, which this test has none of).
    let json = serde_json::json!({
        "palette": ["grassMedium"], "water": [], "entrances": [], "atmosphere": { "sunFog": "#ff0000" }, "objects": [],
    });
    let (st, b) = s.editor("POST", "/api/check?name=new&tiles=3", &[], &export_body(&heights, &json));
    assert!(!b.contains("JSON body"), "{st}: {b}");
    let header: serde_json::Value = serde_json::from_slice(&std::fs::read(exports.join(".check/new.json")).unwrap()).unwrap();
    assert_eq!((&header["palette"], &header["atmosphere"]["sunFog"]), (&json["palette"], &json["atmosphere"]["sunFog"]));
    // A palette that isn't a list of texture ids is refused.
    let (st, b) = s.editor("POST", "/api/check?name=new&tiles=3", &[], &export_body(&heights, &serde_json::json!({ "palette": "a,b" })));
    assert_eq!(st, 400, "{b}");
    assert!(b.contains("palette"), "{b}");
}

/// The names in a folder, sorted (none when it isn't there).
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn deleting_an_area_sets_it_aside_removes_its_export_and_frees_its_name() {
    let s = Server::start("delete", &[], false);
    let (areas, exports, deleted) = (s.data().join("areas"), s.data().join("exports"), s.data().join("deleted"));
    let save = |name: &str, q: &str, body: &[u8]| {
        let (st, b) = s.editor("PUT", &format!("/api/save?name={name}{q}"), &[], body);
        assert_eq!(st, 200, "{name}{q}: {b}");
        saved_version(&b)
    };
    let delete = |name: &str, base: &str| s.editor("DELETE", &format!("/api/area?name={name}&base={base}"), &[], b"");
    // `town`, saved twice, exported and checked, imported (its kept original and picture).
    let first = save("town", "&new=1", &area_json("town", 5, Some("t1")));
    let last = save("town", &format!("&base={first}"), &area_json("town", 6, Some("t1")));
    let town = std::fs::read(areas.join("town.json")).unwrap();
    for dir in ["town.import", "town_nwn", ".check/town_nwn"] {
        std::fs::create_dir_all(exports.join(dir)).unwrap();
    }
    for (f, d) in [
        ("town.json", &br#"{"name":"town","tiles":3,"imported":true}"#[..]),
        ("town.f32", b"h"),
        ("town.materials", b"m"),
        ("town.import/source.json", br#"{"area":""}"#),
        ("town.import/area.are", b"ARE V3.2"),
        ("town.import.view.png", b"png"),
        ("town_nwn/town.mod", b"mod"),
        (".check/town.json", b"{}"),
        ("other.json", br#"{"name":"other","tiles":3}"#),
    ] {
        std::fs::write(exports.join(f), d).unwrap();
    }
    // Another area that paints with town's import.
    let copy = save(
        "copy",
        "&new=1",
        &serde_json::json!({ "name": "copy", "tiles": 3, "heights": "", "palette": ["@import:town"] }).to_string().into_bytes(),
    );
    let before = names_in(&exports);
    // Refused: without a version; with the one before the last save (another tab saved since); while `copy` uses its import.
    let (st, b) = delete("town", "");
    assert_eq!(st, 400, "{b}");
    let (st, b) = s.editor("DELETE", "/api/area?name=town", &[], b"");
    assert!(st == 400 && b.contains("base missing"), "{st}: {b}");
    let (st, b) = delete("town", &first);
    assert_eq!(st, 409, "{b}");
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert!(v["moved"] == true && v["version"] == last.as_str() && v["saved"].as_u64().is_some(), "{b}");
    let (st, b) = delete("town", &last);
    assert_eq!(st, 409, "{b}");
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["used_by"], serde_json::json!(["copy"]), "{b}");
    assert!(v["error"].as_str().unwrap().contains("delete it first"), "{b}");
    assert_eq!(std::fs::read(areas.join("town.json")).unwrap(), town, "nothing was deleted");
    assert_eq!(names_in(&exports), before);
    assert!(!deleted.exists() || names_in(&deleted).is_empty());
    // `copy` goes first, then town.
    let (st, b) = delete("copy", &copy);
    assert_eq!(st, 200, "{b}");
    let (st, b) = delete("town", &last);
    assert_eq!(st, 200, "{b}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&b).unwrap()["deleted"], "town");
    // Set aside: the working copy as it was, and the import with its picture.
    let aside = names_in(&deleted);
    assert_eq!(aside.len(), 2, "{aside:?}");
    let town_aside = deleted.join(aside.iter().find(|n| n.starts_with("town-")).unwrap());
    assert_eq!(std::fs::read(town_aside.join("areas/town.json")).unwrap(), town);
    assert_eq!(names_in(&town_aside.join("exports")), ["town.import", "town.import.view.png"]);
    assert_eq!(std::fs::read(town_aside.join("exports/town.import/area.are")).unwrap(), b"ARE V3.2");
    // Gone: the copy before the last save, the export, its build folder and the Check's copy. Other areas stay.
    assert_eq!(names_in(&areas), Vec::<String>::new());
    assert_eq!(names_in(&exports), [".check", "other.json"]);
    assert_eq!(names_in(&exports.join(".check")), Vec::<String>::new());
    let (_, list) = s.editor("GET", "/api/areas", &[], b"");
    assert!(!list.contains("\"town\"") && list.contains("\"other\""), "{list}");
    assert_eq!(s.editor("GET", "/api/area?name=town", &[], b"").0, 404);
    let (st, b) = delete("town", "none");
    assert!(st == 404 && b.contains("no area called town"), "{st}: {b}");
    // The name is free again, and on your own machine every deleted copy stays.
    for k in 0..3 {
        let v = save("town", "&new=1", &area_json("town", 7 + k, None));
        assert_eq!(delete("town", &v).0, 200);
    }
    assert_eq!(names_in(&deleted).iter().filter(|n| n.starts_with("town-")).count(), 4);
}

#[test]
fn an_underlay_is_kept_beside_its_area_set_aside_with_it_and_not_taken_over_by_a_new_area_of_its_name() {
    let s = Server::start("underlay", &[], false);
    let areas = s.data().join("areas");
    let png = b"\x89PNG\r\n\x1a\n picture".to_vec();
    // Refused: an area that isn't saved, and anything that isn't a picture (a page, whatever type it claims).
    let (st, b) = s.editor("PUT", "/api/underlay?name=town", &[], &png);
    assert!(st == 404 && b.contains("save it first"), "{st}: {b}");
    let (st, b) = s.editor("PUT", "/api/save?name=town&new=1", &[], &area_json("town", 5, Some("t1")));
    assert_eq!(st, 200, "{b}");
    let (st, b) = s.editor("PUT", "/api/underlay?name=town", &[("Content-Type", "image/png")], b"<html><script>alert(1)</script>");
    assert_eq!(st, 415, "{b}");
    assert_eq!(s.editor("GET", "/api/underlay?name=town", &[], b"").0, 404);
    // Kept, and answered as the type its bytes are.
    assert_eq!(s.editor("PUT", "/api/underlay?name=town", &[], &png).0, 200);
    let host = format!("127.0.0.1:{}", s.port);
    let (st, head, body) = s.exchange("GET", "/api/underlay?name=town", &[("Host", &host), ("X-Landscaper", "1")], b"");
    // (`exchange` reads the body as text, so only its ASCII end is compared; the bytes kept are compared below.)
    let length = format!("content-length: {}", png.len());
    assert!(st == 200 && head.contains("content-type: image/png") && head.contains(&length) && body.ends_with(" picture"), "{st} {head}");
    assert_eq!(std::fs::read(areas.join("town.underlay")).unwrap(), png);
    // Removed, and put back.
    assert_eq!(s.editor("DELETE", "/api/underlay?name=town", &[], b"").0, 200);
    assert!(!areas.join("town.underlay").exists());
    assert_eq!(s.editor("PUT", "/api/underlay?name=town", &[], &png).0, 200);
    // Saving the area again keeps it; a new area that replaces town doesn't take it over.
    assert_eq!(s.editor("PUT", "/api/save?name=town", &[], &area_json("town", 6, Some("t1"))).0, 200);
    assert!(areas.join("town.underlay").exists());
    let (st, b) = s.editor("PUT", "/api/save?name=town&new=1&replace=1", &[], &area_json("town", 5, Some("t2")));
    assert_eq!(st, 200, "{b}");
    assert!(!areas.join("town.underlay").exists());
    // Deleting an area sets its underlay aside with its working copy.
    assert_eq!(s.editor("PUT", "/api/underlay?name=town", &[], &png).0, 200);
    let (_, b) = s.editor("PUT", "/api/save?name=town", &[], &area_json("town", 7, Some("t2")));
    let version = saved_version(&b);
    let (st, b) = s.editor("DELETE", &format!("/api/area?name=town&base={version}"), &[], b"");
    assert_eq!(st, 200, "{b}");
    let deleted = s.data().join("deleted");
    let aside = deleted.join(&names_in(&deleted)[0]);
    assert_eq!(names_in(&aside.join("areas")), ["town.json", "town.underlay"]);
    assert!(!areas.join("town.underlay").exists());
}

#[test]
fn a_builder_deletes_only_their_own_areas_and_only_while_they_may_work_on_them() {
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_ADMIN_USERS", "alice"),
        ("LANDSCAPER_ADMIN_GROUPS", "none"),
        ("LANDSCAPER_INSECURE", "1"),
    ];
    let s = Server::start("delete-team", &env, true);
    let who = |user: &'static str| [("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", user)];
    let bob = who("bob");
    let ws = s.data().join("workspaces/bob");
    let save = |name: &str, q: &str| {
        let (st, b) = s.raw("PUT", &format!("/api/save?name={name}{q}"), &bob, &area_json(name, 1, None));
        assert_eq!(st, 200, "{name}: {b}");
        saved_version(&b)
    };
    let town = save("town", "&new=1");
    // alice can't delete it: not from her own workspace (she has no `town`), nor while viewing bob's (read-only).
    let target = format!("/api/area?name=town&base={town}");
    assert_eq!(s.raw("DELETE", &target, &who("alice"), b"").0, 404);
    let mut viewing = who("alice").to_vec();
    viewing.push(("Cookie", "ls_as=bob"));
    assert_eq!(s.raw("DELETE", &target, &viewing, b"").0, 403);
    assert!(ws.join("areas/town.json").exists());
    // An import of a production area whose assignment has ended (bob holds none): refused, nothing moved.
    std::fs::create_dir_all(ws.join("exports/chs.import")).unwrap();
    std::fs::write(ws.join("exports/chs.import/source.json"), r#"{"area":"chess"}"#).unwrap();
    std::fs::write(ws.join("exports/chs.json"), r#"{"name":"chs","tiles":3,"imported":true}"#).unwrap();
    let (st, b) = s.raw("DELETE", "/api/area?name=chs&base=none", &bob, b"");
    assert_eq!(st, 403, "{b}");
    assert!(b.contains("assignment of chess has ended"), "{b}");
    assert!(ws.join("exports/chs.import/source.json").exists() && ws.join("exports/chs.json").exists());
    // His own: deleted, four times over; the newest 3 deleted copies stay.
    let mut base = town;
    for k in 0..4 {
        let (st, b) = s.raw("DELETE", &format!("/api/area?name=town&base={base}"), &bob, b"");
        assert_eq!(st, 200, "{k}: {b}");
        if k < 3 {
            base = save("town", "&new=1");
        }
    }
    let kept: Vec<String> = names_in(&ws.join("deleted"));
    assert_eq!(kept.len(), 3, "{kept:?}");
    assert!(kept.iter().all(|k| k.starts_with("town-")), "{kept:?}");
    // A full workspace (100 areas, with chs) makes room by deleting one.
    let versions: Vec<String> = (1..100).map(|i| save(&format!("a{i}"), "&new=1")).collect();
    let (st, b) = s.raw("PUT", "/api/save?name=fresh&new=1", &bob, &area_json("fresh", 1, None));
    assert_eq!(st, 413, "{b}");
    assert_eq!(s.raw("DELETE", &format!("/api/area?name=a1&base={}", versions[0]), &bob, b"").0, 200);
    let (st, b) = s.raw("PUT", "/api/save?name=fresh&new=1", &bob, &area_json("fresh", 1, None));
    assert_eq!(st, 200, "{b}");
}

/// A pack of one texture (`tpMoss`) and the index listing it, made by `landscaper textures pack` in `dir`; the index's path.
fn sample_pack(dir: &Path) -> PathBuf {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let file = std::io::BufWriter::new(std::fs::File::create(dir.join("moss.png")).unwrap());
    let mut e = png::Encoder::new(file, 64, 64);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&[90u8; 64 * 64 * 3]).unwrap();
    std::fs::write(
        dir.join("list.json"),
        r#"{ "id": "test-pack", "version": "1", "name": "Test pack", "licence": "CC0-1.0", "prefix": "tp",
             "textures": [{ "id": "tpMoss", "name": "Moss", "category": "Grass", "picture": "moss.png" }] }"#,
    )
    .unwrap();
    let index = dir.join("out/index.json");
    let made = Command::new(env!("CARGO_BIN_EXE_landscaper"))
        .args(["textures", "pack"])
        .arg(dir.join("list.json"))
        .arg("--out")
        .arg(dir.join("out"))
        .arg("--index")
        .arg(&index)
        .env("XDG_DATA_HOME", dir.join("xdg"))
        .output()
        .unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    index
}

#[test]
fn on_a_team_server_only_admins_install_or_remove_texture_packs() {
    let src = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serve_guard-packsrc-{}", std::process::id()));
    let index = sample_pack(&src);
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_ADMIN_USERS", "alice"),
        ("LANDSCAPER_ADMIN_GROUPS", "none"),
        ("LANDSCAPER_PUBLIC_HOST", "landscaper.test"),
        ("LANDSCAPER_INSECURE", "1"),
        ("LANDSCAPER_PACKS_URL", index.to_str().unwrap()),
    ];
    let s = Server::start("packs", &env, true);
    let as_user = |user: &str, method: &str, target: &str| {
        s.raw(method, target, &[("Host", "landscaper.test"), ("X-Landscaper", "1"), ("X-Forwarded-User", user)], b"")
    };
    let catalog = || std::fs::read_to_string(s.data().join("textures/catalog.json")).unwrap();
    // A builder sees what is on offer (not where it downloads from), and may change nothing.
    let (st, b) = as_user("bob", "GET", "/api/textures/packs");
    assert_eq!(st, 200, "{b}");
    assert!(b.contains(r#""manage":false"#) && b.contains(r#""id":"test-pack""#) && !b.contains(r#""url""#), "{b}");
    for route in ["/api/textures/packs/install?id=test-pack", "/api/textures/packs/remove?id=test-pack"] {
        let (st, b) = as_user("bob", "POST", route);
        assert_eq!(st, 403, "{route}: {b}");
    }
    assert!(!catalog().contains("tpMoss"));
    // The admin installs it, in the background, and the job says when it is done.
    let (st, b) = as_user("alice", "POST", "/api/textures/packs/install?id=test-pack");
    assert_eq!(st, 202, "{b}");
    let started = Instant::now();
    let job = loop {
        let (_, b) = as_user("alice", "GET", "/api/textures/packs/job");
        if b.contains(r#""running":false"#) {
            break b;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "the install didn't end: {b}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(job.contains(r#""step":"done""#) && job.contains(r#""name":"Test pack""#), "{job}");
    assert!(catalog().contains("tpMoss") && s.data().join("textures/tpMoss_128.png").is_file());
    // A pack the index doesn't list can't be asked for.
    assert_eq!(as_user("alice", "POST", "/api/textures/packs/install?id=..%2F..%2Fetc").0, 400);
    assert_eq!(as_user("alice", "POST", "/api/textures/packs/install?id=not-listed").0, 404);
    // Still refused to the builder; removed by the admin.
    assert_eq!(as_user("bob", "POST", "/api/textures/packs/remove?id=test-pack").0, 403);
    let (st, b) = as_user("alice", "POST", "/api/textures/packs/remove?id=test-pack");
    assert_eq!(st, 200, "{b}");
    assert!(!catalog().contains("tpMoss") && !s.data().join("textures/tpMoss.png").exists());
    let _ = std::fs::remove_dir_all(&src);
}
