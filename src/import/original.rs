// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The original area an import keeps for the export: the fields and objects an export takes from it ([`adopt`]), where it came
//! from ([`recorded`]), and removing an import when its assignment ends.

use super::sources;
use crate::{models, paint};

/// The area fields an import keeps from the original (what scripts, doors
/// and players know it by, and how it behaves); the rest are landscaper's.
pub const KEPT: [&str; 17] = [
    "Tag",
    "Name",
    "OnEnter",
    "OnExit",
    "OnHeartbeat",
    "OnUserDefined",
    "NoRest",
    "PlayerVsPlayer",
    "ModSpotCheck",
    "ModListenCheck",
    "LoadScreenID",
    "Comments",
    "VarTable",
    "WindPower",
    "ChanceRain",
    "ChanceSnow",
    "ChanceLightning",
];

/// What an import keeps of the original area for the export: its ARE and
/// GIT (`<name>.import/area.are`, `area.git`) and its module's haks.
pub struct Original {
    /// The original area's ARE: its settings, size and tiles.
    pub are: mg_gff::Struct,
    /// Its GIT: the objects in it (None: the source had none).
    pub git: Option<mg_gff::Struct>,
    /// Its module's haks, which its tileset and objects may come from.
    pub haks: Vec<String>,
    /// Its module's custom talk table (the names are in it).
    pub tlk: Option<String>,
}

/// What the import of `name` kept of its original area; None for an area that wasn't imported (or whose files are gone).
pub fn original(name: &str) -> Option<Original> {
    let dir = paint::import_dir(name);
    let are = mg_gff::Gff::read(&std::fs::read(dir.join("area.are")).ok()?).ok()?.root;
    let git = std::fs::read(dir.join("area.git")).ok().and_then(|d| mg_gff::Gff::read(&d).ok()).map(|g| g.root);
    let haks = std::fs::read(dir.join("haks.json")).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
    let tlk = std::fs::read_to_string(dir.join("tlk.txt")).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    Some(Original { are, git, haks, tlk })
}

/// The original area's fields and objects into `are` and `git` (a fresh
/// export's): its identity and scripts, and everything placed in it.
pub fn adopt(o: &Original, are: &mut mg_gff::Struct, git: &mut mg_gff::Struct) {
    for f in KEPT {
        if let Some(v) = o.are.get(f) {
            are.set(f, v.clone());
        }
    }
    let Some(og) = &o.git else { return };
    // What the editor did to them: taken out, moved. Each carries its place in the original's list
    // (`ls_orig`), so a re-export can find it again in the installed module.
    let hm = crate::heightmap::current();
    let edits = hm.as_ref().and_then(|h| h.originals.clone()).unwrap_or(serde_json::Value::Null);
    let ground = |x: f32, y: f32| hm.as_ref().map_or(0.0, |h| h.at(x, y));
    for list in crate::things::LISTS {
        let Some(theirs) = og.list(list) else { continue };
        let mut mine = git.list(list).map_or_default(<[_]>::to_vec);
        for (i, s) in theirs.iter().enumerate() {
            let key = crate::things::orig_key(list, i, &s.string("Tag").map_or_default(|t| String::from_utf8_lossy(t).into_owned()));
            if edits["removed"].as_array().is_some_and(|r| r.iter().any(|k| k.as_str() == Some(key.as_str()))) {
                continue;
            }
            let mut s = s.clone();
            if let Some(m) = edits["moved"].get(&key) {
                crate::things::apply_move(list, &mut s, m, &ground);
            }
            if let (Some(e), Some(kind)) = (edits["edited"].get(&key), crate::things::kind_of_list(list)) {
                crate::things::apply_edit(kind, &mut s, e);
                if e.get("inventory").is_some() || e.get("creatures").is_some() || e.get("equipment").is_some() {
                    match models::game().and_then(|g| crate::things::apply_contents(g, kind, &mut s, e)) {
                        Ok(()) => {}
                        Err(err) => eprintln!("{key}: its contents weren't changed: {err:#}"),
                    }
                }
            }
            crate::things::add_var(&mut s, crate::things::ORIG, &serde_json::json!({ "type": "string", "value": key }));
            mine.push(s);
        }
        git.set(list, mg_gff::Value::List(mine));
    }
    // (Area-wide settings and variables, e.g. a zone flag a script reads off the area.)
    for f in ["AreaProperties", "VarTable"] {
        if let Some(v) = og.get(f) {
            git.set(f, v.clone());
        }
    }
}

/// The production area the import called `name` came from (its resref): None when `name` isn't an import (no `source.json`) or
/// is one of the builder's own file (`"area": ""`). An error, naming the file, when its `source.json` is there but can't be read,
/// doesn't parse or has no `area`: read as "not an import", the area would pass as the builder's own, and every check of their
/// assignment of it would be skipped.
pub fn recorded(name: &str) -> anyhow::Result<Option<String>> {
    recorded_in(&paint::import_dir(name).join("source.json"), name)
}

/// [`recorded`] from the note at `path`, of the import called `name`.
fn recorded_in(path: &std::path::Path, name: &str) -> anyhow::Result<Option<String>> {
    let shown = format!("{name}.import/source.json");
    let d = match crate::atomic::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => anyhow::bail!("{shown} can't be read ({e}): nothing was done; restore it, or ask an admin"),
    };
    let damaged = |why: String| anyhow::anyhow!("{shown} {why}: nothing was done; restore it, or ask an admin");
    let v: serde_json::Value = serde_json::from_slice(&d).map_err(|e| damaged(format!("doesn't parse ({e})")))?;
    let area = v["area"].as_str().ok_or_else(|| damaged("doesn't say which area it came from".into()))?;
    Ok(Some(area.to_string()).filter(|a| !a.is_empty()))
}

/// Whether `name` is an existing area's resref or tag (new areas can't take those names: they'd collide when published). An
/// error when the area list isn't available, so the name is refused rather than let through unchecked.
pub fn production_name(name: &str) -> anyhow::Result<bool> {
    let n = name.to_ascii_lowercase();
    Ok(sources()?.iter().any(|s| {
        s["area"].as_str().is_some_and(|a| a.eq_ignore_ascii_case(&n)) || s["tag"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(&n))
    }))
}

/// The imports of the areas an ended assignment of `area` gave `user` (`*`: every area), and what was built from them,
/// removed from their workspace: the kept original, its pictures and objects, the build folder and the hak. An import that
/// another of their assignments still covers (`*` beside the area, or the area beside `*`) stays. Returns what it removed;
/// an error, removing nothing, when the assignments can't be read.
pub fn remove_imports_of(user: &str, area: &str) -> anyhow::Result<Vec<String>> {
    let ws = crate::workspace::workspaces_dir().join(user);
    let mut removed = Vec::new();
    // (Read once, before anything goes: a damaged file stops it here.)
    let held = crate::assignments::active_for(user)?;
    for e in std::fs::read_dir(ws.join("exports")).into_iter().flatten().flatten() {
        let Some(name) = e.file_name().to_str().and_then(|n| n.strip_suffix(".import")).map(str::to_string) else { continue };
        let from = std::fs::read(e.path().join("source.json")).ok().and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok());
        let Some(recorded) = from.as_ref().and_then(|f| f["area"].as_str()).filter(|a| !a.is_empty()) else { continue };
        if !crate::assignments::covers(area, recorded) || held.iter().any(|a| crate::assignments::covers(&a.area, recorded)) {
            continue;
        }
        // Everything but a note of where it came from (so its name still says "your assignment has ended" to an export, a check or a
        // re-import, including a renamed copy that refers to it).
        for f in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
            if f.file_name() != "source.json" {
                let _ = if f.path().is_dir() { std::fs::remove_dir_all(f.path()) } else { std::fs::remove_file(f.path()) };
            }
        }
        for f in [
            // (The whole area's picture earlier versions wrote.)
            format!("{name}.import.png"),
            format!("{name}.import.view.png"),
            format!("{name}.json"),
            format!("{name}.f32"),
            format!("{name}.materials"),
        ] {
            let _ = std::fs::remove_file(ws.join("exports").join(f));
        }
        let _ = std::fs::remove_dir_all(ws.join("exports").join(format!("{name}_nwn")));
        // The scratch copies the path checks leave.
        for f in [format!("{name}.json"), format!("{name}.f32"), format!("{name}.materials")] {
            let _ = std::fs::remove_file(ws.join("exports").join(".check").join(f));
        }
        let _ = std::fs::remove_dir_all(ws.join("exports").join(".check").join(format!("{name}_nwn")));
        let _ = std::fs::remove_file(ws.join("nwn").join("hak").join(format!("{name}.hak")));
        removed.push(name);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_missing_import_note_means_not_an_import() {
        let dir = std::env::temp_dir().join(format!("ls-import-note-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("source.json");
        assert_eq!(recorded_in(&note, "twn").unwrap(), None, "no note: not an import");
        std::fs::write(&note, r#"{"area":"town","by":"bob","at":1}"#).unwrap();
        assert_eq!(recorded_in(&note, "twn").unwrap().as_deref(), Some("town"));
        std::fs::write(&note, r#"{"area":"","upload":{"file":"mine.mod","area":"cave1"}}"#).unwrap();
        assert_eq!(recorded_in(&note, "twn").unwrap(), None, "the builder's own file: no production area");
        // Cut off, without its area, or not a file: an error naming it, never "not an import".
        for (bytes, why) in [(&br#"{"area":"to"#[..], "doesn't parse"), (&br#"{"by":"bob"}"#[..], "doesn't say which area")] {
            std::fs::write(&note, bytes).unwrap();
            let e = format!("{:#}", recorded_in(&note, "twn").unwrap_err());
            assert!(e.starts_with(&format!("twn.import/source.json {why}")) && !e.contains(&*dir.to_string_lossy()), "{e}");
        }
        std::fs::remove_file(&note).unwrap();
        std::fs::create_dir(&note).unwrap();
        let e = format!("{:#}", recorded_in(&note, "twn").unwrap_err());
        assert!(e.starts_with("twn.import/source.json can't be read"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
