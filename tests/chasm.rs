// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A chasm exported against a real game install: nothing walks in it (its tiles' walkmeshes), the ground beside it keeps its
//! tile level, the path check finds no route into it, and the shared hak has its fade. Ignored by default, because it needs the
//! game's files: run it where there is an install with `NWN_ROOT=<the game> cargo test --test chasm -- --ignored`. It works in
//! scratch folders (its own user folder and data folder) and never touches the real ones.

use std::path::{Path, PathBuf};
use std::process::Command;

/// surfacemat.2da's non-walkable row, which the walkmesh gives ground nothing walks on.
const NONWALK: u32 = 7;

/// The scratch folders of one run.
struct Run {
    root: PathBuf,
}

impl Run {
    fn new() -> Run {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("chasm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["home/hak", "home/modules", "xdg", "areas"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Run { root }
    }

    /// `landscaper <args>` with the game from `NWN_ROOT` and this run's folders; its output, or a failure with it.
    fn landscaper(&self, args: &[&str]) -> String {
        let game = std::env::var_os("NWN_ROOT").expect("NWN_ROOT: this test needs the game's files");
        let out = Command::new(env!("CARGO_BIN_EXE_landscaper"))
            .args(args)
            .env("NWN_ROOT", game)
            .env("NWN_HOME", self.root.join("home"))
            .env("XDG_DATA_HOME", self.root.join("xdg"))
            .env("HOME", &self.root)
            .env_remove("LANDSCAPER_MODULE_REPO")
            .output()
            .unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "landscaper {args:?} failed:\n{text}");
        text
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// A walkmesh's (.wok text) vertices and faces with their surfaces.
fn wok(text: &str) -> (Vec<[f32; 3]>, Vec<([usize; 3], u32)>) {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let count = |key: &str| {
        let at = lines.iter().position(|l| l.starts_with(key)).unwrap_or_else(|| panic!("no {key} in the walkmesh"));
        (at, lines[at][key.len()..].trim().parse::<usize>().unwrap())
    };
    let (v, nv) = count("verts ");
    let verts = lines[v + 1..=v + nv]
        .iter()
        .map(|l| {
            let n: Vec<f32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            [n[0], n[1], n[2]]
        })
        .collect();
    let (f, nf) = count("faces ");
    let faces = lines[f + 1..=f + nf]
        .iter()
        .map(|l| {
            let n: Vec<u32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            ([n[0] as usize, n[1] as usize, n[2] as usize], n[7])
        })
        .collect();
    (verts, faces)
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test chasm -- --ignored"]
fn nothing_walks_in_a_chasm_and_the_ground_beside_it_keeps_its_level() {
    let run = Run::new();
    // 4 by 3 tiles of flat ground at 10 m; the chasm is everything west of x = 15 m (tile 0 and the west half of tile 1).
    let (tiles, tiles_y) = (4, 3);
    let (nx, ny) = (tiles * 16 + 1, tiles_y * 16 + 1);
    let edge = 24; // samples west of 15 m (0.625 m apart)
    let dir = run.root.join("areas");
    std::fs::write(dir.join("chs.f32"), (0..nx * ny).flat_map(|_| 10f32.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    std::fs::write(dir.join("chs.materials"), vec![0u8; nx * ny]).unwrap();
    let runs: Vec<[usize; 2]> = (0..ny).map(|j| [j * nx, edge]).collect();
    let header = serde_json::json!({
        "name": "chs", "tiles": tiles, "tilesY": tiles_y, "spacing": 0.625, "width": nx, "height": ny, "data": "chs.f32",
        "materials": "chs.materials", "palette": ["grassMedium"], "chasm": runs,
    });
    let area = dir.join("chs.json");
    std::fs::write(&area, header.to_string()).unwrap();

    // The path check: routes run only on the ground east of it; none fails.
    // (the report is the JSON from its first line that opens one; the output after it is the export's log)
    let said = run.landscaper(&["check", "--json", area.to_str().unwrap()]);
    let report: serde_json::Value = serde_json::Deserializer::from_str(&said[said.find("\n{").map_or(0, |k| k + 1)..])
        .into_iter()
        .next()
        .and_then(Result::ok)
        .unwrap_or_else(|| panic!("no report in: {said}"));
    let failed: Vec<_> = report["warnings"].as_array().into_iter().flatten().filter(|w| w["kind"] == "failed_route").collect();
    assert!(failed.is_empty(), "{failed:?}");

    run.landscaper(&["export", area.to_str().unwrap()]);
    let hak = std::fs::read(run.root.join("home/hak/chs.hak")).unwrap();
    let erf = mg_erf::Erf::read(&hak).unwrap();
    let file = |name: &str| -> String {
        let e = erf.entries.iter().find(|e| e.filename().eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("no {name} in the hak"));
        String::from_utf8(erf.data(e).unwrap().into_owned()).unwrap()
    };
    // A tile all chasm: not one walkable face.
    for ty in 0..tiles_y {
        let (_, faces) = wok(&file(&format!("chs_00{ty:02}.wok")));
        assert!(faces.iter().all(|(_, m)| *m == NONWALK), "tile 0,{ty} has walkable faces");
    }
    // A tile half chasm: its west half (tile x -5..0, area 10..15 m) never walkable; its east half walkable at its tile level,
    // which the chasm's depths left alone (10 m: level 2, so the ground is at 0 in the tile).
    let (verts, faces) = wok(&file("chs_0101.wok"));
    let mid = |f: &[usize; 3]| f.iter().map(|&v| verts[v]).fold([0.0; 3], |a, v| [a[0] + v[0] / 3.0, a[1] + v[1] / 3.0, a[2] + v[2] / 3.0]);
    let walkable: Vec<[f32; 3]> = faces.iter().filter(|(_, m)| *m != NONWALK).map(|(f, _)| mid(f)).collect();
    assert!(!walkable.is_empty(), "the ground east of the chasm is walkable");
    assert!(walkable.iter().all(|c| c[0] > 0.0), "nothing walkable in the chasm: {walkable:?}");
    assert!(walkable.iter().all(|c| c[2].abs() < 0.1), "the walkable ground is at its tile's level: {walkable:?}");
    // The chasm's floor is CHASM_DEPTH (20 m) under the rim.
    assert!(verts.iter().any(|v| (v[2] + 20.0).abs() < 0.01), "the floor is 20 m down");

    // Its walls' fade is in the shared hak.
    let shared = std::fs::read(run.root.join("home/hak/ls_textures.hak")).unwrap();
    let shared = mg_erf::Erf::read(&shared).unwrap();
    assert!(shared.entries.iter().any(|e| e.filename().eq_ignore_ascii_case("ls_chasmfade.dds")));
}
