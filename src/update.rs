// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Re-exporting over a module that has been worked on in the toolset: the freshly generated module gives only its tiles; everything
//! else in the installed module stays (objects, scripts, area settings, other areas, extra haks). Objects keep their height above the
//! ground: each moves up or down by how much the terrain under it changed.

use std::path::Path;

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_gff::{Struct, Value};
use mg_module::Module;
use mg_resman::ResKey;

use crate::heightmap::Heightmap;

use crate::things::LISTS as THING_LISTS;

/// GIT lists and how their objects are placed: (list, x, y, z) fields.
const PLACED: [(&str, &str, &str, &str); 9] = [
    ("Creature List", "XPosition", "YPosition", "ZPosition"),
    ("Door List", "X", "Y", "Z"),
    ("Encounter List", "XPosition", "YPosition", "ZPosition"),
    ("List", "XPosition", "YPosition", "ZPosition"),
    ("Placeable List", "X", "Y", "Z"),
    ("SoundList", "XPosition", "YPosition", "ZPosition"),
    ("StoreList", "XPosition", "YPosition", "ZPosition"),
    ("TriggerList", "XPosition", "YPosition", "ZPosition"),
    ("WaypointList", "XPosition", "YPosition", "ZPosition"),
];

/// Adds the freshly generated module's area to the installed module `installed` (another area of the same module, say a cave beside
/// its surface): its ARE, GIT and GIC, an entry in the module's area list, the hak. Its resref is the generated one, or numbered if
/// that is taken.
pub fn add_area(installed: &Path, fresh: &Path, hak: &str) -> Result<String> {
    let generated = Module::open(fresh).with_context(|| format!("opening {}", fresh.display()))?;
    let area = *generated.areas()?.first().context("the generated module has no area")?;
    add_area_of(installed, fresh, area, hak)
}

/// `add_area` for a named area of the generated module (a module of several areas adds them one by one).
pub fn add_area_of(installed: &Path, fresh: &Path, area: mg_core::ResRef, hak: &str) -> Result<String> {
    let mut m = Module::open(installed).with_context(|| format!("opening {}", installed.display()))?;
    let generated = Module::open(fresh).with_context(|| format!("opening {}", fresh.display()))?;
    let taken = |r: &mg_core::ResRef| m.contains(&ResKey { resref: *r, restype: ResType::ARE });
    let name = if taken(&area) {
        (2..100)
            .map(|k| format!("{}{k}", area.to_string().chars().take(14).collect::<String>()))
            .find(|n| mg_core::ResRef::from_str(n).is_ok_and(|r| !taken(&r)))
            .context("no free area name")?
    } else {
        area.to_string()
    };
    let to = mg_core::ResRef::from_str(&name)?;
    for t in [ResType::ARE, ResType::GIT, ResType::GIC] {
        if let Some(Ok(mut g)) = generated.gff(&ResKey { resref: area, restype: t }) {
            if t == ResType::ARE {
                g.root.set("ResRef", Value::resref(to));
            }
            m.set_gff(ResKey { resref: to, restype: t }, &g)?;
        }
    }
    let mut info = m.info()?;
    let mut entry = Struct::new(6);
    entry.set("Area_Name", Value::resref(to));
    match info.root.list_mut("Mod_Area_list") {
        Some(list) => list.push(entry),
        None => info.root.set("Mod_Area_list", Value::List(vec![entry])),
    }
    info.root.set("Mod_HakList", mg_module::attach::hak_list(&info.root, &installed_haks(&info.root, hak)));
    crate::models::give_tlk(&mut info.root);
    m.set_info(&info)?;
    crate::atomic::write(installed, &m.to_archive_bytes()?)?;
    Ok(name)
}

/// An area's haks: its own, then the shared one.
fn haks(hak: &str) -> [String; 2] {
    [hak.to_string(), crate::shared::HAK.to_string()]
}

/// The hak list of a module the area `hak` is installed in (its `module.ifo` root `info`): the area's and the shared hak first,
/// the module's own after them in its order, then the editor's haks it lacks: the placeables placed and baked from them (their
/// rows, models and textures) are there in the game only with them.
fn installed_haks(info: &Struct, hak: &str) -> Vec<String> {
    crate::generator::module_haks(&haks(hak), mg_module::haks_of(info), crate::models::game_haks())
}

/// The first export after an import: the installed area takes the fresh one's objects (the original's, and landscaper's), its identity
/// and scripts, and the module its haks. (Later exports keep what the toolset changed, as for any area.)
pub fn adopt_import(installed: &Path, fresh: &Path, tileset: &str) -> Result<()> {
    let mut m = Module::open(installed).with_context(|| format!("opening {}", installed.display()))?;
    let generated = Module::open(fresh).with_context(|| format!("opening {}", fresh.display()))?;
    let gen_area = *generated.areas()?.first().context("the generated module has no area")?;
    let gen_are = generated.gff(&ResKey { resref: gen_area, restype: ResType::ARE }).context("generated area")??;
    let gen_git = generated.gff(&ResKey { resref: gen_area, restype: ResType::GIT }).context("generated GIT")??;
    for area in m.areas()? {
        let are_key = ResKey { resref: area, restype: ResType::ARE };
        let Some(Ok(mut are)) = m.gff(&are_key) else { continue };
        if !are.root.resref("Tileset").is_some_and(|t| t.to_string().eq_ignore_ascii_case(tileset)) {
            continue;
        }
        for f in crate::import::KEPT {
            if let Some(v) = gen_are.root.get(f) {
                are.root.set(f, v.clone());
            }
        }
        m.set_gff(are_key, &are)?;
        m.set_gff(ResKey { resref: area, restype: ResType::GIT }, &gen_git)?;
    }
    let haks = generated.haks()?;
    let mut info = m.info()?;
    info.root.set("Mod_HakList", mg_module::attach::hak_list(&info.root, &haks));
    if let Some(t) = generated.info()?.root.get("Mod_CustomTlk").cloned() {
        info.root.set("Mod_CustomTlk", t);
    }
    m.set_info(&info)?;
    crate::atomic::write(installed, &m.to_archive_bytes()?)?;
    Ok(())
}

/// Fields of the area the generator owns (the rest are the builder's).
const GENERATED: [&str; 7] = ["Tile_List", "Width", "Height", "Tileset", "Flags", "SkyBox", "TileBrdrDisabled"];

/// What an update of the installed module changed, for the export's report.
pub struct Updated {
    /// Areas on the generated tileset that were updated.
    pub areas: Vec<String>,
    /// Objects looked at in those areas, to follow the ground.
    pub objects: usize,
    /// Objects whose ground moved (more than 1 cm).
    pub moved: usize,
    /// Objects now outside the area (it shrank): tag and position.
    pub outside: Vec<(String, f32, f32)>,
    /// Entrance doors new to the module.
    pub doors_added: usize,
    /// The tags of the entrance doors taken out: the engine's own doors (their `ls_entrance` mark) of
    /// entrances the area no longer has.
    pub doors_removed: Vec<String>,
    /// Object-mode placeables new to the module.
    pub placeables_added: usize,
    /// Object-mode placeables taken out of it.
    pub placeables_removed: usize,
    /// The editor's things made again.
    pub things_added: usize,
    /// The old copies of the editor's things taken out.
    pub things_removed: usize,
    /// An imported area's original objects taken out, and moved back or elsewhere.
    pub originals_removed: usize,
}

/// Moves `z` by the change in ground height at (x, y); true if it moved.
fn follow_ground(s: &mut Struct, (x, y, z): (&str, &str, &str), old: Option<&Heightmap>, new: &Heightmap) -> bool {
    let (Some(px), Some(py), Some(pz)) = (s.float(x), s.float(y), s.float(z)) else { return false };
    let dz = match old {
        Some(old) => new.at(px, py) - old.at(px, py),
        // Unknown old ground: left where it is.
        None => 0.0,
    };
    if dz.abs() <= 0.01 {
        return false;
    }
    s.set(z, Value::Float(pz + dz));
    true
}

/// What the editor says about the area besides its ground: (all optional: an export that doesn't carry one
/// leaves what the installed module has).
#[derive(Clone, Copy, Default)]
pub struct Changes<'a> {
    /// New light and fog, when changed since the last export.
    pub atmosphere: Option<&'a serde_json::Value>,
    /// New settings, when changed since the last export.
    pub settings: Option<&'a serde_json::Value>,
    /// An import's original objects: what was taken out and moved, now and at the last export.
    pub originals: Option<&'a serde_json::Value>,
    /// The same at the last export: one moved or edited then and not now goes back to how the original had it.
    pub old_originals: Option<&'a serde_json::Value>,
    /// Whether the export carries the editor's things at all (an older editor's doesn't: then none are touched).
    pub things: bool,
}

/// Updates the installed module `installed` from the freshly generated `fresh` (same name, tileset `tileset`): its areas on that
/// tileset take the new tiles, their objects follow the ground from `old` to `new` heights, and the hak is attached. What
/// `changes` carries (light and fog, settings, original objects) replaces the areas' own. Writes `installed` in place.
pub fn update_installed(
    installed: &Path,
    fresh: &Path,
    tileset: &str,
    hak: &str,
    old: Option<&Heightmap>,
    new: &Heightmap,
    changes: &Changes<'_>,
) -> Result<Updated> {
    let Changes { atmosphere, settings, originals, old_originals, things: things_sent } = *changes;
    let mut m = Module::open(installed).with_context(|| format!("opening {}", installed.display()))?;
    let generated = Module::open(fresh).with_context(|| format!("opening {}", fresh.display()))?;
    let gen_area = *generated.areas()?.first().context("the generated module has no area")?;
    let gen_are = generated.gff(&ResKey { resref: gen_area, restype: ResType::ARE }).context("generated area")??;
    // The entrances' doors as generated: placed where they now stand; the
    // installed ones keep everything else (their transition, scripts).
    let entrance_doors: Vec<Struct> = generated
        .gff(&ResKey { resref: gen_area, restype: ResType::GIT })
        .and_then(Result::ok)
        .and_then(|g| g.root.list("Door List").map(<[_]>::to_vec))
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.string("Tag").is_some_and(|t| t.starts_with(format!("{tileset}_ent").as_bytes())))
        .collect();
    // Object-mode placeables as generated (tags <area>_p<n>).
    let ours = |s: &Struct| {
        s.string("Tag").is_some_and(|t| {
            let t = String::from_utf8_lossy(t);
            t.strip_prefix(&format!("{tileset}_p")).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
    };
    let our_placeables: Vec<Struct> = generated
        .gff(&ResKey { resref: gen_area, restype: ResType::GIT })
        .and_then(Result::ok)
        .and_then(|g| g.root.list("Placeable List").map(<[_]>::to_vec))
        .unwrap_or_default()
        .into_iter()
        .filter(|p| ours(p))
        .collect();
    // The editor's things as generated (they carry the `ls_thing` mark), by GIT list.
    let our_things: Vec<(&str, Struct)> =
        generated.gff(&ResKey { resref: gen_area, restype: ResType::GIT }).and_then(Result::ok).map_or_default(|g| {
            THING_LISTS
                .iter()
                .flat_map(|l| {
                    g.root
                        .list(l)
                        .unwrap_or(&[])
                        .iter()
                        .filter(|s| crate::things::mark_of(s).is_some())
                        .map(move |s| (*l, s.clone()))
                        .collect::<Vec<_>>()
                })
                .collect()
        });
    // The imported area's own objects, as they were.
    let orig_git = crate::import::original(tileset).and_then(|o| o.git);
    let (width, height) = (new.tiles as f32 * 10.0, new.tiles_y as f32 * 10.0);

    let mut out = Updated {
        areas: Vec::new(),
        objects: 0,
        moved: 0,
        outside: Vec::new(),
        doors_added: 0,
        doors_removed: Vec::new(),
        placeables_added: 0,
        placeables_removed: 0,
        things_added: 0,
        things_removed: 0,
        originals_removed: 0,
    };
    for area in m.areas()? {
        let are_key = ResKey { resref: area, restype: ResType::ARE };
        let Some(Ok(mut are)) = m.gff(&are_key) else { continue };
        if !are.root.resref("Tileset").is_some_and(|t| t.to_string().eq_ignore_ascii_case(tileset)) {
            continue;
        }
        for label in GENERATED {
            if let Some(v) = gen_are.root.get(label) {
                are.root.set(label, v.clone());
            }
        }
        if let Some(a) = atmosphere {
            crate::atmosphere::apply(&mut are.root, a)?;
        }
        m.set_gff(are_key, &are)?;

        let git_key = ResKey { resref: area, restype: ResType::GIT };
        if let Some(Ok(mut git)) = m.gff(&git_key) {
            for (list, x, y, z) in PLACED {
                let Some(objects) = git.root.list_mut(list) else { continue };
                for o in objects.iter_mut() {
                    out.objects += 1;
                    let moved = follow_ground(o, (x, y, z), old, new);
                    // An encounter's spawn points stand on the ground themselves.
                    if let Some(points) = o.list_mut("SpawnPointList") {
                        for p in points.iter_mut() {
                            follow_ground(p, ("X", "Y", "Z"), old, new);
                        }
                    }
                    out.moved += usize::from(moved);
                    let (px, py) = (o.float(x).unwrap_or(0.0), o.float(y).unwrap_or(0.0));
                    if !(0.0..=width).contains(&px) || !(0.0..=height).contains(&py) {
                        let tag = o.string("Tag").map_or_default(|t| String::from_utf8_lossy(t).into_owned());
                        out.outside.push((tag, px, py));
                    }
                }
            }
            let doors = git.root.list_mut("Door List");
            let mut doors = doors.map_or_default(std::mem::take);
            // The engine's own entrance doors (its mark, and the tag it gives them) of entrances the area no longer has go; a door
            // a builder placed, whatever its tag, has no mark and stays.
            doors.retain(|o| {
                let tag = o.string("Tag").map_or_default(|t| String::from_utf8_lossy(t).into_owned());
                let gone = crate::generator::door_mark(o).is_some_and(|id| {
                    tag == format!("{tileset}_ent{id}") && !entrance_doors.iter().any(|d| d.string("Tag") == o.string("Tag"))
                });
                if gone {
                    out.doors_removed.push(tag);
                }
                !gone
            });
            for d in &entrance_doors {
                match doors.iter_mut().find(|o| o.string("Tag") == d.string("Tag")) {
                    Some(o) => {
                        // Where it stands, and its link when the editor sets one (else the toolset's stays).
                        for label in ["X", "Y", "Z", "Bearing", "LinkedTo", "LinkedToFlags"] {
                            if let Some(v) = d.get(label) {
                                o.set(label, v.clone());
                            }
                        }
                        // (A door made before the mark existed gets it now, so it goes too once its entrance does.)
                        if crate::generator::door_mark(o).is_none()
                            && let Some(id) = crate::generator::door_mark(d)
                        {
                            crate::things::add_var(o, crate::generator::DOOR_MARK, &serde_json::json!({ "type": "int", "value": id }));
                        }
                    }
                    None => {
                        doors.push(d.clone());
                        out.doors_added += 1;
                    }
                }
            }
            git.root.set("Door List", Value::List(doors));
            // Placeables: the editor's object-mode ones added, moved (keeping
            // what the toolset gave them) or, gone from the editor, removed.
            let mut placeables = git.root.list_mut("Placeable List").map_or_default(std::mem::take);
            let before = placeables.len();
            placeables.retain(|p| !ours(p) || our_placeables.iter().any(|g| g.string("Tag") == p.string("Tag")));
            out.placeables_removed += before - placeables.len();
            for g in &our_placeables {
                match placeables.iter_mut().find(|p| p.string("Tag") == g.string("Tag")) {
                    Some(p) => {
                        for label in ["X", "Y", "Z", "Bearing", "Appearance"] {
                            if let Some(v) = g.get(label) {
                                p.set(label, v.clone());
                            }
                        }
                        match g.get("VisTransformList") {
                            Some(v) => p.set("VisTransformList", v.clone()),
                            None => {
                                p.remove("VisTransformList");
                            }
                        }
                    }
                    None => {
                        placeables.push(g.clone());
                        out.placeables_added += 1;
                    }
                }
            }
            git.root.set("Placeable List", Value::List(placeables));
            // The editor's things: its own instances there are made again from the generated ones (so a
            // thing moved, changed or taken out in the editor follows); what the toolset added stays.
            for list in THING_LISTS.iter().filter(|_| things_sent) {
                let mut items = git.root.list_mut(list).map_or_default(std::mem::take);
                let before = items.len();
                items.retain(|s| crate::things::mark_of(s).is_none());
                out.things_removed += before - items.len();
                for (l, s) in &our_things {
                    if *l == *list {
                        items.push(s.clone());
                        out.things_added += 1;
                    }
                }
                if !items.is_empty() || git.root.contains(list) {
                    git.root.set(list, Value::List(items));
                }
            }
            // An imported area's original objects: the editor decides where they stand and which are gone (what
            // the toolset changed about them otherwise stays).
            if let Some(og) = &orig_git {
                let ground = |x: f32, y: f32| new.at(x, y);
                let edits = originals.cloned().unwrap_or(serde_json::Value::Null);
                for list in crate::things::LISTS {
                    let Some(items) = git.root.list_mut(list) else { continue };
                    let mut kept = Vec::with_capacity(items.len());
                    for mut s in std::mem::take(items) {
                        if let Some(key) = crate::things::orig_mark(&s) {
                            if edits["removed"].as_array().is_some_and(|r| r.iter().any(|k| k.as_str() == Some(key.as_str()))) {
                                out.originals_removed += 1;
                                continue;
                            }
                            // Moved in the editor; or, moved there at the last export and not now, back where the
                            // original had it. (One the editor never moved keeps what the toolset did to it.)
                            let target = edits["moved"].get(&key).cloned().or_else(|| {
                                old_originals?["moved"].get(&key)?;
                                let (l, i) = crate::things::orig_place(&key)?;
                                let o = og.list(l)?.get(i)?;
                                let (p, bearing) = crate::things::position_of(l, o)?;
                                // (And its outline, if the editor had reshaped it.)
                                let outline = old_originals?["moved"][&key].get("outline").and_then(|_| crate::things::outline_of(l, o));
                                Some(serde_json::json!({ "x": p[0], "y": p[1], "bearing": bearing, "outline": outline }))
                            });
                            if let Some(m) = target {
                                crate::things::apply_move(list, &mut s, &m, &ground);
                            }
                            // Its properties: as the editor has them now; or, edited at the last export and not now, as the
                            // original had them.
                            if let Some(kind) = crate::things::kind_of_list(list) {
                                if let Some(e) = edits["edited"].get(&key) {
                                    crate::things::apply_edit(kind, &mut s, e);
                                    if e.get("inventory").is_some() || e.get("creatures").is_some() || e.get("equipment").is_some() {
                                        crate::things::apply_contents(crate::models::game()?, kind, &mut s, e)
                                            .with_context(|| format!("the contents of {key}"))?;
                                    }
                                } else if let Some(old) = old_originals.and_then(|o| o["edited"].get(&key))
                                    && let Some(item) = crate::things::orig_place(&key).and_then(|(l, i)| og.list(l)?.get(i))
                                {
                                    crate::things::revert_edit(kind, &mut s, item, old);
                                }
                            }
                        }
                        kept.push(s);
                    }
                    *items = kept;
                }
            }
            if let Some(sett) = settings
                && let Some(Ok(mut are)) = m.gff(&are_key)
            {
                crate::settings::apply(&mut are.root, &mut git.root, sett)?;
                m.set_gff(are_key, &are)?;
            }
            m.set_gff(git_key, &git)?;
        }
        out.areas.push(area.to_string());
    }
    anyhow::ensure!(!out.areas.is_empty(), "{} has no area on the tileset {tileset}", installed.display());

    // The haks, in front of any the builder added; the editor's after the module's own.
    let mut info = m.info()?;
    let listed = mg_module::haks_of(&info.root);
    let want = installed_haks(&info.root, hak);
    let has = |h: &String| listed.iter().any(|l| l.eq_ignore_ascii_case(h));
    let tlk_before = info.root.get("Mod_CustomTlk").cloned();
    crate::models::give_tlk(&mut info.root);
    let tlk_given = info.root.get("Mod_CustomTlk") != tlk_before.as_ref();
    if !want.iter().all(has) {
        info.root.set("Mod_HakList", mg_module::attach::hak_list(&info.root, &want));
    }
    if !want.iter().all(has) || tlk_given {
        m.set_info(&info)?;
    }
    let bytes = m.to_archive_bytes()?;
    crate::atomic::write(installed, &bytes)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use mg_core::ResRef;
    use mg_gff::Gff;

    use super::*;

    /// A 3 by 3 tile area's ground, all at `h` metres (a 10 m grid: the update reads heights, not their spacing).
    fn ground(h: f32) -> Heightmap {
        Heightmap {
            name: "lsupdt".into(),
            tiles: 3,
            tiles_y: 3,
            spacing: 10.0,
            width: 4,
            height: 4,
            data: vec![h; 16],
            materials: vec![],
            palette: vec![],
            atmosphere: None,
            things: vec![],
            settings: None,
            originals: None,
            water: vec![],
            shadow: None,
            entrances: vec![],
            objects: vec![],
            underground: false,
            interior: false,
            roof: None,
            ceiling: vec![],
            rim: vec![],
            serial: 0,
        }
    }

    /// An object of GIT list `list` tagged `tag` at (x, y, z), placed as the list places them ([`PLACED`]).
    fn placed(list: &str, tag: &str, [x, y, z]: [f32; 3]) -> Struct {
        let (_, fx, fy, fz) = PLACED.iter().find(|p| p.0 == list).copied().unwrap();
        let mut s = Struct::new(0);
        s.set("Tag", Value::String(tag.as_bytes().to_vec()));
        for (f, v) in [(fx, x), (fy, y), (fz, z)] {
            s.set(f, Value::Float(v));
        }
        s
    }

    /// A module file `path` with one area `area` on the tileset `lsupdt`, its tiles all `tile`, its ARE's other fields `are`,
    /// and `git`'s lists.
    fn module(path: &Path, area: &str, tile: i32, are: &[(&str, Value)], git: &[(&str, Vec<Struct>)]) {
        let r = ResRef::from_str(area).unwrap();
        let mut m = Module::new();
        let mut info = Gff::new(*b"IFO ");
        let mut entry = Struct::new(6);
        entry.set("Area_Name", Value::resref(r));
        info.root.set("Mod_Area_list", Value::List(vec![entry]));
        m.set_info(&info).unwrap();
        let mut a = Gff::new(*b"ARE ");
        a.root.set("Tileset", Value::resref(ResRef::from_str("lsupdt").unwrap()));
        a.root.set("Width", Value::Int(3));
        a.root.set("Height", Value::Int(3));
        let mut t = Struct::new(1);
        t.set("Tile_ID", Value::Int(tile));
        a.root.set("Tile_List", Value::List(vec![t; 9]));
        for (k, v) in are {
            a.root.set(k, v.clone());
        }
        m.set_gff(ResKey { resref: r, restype: ResType::ARE }, &a).unwrap();
        let mut g = Gff::new(*b"GIT ");
        for (list, items) in git {
            g.root.set(list, Value::List(items.clone()));
        }
        m.set_gff(ResKey { resref: r, restype: ResType::GIT }, &g).unwrap();
        std::fs::write(path, m.to_archive_bytes().unwrap()).unwrap();
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ls-update-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn area_of(path: &Path, area: &str) -> (Struct, Struct) {
        let m = Module::open(path).unwrap();
        let r = ResRef::from_str(area).unwrap();
        let get = |t| m.gff(&ResKey { resref: r, restype: t }).unwrap().unwrap().root;
        (get(ResType::ARE), get(ResType::GIT))
    }

    fn find<'a>(git: &'a Struct, list: &str, tag: &str) -> Option<&'a Struct> {
        git.list(list)?.iter().find(|s| s.string("Tag") == Some(tag.as_bytes()))
    }

    #[test]
    fn a_reexport_takes_the_new_tiles_keeps_the_toolsets_work_and_moves_objects_with_the_ground() {
        let dir = scratch("reexport");
        let (installed, fresh) = (dir.join("cave.mod"), dir.join("fresh.mod"));
        let mut p2 = placed("Placeable List", "lsupdt_p2", [1.0, 1.0, 1.0]);
        p2.set("Appearance", Value::Dword(3));
        p2.set("OnUsed", Value::resref(ResRef::from_str("tool_used").unwrap()));
        // What the toolset made of the last export: a chest, a guard, a waypoint now outside the area, the editor's
        // object-mode placeables (one since taken out of the editor), a script and a name.
        module(
            &installed,
            "cave1",
            1,
            &[("OnEnter", Value::resref(ResRef::from_str("tool_enter").unwrap())), ("SunFogColor", Value::Dword(0x111111))],
            &[
                (
                    "Placeable List",
                    vec![placed("Placeable List", "chest", [5.0, 5.0, 1.0]), placed("Placeable List", "lsupdt_p1", [8.0, 8.0, 1.0]), p2],
                ),
                ("Creature List", vec![placed("Creature List", "guard", [12.0, 12.0, 1.0])]),
                ("WaypointList", vec![placed("WaypointList", "wp_out", [45.0, 5.0, 1.0])]),
            ],
        );
        // The new export: new tiles, p2 moved and given another look, a new p3.
        let mut p2_new = placed("Placeable List", "lsupdt_p2", [20.0, 20.0, 3.0]);
        p2_new.set("Appearance", Value::Dword(7));
        module(&fresh, "lsupdt", 2, &[], &[("Placeable List", vec![p2_new, placed("Placeable List", "lsupdt_p3", [25.0, 25.0, 3.0])])]);
        // The ground rose 2 m.
        let (old, new) = (ground(1.0), ground(3.0));
        let u = update_installed(&installed, &fresh, "lsupdt", "lsupdt", Some(&old), &new, &Changes::default()).unwrap();
        assert_eq!(u.areas, ["cave1"]);
        assert_eq!((u.placeables_added, u.placeables_removed), (1, 1));
        assert_eq!(u.outside.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), ["wp_out"]);
        let (are, git) = area_of(&installed, "cave1");
        assert_eq!(are.list("Tile_List").unwrap()[0].integer("Tile_ID"), Some(2), "the new tiles");
        assert_eq!(are.resref("OnEnter").map(|r| r.to_string()).as_deref(), Some("tool_enter"), "the toolset's script stays");
        assert_eq!(are.integer("SunFogColor"), Some(0x111111), "no new light from the editor: the toolset's stays");
        assert_eq!(find(&git, "Placeable List", "chest").unwrap().float("Z"), Some(3.0), "the chest follows the ground");
        assert_eq!(find(&git, "Creature List", "guard").unwrap().float("ZPosition"), Some(3.0));
        assert!(find(&git, "Placeable List", "lsupdt_p1").is_none(), "taken out in the editor");
        let p2 = find(&git, "Placeable List", "lsupdt_p2").unwrap();
        assert_eq!((p2.float("X"), p2.integer("Appearance")), (Some(20.0), Some(7)), "where and how the editor has it now");
        assert_eq!(p2.resref("OnUsed").map(|r| r.to_string()).as_deref(), Some("tool_used"), "its toolset script stays");
        assert!(find(&git, "Placeable List", "lsupdt_p3").is_some());
        let haks = Module::open(&installed).unwrap().haks().unwrap();
        assert!(haks.iter().any(|h| h == "lsupdt") && haks.iter().any(|h| h == crate::shared::HAK), "{haks:?}");

        // Again with the editor's new light, and ground whose last version is unknown: the light goes in, nothing moves.
        let fog = serde_json::json!({ "sun": { "ambient": "#000000", "diffuse": "#000000", "fog": "#ff0000", "fogAmount": 5 },
            "moon": { "ambient": "#000000", "diffuse": "#000000", "fog": "#ff0000", "fogAmount": 5 } });
        let changes = Changes { atmosphere: Some(&fog), ..Changes::default() };
        let u = update_installed(&installed, &fresh, "lsupdt", "lsupdt", None, &ground(9.0), &changes).unwrap();
        assert_eq!(u.moved, 0);
        let (are, git) = area_of(&installed, "cave1");
        assert_eq!(are.integer("SunFogColor"), Some(0x0000ff), "red, as the game stores it (BGR)");
        assert_eq!(find(&git, "Placeable List", "chest").unwrap().float("Z"), Some(3.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_area_is_added_beside_the_modules_own_under_a_free_name() {
        let dir = scratch("add");
        let (installed, fresh) = (dir.join("world.mod"), dir.join("fresh.mod"));
        // The module already has an area called lsupdt (another builder's, say): the new one is numbered.
        module(&installed, "lsupdt", 1, &[], &[]);
        module(&fresh, "lsupdt", 2, &[], &[("WaypointList", vec![placed("WaypointList", "wp_in", [5.0, 5.0, 0.0])])]);
        assert_eq!(add_area(&installed, &fresh, "lsupdt").unwrap(), "lsupdt2");
        let m = Module::open(&installed).unwrap();
        assert_eq!(m.areas().unwrap().iter().map(ToString::to_string).collect::<Vec<_>>(), ["lsupdt", "lsupdt2"]);
        let (are, git) = area_of(&installed, "lsupdt2");
        assert_eq!(are.resref("ResRef").map(|r| r.to_string()).as_deref(), Some("lsupdt2"));
        assert!(find(&git, "WaypointList", "wp_in").is_some());
        assert_eq!(area_of(&installed, "lsupdt").0.list("Tile_List").unwrap()[0].integer("Tile_ID"), Some(1), "the module's own untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod entrance_door_tests {
    use mg_core::ResRef;
    use mg_gff::Gff;

    use super::*;

    /// A door tagged `tag`, with the engine's mark for entrance `mark` if given.
    fn door(tag: &str, mark: Option<u32>) -> Struct {
        let mut d = Struct::new(8);
        d.set("Tag", Value::String(tag.as_bytes().to_vec()));
        for k in ["X", "Y", "Z", "Bearing"] {
            d.set(k, Value::Float(1.0));
        }
        if let Some(id) = mark {
            crate::things::add_var(&mut d, crate::generator::DOOR_MARK, &serde_json::json!({ "type": "int", "value": id }));
        }
        d
    }

    /// A module file `path` with one area `area` on the tileset `lsdoor` whose doors are `doors`.
    fn module(path: &Path, area: &str, doors: Vec<Struct>) {
        let r = ResRef::from_str(area).unwrap();
        let mut m = Module::new();
        let mut info = Gff::new(*b"IFO ");
        let mut entry = Struct::new(6);
        entry.set("Area_Name", Value::resref(r));
        info.root.set("Mod_Area_list", Value::List(vec![entry]));
        m.set_info(&info).unwrap();
        let mut are = Gff::new(*b"ARE ");
        are.root.set("Tileset", Value::resref(ResRef::from_str("lsdoor").unwrap()));
        m.set_gff(ResKey { resref: r, restype: ResType::ARE }, &are).unwrap();
        let mut git = Gff::new(*b"GIT ");
        git.root.set("Door List", Value::List(doors));
        m.set_gff(ResKey { resref: r, restype: ResType::GIT }, &git).unwrap();
        std::fs::write(path, m.to_archive_bytes().unwrap()).unwrap();
    }

    #[test]
    fn the_door_of_an_entrance_taken_out_goes_and_a_builders_door_stays() {
        let dir = std::env::temp_dir().join(format!("ls-entrance-doors-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (installed, fresh) = (dir.join("cave.mod"), dir.join("fresh.mod"));
        // Installed: the engine's doors of entrances 1 (made before the mark existed) and 2, and two of a builder's, one with a
        // tag of the engine's shape. The new export has entrance 1 only.
        module(
            &installed,
            "cave",
            vec![door("lsdoor_ent1", None), door("lsdoor_ent2", Some(2)), door("lsdoor_ent9", None), door("gate", None)],
        );
        module(&fresh, "lsdoor", vec![door("lsdoor_ent1", Some(1))]);
        let ground = Heightmap {
            name: "lsdoor".into(),
            tiles: 3,
            tiles_y: 3,
            spacing: 10.0,
            width: 4,
            height: 4,
            data: vec![0.0; 16],
            materials: vec![],
            palette: vec![],
            atmosphere: None,
            things: vec![],
            settings: None,
            originals: None,
            water: vec![],
            shadow: None,
            entrances: vec![],
            objects: vec![],
            underground: false,
            interior: false,
            roof: None,
            ceiling: vec![],
            rim: vec![],
            serial: 0,
        };
        let u = update_installed(&installed, &fresh, "lsdoor", "lsdoor", None, &ground, &Changes::default()).unwrap();
        assert_eq!(u.doors_removed, ["lsdoor_ent2"]);
        let m = Module::open(&installed).unwrap();
        let git = m.gff(&ResKey { resref: ResRef::from_str("cave").unwrap(), restype: ResType::GIT }).unwrap().unwrap().root;
        let doors = git.list("Door List").unwrap();
        let tags: Vec<String> = doors.iter().map(|d| String::from_utf8_lossy(d.string("Tag").unwrap()).into_owned()).collect();
        assert_eq!(tags, ["lsdoor_ent1", "lsdoor_ent9", "gate"]);
        // The door made before the mark existed has it now, so it goes too once its entrance does; the builder's have none.
        let marks: Vec<Option<i64>> = doors.iter().map(crate::generator::door_mark).collect();
        assert_eq!(marks, [Some(1), None, None]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
