// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Canvas storage (docs/DESIGN-canvas-cut.md, step 3) from the outside: a real `landscaper serve` on a scratch data folder.
//!
//! - A 128 by 128 tile canvas: every block saved in one request, loaded back byte for byte, and its size on disk.
//! - Saves at once from many tabs: of different blocks, all kept; of one block from one version, one kept and the others told.
//! - A tab whose versions are stale: its save and its delete are refused with the versions there now, and nothing changes.
//! - A damaged or unreadable block or header: named, never read as flat ground, never written over.
//! - A team: a builder without the assignment of an area the canvas is built from can't save it; an admin reads it only.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Value, json};

/// What a Windows program can't start without, kept through `env_clear` (as tests/serve_guard.rs).
fn windows_basics() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let wanted = ["SystemRoot", "SystemDrive", "TEMP", "TMP"];
    std::env::vars_os().filter(|(k, _)| cfg!(windows) && wanted.iter().any(|w| k.eq_ignore_ascii_case(w))).collect()
}

/// A server on a scratch folder; stopped, and the folder removed, when dropped.
struct Server {
    child: Child,
    port: u16,
    root: PathBuf,
    /// The headers every request carries: the editor's, and on a team server who is asking.
    host: String,
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
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("canvas_storage-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("xdg/landscaper");
        std::fs::create_dir_all(data.join("textures")).unwrap();
        std::fs::write(data.join("textures/catalog.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::write(root.join("web/index.html"), "<!doctype html><title>t</title>").unwrap();
        for d in ["home", "nogame"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_landscaper"));
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
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("start landscaper serve");
        let port = printed_port(child.stdout.take().unwrap());
        let started = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(started.elapsed() < Duration::from_secs(60), "the server didn't start");
            std::thread::sleep(Duration::from_millis(50));
        }
        let host = if team { "landscaper.test".to_string() } else { format!("127.0.0.1:{port}") };
        Server { child, port, root, host }
    }

    fn data(&self) -> PathBuf {
        self.root.join("xdg/landscaper")
    }

    /// Sends a request as the editor does (as `user` on a team server, else on your own machine): status, headers in lower
    /// case, body.
    fn ask(&self, user: Option<&str>, method: &str, target: &str, extra: &[(&str, &str)], body: &[u8]) -> (u16, String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
        let mut req = format!(
            "{method} {target} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\nHost: {}\r\nX-Landscaper: 1\r\n",
            body.len(),
            self.host
        );
        if let Some(u) = user {
            req.push_str(&format!("X-Forwarded-User: {u}\r\n"));
        } else {
            req.push_str(&format!("Origin: http://{}\r\n", self.host));
        }
        for (k, v) in extra {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).unwrap();
        s.write_all(body).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").map_or(out.len(), |k| k + 4);
        let head = String::from_utf8_lossy(&out[..split]).to_ascii_lowercase();
        let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = if head.contains("transfer-encoding: chunked") { unchunk(&out[split..]) } else { out[split..].to_vec() };
        (status, head, body)
    }

    /// [`Server::ask`] with a JSON answer.
    fn json(&self, user: Option<&str>, method: &str, target: &str, body: &[u8]) -> (u16, Value) {
        let (st, _, b) = self.ask(user, method, target, &[], body);
        let v = serde_json::from_slice(&b).unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&b) }));
        (st, v)
    }
}

/// A body sent in chunks (a large answer), put back together.
fn unchunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let line = raw.windows(2).position(|w| w == b"\r\n").expect("a chunk's length");
        let n = usize::from_str_radix(String::from_utf8_lossy(&raw[..line]).split(';').next().unwrap().trim(), 16).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&raw[line + 2..line + 2 + n]);
        raw = &raw[line + 2 + n + 2..];
    }
}

/// The port a server started with `--port 0` says it listens on (as tests/serve_guard.rs).
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

const GROUND: f32 = 2.5;

fn header(name: &str, tiles: u64) -> Value {
    json!({ "name": name, "tiles": tiles, "tilesY": tiles, "ground": GROUND, "palette": ["grassMedium", "dirt"], "created": "t1" })
}

/// Block (bx, by)'s size in samples on a canvas `tiles` a side.
fn samples(tiles: usize, bx: usize, by: usize) -> (usize, usize) {
    let side = |k: usize| (tiles - k * 16).min(16) * 16 + 1;
    (side(bx), side(by))
}

/// A block of a save: ground `f(i, j)` on canvas samples (so neighbours agree on their shared rows), paint 1 where it is above
/// 5 m, a chasm where `pit` says.
fn block(tiles: usize, bx: usize, by: usize, base: &str, f: &dyn Fn(usize, usize) -> f32, pit: &dyn Fn(usize, usize) -> bool) -> Value {
    let (w, h) = samples(tiles, bx, by);
    let (x0, y0) = (bx * 256, by * 256);
    let mut heights = Vec::with_capacity(w * h * 4);
    let (mut paint, mut chasm) = (Vec::with_capacity(w * h), Vec::with_capacity(w * h));
    for j in 0..h {
        for i in 0..w {
            let z = f(x0 + i, y0 + j);
            heights.extend_from_slice(&z.to_le_bytes());
            paint.push(u8::from(z > 5.0));
            chasm.push(u8::from(pit(x0 + i, y0 + j)));
        }
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut v = json!({ "block": format!("b{bx:02}_{by:02}"), "base": base, "heights": b64.encode(heights), "paint": b64.encode(paint) });
    if chasm.contains(&1) {
        v["chasm"] = b64.encode(chasm).into();
    }
    v
}

fn flat(_: usize, _: usize) -> f32 {
    GROUND
}

fn no_pit(_: usize, _: usize) -> bool {
    false
}

/// Rolling ground over the whole canvas, never flat at a block's edge.
fn rolling(i: usize, j: usize) -> f32 {
    GROUND + 4.0 * ((i as f32) * 0.013).sin() * ((j as f32) * 0.009).cos() + (i + j) as f32 * 0.002
}

/// The files of a canvas folder and their sizes, sorted.
fn files(dir: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                out.push((e.path().strip_prefix(dir).unwrap().display().to_string(), e.metadata().unwrap().len()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_128_by_128_canvas_saves_every_block_at_once_and_reads_back_whole() {
    let s = Server::start("big", &[], false);
    let (st, made) = s.json(None, "POST", "/api/canvas-new?name=vast", header("vast", 128).to_string().as_bytes());
    assert_eq!(st, 200, "{made}");
    assert_eq!(made["blocks"], json!({}));
    let dir = s.data().join("canvases/vast");
    let fresh: u64 = files(&dir).iter().map(|f| f.1).sum();
    // A new canvas: its header alone. Every block is flat ground until saved.
    let (st, _, b) = s.ask(None, "GET", "/api/canvas-block?name=vast&block=b07_07", &[], b"");
    assert_eq!(st, 200);
    assert_eq!(b.len(), 12 + 257 * 257 * 5 + 8);
    assert_eq!(f32::from_le_bytes(b[12..16].try_into().unwrap()), GROUND);
    // Every block in one save (34 MB of base64 with the chasm masks), with a chasm across a block corner in the middle.
    let pit = |i: usize, j: usize| (i as f32 - 1024.0).hypot(j as f32 - 1024.0) < 40.0;
    let blocks: Vec<Value> =
        (0..8).flat_map(|by| (0..8).map(move |bx| (bx, by))).map(|(bx, by)| block(128, bx, by, "none", &rolling, &pit)).collect();
    let body = json!({ "blocks": blocks }).to_string();
    let t = Instant::now();
    let (st, saved) = s.json(None, "PUT", "/api/canvas-save?name=vast", body.as_bytes());
    let took = t.elapsed();
    assert_eq!(st, 200, "{saved}");
    assert_eq!(saved["blocks"].as_object().unwrap().len(), 64);
    assert_eq!(saved["version"], made["version"], "the header is unchanged");
    // Loaded back: the same versions, and each block's file says them.
    let (st, loaded) = s.json(None, "GET", "/api/canvas?name=vast", b"");
    assert_eq!(st, 200, "{loaded}");
    assert_eq!(loaded["blocks"], saved["blocks"]);
    assert_eq!(loaded["header"]["tiles"], 128);
    for key in ["b00_00", "b03_03", "b04_04", "b07_07"] {
        let (st, head, bytes) = s.ask(None, "GET", &format!("/api/canvas-block?name=vast&block={key}"), &[], b"");
        assert_eq!(st, 200, "{key}");
        let v = saved["blocks"][key].as_str().unwrap();
        assert!(head.contains(&format!("x-landscaper-version: {v}")), "{key}: {head}");
        assert_eq!(bytes, std::fs::read(dir.join(format!("blocks/{key}.blk"))).unwrap(), "{key}");
        // Heights as sent.
        let (bx, by) = (key[1..3].parse::<usize>().unwrap(), key[4..6].parse::<usize>().unwrap());
        let z = f32::from_le_bytes(bytes[12 + 4 * 300..12 + 4 * 301].try_into().unwrap());
        assert_eq!(z, rolling(bx * 256 + 300 % 257, by * 256 + 300 / 257), "{key}");
    }
    let listed = files(&dir);
    let painted: u64 = listed.iter().map(|f| f.1).sum();
    let with_chasm: Vec<&str> = listed.iter().filter(|f| f.1 == 12 + 257 * 257 * 6 + 8).map(|f| f.0.as_str()).collect();
    assert_eq!(with_chasm, ["blocks/b03_03.blk", "blocks/b03_04.blk", "blocks/b04_03.blk", "blocks/b04_04.blk"]);
    assert_eq!(listed.len(), 65);
    // One brush stroke inside one block: a save of that block alone.
    let bump = |i: usize, j: usize| rolling(i, j) + if (i as f32 - 600.0).hypot(j as f32 - 400.0) < 30.0 { 1.0 } else { 0.0 };
    let one = json!({ "blocks": [block(128, 2, 1, saved["blocks"]["b02_01"].as_str().unwrap(), &bump, &pit)] }).to_string();
    let (st, again) = s.json(None, "PUT", "/api/canvas-save?name=vast", one.as_bytes());
    assert_eq!(st, 200, "{again}");
    assert_ne!(again["blocks"]["b02_01"], saved["blocks"]["b02_01"]);
    println!(
        "128 by 128: new {fresh} bytes; every block saved {painted} bytes in {} files ({} with a chasm mask); the save took {took:?} \
         for a {} byte body; a one-block save {} bytes",
        listed.len(),
        with_chasm.len(),
        body.len(),
        one.len()
    );
    std::fs::create_dir_all(Path::new(env!("CARGO_TARGET_TMPDIR"))).unwrap();
    std::fs::write(
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("canvas-128-layout.txt"),
        listed.iter().map(|(f, n)| format!("{n:>9} {f}\n")).collect::<String>(),
    )
    .unwrap();
}

#[test]
fn many_tabs_saving_at_once_never_lose_or_cut_off_a_block() {
    // (A team server, which answers on many threads: your own machine answers one request at a time.)
    let env = [("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_INSECURE", "1")];
    let s = Server::start("race", &env, true);
    let bob = Some("bob");
    assert_eq!(s.json(bob, "POST", "/api/canvas-new?name=race", header("race", 64).to_string().as_bytes()).0, 200);
    let dir = s.data().join("workspaces/bob/canvases/race/blocks");
    // Eight tabs, each saving its own block (inside it only: no shared row changes), five times each.
    let own: Vec<(usize, usize)> = (0..4).flat_map(|by| (0..2).map(move |bx| (bx * 2 + by % 2, by))).collect();
    let bases: Vec<std::sync::Mutex<String>> = own.iter().map(|_| std::sync::Mutex::new("none".to_string())).collect();
    for round in 0..5 {
        std::thread::scope(|sc| {
            for (k, &(bx, by)) in own.iter().enumerate() {
                let (s, base) = (&s, &bases[k]);
                sc.spawn(move || {
                    let mid = |i: usize, j: usize| {
                        let (li, lj) = (i - bx * 256, j - by * 256);
                        if (1..256).contains(&li) && (1..256).contains(&lj) { GROUND + (round * 8 + k) as f32 } else { GROUND }
                    };
                    let b = block(64, bx, by, &base.lock().unwrap(), &mid, &no_pit);
                    let (st, v) = s.json(bob, "PUT", "/api/canvas-save?name=race", json!({ "blocks": [b] }).to_string().as_bytes());
                    assert_eq!(st, 200, "round {round}, block {k}: {v}");
                    *base.lock().unwrap() = v["blocks"][format!("b{bx:02}_{by:02}")].as_str().unwrap().to_string();
                });
            }
        });
    }
    let (_, loaded) = s.json(bob, "GET", "/api/canvas?name=race", b"");
    assert_eq!(loaded["blocks"].as_object().unwrap().len(), 8);
    for (k, &(bx, by)) in own.iter().enumerate() {
        let key = format!("b{bx:02}_{by:02}");
        assert_eq!(loaded["blocks"][&key].as_str().unwrap(), *bases[k].lock().unwrap(), "{key}: the last save's version");
        let (_, _, bytes) = s.ask(bob, "GET", &format!("/api/canvas-block?name=race&block={key}"), &[], b"");
        let z = f32::from_le_bytes(bytes[12 + 4 * 258..12 + 4 * 259].try_into().unwrap());
        assert_eq!(z, GROUND + (4 * 8 + k) as f32, "{key}: its last round");
    }
    // Eight tabs that loaded the same version of one block, saving it at once: one is kept, whole; the others are told.
    for round in 0..3 {
        let (_, now) = s.json(bob, "GET", "/api/canvas?name=race", b"");
        let base = now["blocks"]["b00_00"].as_str().unwrap().to_string();
        let answers: Vec<(u16, Value, f32)> = std::thread::scope(|sc| {
            let calls: Vec<_> = (0..8)
                .map(|k| {
                    let (s, base) = (&s, &base);
                    sc.spawn(move || {
                        let z = 100.0 + (round * 8 + k) as f32;
                        let mid = move |i: usize, j: usize| if (1..256).contains(&i) && (1..256).contains(&j) { z } else { GROUND };
                        let body = json!({ "blocks": [block(64, 0, 0, base, &mid, &no_pit)] }).to_string();
                        let (st, v) = s.json(bob, "PUT", "/api/canvas-save?name=race", body.as_bytes());
                        (st, v, z)
                    })
                })
                .collect();
            calls.into_iter().map(|c| c.join().unwrap()).collect()
        });
        let won: Vec<&(u16, Value, f32)> = answers.iter().filter(|a| a.0 == 200).collect();
        assert_eq!(won.len(), 1, "round {round}: {:?}", answers.iter().map(|a| a.0).collect::<Vec<_>>());
        let winner = won[0].1["blocks"]["b00_00"].as_str().unwrap();
        for (st, v, _) in answers.iter().filter(|a| a.0 != 200) {
            assert_eq!(*st, 409, "{v}");
            assert_eq!(v["moved"]["b00_00"], winner, "told the version there now: {v}");
        }
        let bytes = std::fs::read(dir.join("b00_00.blk")).unwrap();
        assert_eq!(f32::from_le_bytes(bytes[12 + 4 * 258..12 + 4 * 259].try_into().unwrap()), won[0].2, "the winner's, whole");
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.retain(|n| n.starts_with('.'));
    assert!(names.is_empty(), "no temporary files left: {names:?}");
}

#[test]
fn a_stale_tab_is_refused_and_told_the_versions_there_now() {
    let s = Server::start("stale", &[], false);
    let (_, made) = s.json(None, "POST", "/api/canvas-new?name=rivl", header("rivl", 40).to_string().as_bytes());
    let v0 = made["version"].as_str().unwrap().to_string();
    // Tab B renames the canvas's first piece and paints a block; tab A still holds what it loaded at first.
    let mut h = header("rivl", 40);
    h["pieces"] = json!([{ "id": 1, "name": "rivl01", "x": 0, "y": 0, "w": 20, "h": 20 }]);
    let bump = |i: usize, j: usize| if (i as f32 - 100.0).hypot(j as f32 - 100.0) < 50.0 { 9.0 } else { GROUND };
    let body = json!({ "header": h, "base": v0, "blocks": [block(40, 0, 0, "none", &bump, &no_pit)] }).to_string();
    let (st, b) = s.json(None, "PUT", "/api/canvas-save?name=rivl", body.as_bytes());
    assert_eq!(st, 200, "{b}");
    let before = files(&s.data().join("canvases/rivl"));
    // Tab A's save of the same block and the header: refused whole, with both versions there now.
    let mut mine = header("rivl", 40);
    mine["palette"] = json!(["grassMedium", "dirt", "rockCliff"]);
    let body = json!({ "header": mine, "base": v0, "blocks": [block(40, 0, 0, "none", &flat, &no_pit)] }).to_string();
    let (st, v) = s.json(None, "PUT", "/api/canvas-save?name=rivl", body.as_bytes());
    assert_eq!(st, 409, "{v}");
    assert_eq!(v["moved"], json!({ "canvas.json": b["version"], "b00_00": b["blocks"]["b00_00"] }), "{v}");
    assert!(v["error"].as_str().unwrap().contains("saved from another tab"), "{v}");
    // Its delete, holding the first versions: refused too, and nothing is set aside.
    let (st, v) = s.json(None, "DELETE", "/api/canvas?name=rivl", json!({ "base": v0 }).to_string().as_bytes());
    assert_eq!(st, 409, "{v}");
    assert_eq!(files(&s.data().join("canvases/rivl")), before, "nothing changed");
    // A save without a header's base, and one of a header that would shrink the canvas: refused.
    let (st, v) = s.json(None, "PUT", "/api/canvas-save?name=rivl", json!({ "header": h }).to_string().as_bytes());
    assert_eq!(st, 400, "{v}");
    let mut small = h.clone();
    small["tiles"] = 32.into();
    let (st, v) =
        s.json(None, "PUT", "/api/canvas-save?name=rivl", json!({ "header": small, "base": b["version"] }).to_string().as_bytes());
    assert!(st == 400 && v["error"].as_str().unwrap().contains("keeps its size"), "{st} {v}");
    // Loaded again: the versions of everything there, and the delete with them goes through.
    let (_, now) = s.json(None, "GET", "/api/canvas?name=rivl", b"");
    assert_eq!((&now["version"], &now["blocks"]), (&b["version"], &b["blocks"]));
    assert_eq!(now["header"]["pieceNextId"], 2, "the piece counter is kept");
    let (st, v) =
        s.json(None, "DELETE", "/api/canvas?name=rivl", json!({ "base": now["version"], "blocks": now["blocks"] }).to_string().as_bytes());
    assert_eq!(st, 200, "{v}");
    assert!(!s.data().join("canvases/rivl").exists());
    let aside: Vec<PathBuf> = std::fs::read_dir(s.data().join("deleted")).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(files(&aside[0].join("canvases/rivl")), before, "set aside whole");
    assert_eq!(s.json(None, "GET", "/api/canvas?name=rivl", b"").0, 404);
}

#[test]
fn a_damaged_block_or_header_is_named_and_never_written_over() {
    let s = Server::start("damaged", &[], false);
    let (_, made) = s.json(None, "POST", "/api/canvas-new?name=rivl", header("rivl", 40).to_string().as_bytes());
    let body = json!({ "blocks": [block(40, 1, 1, "none", &flat, &no_pit)] }).to_string();
    let (st, saved) = s.json(None, "PUT", "/api/canvas-save?name=rivl", body.as_bytes());
    assert_eq!(st, 200, "{saved}");
    let dir = s.data().join("canvases/rivl");
    let file = dir.join("blocks/b01_01.blk");
    // One byte changed on disk (a bad sector, a hand edit): the check sum no longer matches.
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[5000] ^= 0x40;
    std::fs::write(&file, &bytes).unwrap();
    let named = |v: &Value| v["error"].as_str().unwrap_or_default().contains("canvases/rivl/blocks/b01_01.blk is damaged");
    // The canvas's load reads each block's version and size only; the block's own load reads it whole, and says so.
    let (st, v) = s.json(None, "GET", "/api/canvas?name=rivl", b"");
    assert_eq!((st, &v["blocks"]), (200, &saved["blocks"]), "{v}");
    let (st, _, b) = s.ask(None, "GET", "/api/canvas-block?name=rivl&block=b01_01", &[], b"");
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert!(st == 500 && named(&v) && v["error"].as_str().unwrap().contains("check sum"), "{st} {v}");
    // A save of it, from the version it had, or of a block beside it that changes their shared row (which needs it to
    // compare): refused, the file as it was.
    let base = saved["blocks"]["b01_01"].as_str().unwrap();
    // (Raised along b01_01's south row and east column, short of their ends, so only that block's neighbours there change.)
    let raised =
        |i: usize, j: usize| if (j == 256 && 256 < i && i < 512) || (i == 512 && 256 < j && j < 512) { GROUND + 1.0 } else { GROUND };
    for (bx, by, base) in [(1, 1, base), (1, 0, "none"), (2, 1, "none")] {
        let body = json!({ "blocks": [block(40, bx, by, base, &raised, &no_pit)] }).to_string();
        let (st, v) = s.json(None, "PUT", "/api/canvas-save?name=rivl", body.as_bytes());
        assert!(st == 500 && named(&v), "({bx}, {by}): {st} {v}");
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "never written over");
    }
    // A block far from it still saves.
    let body = json!({ "blocks": [block(40, 0, 2, "none", &flat, &no_pit)] }).to_string();
    assert_eq!(s.json(None, "PUT", "/api/canvas-save?name=rivl", body.as_bytes()).0, 200);
    // A block that can't be read at all (a folder where the file is): named too, never flat ground.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let (st, _, b) = s.ask(None, "GET", "/api/canvas-block?name=rivl&block=b01_01", &[], b"");
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert!(st == 500 && named(&v), "{st} {v}");
    std::fs::remove_dir(&file).unwrap();
    // The header cut off: every load, save and delete stops with its name, and it stays as it was.
    let head = dir.join("canvas.json");
    let cut = std::fs::read(&head).unwrap()[..20].to_vec();
    std::fs::write(&head, &cut).unwrap();
    let (st, v) = s.json(None, "GET", "/api/canvas?name=rivl", b"");
    assert!(st == 500 && v["error"].as_str().unwrap().contains("canvases/rivl/canvas.json is damaged (it doesn't parse"), "{v}");
    let body = json!({ "header": header("rivl", 40), "base": made["version"] }).to_string();
    assert_eq!(s.json(None, "PUT", "/api/canvas-save?name=rivl", body.as_bytes()).0, 500);
    assert_eq!(s.json(None, "DELETE", "/api/canvas?name=rivl", json!({ "base": made["version"] }).to_string().as_bytes()).0, 500);
    assert_eq!(std::fs::read(&head).unwrap(), cut);
    // The list says so rather than leaving it out.
    let (_, list) = s.json(None, "GET", "/api/canvases", b"");
    assert!(list["canvases"][0]["damaged"].as_str().is_some_and(|d| d.contains("canvas.json is damaged")), "{list}");
    // A header missing from its folder: damaged, not "no such canvas", and its name can't be taken.
    std::fs::remove_file(&head).unwrap();
    let (st, v) = s.json(None, "GET", "/api/canvas?name=rivl", b"");
    assert!(st == 500 && v["error"].as_str().unwrap().contains("canvas.json is damaged (it is missing)"), "{v}");
    assert_eq!(s.json(None, "POST", "/api/canvas-new?name=rivl", header("rivl", 40).to_string().as_bytes()).0, 400);
}

#[test]
fn a_canvas_and_an_area_never_share_a_name() {
    let s = Server::start("names", &[], false);
    let area = json!({ "name": "town", "tiles": 3, "heights": "AAAA" }).to_string();
    assert_eq!(s.json(None, "PUT", "/api/save?name=town&new=1", area.as_bytes()).0, 200);
    let (st, v) = s.json(None, "POST", "/api/canvas-new?name=town", header("town", 40).to_string().as_bytes());
    assert!(st == 400 && v["error"].as_str().unwrap().contains("already an area or a canvas called town"), "{v}");
    assert_eq!(s.json(None, "POST", "/api/canvas-new?name=rivl", header("rivl", 40).to_string().as_bytes()).0, 200);
    for q in ["&new=1", "&new=1&replace=1"] {
        let area = json!({ "name": "rivl", "tiles": 3, "heights": "AAAA" }).to_string();
        let (st, v) = s.json(None, "PUT", &format!("/api/save?name=rivl{q}"), area.as_bytes());
        assert!(st == 400 && v["error"].as_str().unwrap().contains("there is a canvas called rivl"), "{q}: {v}");
    }
    assert!(!s.data().join("areas/rivl.json").exists());
    // A canvas name is 1 to 7 characters; its size 3 to 128 tiles.
    assert_eq!(s.json(None, "POST", "/api/canvas-new?name=toolong8", header("toolong8", 40).to_string().as_bytes()).0, 400);
    assert_eq!(s.json(None, "POST", "/api/canvas-new?name=huge", header("huge", 129).to_string().as_bytes()).0, 400);
    let (_, list) = s.json(None, "GET", "/api/canvases", b"");
    assert_eq!(list["canvases"].as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list["canvases"][0]["tiles"], 40);
}

/// A stand-in for the review service's area list (docs/REVIEW-SERVICE.md), answering `areas` to it and 404 to the rest.
fn review(areas: &'static str) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for mut c in listener.incoming().flatten() {
            let mut head = Vec::new();
            let mut byte = [0u8];
            while !head.ends_with(b"\r\n\r\n") && c.read(&mut byte).is_ok_and(|n| n == 1) {
                head.push(byte[0]);
            }
            let found = String::from_utf8_lossy(&head).split(' ').nth(1) == Some("/api/v1/areas/index");
            let (status, body) = if found { ("200 OK", areas) } else { ("404 Not Found", "{}") };
            let _ = write!(
                c,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

#[test]
fn a_builder_without_the_assignment_cant_save_a_canvas_built_from_an_existing_area() {
    let url = review(r#"{"areas":[{"area":"town","tag":"TOWN"},{"area":"mill","tag":"MILL"}]}"#);
    let env = [
        ("LANDSCAPER_AUTH", "headers"),
        ("LANDSCAPER_ALLOWED_GROUPS", "*"),
        ("LANDSCAPER_ADMIN_USERS", "alice"),
        ("LANDSCAPER_INSECURE", "1"),
        ("LANDSCAPER_REVIEW_URL", url.as_str()),
        ("LANDSCAPER_REVIEW_KEY", "k"),
    ];
    let s = Server::start("team", &env, true);
    let bob = Some("bob");
    let admin = |target: &str| assert_eq!(s.json(Some("alice"), "POST", target, b"").0, 200, "{target}");
    // bob's import of town, as `twn`: the note of where it came from.
    let import = s.data().join("workspaces/bob/exports/twn.import");
    std::fs::create_dir_all(&import).unwrap();
    std::fs::write(import.join("source.json"), r#"{"area":"town"}"#).unwrap();
    // A new canvas named like an existing area: refused, as a new area is.
    let (st, v) = s.json(bob, "POST", "/api/canvas-new?name=town", header("town", 40).to_string().as_bytes());
    assert!(st == 403 && v["error"].as_str().unwrap().contains("existing area"), "{st} {v}");
    let (st, made) = s.json(bob, "POST", "/api/canvas-new?name=rivl", header("rivl", 40).to_string().as_bytes());
    assert_eq!(st, 200, "{made}");
    assert!(s.data().join("workspaces/bob/canvases/rivl/canvas.json").exists(), "in bob's own workspace");
    // Painting with town's ground (its import), without town's assignment: refused, nothing written.
    let mut h = header("rivl", 40);
    h["palette"] = json!(["grassMedium", "@import:twn"]);
    let body = json!({ "header": h, "base": made["version"], "blocks": [block(40, 0, 0, "none", &flat, &no_pit)] }).to_string();
    let (st, v) = s.json(bob, "PUT", "/api/canvas-save?name=rivl", body.as_bytes());
    assert!(st == 403 && v["error"].as_str().unwrap().contains("town"), "{st} {v}");
    assert!(!s.data().join("workspaces/bob/canvases/rivl/blocks/b00_00.blk").exists());
    // A piece named like an existing area (its publish would replace it), without that area's assignment: refused.
    let mut p = header("rivl", 40);
    p["pieces"] = json!([{ "id": 1, "name": "mill", "x": 0, "y": 0, "w": 20, "h": 20 }]);
    let (st, v) = s.json(bob, "PUT", "/api/canvas-save?name=rivl", json!({ "header": p, "base": made["version"] }).to_string().as_bytes());
    assert!(st == 403 && v["error"].as_str().unwrap().contains("mill isn't assigned to you"), "{st} {v}");
    // Assigned town: the save goes through. carol, who has no canvas, sees none of bob's.
    admin("/api/assignments?user=bob&area=town");
    let (st, saved) = s.json(bob, "PUT", "/api/canvas-save?name=rivl", body.as_bytes());
    assert_eq!(st, 200, "{saved}");
    assert_eq!(s.json(Some("carol"), "GET", "/api/canvas?name=rivl", b"").0, 404);
    assert_eq!(s.json(Some("carol"), "GET", "/api/canvases", b"").1["canvases"], json!([]));
    // alice looks at bob's workspace: she reads his canvas and its blocks, and can't change them.
    let view = [("Cookie", "ls_as=bob")];
    let (st, _, b) = s.ask(Some("alice"), "GET", "/api/canvas?name=rivl", &view, b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&b));
    assert_eq!(s.ask(Some("alice"), "GET", "/api/canvas-block?name=rivl&block=b00_00", &view, b"").0, 200);
    let again = json!({ "blocks": [block(40, 0, 0, saved["blocks"]["b00_00"].as_str().unwrap(), &flat, &no_pit)] }).to_string();
    let (st, _, b) = s.ask(Some("alice"), "PUT", "/api/canvas-save?name=rivl", &view, again.as_bytes());
    assert!(st == 403 && String::from_utf8_lossy(&b).contains("read-only"), "{st}");
    let (st, _, _) = s.ask(
        Some("alice"),
        "DELETE",
        "/api/canvas?name=rivl",
        &view,
        json!({ "base": saved["version"], "blocks": saved["blocks"] }).to_string().as_bytes(),
    );
    assert_eq!(st, 403);
    // The assignment revoked: even a save of ground alone is refused, as the canvas still uses town's import; so is its delete.
    admin("/api/assignments/revoke?user=bob&area=town");
    let (st, v) = s.json(bob, "PUT", "/api/canvas-save?name=rivl", again.as_bytes());
    assert!(st == 403 && v["error"].as_str().unwrap().contains("town"), "{st} {v}");
    // (The revoke removed the import's kept files but its note, so the canvas is still known to be built from town.)
    assert!(import.join("source.json").exists());
    let del = json!({ "base": saved["version"], "blocks": saved["blocks"] }).to_string();
    assert_eq!(s.json(bob, "DELETE", "/api/canvas?name=rivl", del.as_bytes()).0, 403);
    assert!(s.data().join("workspaces/bob/canvases/rivl").exists());
    // An admin works on any canvas of their own.
    assert_eq!(s.json(Some("alice"), "POST", "/api/canvas-new?name=mill1", header("mill1", 40).to_string().as_bytes()).0, 200);
}
