// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The whole cycle against a real game install: export a new area, export it again over the installed module, import the
//! exported area back, and export and re-export the import. Ignored by default, because it needs the game's files: run it
//! where there is an install with `NWN_ROOT=<the game> cargo test --test game_cycle -- --ignored`. It works in scratch folders
//! (its own user folder, data folder and texture library) and never touches the real ones.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The scratch folders of one run.
struct Run {
    root: PathBuf,
}

impl Run {
    fn new() -> Run {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("game_cycle-{}", std::process::id()));
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

    /// A 3 by 3 tile area `name` as the editor exports one: ground at `base` metres with a hill, grass, `objects`.
    fn area(&self, name: &str, base: f32, objects: serde_json::Value) -> PathBuf {
        let n = 3 * 16 + 1;
        let heights: Vec<u8> = (0..n * n)
            .flat_map(|k| {
                let (i, j) = ((k % n) as f32, (k / n) as f32);
                let hill = 3.0 * (-((i - 24.0).powi(2) + (j - 24.0).powi(2)) / 60.0).exp();
                (base + hill).to_le_bytes()
            })
            .collect();
        let dir = self.root.join("areas");
        std::fs::write(dir.join(format!("{name}.f32")), heights).unwrap();
        std::fs::write(dir.join(format!("{name}.materials")), vec![0u8; n * n]).unwrap();
        let header = serde_json::json!({
            "name": name, "tiles": 3, "tilesY": 3, "spacing": 0.625, "width": n, "height": n, "data": format!("{name}.f32"),
            "materials": format!("{name}.materials"), "palette": ["grassMedium"], "objects": objects,
        });
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, header.to_string()).unwrap();
        path
    }

    fn module(&self, name: &str) -> PathBuf {
        self.root.join("home/modules").join(format!("{name}.mod"))
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test game_cycle -- --ignored"]
fn export_reexport_import_and_export_the_import() {
    let run = Run::new();
    // A new area: its hak and module go into the user folder.
    let area = run.area("cyc", 2.0, serde_json::json!([]));
    let said = run.landscaper(&["export", area.to_str().unwrap()]);
    assert!(said.contains("installed"), "{said}");
    assert!(run.root.join("home/hak/cyc.hak").is_file() && run.module("cyc").is_file());
    let first = std::fs::read(run.module("cyc")).unwrap();
    // Exported again, the ground a metre higher and a rock placed: the installed module takes the new tiles.
    let rock =
        serde_json::json!([{ "model": "plc_a01", "x": 12, "y": 18, "z": 3, "bearing": 0.5, "scale": 1.5, "solid": true, "baked": true }]);
    run.landscaper(&["export", run.area("cyc", 3.0, rock).to_str().unwrap()]);
    assert_ne!(std::fs::read(run.module("cyc")).unwrap(), first, "the installed module was updated");
    // The exported area imported back: its ground, pictures and objects, its original kept for the export.
    // (Into the data folder's exports, where an export looks for an import's models.)
    let exports = run.root.join("xdg/landscaper/exports");
    let source = format!("{}:cyc", run.module("cyc").display());
    let said = run.landscaper(&["import", &source, "imp"]);
    assert!(said.contains("imported"), "{said}");
    let header: serde_json::Value = serde_json::from_slice(&std::fs::read(exports.join("imp.json")).unwrap()).unwrap();
    assert_eq!((header["tiles"].as_u64(), header["imported"].as_bool()), (Some(3), Some(true)), "{header}");
    assert!(exports.join("imp.import/area.are").is_file(), "the original is kept");
    // The import exported, and exported again.
    for _ in 0..2 {
        run.landscaper(&["export", exports.join("imp.json").to_str().unwrap()]);
    }
    assert!(run.module("imp").is_file());
}
