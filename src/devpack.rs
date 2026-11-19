// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A test build for the dev server: the production module (built from `ee`) plus a builder's area, a temporary
//! way in from an existing area, and the files the area needs (its hak and the shared one).
//!
//! ```text
//! landscaper dev-pack <base.mod> <workspace.mod> <hak-dir> <area> <waypoint> <out-dir> [entry-area]
//! ```
//!
//! `base.mod` is the production module, `workspace.mod` the builder's exported module: all its areas are added,
//! each with its hak (named by its tileset). `area`/`waypoint` are the existing area and one of its waypoints
//! where the way in stands; it leads to `entry-area` (the first area when not given), and the doors between the
//! module's areas carry the rest. Writes `<out-dir>/<dev module>.mod` (`LANDSCAPER_DEV_MODULE`), the haks and `pack.json`.

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_gff::{Struct, Value};
use mg_module::Module;
use mg_resman::ResKey;
use serde_json::{Value as Json, json};
use std::path::Path;

/// Tag of the waypoint the way in leads to.
pub const ARRIVE: &str = "lstest_wp";
/// Tag of the trigger that leads into the area under test.
pub const GO_IN: &str = "lstest_in";
/// Tag of the trigger that leads back out.
pub const GO_OUT: &str = "lstest_out";
/// Offset of a transition trigger from the waypoint it sits beside, so arriving never lands inside one.
const BESIDE: f32 = 3.0;
const HALF: f32 = 2.0;

/// A transition trigger as the editor's JSON, centred at (x, y).
fn trigger(id: u32, tag: &str, x: f32, y: f32, to: &str, flags: u8) -> Json {
    json!({
        "id": id, "kind": "trigger", "x": x, "y": y, "tag": tag,
        "outline": [[x - HALF, y - HALF], [x + HALF, y - HALF], [x + HALF, y + HALF], [x - HALF, y + HALF]],
        "fields": { "Type": 1, "LinkedTo": to, "LinkedToFlags": flags },
    })
}

fn waypoint(id: u32, tag: &str, x: f32, y: f32) -> Json {
    json!({ "id": id, "kind": "waypoint", "x": x, "y": y, "tag": tag })
}

/// Appends `s` to the GIT list `list`, creating the list if the area has none.
fn push(git: &mut Struct, list: &str, s: Struct) {
    match git.list_mut(list) {
        Some(l) => l.push(s),
        None => git.set(list, Value::List(vec![s])),
    }
}

/// Position of the object tagged `tag` in a GIT list (waypoints, doors).
fn find(git: &Struct, list: &str, tag: &str) -> Option<[f32; 3]> {
    git.list(list)?
        .iter()
        .find(|s| s.string("Tag") == Some(tag.as_bytes()))
        .and_then(|s| crate::things::position_of(list, s))
        .map(|(p, _)| p)
}

/// Where players arrive in the builder's area: the landscaper entrance door with the lowest id (`<area>_ent<id>`: its oldest
/// entrance still there), else its first
/// waypoint, else the middle of the area (`size` is its width and height in tiles).
fn arrival(git: &Struct, size: (u32, u32)) -> [f32; 3] {
    let mut doors: Vec<(u32, [f32; 3])> = git
        .list("Door List")
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let tag = String::from_utf8_lossy(d.string("Tag")?).into_owned();
            let n: u32 = tag.rsplit_once("_ent")?.1.parse().ok()?;
            Some((n, crate::things::position_of("Door List", d)?.0))
        })
        .collect();
    doors.sort_by_key(|d| d.0);
    if let Some((_, p)) = doors.first() {
        return *p;
    }
    if let Some(p) = git.list("WaypointList").and_then(|l| l.first()).and_then(|w| crate::things::position_of("WaypointList", w)) {
        return p.0;
    }
    [size.0 as f32 * 5.0, size.1 as f32 * 5.0, 0.0]
}

fn gff_of(m: &Module, area: ResRef, t: ResType) -> Result<mg_gff::Gff> {
    m.gff(&ResKey { resref: area, restype: t })
        .with_context(|| format!("the module has no {area}.{t:?}"))?
        .map_err(|e| anyhow::anyhow!("{area}: {e}"))
}

/// The two ways of the link as things: (in the existing area, in the builder's area).
pub fn link_things(from: [f32; 3], arrive: [f32; 3], back_to: &str) -> (Vec<Json>, Vec<Json>) {
    let existing = vec![trigger(9001, GO_IN, from[0] + BESIDE, from[1], ARRIVE, 2)];
    let builders = vec![waypoint(9002, ARRIVE, arrive[0], arrive[1]), trigger(9003, GO_OUT, arrive[0] + BESIDE, arrive[1], back_to, 2)];
    (existing, builders)
}

/// Adds every area of the builder's module (each on its own hak, named by its tileset) to the production module,
/// and the way in to the `entry` one (an area's resref or tileset name; the first area when not given).
pub fn pack(base: &Path, workspace: &Path, hak_dir: &Path, area: &str, wp: &str, entry: Option<&str>, out: &Path) -> Result<Json> {
    std::fs::create_dir_all(out)?;
    let module_file = format!("{}.mod", crate::config::dev_module());
    let test = out.join(&module_file);
    std::fs::copy(base, &test).with_context(|| format!("copying {}", base.display()))?;

    let ws = Module::open(workspace).with_context(|| format!("opening {}", workspace.display()))?;
    let originals = ws.areas()?;
    anyhow::ensure!(!originals.is_empty(), "{} has no area", workspace.display());
    // (original resref, its name in the test module, its hak)
    let mut added: Vec<(String, String, String)> = Vec::new();
    for a in &originals {
        let tileset = gff_of(&ws, *a, ResType::ARE)?
            .root
            .resref("Tileset")
            .map(|t| t.to_string().to_ascii_lowercase())
            .with_context(|| format!("{a} has no tileset"))?;
        let name = crate::update::add_area_of(&test, workspace, *a, &tileset)?;
        added.push((a.to_string().to_ascii_lowercase(), name, tileset));
    }
    let wanted = entry.map(str::to_ascii_lowercase);
    let at_entry = match &wanted {
        Some(w) => added.iter().find(|(o, _, t)| o == w || t == w).with_context(|| format!("the module has no area {w}"))?,
        None => &added[0],
    };
    let entry_name = at_entry.1.clone();

    let game = crate::models::game()?;
    let mut m = Module::open(&test)?;
    let existing = ResRef::from_str(area).with_context(|| format!("area {area}"))?;
    let new = ResRef::from_str(&entry_name)?;
    let mut egit = gff_of(&m, existing, ResType::GIT)?;
    let mut ngit = gff_of(&m, new, ResType::GIT)?;
    let at = find(&egit.root, "WaypointList", wp).with_context(|| format!("area {area} has no waypoint tagged {wp}"))?;
    let size = gff_of(&m, new, ResType::ARE)
        .map(|a| (a.root.integer("Width").unwrap_or(0) as u32, a.root.integer("Height").unwrap_or(0) as u32))?;
    let arrive = arrival(&ngit.root, size);
    let (in_existing, in_new) = link_things(at, arrive, wp);
    let place = |git: &mut Struct, things: &[Json], z: f32| -> Result<()> {
        for j in things {
            let t = crate::things::Thing::parse(j).map_err(anyhow::Error::msg)?;
            let (list, s) = crate::things::place(game, &t, &|_, _| z)?;
            push(git, list, s);
        }
        Ok(())
    };
    place(&mut egit.root, &in_existing, at[2])?;
    place(&mut ngit.root, &in_new, arrive[2])?;
    m.set_gff(ResKey { resref: existing, restype: ResType::GIT }, &egit)?;
    m.set_gff(ResKey { resref: new, restype: ResType::GIT }, &ngit)?;
    std::fs::write(&test, m.to_archive_bytes()?)?;

    let mut haks: Vec<String> = Vec::new();
    for name in added.iter().map(|a| a.2.as_str()).chain([crate::shared::HAK]) {
        if haks.iter().any(|h| h == &format!("{name}.hak")) {
            continue;
        }
        let from = hak_dir.join(format!("{name}.hak"));
        anyhow::ensure!(from.exists(), "no hak {}", from.display());
        std::fs::copy(&from, out.join(format!("{name}.hak")))?;
        haks.push(format!("{name}.hak"));
    }
    let areas: Vec<Json> = added.iter().map(|(o, n, h)| json!({ "area": o, "as": n, "hak": h })).collect();
    let report = json!({
        "module": module_file, "areas": areas, "entry": entry_name, "haks": haks,
        "link": { "from_area": area, "from_waypoint": wp, "arrives_at": arrive },
    });
    std::fs::write(out.join("pack.json"), serde_json::to_string_pretty(&report)?)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, tag: &str) -> Struct {
        let mut s = Struct::new(0);
        s.set("Tag", Value::String(tag.as_bytes().to_vec()));
        s.set("XPosition", Value::Float(x));
        s.set("YPosition", Value::Float(y));
        s.set("ZPosition", Value::Float(1.0));
        s
    }

    #[test]
    fn arrives_at_the_lowest_entrance_door() {
        let mut git = Struct::new(0);
        for (n, x) in [(2, 20.0), (1, 10.0)] {
            let mut d = Struct::new(0);
            d.set("Tag", Value::String(format!("bay_ent{n}").into_bytes()));
            d.set("X", Value::Float(x));
            d.set("Y", Value::Float(5.0));
            d.set("Z", Value::Float(0.0));
            push(&mut git, "Door List", d);
        }
        push(&mut git, "WaypointList", at(99.0, 99.0, "wp"));
        assert_eq!(arrival(&git, (4, 4))[0], 10.0);
    }

    #[test]
    fn arrives_at_a_waypoint_then_the_middle() {
        let mut git = Struct::new(0);
        assert_eq!(arrival(&git, (4, 6)), [20.0, 30.0, 0.0]);
        push(&mut git, "WaypointList", at(7.0, 8.0, "wp"));
        assert_eq!(arrival(&git, (4, 6)), [7.0, 8.0, 1.0]);
    }

    #[test]
    fn finds_a_waypoint_by_tag() {
        let mut git = Struct::new(0);
        push(&mut git, "WaypointList", at(3.0, 4.0, "gate"));
        assert_eq!(find(&git, "WaypointList", "gate"), Some([3.0, 4.0, 1.0]));
        assert_eq!(find(&git, "WaypointList", "nope"), None);
    }

    #[test]
    fn the_link_things_parse_and_point_at_each_other() {
        let (a, b) = link_things([10.0, 10.0, 0.0], [50.0, 50.0, 0.0], "gate");
        for j in a.iter().chain(&b) {
            crate::things::Thing::parse(j).unwrap();
        }
        assert_eq!(a[0]["fields"]["LinkedTo"], ARRIVE);
        assert_eq!(b[1]["fields"]["LinkedTo"], "gate");
        assert_eq!(b[0]["tag"], ARRIVE);
        // beside the waypoints, never on them
        assert_eq!(a[0]["x"].as_f64().unwrap() as f32, 13.0);
    }
}
