// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Things placed in an area beyond the terrain's own placeables: creatures, doors, encounters, items on the
//! ground, sounds, stores, triggers (area transitions too), waypoints and real placeables. Each is a
//! blueprint (from the module's `core/`, else the game's and the haks') placed where the editor says,
//! with the overrides the builder gave (tag, name, scripts, a few fields, local variables). Moonglow's
//! `mg_module::instances::instance` writes the GIT entry as Aurora would.
//!
//! ```json
//! { "id": 7, "kind": "creature", "blueprint": "nw_goblina", "x": 12.5, "y": 30, "bearing": 1.57,
//!   "outline": [[10, 10], [20, 10], [20, 20]],            // triggers and encounters
//!   "tag": "GOBLIN_1", "name": "A goblin", "vars": [{ "name": "n", "type": "int", "value": 1 }],
//!   "fields": { "ScriptSpawn": "gob_spawn" } }
//! ```
//! Every instance carries a local variable `ls_thing` (the editor's id), so a re-export can tell its own
//! objects from what a toolset added: its own are made again, everything else stays.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_gff::{Struct, Value};
use mg_resman::ResKey;
use mg_rules::{GameData, ObjectKind};
use serde_json::Value as Json;

mod originals;

pub use self::originals::*;

/// The local variable that marks an instance as the editor's.
pub const MARK: &str = "ls_thing";

/// A thing the editor placed: a blueprint, where it stands, and what the builder changed on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Thing {
    /// The editor's id: marked on the instance (`ls_thing`), so a re-export finds its own objects.
    pub id: u32,
    /// What it is: creature, door, trigger...
    pub kind: ObjectKind,
    /// The blueprint's resref; empty: a new, plain one.
    pub blueprint: String,
    /// Where it stands, east (area metres).
    pub x: f32,
    /// Where it stands, north (area metres).
    pub y: f32,
    /// Its facing: radians anticlockwise from east, as the game's.
    pub bearing: f32,
    /// Triggers' and encounters' outline, area metres.
    pub outline: Vec<[f32; 2]>,
    /// Its tag, over the blueprint's.
    pub tag: Option<String>,
    /// Its name, over the blueprint's.
    pub name: Option<String>,
    /// Local variables to add: `[{name, type: int|float|string, value}]`.
    pub vars: Vec<Json>,
    /// Settings over the blueprint's, by GFF label (only those [`FIELDS`] allows).
    pub fields: Vec<(String, Json)>,
    /// What a container, creature or store holds, replacing the blueprint's: (item blueprint, infinite). None: the
    /// blueprint's own.
    pub inventory: Option<Vec<(String, bool)>>,
    /// The creature blueprints an encounter spawns, replacing the blueprint's. None: the blueprint's own.
    pub creatures: Option<Vec<String>>,
    /// What a creature wears and wields, (slot, item blueprint), replacing the blueprint's. None: the blueprint's own.
    pub equipment: Option<Vec<(String, String)>>,
}

/// The kind the editor names `name` (`creature`, `door`...); None for a name it doesn't use.
pub fn kind_of(name: &str) -> Option<ObjectKind> {
    Some(match name {
        "creature" => ObjectKind::Creature,
        "door" => ObjectKind::Door,
        "encounter" => ObjectKind::Encounter,
        "item" => ObjectKind::Item,
        "placeable" => ObjectKind::Placeable,
        "sound" => ObjectKind::Sound,
        "store" => ObjectKind::Store,
        "trigger" => ObjectKind::Trigger,
        "waypoint" => ObjectKind::Waypoint,
        _ => return None,
    })
}

/// The editor's name for a kind, the other way from [`kind_of`].
pub fn kind_name(k: ObjectKind) -> &'static str {
    match k {
        ObjectKind::Creature => "creature",
        ObjectKind::Door => "door",
        ObjectKind::Encounter => "encounter",
        ObjectKind::Item => "item",
        ObjectKind::Placeable => "placeable",
        ObjectKind::Sound => "sound",
        ObjectKind::Store => "store",
        ObjectKind::Trigger => "trigger",
        ObjectKind::Waypoint => "waypoint",
    }
}

/// The file extension of a kind's blueprints (`utc`).
pub fn ext(k: ObjectKind) -> String {
    k.restype().to_string().to_ascii_lowercase()
}

impl Thing {
    /// A thing from the editor's JSON; what is wrong with it if it can't be read (a thing that vanished
    /// silently from an export would be worse than an export that says why it can't be made).
    pub fn parse(j: &Json) -> std::result::Result<Thing, String> {
        let f = |k: &str| j[k].as_f64().map(|v| v as f32);
        let id = j["id"].as_u64().ok_or("a thing without an id")? as u32;
        let bad = |what: &str| format!("thing {id}: {what}");
        let mut outline = Vec::new();
        for p in j["outline"].as_array().into_iter().flatten() {
            match (p[0].as_f64(), p[1].as_f64()) {
                (Some(x), Some(y)) => outline.push([x as f32, y as f32]),
                _ => return Err(bad("an outline point isn't a pair of numbers")),
            }
        }
        Ok(Thing {
            id,
            kind: kind_of(j["kind"].as_str().unwrap_or("")).ok_or_else(|| bad(&format!("unknown kind {:?}", j["kind"])))?,
            blueprint: j["blueprint"].as_str().unwrap_or("").to_ascii_lowercase(),
            x: f("x").ok_or_else(|| bad("no east position"))?,
            y: f("y").ok_or_else(|| bad("no north position"))?,
            bearing: f("bearing").unwrap_or(0.0),
            outline,
            tag: j["tag"].as_str().map(str::to_string).filter(|t| !t.is_empty()),
            name: j["name"].as_str().map(str::to_string).filter(|t| !t.is_empty()),
            vars: j["vars"].as_array().cloned().unwrap_or_default(),
            fields: j["fields"].as_object().map_or_default(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            inventory: j["inventory"].as_array().map(|a| {
                a.iter()
                    .filter_map(|i| Some((i["resref"].as_str()?.to_ascii_lowercase(), i["infinite"].as_bool().unwrap_or(false))))
                    .collect()
            }),
            creatures: j["creatures"]
                .as_array()
                .map(|a| a.iter().filter_map(|i| Some(i["resref"].as_str()?.to_ascii_lowercase())).collect()),
            equipment: j["equipment"].as_array().map(|a| {
                a.iter().filter_map(|i| Some((i["slot"].as_str()?.to_string(), i["resref"].as_str()?.to_ascii_lowercase()))).collect()
            }),
        })
    }
}

// --- Blueprints ----------------------------------------------------------------------------

/// Where a module's blueprints come from: the review service on a team server that has it set up, else the module checkout
/// (`LANDSCAPER_MODULE_REPO`, laid out as config::Layout says).
fn module_json(ext: &str, resref: &str) -> Result<Option<String>> {
    if crate::review::active() {
        return crate::review::blueprint(ext, resref);
    }
    Ok(std::fs::read_to_string(crate::config::Layout::from_env().blueprint_dir(ext).join(format!("{resref}.{ext}.json"))).ok())
}

/// A GFF JSON document (nwn_gff's, as the module repository keeps it) as a struct.
pub(crate) fn from_json(text: &str, ext: &str, resref: &str) -> Result<Struct> {
    Ok(nwn_bake::module::gff_from(&format!("{resref}.{ext}.json"), text.as_bytes())?.root)
}

/// The module's and the game's blueprints looked up (and the ones not found) are kept a couple of minutes: an edited or newly
/// added one is seen soon. The same for everyone, so a builder's own new blueprints (in their workspace) are never kept here.
static CACHE: Mutex<Option<HashMap<(String, String), (std::time::Instant, Option<Struct>)>>> = Mutex::new(None);
const CACHE_SECS: u64 = 120;

/// A blueprint: the builder's own new one (`newbp`), else the module's, else the game's (haks and base game); None when there
/// is none.
pub fn blueprint(game: &GameData, restype: ResType, resref: &str) -> Result<Option<Struct>> {
    let e = restype.to_string().to_ascii_lowercase();
    anyhow::ensure!(resref.len() <= 16 && resref.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "bad blueprint name {resref:?}");
    blueprint_from(&e, resref, crate::newbp::read(&e, resref), || {
        Ok(match module_json(&e, resref)? {
            Some(text) => Some(from_json(&text, &e, resref)?),
            None => {
                let r = ResRef::from_str(resref)?;
                match game.resman.get(&ResKey { resref: r, restype }) {
                    Ok(bytes) => Some(mg_gff::Gff::read(&bytes)?.root),
                    Err(_) => None,
                }
            }
        })
    })
}

/// [`blueprint`] given the builder's own (`own`, GFF JSON text, read fresh on every call: it is per workspace, and on a team
/// server one process answers everyone) and the module's or game's lookup (`shared`, kept in [`CACHE`]).
fn blueprint_from(ext: &str, resref: &str, own: Option<String>, shared: impl FnOnce() -> Result<Option<Struct>>) -> Result<Option<Struct>> {
    if let Some(text) = own {
        return Ok(Some(from_json(&text, ext, resref)?));
    }
    let key = (ext.to_string(), resref.to_string());
    if let Some((at, hit)) = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|m| m.get(&key))
        && at.elapsed().as_secs() < CACHE_SECS
    {
        return Ok(hit.clone());
    }
    let found = shared()?;
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(key, (std::time::Instant::now(), found.clone()));
    Ok(found)
}

/// Forgets what was last read of a blueprint (one made, changed or deleted since is seen at once).
pub fn forget_blueprint(ext: &str, resref: &str) {
    if let Some(m) = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
        m.remove(&(ext.to_string(), resref.to_string()));
    }
}

// --- Overrides -----------------------------------------------------------------------------

/// A field the editor may set on an instance: (kinds, label, type: b byte, h short, i int, w word, d dword, f float,
/// s string, r resref, l localized string).
const FIELDS: &[(&str, &str, char)] = &[
    // creature: its scripts and conversation
    ("creature", "Conversation", 'r'),
    ("creature", "ScriptSpawn", 'r'),
    ("creature", "ScriptAttacked", 'r'),
    ("creature", "ScriptDamaged", 'r'),
    ("creature", "ScriptDeath", 'r'),
    ("creature", "ScriptDialogue", 'r'),
    ("creature", "ScriptDisturbed", 'r'),
    ("creature", "ScriptEndRound", 'r'),
    ("creature", "ScriptHeartbeat", 'r'),
    ("creature", "ScriptOnBlocked", 'r'),
    ("creature", "ScriptOnNotice", 'r'),
    ("creature", "ScriptRested", 'r'),
    ("creature", "ScriptSpellAt", 'r'),
    ("creature", "ScriptUserDefine", 'r'),
    // placeable
    ("placeable", "Conversation", 'r'),
    ("placeable", "OnUsed", 'r'),
    ("placeable", "OnOpen", 'r'),
    ("placeable", "OnClosed", 'r'),
    ("placeable", "OnDeath", 'r'),
    ("placeable", "OnDamaged", 'r'),
    ("placeable", "OnHeartbeat", 'r'),
    ("placeable", "OnInvDisturbed", 'r'),
    ("placeable", "OnLock", 'r'),
    ("placeable", "OnUnlock", 'r'),
    ("placeable", "OnMeleeAttacked", 'r'),
    ("placeable", "OnSpellCastAt", 'r'),
    ("placeable", "OnUserDefined", 'r'),
    ("placeable", "OnClick", 'r'),
    ("placeable", "Locked", 'b'),
    ("placeable", "Plot", 'b'),
    ("placeable", "Static", 'b'),
    ("placeable", "Useable", 'b'),
    // placeable and door: lock, trap, how hard to destroy
    ("placeable", "Lockable", 'b'),
    ("placeable", "KeyRequired", 'b'),
    ("placeable", "KeyName", 's'),
    ("placeable", "AutoRemoveKey", 'b'),
    ("placeable", "OpenLockDC", 'b'),
    ("placeable", "CloseLockDC", 'b'),
    ("placeable", "TrapType", 'b'),
    ("placeable", "TrapFlag", 'b'),
    ("placeable", "TrapOneShot", 'b'),
    ("placeable", "TrapDetectable", 'b'),
    ("placeable", "TrapDetectDC", 'b'),
    ("placeable", "TrapDisarmable", 'b'),
    ("placeable", "DisarmDC", 'b'),
    ("placeable", "HP", 'h'),
    ("placeable", "CurrentHP", 'h'),
    ("placeable", "Hardness", 'b'),
    ("placeable", "Fort", 'b'),
    ("placeable", "Ref", 'b'),
    ("placeable", "Will", 'b'),
    // door
    ("door", "Conversation", 'r'),
    ("door", "OnOpen", 'r'),
    ("door", "OnClosed", 'r'),
    ("door", "OnDeath", 'r'),
    ("door", "OnDamaged", 'r'),
    ("door", "OnHeartbeat", 'r'),
    ("door", "OnLock", 'r'),
    ("door", "OnUnlock", 'r'),
    ("door", "OnMeleeAttacked", 'r'),
    ("door", "OnSpellCastAt", 'r'),
    ("door", "OnUserDefined", 'r'),
    ("door", "OnClick", 'r'),
    ("door", "OnFailToOpen", 'r'),
    ("door", "LinkedTo", 's'),
    ("door", "LinkedToFlags", 'b'),
    ("door", "TransitionDestin", 'l'),
    ("door", "Locked", 'b'),
    ("door", "Plot", 'b'),
    ("door", "Lockable", 'b'),
    ("door", "KeyRequired", 'b'),
    ("door", "KeyName", 's'),
    ("door", "AutoRemoveKey", 'b'),
    ("door", "OpenLockDC", 'b'),
    ("door", "CloseLockDC", 'b'),
    ("door", "TrapType", 'b'),
    ("door", "TrapFlag", 'b'),
    ("door", "TrapOneShot", 'b'),
    ("door", "TrapDetectable", 'b'),
    ("door", "TrapDetectDC", 'b'),
    ("door", "TrapDisarmable", 'b'),
    ("door", "DisarmDC", 'b'),
    ("door", "HP", 'h'),
    ("door", "CurrentHP", 'h'),
    ("door", "Hardness", 'b'),
    ("door", "Fort", 'b'),
    ("door", "Ref", 'b'),
    ("door", "Will", 'b'),
    // trigger: 0 generic, 1 area transition, 2 trap
    ("trigger", "Type", 'i'),
    ("trigger", "LinkedTo", 's'),
    ("trigger", "LinkedToFlags", 'b'),
    ("trigger", "ScriptOnEnter", 'r'),
    ("trigger", "ScriptOnExit", 'r'),
    ("trigger", "ScriptHeartbeat", 'r'),
    ("trigger", "ScriptUserDefine", 'r'),
    ("trigger", "OnClick", 'r'),
    ("trigger", "OnTrapTriggered", 'r'),
    ("trigger", "OnDisarm", 'r'),
    ("trigger", "TrapType", 'b'),
    ("trigger", "TrapFlag", 'b'),
    ("trigger", "TrapOneShot", 'b'),
    ("trigger", "TrapDetectable", 'b'),
    ("trigger", "TrapDetectDC", 'b'),
    ("trigger", "TrapDisarmable", 'b'),
    ("trigger", "DisarmDC", 'b'),
    ("trigger", "AutoRemoveKey", 'b'),
    ("trigger", "KeyName", 's'),
    ("trigger", "Cursor", 'b'),
    ("trigger", "HighlightHeight", 'f'),
    // waypoint: a map note
    ("waypoint", "HasMapNote", 'b'),
    ("waypoint", "MapNoteEnabled", 'b'),
    ("waypoint", "MapNote", 'l'),
    // encounter
    ("encounter", "Active", 'b'),
    ("encounter", "MaxCreatures", 'i'),
    ("encounter", "RecCreatures", 'i'),
    ("encounter", "Reset", 'b'),
    ("encounter", "ResetTime", 'i'),
    ("encounter", "Respawns", 'i'),
    ("encounter", "OnEntered", 'r'),
    ("encounter", "OnExit", 'r'),
    ("encounter", "OnExhausted", 'r'),
    ("encounter", "OnHeartbeat", 'r'),
    ("encounter", "OnUserDefined", 'r'),
    ("encounter", "DifficultyIndex", 'i'),
    ("encounter", "SpawnOption", 'i'),
    ("encounter", "PlayerOnly", 'b'),
    // store
    ("store", "MarkUp", 'i'),
    ("store", "MarkDown", 'i'),
    ("store", "OnOpenStore", 'r'),
    ("store", "OnStoreClosed", 'r'),
    ("store", "BlackMarket", 'b'),
    ("store", "BM_MarkDown", 'i'),
    ("store", "IdentifyPrice", 'i'),
    ("store", "MaxBuyPrice", 'i'),
    // sound
    ("sound", "Active", 'b'),
    ("sound", "Volume", 'b'),
    ("sound", "Looping", 'b'),
    ("sound", "Positional", 'b'),
    ("sound", "MaxDistance", 'f'),
    ("sound", "MinDistance", 'f'),
    ("sound", "Random", 'b'),
    ("sound", "RandomPosition", 'b'),
    ("sound", "RandomRangeX", 'f'),
    ("sound", "RandomRangeY", 'f'),
    ("sound", "Interval", 'd'),
    ("sound", "IntervalVrtn", 'd'),
    ("sound", "PitchVariation", 'f'),
    ("sound", "VolumeVrtn", 'b'),
    ("sound", "Times", 'b'),
    // an item on the ground
    ("item", "Charges", 'b'),
    ("item", "StackSize", 'w'),
    ("item", "Identified", 'b'),
    ("item", "Plot", 'b'),
    ("item", "Cursed", 'b'),
    ("item", "Stolen", 'b'),
    // creature: flags
    ("creature", "Plot", 'b'),
    ("creature", "IsImmortal", 'b'),
    ("creature", "NoPermDeath", 'b'),
    ("creature", "Interruptable", 'b'),
    ("creature", "Lootable", 'b'),
];

/// The fields the editor may set on a kind, and their types (for the editor's form).
pub fn fields_of(kind: ObjectKind) -> Vec<(&'static str, char)> {
    let k = kind_name(kind);
    FIELDS.iter().filter(|(kk, ..)| *kk == k).map(|(_, l, t)| (*l, *t)).collect()
}

/// The GFF label that holds a kind's name: the game spells it three ways.
pub fn name_label(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Creature => "FirstName",
        ObjectKind::Waypoint | ObjectKind::Trigger | ObjectKind::Encounter | ObjectKind::Item => "LocalizedName",
        _ => "LocName",
    }
}

fn set_field(s: &mut Struct, label: &str, ty: char, v: &Json) -> Result<()> {
    let num = || {
        v.as_f64()
            .or_else(|| v.as_bool().map(|b| f64::from(u8::from(b))))
            .or_else(|| v.as_str().and_then(|t| t.trim().parse().ok()))
            .with_context(|| format!("{label}: a number"))
    };
    let value = match ty {
        'b' => Value::Byte(num()?.clamp(0.0, 255.0) as u8),
        'h' => Value::Short(num()?.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16),
        'i' => Value::Int(num()? as i32),
        'w' => Value::Word(num()?.clamp(0.0, 65535.0) as u16),
        'd' => Value::Dword(num()?.max(0.0) as u32),
        'f' => Value::Float(num()? as f32),
        's' => Value::String(v.as_str().unwrap_or("").as_bytes().to_vec()),
        'r' => {
            let t = v.as_str().unwrap_or("").to_ascii_lowercase();
            anyhow::ensure!(t.is_empty() || crate::newbp::resref_ok(&t), "{label}: '{t}' isn't a resref (up to 16 letters, digits or _)");
            Value::resref(ResRef::from_str(&t)?)
        }
        'l' => crate::settings::loc(v.as_str().unwrap_or("")),
        _ => anyhow::bail!("{label}: unknown field type"),
    };
    s.set(label, value);
    Ok(())
}

/// Adds a local variable (an int, float or string) to the struct's `VarTable`, replacing one of that name.
pub fn add_var(s: &mut Struct, name: &str, v: &Json) {
    let Some(Value::List(new)) = crate::settings::var_table(&[serde_json::json!({ "name": name, "type": v["type"], "value": v["value"] })])
    else {
        return;
    };
    let mut list = s.list("VarTable").map_or_default(<[_]>::to_vec);
    list.retain(|e| e.string("Name") != Some(name.as_bytes()));
    list.extend(new);
    s.set("VarTable", Value::List(list));
}

/// An inventory is a grid this many cells wide (a pack, a container, a store's page).
const GRID_WIDTH: u32 = 10;
/// A store page's `StoreList` struct id for each baseitems.2da `StorePanel` (armor, miscellaneous, potions, rings, weapons).
const PANEL_PAGE: [u32; 5] = [0, 4, 2, 3, 1];
/// The pages of a store, in the order Aurora writes them.
const STORE_PAGES: [u32; 5] = [0, 4, 2, 3, 1];
/// The most an inventory the editor writes may hold.
pub const MAX_STOCK: usize = 250;

/// The first free place for a `w` by `h` item, row by row, among the taken rectangles (x, y, w, h).
fn first_free(taken: &[(u32, u32, u32, u32)], w: u32, h: u32) -> (u32, u32) {
    let free = |x: u32, y: u32| taken.iter().all(|&(tx, ty, tw, th)| x + w <= tx || tx + tw <= x || y + h <= ty || ty + th <= y);
    (0..).find_map(|y| (0..=GRID_WIDTH - w).find(|&x| free(x, y)).map(|x| (x, y))).expect("an empty row fits any item")
}

/// How an item blueprint sits in a grid: its store page and its size in cells.
fn item_fit(game: &GameData, resref: &str) -> Result<(usize, u32, u32)> {
    let bp = blueprint(game, ResType::UTI, resref)?.with_context(|| format!("no item blueprint {resref}"))?;
    let base = bp.integer("BaseItem").unwrap_or(-1);
    let table = game.table("baseitems").ok();
    let cell = |col: &str| table.as_ref().and_then(|t| t.get_int(usize::try_from(base).ok()?, col)).and_then(|v| u32::try_from(v).ok());
    Ok((
        cell("StorePanel").unwrap_or(4).min(4) as usize,
        cell("InvSlotWidth").unwrap_or(1).clamp(1, GRID_WIDTH),
        cell("InvSlotHeight").unwrap_or(1).max(1),
    ))
}

/// One inventory entry as a blueprint holds it: the item's resref and its place in the grid.
fn entry(id: u32, resref: &str, at: (u32, u32), infinite: Option<bool>) -> Result<Struct> {
    let mut e = Struct::new(id);
    e.set("InventoryRes", Value::resref(ResRef::from_str(resref)?));
    e.set("Repos_PosX", Value::Word(at.0 as u16));
    e.set("Repos_Posy", Value::Word(at.1 as u16));
    if let Some(i) = infinite {
        e.set("Infinite", Value::Byte(u8::from(i)));
    }
    Ok(e)
}

/// Gives a container, a creature or a store the items `inv` instead of its blueprint's (the instance expands them
/// into whole items, as Aurora does).
pub(crate) fn set_inventory(game: &GameData, kind: ObjectKind, bp: &mut Struct, inv: &[(String, bool)]) -> Result<()> {
    anyhow::ensure!(
        matches!(kind, ObjectKind::Placeable | ObjectKind::Creature | ObjectKind::Store),
        "a {} holds no items",
        kind_name(kind)
    );
    anyhow::ensure!(inv.len() <= MAX_STOCK, "up to {MAX_STOCK} items");
    for (r, _) in inv {
        anyhow::ensure!(crate::newbp::resref_ok(&r.to_ascii_lowercase()), "'{r}' isn't an item resref");
    }
    if kind == ObjectKind::Store {
        let mut taken: [Vec<(u32, u32, u32, u32)>; 5] = Default::default();
        let mut pages: [Vec<Struct>; 5] = Default::default();
        for (r, infinite) in inv {
            let (panel, w, h) = item_fit(game, r)?;
            let page = panel;
            let at = first_free(&taken[page], w, h);
            taken[page].push((at.0, at.1, w, h));
            let id = pages[page].len() as u32;
            pages[page].push(entry(id, r, at, Some(*infinite))?);
        }
        let list = STORE_PAGES
            .iter()
            .map(|&id| {
                let panel = PANEL_PAGE.iter().position(|&p| p == id).unwrap_or(4);
                let mut page = Struct::new(id);
                page.set("ItemList", Value::List(std::mem::take(&mut pages[panel])));
                page
            })
            .collect();
        bp.set("StoreList", Value::List(list));
    } else {
        let mut taken = Vec::new();
        let mut items = Vec::new();
        for (i, (r, _)) in inv.iter().enumerate() {
            let (_, w, h) = item_fit(game, r)?;
            let at = first_free(&taken, w, h);
            taken.push((at.0, at.1, w, h));
            items.push(entry(i as u32, r, at, None)?);
        }
        bp.set("ItemList", Value::List(items));
    }
    Ok(())
}

/// A creature's equipment slots: (name the editor uses, the slot's bit, which is the struct id of its entry).
pub const SLOTS: [(&str, u32); 18] = [
    ("head", 1),
    ("chest", 2),
    ("boots", 4),
    ("arms", 8),
    ("right hand", 16),
    ("left hand", 32),
    ("cloak", 64),
    ("left ring", 128),
    ("right ring", 256),
    ("neck", 512),
    ("belt", 1024),
    ("arrows", 2048),
    ("bullets", 4096),
    ("bolts", 8192),
    ("claw left", 16384),
    ("claw right", 32768),
    ("bite", 65536),
    ("skin", 131072),
];

fn slot_id(name: &str) -> Option<u32> {
    SLOTS.iter().find(|(n, _)| *n == name).map(|(_, id)| *id)
}

fn slot_name(id: u32) -> Option<&'static str> {
    SLOTS.iter().find(|(_, i)| *i == id).map(|(n, _)| *n)
}

/// Gives a creature blueprint what it wears and wields (one item to a slot) instead of its own.
pub(crate) fn set_equipment(game: &GameData, bp: &mut Struct, eq: &[(String, String)]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let mut list = Vec::new();
    for (slot, resref) in eq {
        let id = slot_id(slot).with_context(|| format!("'{slot}' isn't a slot a creature has"))?;
        anyhow::ensure!(seen.insert(id), "two items in the {slot} slot");
        anyhow::ensure!(crate::newbp::resref_ok(&resref.to_ascii_lowercase()), "'{resref}' isn't an item resref");
        blueprint(game, ResType::UTI, resref)?.with_context(|| format!("no item blueprint {resref}"))?;
        let mut e = Struct::new(id);
        e.set("EquippedRes", Value::resref(ResRef::from_str(resref)?));
        list.push(e);
    }
    bp.set("Equip_ItemList", Value::List(list));
    Ok(())
}

/// Changes what an original creature wears and wields to the editor's list: a slot whose item stays is kept as it is (the
/// toolset may have changed it), a slot that is emptied or given another item is rebuilt from the blueprint as Aurora
/// holds it.
fn apply_equipment(game: &GameData, s: &mut Struct, eq: &[Json]) -> Result<()> {
    let item_bp = |r: ResRef| blueprint(game, ResType::UTI, &r.to_string()).ok().flatten();
    let placing = mg_module::instances::Placing { game, item: &item_bp };
    let old: Vec<Struct> = s.list("Equip_ItemList").map_or_default(<[_]>::to_vec);
    let mut list = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for e in eq {
        let (Some(slot), Some(resref)) = (e["slot"].as_str(), e["resref"].as_str()) else { continue };
        let id = slot_id(slot).with_context(|| format!("'{slot}' isn't a slot a creature has"))?;
        anyhow::ensure!(seen.insert(id), "two items in the {slot} slot");
        let wanted = resref.to_ascii_lowercase();
        match old.iter().find(|o| o.id == id && o.resref("TemplateResRef").is_some_and(|r| r.to_string().eq_ignore_ascii_case(&wanted))) {
            Some(same) => list.push(same.clone()),
            None => {
                let bp = blueprint(game, ResType::UTI, &wanted)?.with_context(|| format!("no item blueprint {wanted}"))?;
                list.push(mg_module::instances::held(&placing, &bp, id));
            }
        }
    }
    s.set("Equip_ItemList", Value::List(list));
    Ok(())
}

/// Gives an encounter the creatures `cs` to spawn instead of its blueprint's.
fn set_creatures(game: &GameData, bp: &mut Struct, cs: &[String]) -> Result<()> {
    anyhow::ensure!(cs.len() <= 30, "up to 30 creatures");
    let mut list = Vec::new();
    for r in cs {
        anyhow::ensure!(crate::newbp::resref_ok(&r.to_ascii_lowercase()), "'{r}' isn't a creature resref");
        let c = blueprint(game, ResType::UTC, r)?.with_context(|| format!("no creature blueprint {r}"))?;
        let mut e = Struct::new(0);
        e.set("ResRef", Value::resref(ResRef::from_str(r)?));
        e.set("Appearance", Value::Int(c.integer("Appearance_Type").unwrap_or(0) as i32));
        e.set("CR", Value::Float(c.float("ChallengeRating").unwrap_or(1.0)));
        e.set("SingleSpawn", Value::Byte(0));
        list.push(e);
    }
    bp.set("CreatureList", Value::List(list));
    Ok(())
}

/// What a blueprint holds now, for the editor to start from: items (container, creature, store: resref, name,
/// infinite) or creatures (an encounter), with whether a placeable is a container at all.
pub fn contents_of(game: &GameData, kind: ObjectKind, resref: &str) -> Result<Json> {
    let bp = blueprint(game, kind.restype(), resref)?.with_context(|| format!("no {} blueprint {resref}", kind_name(kind)))?;
    let name_of = |res: ResType, r: &str, label: &str| {
        blueprint(game, res, r).ok().flatten().map_or_default(|b| crate::blueprints::struct_name(game, &b, label))
    };
    let item = |e: &Struct, infinite: bool| {
        let r = e.resref("InventoryRes").map_or_default(|r| r.to_string());
        serde_json::json!({ "resref": r, "name": name_of(ResType::UTI, &r, "LocalizedName"), "infinite": infinite })
    };
    Ok(match kind {
        ObjectKind::Placeable | ObjectKind::Creature => {
            let items: Vec<Json> = bp.list("ItemList").unwrap_or(&[]).iter().map(|e| item(e, false)).collect();
            let worn: Vec<Json> = bp
                .list("Equip_ItemList")
                .unwrap_or(&[])
                .iter()
                .filter_map(|e| {
                    let r = e.resref("EquippedRes")?.to_string();
                    Some(serde_json::json!({ "slot": slot_name(e.id)?, "resref": r, "name": name_of(ResType::UTI, &r, "LocalizedName") }))
                })
                .collect();
            let container = kind != ObjectKind::Placeable || bp.integer("HasInventory").unwrap_or(0) != 0;
            serde_json::json!({ "container": container, "items": items, "equipment": worn })
        }
        ObjectKind::Store => {
            let items: Vec<Json> = bp
                .list("StoreList")
                .unwrap_or(&[])
                .iter()
                .flat_map(|pg| pg.list("ItemList").unwrap_or(&[]).iter().map(|e| item(e, e.integer("Infinite").unwrap_or(0) != 0)))
                .collect();
            serde_json::json!({ "container": true, "items": items })
        }
        ObjectKind::Encounter => {
            let creatures: Vec<Json> = bp
                .list("CreatureList")
                .unwrap_or(&[])
                .iter()
                .map(|e| {
                    let r = e.resref("ResRef").map_or_default(|r| r.to_string());
                    serde_json::json!({ "resref": r, "name": name_of(ResType::UTC, &r, "FirstName") })
                })
                .collect();
            serde_json::json!({ "creatures": creatures })
        }
        _ => serde_json::json!({}),
    })
}

/// The blueprint of `t` (or a plain one) with the thing's overrides applied.
fn prepared(game: &GameData, t: &Thing) -> Result<Struct> {
    let restype = t.kind.restype();
    let mut bp = if t.blueprint.is_empty() {
        Struct::new(0)
    } else {
        blueprint(game, restype, &t.blueprint)?.with_context(|| format!("no {} blueprint {}", kind_name(t.kind), t.blueprint))?
    };
    if !t.blueprint.is_empty() {
        bp.set("TemplateResRef", Value::resref(ResRef::from_str(&t.blueprint)?));
    }
    if let Some(tag) = &t.tag {
        anyhow::ensure!(tag.len() <= 32 && !tag.contains(char::is_whitespace), "tag '{tag}': up to 32 characters, no spaces");
        bp.set("Tag", Value::String(tag.as_bytes().to_vec()));
    }
    if let Some(n) = &t.name {
        bp.set(name_label(t.kind), crate::settings::loc(n));
        if t.kind == ObjectKind::Creature {
            bp.set("LastName", crate::settings::loc(""));
        }
    }
    let allowed = fields_of(t.kind);
    for (label, v) in &t.fields {
        match allowed.iter().find(|(l, _)| l == label) {
            Some((l, ty)) => set_field(&mut bp, l, *ty, v)?,
            None => anyhow::bail!("{} {}: '{label}' can't be set here", kind_name(t.kind), t.blueprint),
        }
    }
    if let Some(inv) = &t.inventory {
        set_inventory(game, t.kind, &mut bp, inv)?;
    }
    if let Some(eq) = &t.equipment {
        anyhow::ensure!(t.kind == ObjectKind::Creature, "only a creature wears things");
        set_equipment(game, &mut bp, eq)?;
    }
    if let Some(cs) = &t.creatures {
        anyhow::ensure!(t.kind == ObjectKind::Encounter, "only an encounter has creatures to spawn");
        set_creatures(game, &mut bp, cs)?;
    }
    for v in &t.vars {
        if let Some(name) = v["name"].as_str().filter(|n| !n.is_empty()) {
            add_var(&mut bp, name, v);
        }
    }
    add_var(&mut bp, MARK, &serde_json::json!({ "type": "int", "value": t.id }));
    Ok(bp)
}

/// One thing as its GIT entry: (list name, instance). `ground` gives the terrain height at a point.
pub fn place(game: &GameData, t: &Thing, ground: &dyn Fn(f32, f32) -> f32) -> Result<(&'static str, Struct)> {
    let item = |r: ResRef| blueprint(game, ResType::UTI, &r.to_string()).ok().flatten();
    let placing = mg_module::instances::Placing { game, item: &item };
    let bp = prepared(game, t).with_context(|| format!("thing {} ({})", t.id, kind_name(t.kind)))?;
    let outline: Vec<[f32; 3]> = match t.kind {
        ObjectKind::Trigger | ObjectKind::Encounter => {
            anyhow::ensure!(t.outline.len() >= 3, "{} {}: an outline needs at least three points", kind_name(t.kind), t.id);
            let (x0, y0) = (t.outline[0][0], t.outline[0][1]);
            t.outline.iter().map(|p| [p[0] - x0, p[1] - y0, ground(p[0], p[1]) + mg_module::instances::OUTLINE_LIFT]).collect()
        }
        _ => Vec::new(),
    };
    let (px, py) = match t.kind {
        ObjectKind::Trigger | ObjectKind::Encounter => (t.outline[0][0], t.outline[0][1]),
        _ => (t.x, t.y),
    };
    // The editor's bearing is anticlockwise from east, as a placeable's own; Aurora's placement turns
    // the others (their facing vector) from north.
    let rotation = match t.kind {
        ObjectKind::Placeable | ObjectKind::Door => t.bearing,
        _ => t.bearing - std::f32::consts::FRAC_PI_2,
    };
    let at = mg_module::instances::Placement { position: [px, py, ground(px, py)], rotation };
    let s = mg_module::instances::instance(&placing, t.kind.restype(), &bp, at, &outline)
        .with_context(|| format!("placing {} {}", kind_name(t.kind), t.blueprint))?;
    Ok((t.kind.list(), s))
}

/// Every thing of the loaded terrain as GIT entries: (list name, instance).
pub fn instances(game: &GameData) -> Result<Vec<(&'static str, Struct)>> {
    let Some(hm) = crate::heightmap::current() else { return Ok(Vec::new()) };
    let ground = |x: f32, y: f32| hm.at(x, y);
    hm.things.iter().map(|t| place(game, t, &ground)).collect()
}

/// Whether an instance is one of the editor's (carries the mark), and its editor id.
pub fn mark_of(s: &Struct) -> Option<i64> {
    s.list("VarTable")?.iter().find(|v| v.string("Name") == Some(MARK.as_bytes())).and_then(|v| v.integer("Value"))
}

/// The model a placeable's or door's blueprint shows (lower case): by its appearance row, as the game reads it.
/// None when the row names none or a model nobody has (the preview then has nothing to draw).
pub fn model_of(game: &GameData, kind: ObjectKind, resref: &str) -> Result<Option<String>> {
    let bp = blueprint(game, kind.restype(), resref)?.with_context(|| format!("no {} blueprint {resref}", kind_name(kind)))?;
    Ok(struct_model(game, kind, &bp))
}

/// The model object `s` (a blueprint, or an object in an area's GIT) is drawn with, from its appearance; None when it names none.
pub fn struct_model(game: &GameData, kind: ObjectKind, s: &Struct) -> Option<String> {
    let at = mg_rules::appearance::AppearanceRow::of(kind, s)?;
    let table = game.table(at.table).ok();
    at.model(table.as_deref(), &game.resman).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_field_is_written_as_a_short_and_clamped() {
        let mut s = Struct::new(0);
        set_field(&mut s, "HP", 'h', &serde_json::json!(30)).unwrap();
        assert_eq!(s.get("HP"), Some(&Value::Short(30)));
        set_field(&mut s, "HP", 'h', &serde_json::json!(99999)).unwrap();
        assert_eq!(s.get("HP"), Some(&Value::Short(i16::MAX)));
        set_field(&mut s, "Lockable", 'b', &serde_json::json!(true)).unwrap();
        assert_eq!(s.get("Lockable"), Some(&Value::Byte(1)));
    }

    #[test]
    fn no_field_is_listed_twice_and_resrefs_are_scripts_or_conversations() {
        let mut seen = std::collections::HashSet::new();
        for (k, l, t) in FIELDS {
            assert!(seen.insert((*k, *l)), "{k} {l} twice");
            if *t == 'r' {
                assert!(
                    l.starts_with("Script") || l.starts_with("On") || *l == "Conversation",
                    "{k} {l}: a resref that isn't a script or conversation (the check before publishing treats every one as one)"
                );
            }
        }
    }

    /// A creature blueprint as GFF JSON (as a builder's new one is kept), tagged `tag`.
    fn creature(tag: &str) -> Option<String> {
        Some(serde_json::json!({ "__data_type": "UTC ", "Tag": { "type": "cexostring", "value": tag } }).to_string())
    }

    fn tag(found: &Option<Struct>) -> Option<String> {
        found.as_ref().and_then(|s| s.string("Tag")).map(|t| String::from_utf8_lossy(t).into_owned())
    }

    #[test]
    fn a_builders_own_blueprint_is_theirs_alone_whoever_asks_first() {
        // On a team server one process answers everyone: alice has a blueprint of this resref in her workspace, bob hasn't, and
        // neither the module nor the game has one. (`own` is what each one's workspace holds.)
        let nothing_shared = || Ok(None);
        for (resref, bob_first) in [("lsguardtest1", true), ("lsguardtest2", false)] {
            if bob_first {
                assert_eq!(tag(&blueprint_from("utc", resref, None, nothing_shared).unwrap()), None, "bob: none");
            }
            // A miss cached from bob's lookup doesn't hide alice's own from her.
            assert_eq!(
                tag(&blueprint_from("utc", resref, creature("AliceSecret"), nothing_shared).unwrap()).as_deref(),
                Some("AliceSecret")
            );
            // And hers is never what bob gets, now or later.
            for _ in 0..2 {
                assert_eq!(tag(&blueprint_from("utc", resref, None, nothing_shared).unwrap()), None, "bob gets alice's blueprint");
            }
        }
        // A module blueprint is kept for everyone, and a builder's own of the same resref still comes first for them.
        let module = || Ok(from_json(&creature("Module").unwrap(), "utc", "lsguardtest3").ok());
        assert_eq!(tag(&blueprint_from("utc", "lsguardtest3", None, module).unwrap()).as_deref(), Some("Module"));
        let looked_again = || -> Result<Option<Struct>> { panic!("the module's blueprint is kept, not looked up again") };
        assert_eq!(tag(&blueprint_from("utc", "lsguardtest3", None, looked_again).unwrap()).as_deref(), Some("Module"));
        assert_eq!(tag(&blueprint_from("utc", "lsguardtest3", creature("Mine"), looked_again).unwrap()).as_deref(), Some("Mine"));
    }

    #[test]
    #[ignore = "needs the game's files: NWN_ROOT=<the game> cargo test -- --ignored"]
    fn a_blueprints_own_settings_are_read_for_the_editor() {
        let game = crate::models::game().expect("this test needs the game's files (NWN_ROOT)");
        // The game's treasure chest: usable, a lock DC of 18, not locked.
        let chest = blueprint_fields(game, ObjectKind::Placeable, "plc_chest1").unwrap();
        assert_eq!((chest["kind"].as_str(), chest["resref"].as_str()), (Some("placeable"), Some("plc_chest1")));
        let f = &chest["fields"];
        assert_eq!((f["Useable"].as_i64(), f["Locked"].as_i64(), f["OpenLockDC"].as_i64()), (Some(1), Some(0), Some(18)), "{chest}");
        assert_eq!(f["OnOpen"].as_str(), Some("nw_o2_classlow"));
        // Only the kind's settings, none of another kind's.
        assert!(f.get("Type").is_none() && f.get("ScriptSpawn").is_none());
        let goblin = blueprint_fields(game, ObjectKind::Creature, "nw_goblina").unwrap();
        assert_eq!(goblin["fields"]["Interruptable"].as_i64(), Some(1), "{goblin}");
        assert!(blueprint_fields(game, ObjectKind::Door, "nosuchdoor").unwrap_err().to_string().contains("no door blueprint nosuchdoor"));
    }

    #[test]
    fn a_thing_reads_from_the_editors_json() {
        let j = serde_json::json!({
            "id": 3, "kind": "trigger", "blueprint": "", "x": 1, "y": 2, "outline": [[0, 0], [5, 0], [5, 5]],
            "tag": "T1", "name": "gate", "vars": [{ "name": "a", "type": "int", "value": 2 }], "fields": { "Type": 1, "LinkedTo": "WP_x" },
        });
        let t = Thing::parse(&j).unwrap();
        assert_eq!((t.id, t.kind, t.outline.len(), t.tag.as_deref()), (3, ObjectKind::Trigger, 3, Some("T1")));
        assert!(Thing::parse(&serde_json::json!({ "id": 1, "kind": "dragon", "x": 0, "y": 0 })).unwrap_err().contains("unknown kind"));
        assert!(Thing::parse(&serde_json::json!({ "id": 2, "kind": "creature", "y": 0 })).unwrap_err().contains("east"));
        assert!(
            Thing::parse(&serde_json::json!({ "id": 3, "kind": "trigger", "x": 0, "y": 0, "outline": [[0, 0], [1, null]] }))
                .unwrap_err()
                .contains("outline")
        );
    }

    #[test]
    fn overrides_are_typed_and_checked() {
        let mut s = Struct::new(0);
        set_field(&mut s, "Type", 'i', &serde_json::json!(1)).unwrap();
        set_field(&mut s, "HasMapNote", 'b', &serde_json::json!(true)).unwrap();
        set_field(&mut s, "ScriptOnEnter", 'r', &serde_json::json!("gate_enter")).unwrap();
        assert_eq!((s.integer("Type"), s.integer("HasMapNote")), (Some(1), Some(1)));
        assert!(set_field(&mut s, "ScriptOnEnter", 'r', &serde_json::json!("a_resref_that_is_much_too_long")).is_err());
        assert!(fields_of(ObjectKind::Waypoint).iter().any(|(l, _)| *l == "MapNote"));
        assert!(!fields_of(ObjectKind::Waypoint).iter().any(|(l, _)| *l == "Type"));
    }

    #[test]
    fn the_mark_is_a_variable_the_editors_id_is_read_from() {
        let mut s = Struct::new(0);
        add_var(&mut s, "other", &serde_json::json!({ "type": "string", "value": "x" }));
        add_var(&mut s, MARK, &serde_json::json!({ "type": "int", "value": 42 }));
        assert_eq!(mark_of(&s), Some(42));
        assert_eq!(s.list("VarTable").unwrap().len(), 2);
        add_var(&mut s, MARK, &serde_json::json!({ "type": "int", "value": 43 }));
        assert_eq!((mark_of(&s), s.list("VarTable").unwrap().len()), (Some(43), 2));
    }

    #[test]
    fn items_take_the_first_free_place_row_by_row() {
        let mut taken = Vec::new();
        let mut at = Vec::new();
        for (w, h) in [(1, 1), (2, 2), (1, 1), (10, 1), (1, 1)] {
            let p = first_free(&taken, w, h);
            taken.push((p.0, p.1, w, h));
            at.push(p);
        }
        // the 1x1, the 2x2 beside it, the next 1x1 after it, the ten-wide one on the next free row, the last in the gap on row 0
        assert_eq!(at, vec![(0, 0), (1, 0), (3, 0), (0, 2), (4, 0)]);
    }

    #[test]
    fn an_inventory_is_parsed_with_its_infinite_flags() {
        let j = serde_json::json!({
            "id": 1, "kind": "store", "x": 1, "y": 1, "blueprint": "a",
            "inventory": [{ "resref": "NW_WSWDG001", "infinite": true }, { "resref": "x" }, {}],
        });
        let t = Thing::parse(&j).unwrap();
        assert_eq!(t.inventory, Some(vec![("nw_wswdg001".to_string(), true), ("x".to_string(), false)]));
        assert_eq!(Thing::parse(&serde_json::json!({ "id": 1, "kind": "store", "x": 1, "y": 1 })).unwrap().inventory, None);
        let c = Thing::parse(&serde_json::json!({ "id": 2, "kind": "encounter", "x": 1, "y": 1, "creatures": [{ "resref": "Goblin" }] }))
            .unwrap();
        assert_eq!(c.creatures, Some(vec!["goblin".to_string()]));
    }

    #[test]
    fn equipment_slots_are_the_struct_ids_aurora_writes() {
        assert_eq!(slot_id("chest"), Some(2));
        assert_eq!(slot_id("right hand"), Some(16));
        assert_eq!(slot_id("skin"), Some(131072));
        assert_eq!(slot_name(512), Some("neck"));
        assert_eq!(slot_id("tail"), None);
        assert!(SLOTS.iter().enumerate().all(|(i, (_, id))| *id == 1 << i));
    }
}
