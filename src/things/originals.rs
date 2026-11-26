// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The objects an imported area already has (its "originals"): how the editor finds them again on a re-export, and the edits it
//! makes to them (taken out, moved, reshaped, settings, variables and contents changed), each kept apart from what the toolset
//! did to the same object.

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_gff::{Struct, Value};
use mg_rules::{GameData, ObjectKind};
use serde_json::Value as Json;

use super::{
    GRID_WIDTH, MARK, MAX_STOCK, PANEL_PAGE, STORE_PAGES, apply_equipment, blueprint, fields_of, first_free, item_fit, kind_name,
    name_label, set_creatures, set_field, slot_name,
};

/// The GIT lists an area's objects are in.
pub const LISTS: [&str; 9] =
    ["Creature List", "Door List", "Encounter List", "List", "Placeable List", "SoundList", "StoreList", "TriggerList", "WaypointList"];

/// The local variable on an imported area's original objects: their place in the original's list
/// (`"Creature List:3"`), so an edit made in the editor (taken out, moved) finds them again on a re-export.
pub const ORIG: &str = "ls_orig";

/// `"<list>:<index>:<tag>"`. The tag is there so that an edit can't land on another object if the original's
/// lists shift (an object added or taken out of the module's area): the key then matches nothing.
pub fn orig_key(list: &str, index: usize, tag: &str) -> String {
    format!("{list}:{index}:{tag}")
}

/// The list and index a key names.
pub fn orig_place(key: &str) -> Option<(&str, usize)> {
    let mut p = key.splitn(3, ':');
    let (list, index) = (p.next()?, p.next()?.parse().ok()?);
    Some((list, index))
}

/// The place in the original's lists an original object was marked with.
pub fn orig_mark(s: &Struct) -> Option<String> {
    s.list("VarTable")?
        .iter()
        .find(|v| v.string("Name") == Some(ORIG.as_bytes()))
        .and_then(|v| v.string("Value"))
        .map(|b| String::from_utf8_lossy(b).into_owned())
}

/// Where an object stands: (x, y, z) and its facing (radians anticlockwise from east), as its list keeps them.
pub fn position_of(list: &str, s: &Struct) -> Option<([f32; 3], f32)> {
    let f = |l: &str| s.float(l);
    if matches!(list, "Placeable List" | "Door List") {
        Some(([f("X")?, f("Y")?, f("Z").unwrap_or(0.0)], f("Bearing").unwrap_or(0.0)))
    } else {
        let facing = match (f("XOrientation"), f("YOrientation")) {
            (Some(x), Some(y)) if x != 0.0 || y != 0.0 => y.atan2(x),
            _ => 0.0,
        };
        Some(([f("XPosition")?, f("YPosition")?, f("ZPosition").unwrap_or(0.0)], facing))
    }
}

/// The kind of object a GIT list holds.
pub fn kind_of_list(list: &str) -> Option<ObjectKind> {
    ObjectKind::ALL.iter().copied().find(|k| k.list() == list)
}

/// The local variables of an object that are its own (not the editor's marks), as the editor lists them:
/// `[{name, type: int|float|string, value}]`.
pub fn plain_vars(s: &Struct) -> Vec<Json> {
    let marks = [crate::marker::VAR, MARK, ORIG];
    s.list("VarTable")
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let name = String::from_utf8_lossy(e.string("Name")?).into_owned();
            if marks.contains(&name.as_str()) {
                return None;
            }
            Some(match e.integer("Type")? {
                1 => serde_json::json!({ "name": name, "type": "int", "value": e.integer("Value")? }),
                2 => serde_json::json!({ "name": name, "type": "float", "value": e.float("Value")? }),
                3 => serde_json::json!({ "name": name, "type": "string", "value": String::from_utf8_lossy(e.string("Value")?) }),
                _ => return None,
            })
        })
        .collect()
}

/// What an original object holds now, for the editor's list: a container's, creature's or store's items (resref, name,
/// infinite) or an encounter's creatures, null for the rest.
pub fn current_contents(game: &GameData, kind: ObjectKind, s: &Struct) -> Json {
    let item = |e: &Struct| {
        let r = e.resref("TemplateResRef").map_or_default(|r| r.to_string());
        let name = crate::blueprints::struct_name(game, e, "LocalizedName");
        serde_json::json!({ "resref": r, "name": name, "infinite": e.integer("Infinite").unwrap_or(0) != 0 })
    };
    match kind {
        ObjectKind::Placeable | ObjectKind::Creature => {
            let worn: Vec<Json> = s
                .list("Equip_ItemList")
                .unwrap_or(&[])
                .iter()
                .filter_map(|e| {
                    let (slot, resref) = (slot_name(e.id)?, e.resref("TemplateResRef")?.to_string());
                    let name = crate::blueprints::struct_name(game, e, "LocalizedName");
                    Some(serde_json::json!({ "slot": slot, "resref": resref, "name": name }))
                })
                .collect();
            let container = kind == ObjectKind::Creature || s.integer("HasInventory").unwrap_or(0) != 0;
            let items: Vec<Json> = s.list("ItemList").unwrap_or(&[]).iter().map(item).collect();
            serde_json::json!({ "container": container, "items": items, "equipment": worn })
        }
        ObjectKind::Store => {
            let items: Vec<Json> =
                s.list("StoreList").unwrap_or(&[]).iter().flat_map(|pg| pg.list("ItemList").unwrap_or(&[]).iter().map(item)).collect();
            serde_json::json!({ "container": true, "items": items })
        }
        ObjectKind::Encounter => {
            let name = |r: &str| {
                blueprint(game, ResType::UTC, r).ok().flatten().map_or_default(|b| crate::blueprints::struct_name(game, &b, "FirstName"))
            };
            let creatures: Vec<Json> = s
                .list("CreatureList")
                .unwrap_or(&[])
                .iter()
                .map(|e| {
                    let r = e.resref("ResRef").map_or_default(|r| r.to_string());
                    serde_json::json!({ "resref": r, "name": name(&r) })
                })
                .collect();
            serde_json::json!({ "creatures": creatures })
        }
        _ => Json::Null,
    }
}

/// Changes what an original container, creature or store holds, or an encounter spawns, to the editor's list. The
/// entries already there (they may have been changed in the toolset: stacks, properties) are kept; the ones the list
/// no longer has go; new ones are made from their blueprints as Aurora holds them, at the first free place.
pub fn apply_contents(game: &GameData, kind: ObjectKind, s: &mut Struct, e: &Json) -> Result<()> {
    if kind == ObjectKind::Encounter {
        if let Some(cs) = e["creatures"].as_array() {
            let list: Vec<String> = cs.iter().filter_map(|c| c["resref"].as_str().map(str::to_ascii_lowercase)).collect();
            set_creatures(game, s, &list)?;
        }
        return Ok(());
    }
    if kind == ObjectKind::Creature
        && let Some(eq) = e["equipment"].as_array()
    {
        apply_equipment(game, s, eq)?;
    }
    let Some(inv) = e["inventory"].as_array() else { return Ok(()) };
    anyhow::ensure!(
        matches!(kind, ObjectKind::Placeable | ObjectKind::Creature | ObjectKind::Store),
        "a {} holds no items",
        kind_name(kind)
    );
    let wanted: Vec<(String, bool)> =
        inv.iter().filter_map(|i| Some((i["resref"].as_str()?.to_ascii_lowercase(), i["infinite"].as_bool().unwrap_or(false)))).collect();
    anyhow::ensure!(wanted.len() <= MAX_STOCK, "up to {MAX_STOCK} items");
    let item_bp = |r: ResRef| blueprint(game, ResType::UTI, &r.to_string()).ok().flatten();
    let placing = mg_module::instances::Placing { game, item: &item_bp };
    // The entries now held, by page for a store (a page is its own grid).
    let existing: Vec<(usize, Struct)> = if kind == ObjectKind::Store {
        s.list("StoreList")
            .unwrap_or(&[])
            .iter()
            .flat_map(|pg| {
                let page = PANEL_PAGE.iter().position(|&p| p == pg.id).unwrap_or(4);
                pg.list("ItemList").unwrap_or(&[]).iter().cloned().map(move |i| (page, i))
            })
            .collect()
    } else {
        s.list("ItemList").unwrap_or(&[]).iter().cloned().map(|i| (0, i)).collect()
    };
    let mut pool: Vec<Option<(usize, Struct)>> = existing.into_iter().map(Some).collect();
    let mut kept: Vec<(usize, Struct)> = Vec::new();
    let mut fresh: Vec<(String, bool)> = Vec::new();
    for (r, inf) in &wanted {
        let found = pool.iter_mut().find(|slot| {
            slot.as_ref().is_some_and(|(_, it)| it.resref("TemplateResRef").is_some_and(|t| t.to_string().eq_ignore_ascii_case(r)))
        });
        match found.and_then(Option::take) {
            Some((page, mut it)) => {
                if kind == ObjectKind::Store {
                    it.set("Infinite", Value::Byte(u8::from(*inf)));
                }
                kept.push((page, it));
            }
            None => fresh.push((r.clone(), *inf)),
        }
    }
    let fit_of = |it: &Struct| {
        let base = it.integer("BaseItem").unwrap_or(-1);
        let table = game.table("baseitems").ok();
        let cell = |col: &str| table.as_ref().and_then(|t| t.get_int(usize::try_from(base).ok()?, col)).and_then(|v| u32::try_from(v).ok());
        (cell("InvSlotWidth").unwrap_or(1).clamp(1, GRID_WIDTH), cell("InvSlotHeight").unwrap_or(1).max(1))
    };
    let mut taken: [Vec<(u32, u32, u32, u32)>; 5] = Default::default();
    for (page, it) in &kept {
        let (w, h) = fit_of(it);
        taken[*page].push((it.integer("Repos_PosX").unwrap_or(0) as u32, it.integer("Repos_Posy").unwrap_or(0) as u32, w, h));
    }
    for (r, inf) in fresh {
        let bp = blueprint(game, ResType::UTI, &r)?.with_context(|| format!("no item blueprint {r}"))?;
        let (panel, w, h) = item_fit(game, &r)?;
        let page = if kind == ObjectKind::Store { panel } else { 0 };
        let at = first_free(&taken[page], w, h);
        taken[page].push((at.0, at.1, w, h));
        let mut it = mg_module::instances::held(&placing, &bp, 0);
        it.set("Repos_PosX", Value::Word(at.0 as u16));
        it.set("Repos_Posy", Value::Word(at.1 as u16));
        if kind == ObjectKind::Store {
            it.set("Infinite", Value::Byte(u8::from(inf)));
        }
        kept.push((page, it));
    }
    if kind == ObjectKind::Store {
        let pages: Vec<Struct> = STORE_PAGES
            .iter()
            .map(|&id| {
                let panel = PANEL_PAGE.iter().position(|&p| p == id).unwrap_or(4);
                let mut page = Struct::new(id);
                let items: Vec<Struct> = kept
                    .iter()
                    .filter(|(p, _)| *p == panel)
                    .enumerate()
                    .map(|(i, (_, it))| {
                        let mut it = it.clone();
                        it.id = i as u32;
                        it
                    })
                    .collect();
                page.set("ItemList", Value::List(items));
                page
            })
            .collect();
        s.set("StoreList", Value::List(pages));
    } else {
        let items: Vec<Struct> = kept
            .into_iter()
            .enumerate()
            .map(|(i, (_, mut it))| {
                it.id = i as u32;
                it
            })
            .collect();
        s.set("ItemList", Value::List(items));
    }
    Ok(())
}

/// The settings the editor may edit on an object, as they stand: `{label: value}` for each of the kind's
/// settings that the object has (scripts and conversation as text, flags and numbers as numbers).
pub fn current_fields(game: &GameData, kind: ObjectKind, s: &Struct) -> Json {
    let mut out = serde_json::Map::new();
    for (label, ty) in fields_of(kind) {
        let v = match ty {
            'r' => s.resref(label).map(|r| Json::from(r.to_string())),
            's' => s.string(label).map(|b| Json::from(String::from_utf8_lossy(b).into_owned())),
            'l' => s.locstring(label).map(|_| Json::from(crate::blueprints::struct_name(game, s, label))),
            'f' => s.float(label).map(|f| Json::from(f64::from(f))),
            _ => s.integer(label).map(Json::from),
        };
        if let Some(v) = v {
            out.insert(label.to_string(), v);
        }
    }
    Json::Object(out)
}

/// A blueprint's own values for the settings the editor may set on a thing of `kind` made from it (see [`current_fields`]):
/// `{ "kind", "resref", "fields": { label: value } }`. A setting the blueprint doesn't have is left out (the game then reads
/// it as 0, empty or none).
pub fn blueprint_fields(game: &GameData, kind: ObjectKind, resref: &str) -> Result<Json> {
    let bp = blueprint(game, kind.restype(), resref)?.with_context(|| format!("no {} blueprint {resref}", kind_name(kind)))?;
    Ok(serde_json::json!({ "kind": kind_name(kind), "resref": resref, "fields": current_fields(game, kind, &bp) }))
}

/// Applies the editor's changes to an original object's properties: `{ tag?, name?, fields?: {label: value|null},
/// vars? }`. Only what is named changes; a field set to null is taken out; `vars`, when given, is the object's whole
/// list of its own variables (the editor's marks stay).
pub fn apply_edit(kind: ObjectKind, s: &mut Struct, e: &Json) {
    if let Some(tag) = e["tag"].as_str().filter(|t| t.len() <= 32 && !t.contains(char::is_whitespace)) {
        s.set("Tag", Value::String(tag.as_bytes().to_vec()));
    }
    if let Some(name) = e["name"].as_str() {
        s.set(name_label(kind), crate::settings::loc(name));
        if kind == ObjectKind::Creature && s.get("LastName").is_some() {
            s.set("LastName", crate::settings::loc(""));
        }
    }
    let allowed = fields_of(kind);
    for (label, v) in e["fields"].as_object().into_iter().flatten() {
        let Some((l, ty)) = allowed.iter().find(|(l, _)| l == label) else { continue };
        if v.is_null() {
            s.remove(l);
        } else {
            let _ = set_field(s, l, *ty, v);
        }
    }
    if let Some(vars) = e["vars"].as_array() {
        let marks = [crate::marker::VAR, MARK, ORIG];
        let mut list: Vec<Struct> = s.list("VarTable").map_or_default(<[_]>::to_vec);
        list.retain(|v| v.string("Name").is_some_and(|n| marks.contains(&String::from_utf8_lossy(n).as_ref())));
        if let Some(Value::List(new)) = crate::settings::var_table(vars) {
            list.extend(new);
        }
        s.set("VarTable", Value::List(list));
    }
}

/// Undoes what `apply_edit` of `old` changed, from the original object `og`: its tag, name, those settings and its
/// variables as they were.
pub fn revert_edit(kind: ObjectKind, s: &mut Struct, og: &Struct, old: &Json) {
    let copy = |s: &mut Struct, label: &str| match og.get(label) {
        Some(v) => s.set(label, v.clone()),
        None => {
            s.remove(label);
        }
    };
    if old.get("tag").is_some() {
        copy(s, "Tag");
    }
    if old.get("name").is_some() {
        copy(s, name_label(kind));
        if kind == ObjectKind::Creature {
            copy(s, "LastName");
        }
    }
    for label in old["fields"].as_object().into_iter().flat_map(|f| f.keys()) {
        if fields_of(kind).iter().any(|(l, _)| l == label) {
            copy(s, label);
        }
    }
    if old.get("inventory").is_some() {
        copy(s, if kind == ObjectKind::Store { "StoreList" } else { "ItemList" });
    }
    if old.get("equipment").is_some() {
        copy(s, "Equip_ItemList");
    }
    if old.get("creatures").is_some() {
        copy(s, "CreatureList");
    }
    if old.get("vars").is_some() {
        let marks = [crate::marker::VAR, MARK, ORIG];
        let mut list: Vec<Struct> = s.list("VarTable").map_or_default(<[_]>::to_vec);
        list.retain(|v| v.string("Name").is_some_and(|n| marks.contains(&String::from_utf8_lossy(n).as_ref())));
        list.extend(
            og.list("VarTable")
                .unwrap_or(&[])
                .iter()
                .filter(|v| v.string("Name").is_none_or(|n| !marks.contains(&String::from_utf8_lossy(n).as_ref())))
                .cloned(),
        );
        s.set("VarTable", Value::List(list));
    }
}

/// A trigger's or encounter's outline in absolute coordinates (its points are stored relative to where it stands).
pub fn outline_of(list: &str, s: &Struct) -> Option<Vec<[f32; 2]>> {
    let (px, py) = if list == "TriggerList" { ("PointX", "PointY") } else { ("X", "Y") };
    let ([x, y, _], _) = position_of(list, s)?;
    let g = s.list("Geometry").filter(|g| g.len() >= 3)?;
    Some(g.iter().map(|p| [x + p.float(px).unwrap_or(0.0), y + p.float(py).unwrap_or(0.0)]).collect())
}

/// Moves an object to `m` (`{ x, y, bearing?, outline? }`), keeping its height above the ground.
pub fn apply_move(list: &str, s: &mut Struct, m: &Json, ground: &dyn Fn(f32, f32) -> f32) {
    let Some(([ox, oy, oz], facing)) = position_of(list, s) else { return };
    let (Some(x), Some(y)) = (m["x"].as_f64().map(|v| v as f32), m["y"].as_f64().map(|v| v as f32)) else { return };
    let bearing = m["bearing"].as_f64().map_or(facing, |v| v as f32);
    let z = ground(x, y) + (oz - ground(ox, oy));
    let (dx, dy) = (x - ox, y - oy);
    // An outline's points are relative to where it stands (their heights absolute); an encounter's spawn
    // points are absolute: they move with it, each keeping its height above the ground.
    if let Some(points) = s.list_mut("Geometry") {
        let labels = if list == "TriggerList" { ["PointX", "PointY", "PointZ"] } else { ["X", "Y", "Z"] };
        for p in points {
            let (px, py, pz) = (p.float(labels[0]).unwrap_or(0.0), p.float(labels[1]).unwrap_or(0.0), p.float(labels[2]).unwrap_or(0.0));
            p.set(labels[2], Value::Float(pz + ground(x + px, y + py) - ground(ox + px, oy + py)));
        }
    }
    // The editor reshaped the outline (corners moved, added or taken out): the points are rewritten from its absolute
    // corners, relative to where the object now stands, at the ground's height.
    let corners: Vec<(f32, f32)> =
        m["outline"].as_array().map_or_default(|a| a.iter().filter_map(|p| Some((p[0].as_f64()? as f32, p[1].as_f64()? as f32))).collect());
    if corners.len() >= 3
        && let Some(points) = s.list_mut("Geometry")
    {
        let labels = if list == "TriggerList" { ["PointX", "PointY", "PointZ"] } else { ["X", "Y", "Z"] };
        let proto = points.first().cloned().unwrap_or_else(|| Struct::new(0));
        *points = corners
            .iter()
            .map(|&(cx, cy)| {
                let mut p = proto.clone();
                p.set(labels[0], Value::Float(cx - x));
                p.set(labels[1], Value::Float(cy - y));
                p.set(labels[2], Value::Float(ground(cx, cy) + mg_module::instances::OUTLINE_LIFT));
                p
            })
            .collect();
    }
    if let Some(points) = s.list_mut("SpawnPointList") {
        for p in points {
            let (px, py, pz) = (p.float("X").unwrap_or(0.0), p.float("Y").unwrap_or(0.0), p.float("Z").unwrap_or(0.0));
            p.set("X", Value::Float(px + dx));
            p.set("Y", Value::Float(py + dy));
            p.set("Z", Value::Float(pz + ground(px + dx, py + dy) - ground(px, py)));
        }
    }
    if matches!(list, "Placeable List" | "Door List") {
        for (l, v) in [("X", x), ("Y", y), ("Z", z), ("Bearing", bearing)] {
            s.set(l, Value::Float(v));
        }
    } else {
        for (l, v) in [("XPosition", x), ("YPosition", y), ("ZPosition", z)] {
            s.set(l, Value::Float(v));
        }
        // Sounds, triggers and encounters don't face anywhere.
        if !matches!(list, "SoundList" | "TriggerList" | "Encounter List") {
            s.set("XOrientation", Value::Float(bearing.cos()));
            s.set("YOrientation", Value::Float(bearing.sin()));
        }
    }
}

/// What the editor shows of an imported area's original objects: `[{ key, list, kind, tag, resref, name, x, y,
/// bearing, outline?, model? }]`, for the editor to draw them, take them out or move them. A door's or placeable's `model` is
/// its own appearance's (null when it names none): the toolset often gives a door another look than its blueprint's.
pub fn describe_originals(game: &GameData, git: &Struct) -> Vec<Json> {
    let mut out = Vec::new();
    for list in LISTS {
        let Some(kind) = ObjectKind::ALL.iter().copied().find(|k| k.list() == list) else { continue };
        for (i, s) in git.list(list).unwrap_or(&[]).iter().enumerate() {
            let Some(([x, y, _], facing)) = position_of(list, s) else { continue };
            let text = |l: &str| s.string(l).map_or_default(|b| String::from_utf8_lossy(b).into_owned());
            let name = crate::blueprints::struct_name(game, s, name_label(kind));
            let outline = outline_of(list, s);
            let mut o = serde_json::json!({
                "key": orig_key(list, i, &text("Tag")), "list": list, "kind": kind_name(kind), "tag": text("Tag"),
                "resref": s.resref("TemplateResRef").map_or_default(|r| r.to_string()), "name": name,
                "x": x, "y": y, "bearing": facing, "outline": outline,
                "fields": current_fields(game, kind, s), "vars": plain_vars(s), "contents": current_contents(game, kind, s),
            });
            if matches!(kind, ObjectKind::Door | ObjectKind::Placeable) {
                o["model"] = super::struct_model(game, kind, s).into();
            }
            out.push(o);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::things::{add_var, mark_of};

    fn placeable(x: f32, y: f32, z: f32) -> Struct {
        let mut s = Struct::new(9);
        for (l, v) in [("X", x), ("Y", y), ("Z", z), ("Bearing", 0.5)] {
            s.set(l, Value::Float(v));
        }
        s
    }

    #[test]
    fn a_moved_object_keeps_its_height_above_the_ground() {
        // The ground rises 1 m for every 10 m east.
        let ground = |x: f32, _y: f32| x / 10.0;
        let mut s = placeable(10.0, 5.0, 3.0);
        apply_move("Placeable List", &mut s, &serde_json::json!({ "x": 30.0, "y": 8.0, "bearing": 1.0 }), &ground);
        assert_eq!((s.float("X"), s.float("Y"), s.float("Bearing")), (Some(30.0), Some(8.0), Some(1.0)));
        assert_eq!(s.float("Z"), Some(5.0)); // 2 m above its ground, as it was (3 - 1)
    }

    #[test]
    fn creatures_turn_by_their_facing_vector_and_sounds_do_not_face() {
        let mut c = Struct::new(4);
        for (l, v) in [("XPosition", 1.0), ("YPosition", 1.0), ("ZPosition", 0.0), ("XOrientation", 0.0), ("YOrientation", 1.0)] {
            c.set(l, Value::Float(v));
        }
        assert!((position_of("Creature List", &c).unwrap().1 - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        apply_move("Creature List", &mut c, &serde_json::json!({ "x": 2.0, "y": 3.0, "bearing": 0.0 }), &|_, _| 0.0);
        assert_eq!((c.float("XPosition"), c.float("XOrientation"), c.float("YOrientation")), (Some(2.0), Some(1.0), Some(0.0)));
        let mut snd = Struct::new(0);
        for (l, v) in [("XPosition", 1.0), ("YPosition", 1.0), ("ZPosition", 0.0)] {
            snd.set(l, Value::Float(v));
        }
        apply_move("SoundList", &mut snd, &serde_json::json!({ "x": 5.0, "y": 5.0 }), &|_, _| 0.0);
        assert!(snd.float("XOrientation").is_none());
    }

    #[test]
    fn an_original_is_found_again_by_its_mark() {
        let mut s = placeable(1.0, 1.0, 0.0);
        add_var(&mut s, ORIG, &serde_json::json!({ "type": "string", "value": orig_key("Placeable List", 7, "chest") }));
        assert_eq!(orig_mark(&s).as_deref(), Some("Placeable List:7:chest"));
        assert_eq!(orig_place("Placeable List:7:chest"), Some(("Placeable List", 7)));
        assert_eq!(orig_place("Creature List:3:a:b"), Some(("Creature List", 3)));
        assert_eq!(mark_of(&s), None);
    }

    fn trigger() -> Struct {
        let mut s = Struct::new(0);
        for (l, v) in [("XPosition", 10.0), ("YPosition", 20.0), ("ZPosition", 1.0)] {
            s.set(l, Value::Float(v));
        }
        let pts: Vec<Struct> = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)]
            .iter()
            .map(|&(x, y)| {
                let mut p = Struct::new(0);
                p.set("PointX", Value::Float(x));
                p.set("PointY", Value::Float(y));
                p.set("PointZ", Value::Float(1.2));
                p
            })
            .collect();
        s.set("Geometry", Value::List(pts));
        s
    }

    #[test]
    fn an_outline_reads_as_absolute_corners() {
        assert_eq!(outline_of("TriggerList", &trigger()).unwrap(), vec![[10.0, 20.0], [14.0, 20.0], [14.0, 24.0]]);
    }

    #[test]
    fn a_reshaped_outline_is_written_relative_to_where_the_object_stands_with_any_number_of_corners() {
        let mut s = trigger();
        let m = serde_json::json!({ "x": 10.0, "y": 20.0, "outline": [[10, 20], [16, 20], [16, 26], [12, 28], [10, 24]] });
        apply_move("TriggerList", &mut s, &m, &|_, _| 0.0);
        let g = s.list("Geometry").unwrap();
        assert_eq!(g.len(), 5);
        assert_eq!((g[1].float("PointX"), g[1].float("PointY")), (Some(6.0), Some(0.0)));
        assert_eq!((g[3].float("PointX"), g[3].float("PointY")), (Some(2.0), Some(8.0)));
        assert!(g.iter().all(|p| p.float("PointZ").is_some_and(|z| z > 0.0)));
        // read back: the same corners
        assert_eq!(outline_of("TriggerList", &s).unwrap().len(), 5);
        // moved without a new outline: the points stay relative, so it all moves
        let mut s = trigger();
        apply_move("TriggerList", &mut s, &serde_json::json!({ "x": 30.0, "y": 40.0 }), &|_, _| 0.0);
        assert_eq!(outline_of("TriggerList", &s).unwrap()[1], [34.0, 40.0]);
        // fewer than three corners is ignored
        let mut s = trigger();
        apply_move("TriggerList", &mut s, &serde_json::json!({ "x": 10.0, "y": 20.0, "outline": [[1, 1], [2, 2]] }), &|_, _| 0.0);
        assert_eq!(s.list("Geometry").unwrap().len(), 3);
    }

    fn var(name: &str, v: i32) -> Struct {
        let mut e = Struct::new(0);
        e.set("Name", Value::String(name.as_bytes().to_vec()));
        e.set("Type", Value::Dword(1));
        e.set("Value", Value::Int(v));
        e
    }

    fn creature() -> Struct {
        let mut s = Struct::new(0);
        s.set("Tag", Value::String(b"old_tag".to_vec()));
        s.set("FirstName", crate::settings::loc("Old Name"));
        s.set("ScriptSpawn", Value::resref(ResRef::from_str("oldspawn").unwrap()));
        s.set("ScriptDeath", Value::resref(ResRef::from_str("olddeath").unwrap()));
        s.set("VarTable", Value::List(vec![var("kept", 1), var(ORIG, 9), var("mine", 2)]));
        s
    }

    fn names(s: &Struct) -> Vec<String> {
        s.list("VarTable").unwrap().iter().map(|v| String::from_utf8_lossy(v.string("Name").unwrap()).into_owned()).collect()
    }

    #[test]
    fn an_edit_changes_what_it_names_and_leaves_the_rest() {
        let mut s = creature();
        let e = serde_json::json!({
            "tag": "new_tag", "name": "New Name", "fields": { "ScriptSpawn": "newspawn", "ScriptDeath": null, "Plot": 1 },
            "vars": [{ "name": "fresh", "type": "int", "value": 5 }],
        });
        apply_edit(ObjectKind::Creature, &mut s, &e);
        assert_eq!(s.string("Tag"), Some(&b"new_tag"[..]));
        assert_eq!(s.resref("ScriptSpawn").unwrap().to_string(), "newspawn");
        assert!(s.get("ScriptDeath").is_none(), "null takes a setting out");
        assert_eq!(s.integer("Plot"), Some(1));
        // the object's own variables are replaced, the editor's mark stays
        assert_eq!(names(&s), vec![ORIG.to_string(), "fresh".to_string()]);
        // an edit that names nothing changes nothing; a setting that isn't the kind's is ignored
        let mut t = creature();
        apply_edit(ObjectKind::Creature, &mut t, &serde_json::json!({ "fields": { "OnUsed": "x" } }));
        assert_eq!(t, creature());
    }

    #[test]
    fn reverting_puts_the_original_back() {
        let og = creature();
        let mut s = creature();
        let e = serde_json::json!({
            "tag": "new_tag", "name": "New Name", "fields": { "ScriptSpawn": "newspawn", "ScriptDeath": null }, "vars": [],
        });
        apply_edit(ObjectKind::Creature, &mut s, &e);
        revert_edit(ObjectKind::Creature, &mut s, &og, &e);
        assert_eq!(s.string("Tag"), og.string("Tag"));
        assert_eq!(s.resref("ScriptSpawn"), og.resref("ScriptSpawn"));
        assert_eq!(s.resref("ScriptDeath"), og.resref("ScriptDeath"));
        assert_eq!(names(&s), vec![ORIG.to_string(), "kept".to_string(), "mine".to_string()]);
    }

    #[test]
    fn an_original_door_is_drawn_with_its_own_look() {
        // A generic door (Appearance 0: genericdoors.2da's GenericType_New row), a tile door (doortypes.2da's row), and one
        // whose model the game hasn't.
        let mut rm = mg_resman::ResMan::new();
        let mut mem = mg_resman::MemContainer::new();
        let table = |head: &str, rows: &str| format!("2DA V2.0\n\n {head}\n{rows}").into_bytes();
        mem.insert(
            mg_resman::ResKey::parse("genericdoors", ResType::TWODA).unwrap(),
            table("Label ModelName", "0 Wood door_x\n1 Iron Door_Iron"),
        );
        mem.insert(
            mg_resman::ResKey::parse("doortypes", ResType::TWODA).unwrap(),
            table("Label Model", "0 Generic ****\n1 Gate tic01_d01\n2 Lost gone"),
        );
        for m in ["door_x", "door_iron", "tic01_d01"] {
            mem.insert(mg_resman::ResKey::parse(m, ResType::MDL).unwrap(), b"model".to_vec());
        }
        rm.add(mg_resman::priority::KEY, "key:data", mg_resman::LayerClass::Key, mem);
        let game = GameData::new(rm, mg_tlk::Tlk::new(mg_core::Language::ENGLISH));
        let door = |appearance: u32, generic: u8| {
            let mut s = placeable(1.0, 1.0, 0.0);
            s.set("Appearance", Value::Dword(appearance));
            s.set("GenericType_New", Value::Byte(generic));
            s
        };
        let mut git = Struct::new(0);
        git.set("Door List", Value::List(vec![door(0, 1), door(1, 0), door(2, 0)]));
        let models: Vec<Json> = describe_originals(&game, &git).iter().map(|o| o["model"].clone()).collect();
        assert_eq!(models, [serde_json::json!("door_iron"), serde_json::json!("tic01_d01"), Json::Null]);
    }

    #[test]
    fn plain_vars_leave_out_the_marks() {
        let v = plain_vars(&creature());
        assert_eq!(v.iter().map(|x| x["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["kept", "mine"]);
        assert_eq!(v[0]["type"], "int");
    }
}
