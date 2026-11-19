// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A package (Publish without a review service) holds the area's files in the module checkout's layout: nasher's (the default)
//! and `folders`. Through `landscaper serve`, as the editor does it: export a new area, then download its package. Ignored by
//! default, as it needs the game's files (and `nwn_gff` on the `PATH`): `NWN_ROOT=<the game> cargo test --test package_layouts
//! -- --ignored`. It works in scratch folders.

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

/// A server on your own machine with `layout`, whose module checkout `root/module` has only its `module.ifo.json` (at `ifo`),
/// listing one area, `town`.
fn start(root: &Path, layout: &str, ifo: &str) -> Server {
    let game = std::env::var_os("NWN_ROOT").expect("NWN_ROOT: this test needs the game's files");
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/index.html"), "<!doctype html><title>t</title>").unwrap();
    for d in ["home/hak", "home/modules", "xdg"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let ifo_path = root.join("module").join(ifo);
    std::fs::create_dir_all(ifo_path.parent().unwrap()).unwrap();
    let list = serde_json::json!({ "Mod_Area_list": { "type": "list", "value": [
        { "__struct_id": 6, "Area_Name": { "type": "resref", "value": "town" } } ] } });
    std::fs::write(&ifo_path, list.to_string()).unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = Command::new(env!("CARGO_BIN_EXE_landscaper"))
        .args(["serve", "--port", &port.to_string(), "--web"])
        .arg(root.join("web"))
        .env("NWN_ROOT", game)
        .env("NWN_HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("HOME", root)
        .env("LANDSCAPER_MODULE_REPO", root.join("module"))
        .env("LANDSCAPER_LAYOUT", layout)
        .env_remove("LANDSCAPER_AREA_FOLDER")
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

/// `method target` with `body`, as the editor sends it; the status line and the body.
fn request(s: &Server, method: &str, target: &str, body: &[u8]) -> (String, Vec<u8>) {
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
    let mut body = out[split + 4..].to_vec();
    if head.lines().any(|l| l.eq_ignore_ascii_case("transfer-encoding: chunked")) {
        body = unchunk(&body);
    }
    (head.lines().next().unwrap_or("").to_string(), body)
}

/// A chunked body's data.
fn unchunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n").expect("a chunk's size line");
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..eol]).split(';').next().unwrap().trim(), 16).unwrap();
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&b[eol + 2..eol + 2 + size]);
        b = &b[eol + 2 + size + 2..];
    }
}

/// Exports a new 3 by 3 tile area `pkg` (flat grass), then downloads its package: the zip's file names.
fn package(s: &Server) -> Vec<String> {
    let n = 3 * 16 + 1;
    let mut body: Vec<u8> = (0..n * n).flat_map(|_| 1.0f32.to_le_bytes()).collect();
    body.extend(vec![0u8; n * n]);
    body.extend(br#"{"palette":["grassMedium"]}"#);
    let (status, answer) = request(s, "POST", "/api/export?name=pkg&tiles=3", &body);
    assert!(status.contains(" 200"), "export: {status}\n{}", String::from_utf8_lossy(&answer));
    let (status, zip) = request(s, "GET", "/api/package-download?name=pkg", b"");
    assert!(status.contains(" 200"), "package: {status}\n{}", String::from_utf8_lossy(&zip));
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    let mut names: Vec<String> = z.file_names().map(str::to_string).collect();
    names.sort();
    // The module's area list, as the package writes it, has the new area beside the module's own.
    let ifo = names.iter().find(|n| n.ends_with("module.ifo.json")).expect("module.ifo.json in the package").clone();
    let mut text = String::new();
    z.by_name(&ifo).unwrap().read_to_string(&mut text).unwrap();
    let ifo: serde_json::Value = serde_json::from_str(&text).unwrap();
    let areas: Vec<&str> =
        ifo["Mod_Area_list"]["value"].as_array().unwrap().iter().filter_map(|a| a["Area_Name"]["value"].as_str()).collect();
    assert_eq!(areas, ["town", "pkg"]);
    names
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test package_layouts -- --ignored"]
fn a_package_holds_the_areas_files_in_the_checkouts_layout() {
    for (layout, ifo, area) in [("nasher", "src/module.ifo.json", "src/pkg"), ("folders", "module.ifo.json", "areas/core/pkg")] {
        let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("package_layouts-{layout}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let s = start(&root, layout, ifo);
        let names = package(&s);
        for f in [format!("{area}.are.json"), format!("{area}.git.json"), ifo.to_string(), "MANIFEST.md".to_string()] {
            assert!(names.contains(&f), "{layout}: no {f} in {names:?}");
        }
        assert!(
            names.iter().any(|n| n.starts_with("haks/pkg/") && Path::new(n).extension().is_some_and(|e| e == "set")),
            "{layout}: no tileset in {names:?}"
        );
        let module_files: Vec<&String> = names.iter().filter(|n| !n.starts_with("haks/") && *n != "MANIFEST.md").collect();
        assert!(module_files.iter().all(|n| n.starts_with(area) || n.as_str() == ifo), "{layout}: {module_files:?}");
        drop(s);
        let _ = std::fs::remove_dir_all(&root);
    }
}
