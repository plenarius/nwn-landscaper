// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `landscaper import <source> <name> <dir>` into a folder of its own, then `landscaper export <dir>/<name>.json`: the export
//! finds the import's models beside the area, not in the data folder's exports. Ignored by default, as it needs the game's
//! files: `NWN_ROOT=<the game> cargo test --test import_dir -- --ignored`. It works in scratch folders only.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `landscaper <args>` with the game from `NWN_ROOT`, the user folder `root/home` and the data folder `root/xdg`.
fn landscaper(root: &Path, args: &[&str]) -> String {
    let game = std::env::var_os("NWN_ROOT").expect("NWN_ROOT: this test needs the game's files");
    let out = Command::new(env!("CARGO_BIN_EXE_landscaper"))
        .args(args)
        .env("NWN_ROOT", game)
        .env("NWN_HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("HOME", root)
        .env_remove("LANDSCAPER_MODULE_REPO")
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "landscaper {args:?} failed:\n{text}");
    text
}

/// A 3 by 3 tile area `name` in `dir`, as the editor exports one: flat ground at 2 m, grass. Its header's path.
fn area(dir: &Path, name: &str) -> PathBuf {
    let n = 3 * 16 + 1;
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{name}.f32")), std::iter::repeat_n(2.0f32.to_le_bytes(), n * n).flatten().collect::<Vec<u8>>())
        .unwrap();
    std::fs::write(dir.join(format!("{name}.materials")), vec![0u8; n * n]).unwrap();
    let header = format!(
        concat!(
            r#"{{"name":"{name}","tiles":3,"spacing":0.625,"width":{n},"height":{n},"data":"{name}.f32","#,
            r#""materials":"{name}.materials","palette":["grassMedium"]}}"#
        ),
        name = name,
        n = n
    );
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, header).unwrap();
    path
}

#[test]
#[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test --test import_dir -- --ignored"]
fn an_import_into_a_folder_of_its_own_exports_from_there() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("import_dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["home/hak", "home/modules", "xdg"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    // An area to import: a new one, exported (its module is in the user folder, its tileset in its hak).
    landscaper(&root, &["export", area(&root.join("areas"), "src1").to_str().unwrap()]);
    let module = root.join("home/modules/src1.mod");
    // Imported into a folder of its own, not the data folder's exports.
    let mine = root.join("mine");
    landscaper(&root, &["import", &format!("{}:src1", module.display()), "imp", mine.to_str().unwrap()]);
    assert!(mine.join("imp.import").is_dir() && mine.join("imp.json").is_file());
    assert!(!root.join("xdg/landscaper/exports/imp.import").exists(), "nothing of it in the default folder");
    // Exported from there: its models are found beside it (before, "no imported model imp:0101").
    let said = landscaper(&root, &["export", mine.join("imp.json").to_str().unwrap()]);
    assert!(said.contains("installed"), "{said}");
    assert!(root.join("home/modules/imp.mod").is_file());
    let _ = std::fs::remove_dir_all(&root);
}
