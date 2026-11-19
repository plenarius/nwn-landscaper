// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Taking an entrance out of an area takes its door out of the installed module on the next export, and only the engine's own
//! door: one a builder placed by hand stays. Through `landscaper serve`'s export, as the editor does it. Ignored by default, as it
//! needs the game's files: `NWN_ROOT=<the game> cargo test --test entrance_doors -- --ignored`. It works in scratch folders.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use mg_core::{ResRef, ResType};
use mg_gff::{Struct, Value};
use mg_resman::ResKey;

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

/// Exports area `ent` (3 by 3 tiles, a ridge rising east of x = 18 m, grass) with the entrances `ids` (each at y = 10 * id - 5
/// on the ridge's foot), as the editor does; the export's report.
fn export(s: &Server, ids: &[u32]) -> serde_json::Value {
    let n = 3 * 16 + 1;
    let heights: Vec<u8> = (0..n * n).flat_map(|k| (((k % n) as f32 * 0.625 - 18.0) * 1.2).clamp(0.0, 8.0).to_le_bytes()).collect();
    let mut body = heights;
    body.extend(vec![0u8; n * n]);
    let entrances: Vec<serde_json::Value> = ids
        .iter()
        .map(|&id| serde_json::json!({ "id": id, "x": 19, "y": 10 * id - 5, "dir": 0, "width": 4, "depth": 4, "floor": 0 }))
        .collect();
    body.extend(serde_json::json!({ "palette": ["grassMedium"], "entrances": entrances }).to_string().into_bytes());
    let target = "/api/export?name=ent&tiles=3";
    let mut c = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(900))).unwrap();
    let host = format!("127.0.0.1:{}", s.port);
    write!(
        c,
        "POST {target} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nX-Landscaper: 1\r\nConnection: close\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    c.write_all(&body).unwrap();
    let mut out = String::new();
    c.read_to_string(&mut out).unwrap();
    let (head, json) = out.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}\n{json}");
    serde_json::from_str(json).unwrap()
}

/// The installed module's area `ent`: its doors' tags and north positions, sorted.
fn doors(module: &Path) -> Vec<(String, i32)> {
    let m = mg_module::Module::open(module).unwrap();
    let git = m.gff(&ResKey { resref: ResRef::from_str("ent").unwrap(), restype: ResType::GIT }).unwrap().unwrap();
    let mut out: Vec<(String, i32)> = git
        .root
        .list("Door List")
        .unwrap_or_default()
        .iter()
        .map(|d| (String::from_utf8_lossy(d.string("Tag").unwrap_or_default()).into_owned(), d.float("Y").unwrap_or(0.0).round() as i32))
        .collect();
    out.sort();
    out
}

/// Adds a door a builder placed by hand in the toolset: tagged `tag`, no engine mark.
fn place_by_hand(module: &Path, tag: &str) {
    let mut m = mg_module::Module::open(module).unwrap();
    let key = ResKey { resref: ResRef::from_str("ent").unwrap(), restype: ResType::GIT };
    let mut git = m.gff(&key).unwrap().unwrap();
    let mut d = Struct::new(8);
    d.set("Tag", Value::String(tag.as_bytes().to_vec()));
    for (k, v) in [("X", 5.0), ("Y", 28.0), ("Z", 0.0), ("Bearing", 0.0)] {
        d.set(k, Value::Float(v));
    }
    let mut list = git.root.list("Door List").map_or_default(<[_]>::to_vec);
    list.push(d);
    git.root.set("Door List", Value::List(list));
    m.set_gff(key, &git).unwrap();
    std::fs::write(module, m.to_archive_bytes().unwrap()).unwrap();
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test entrance_doors -- --ignored"]
fn taking_out_the_middle_entrance_takes_out_its_door_and_nothing_else() {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("entrance_doors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let s = start(&root);
    let module = root.join("home/modules/ent.mod");
    // Three entrances, exported: three doors.
    export(&s, &[1, 2, 3]);
    assert_eq!(doors(&module), [("ent_ent1".to_string(), 5), ("ent_ent2".to_string(), 15), ("ent_ent3".to_string(), 25)]);
    // Two doors a builder placed in the toolset: one with a tag of the engine's shape, one of their own.
    place_by_hand(&module, "ent_ent9");
    place_by_hand(&module, "gate1");
    // The middle one taken out in the editor, exported again.
    let report = export(&s, &[1, 3]);
    assert_eq!(report["updated"]["doors_removed"], serde_json::json!(["ent_ent2"]), "{report}");
    assert_eq!(
        doors(&module),
        [("ent_ent1".to_string(), 5), ("ent_ent3".to_string(), 25), ("ent_ent9".to_string(), 28), ("gate1".to_string(), 28)],
        "ent_ent2's door is gone; the others and the builder's doors are as they were"
    );
    // Again with nothing taken out: nothing removed.
    assert_eq!(export(&s, &[1, 3])["updated"]["doors_removed"], serde_json::json!([]));
    drop(s);
    let _ = std::fs::remove_dir_all(&root);
}
