// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! New blueprints a builder makes: a copy of an existing creature or item with its own resref, name and settings.
//!
//! They live in the builder's workspace (`blueprints/<resref>.<ext>.json`, nwn_gff JSON like the module's `core/`).
//! The editor places them like any other blueprint (`things::blueprint` looks here first), and Publish sends the ones
//! an area uses to the review service, which puts them in `core/` in the same review PR as the area.

use std::path::PathBuf;

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_resman::ResKey;
use mg_rules::GameData;
use serde_json::{Value as Json, json};

/// What may be set on a new blueprint: (kind's file extension, label, type), types as in `things::FIELDS` plus 'l' for
/// a localized string and 'r' for a resref.
const FIELDS: &[(&str, &str, char)] = &[
    ("utc", "FirstName", 'l'),
    ("utc", "LastName", 'l'),
    ("utc", "Description", 'l'),
    ("utc", "Tag", 's'),
    ("utc", "Comment", 's'),
    ("utc", "Conversation", 'r'),
    ("utc", "ScriptSpawn", 'r'),
    ("utc", "ScriptAttacked", 'r'),
    ("utc", "ScriptDamaged", 'r'),
    ("utc", "ScriptDeath", 'r'),
    ("utc", "ScriptDialogue", 'r'),
    ("utc", "ScriptDisturbed", 'r'),
    ("utc", "ScriptEndRound", 'r'),
    ("utc", "ScriptHeartbeat", 'r'),
    ("utc", "ScriptOnBlocked", 'r'),
    ("utc", "ScriptOnNotice", 'r'),
    ("utc", "ScriptRested", 'r'),
    ("utc", "ScriptSpellAt", 'r'),
    ("utc", "ScriptUserDefine", 'r'),
    ("utc", "FactionID", 'w'),
    ("utc", "Appearance_Type", 'w'),
    ("utc", "Phenotype", 'i'),
    ("utc", "Gender", 'b'),
    ("utc", "Race", 'b'),
    ("utc", "SoundSetFile", 'w'),
    ("utc", "PortraitId", 'w'),
    ("utc", "Str", 'b'),
    ("utc", "Dex", 'b'),
    ("utc", "Con", 'b'),
    ("utc", "Int", 'b'),
    ("utc", "Wis", 'b'),
    ("utc", "Cha", 'b'),
    ("utc", "NaturalAC", 'b'),
    ("utc", "ChallengeRating", 'f'),
    ("utc", "CRAdjust", 'i'),
    ("utc", "WalkRate", 'i'),
    ("utc", "HitPoints", 'h'),
    ("utc", "CurrentHitPoints", 'h'),
    ("utc", "MaxHitPoints", 'h'),
    ("utc", "fortbonus", 'h'),
    ("utc", "refbonus", 'h'),
    ("utc", "willbonus", 'h'),
    ("utc", "GoodEvil", 'b'),
    ("utc", "LawfulChaotic", 'b'),
    ("utc", "PerceptionRange", 'b'),
    ("utc", "Plot", 'b'),
    ("utc", "IsImmortal", 'b'),
    ("utc", "NoPermDeath", 'b'),
    ("utc", "Interruptable", 'b'),
    ("utc", "Lootable", 'b'),
    ("utc", "Disarmable", 'b'),
    ("utp", "LocName", 'l'),
    ("utp", "Description", 'l'),
    ("utp", "Tag", 's'),
    ("utp", "Comment", 's'),
    ("utp", "Appearance", 'd'),
    ("utp", "Faction", 'd'),
    ("utp", "Conversation", 'r'),
    ("utp", "Static", 'b'),
    ("utp", "Useable", 'b'),
    ("utp", "Plot", 'b'),
    ("utp", "HasInventory", 'b'),
    ("utp", "BodyBag", 'b'),
    ("utp", "Locked", 'b'),
    ("utp", "Lockable", 'b'),
    ("utp", "KeyRequired", 'b'),
    ("utp", "AutoRemoveKey", 'b'),
    ("utp", "KeyName", 's'),
    ("utp", "OpenLockDC", 'b'),
    ("utp", "CloseLockDC", 'b'),
    ("utp", "TrapType", 'b'),
    ("utp", "TrapFlag", 'b'),
    ("utp", "TrapOneShot", 'b'),
    ("utp", "TrapDetectable", 'b'),
    ("utp", "TrapDetectDC", 'b'),
    ("utp", "TrapDisarmable", 'b'),
    ("utp", "DisarmDC", 'b'),
    ("utp", "HP", 'h'),
    ("utp", "CurrentHP", 'h'),
    ("utp", "Hardness", 'b'),
    ("utp", "Fort", 'b'),
    ("utp", "Ref", 'b'),
    ("utp", "Will", 'b'),
    ("utp", "OnUsed", 'r'),
    ("utp", "OnOpen", 'r'),
    ("utp", "OnClosed", 'r'),
    ("utp", "OnDeath", 'r'),
    ("utp", "OnDamaged", 'r'),
    ("utp", "OnHeartbeat", 'r'),
    ("utp", "OnInvDisturbed", 'r'),
    ("utp", "OnLock", 'r'),
    ("utp", "OnUnlock", 'r'),
    ("utp", "OnMeleeAttacked", 'r'),
    ("utp", "OnSpellCastAt", 'r'),
    ("utp", "OnTrapTriggered", 'r'),
    ("utp", "OnDisarm", 'r'),
    ("utp", "OnUserDefined", 'r'),
    ("utp", "OnClick", 'r'),
    ("utd", "LocName", 'l'),
    ("utd", "Description", 'l'),
    ("utd", "Tag", 's'),
    ("utd", "Comment", 's'),
    ("utd", "Appearance", 'd'),
    ("utd", "GenericType_New", 'd'),
    ("utd", "GenericType", 'b'),
    ("utd", "Faction", 'd'),
    ("utd", "Conversation", 'r'),
    ("utd", "Plot", 'b'),
    ("utd", "Interruptable", 'b'),
    ("utd", "Locked", 'b'),
    ("utd", "Lockable", 'b'),
    ("utd", "KeyRequired", 'b'),
    ("utd", "AutoRemoveKey", 'b'),
    ("utd", "KeyName", 's'),
    ("utd", "OpenLockDC", 'b'),
    ("utd", "CloseLockDC", 'b'),
    ("utd", "TrapType", 'b'),
    ("utd", "TrapFlag", 'b'),
    ("utd", "TrapOneShot", 'b'),
    ("utd", "TrapDetectable", 'b'),
    ("utd", "TrapDetectDC", 'b'),
    ("utd", "TrapDisarmable", 'b'),
    ("utd", "DisarmDC", 'b'),
    ("utd", "HP", 'h'),
    ("utd", "CurrentHP", 'h'),
    ("utd", "Hardness", 'b'),
    ("utd", "Fort", 'b'),
    ("utd", "Ref", 'b'),
    ("utd", "Will", 'b'),
    ("utd", "OnOpen", 'r'),
    ("utd", "OnClosed", 'r'),
    ("utd", "OnDeath", 'r'),
    ("utd", "OnDamaged", 'r'),
    ("utd", "OnHeartbeat", 'r'),
    ("utd", "OnLock", 'r'),
    ("utd", "OnUnlock", 'r'),
    ("utd", "OnMeleeAttacked", 'r'),
    ("utd", "OnSpellCastAt", 'r'),
    ("utd", "OnTrapTriggered", 'r'),
    ("utd", "OnDisarm", 'r'),
    ("utd", "OnFailToOpen", 'r'),
    ("utd", "OnUserDefined", 'r'),
    ("utd", "OnClick", 'r'),
    ("utm", "LocName", 'l'),
    ("utm", "Tag", 's'),
    ("utm", "Comment", 's'),
    ("utm", "MarkUp", 'i'),
    ("utm", "MarkDown", 'i'),
    ("utm", "BlackMarket", 'b'),
    ("utm", "BM_MarkDown", 'i'),
    ("utm", "IdentifyPrice", 'i'),
    ("utm", "MaxBuyPrice", 'i'),
    ("utm", "StoreGold", 'i'),
    ("utm", "OnOpenStore", 'r'),
    ("utm", "OnStoreClosed", 'r'),
    ("uti", "LocalizedName", 'l'),
    ("uti", "Description", 'l'),
    ("uti", "DescIdentified", 'l'),
    ("uti", "Tag", 's'),
    ("uti", "Comment", 's'),
    ("uti", "AddCost", 'd'),
    ("uti", "StackSize", 'w'),
    ("uti", "Charges", 'b'),
    ("uti", "Plot", 'b'),
    ("uti", "Cursed", 'b'),
    ("uti", "Stolen", 'b'),
    ("uti", "Identified", 'b'),
];

/// The kinds a builder can make a blueprint of for now.
pub const EXTS: [&str; 6] = ["utc", "uti", "utp", "utd", "utm", "dlg"];

/// The field that holds a blueprint's own resref (a store's is `ResRef`, the rest `TemplateResRef`).
fn resref_label(ext: &str) -> &'static str {
    if ext == "utm" { "ResRef" } else { "TemplateResRef" }
}

/// The builder's own new blueprints and conversations: `blueprints/` in their workspace.
pub(crate) fn dir() -> PathBuf {
    crate::workspace::root().join("blueprints")
}

/// The file of the builder's own blueprint `resref` of type `ext` (`<resref>.<ext>.json`, GFF JSON).
pub(crate) fn path(ext: &str, resref: &str) -> PathBuf {
    dir().join(format!("{resref}.{ext}.json"))
}

/// Creatures and items are copied and edited here; conversations have their own editor (`dialog.rs`).
fn check_ext(ext: &str) -> Result<()> {
    anyhow::ensure!(
        ["utc", "uti", "utp", "utd", "utm"].contains(&ext),
        "a new {ext} blueprint can't be made here (creatures, items, placeables, doors and stores)"
    );
    Ok(())
}

/// A resref, the one rule for every name the editor makes or refers to (blueprints, scripts, conversations, items, the
/// area's scripts): 1 to 16 lower case letters, digits or `_`. The game stores resrefs in lower case and looks them up
/// without regard to case, so a caller with a reference from an existing file lowers it first; an empty reference ("none")
/// is the caller's to allow.
pub fn resref_ok(r: &str) -> bool {
    (1..=16).contains(&r.len()) && r.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A new blueprint of the builder's, as GFF JSON text.
pub fn read(ext: &str, resref: &str) -> Option<String> {
    if !EXTS.contains(&ext) || !resref_ok(resref) {
        return None;
    }
    std::fs::read_to_string(path(ext, resref)).ok()
}

/// The builder's new blueprints of `ext`: (resref, name).
pub fn list(ext: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let fname = e.file_name().to_string_lossy().into_owned();
        let Some(resref) = fname.strip_suffix(&format!(".{ext}.json")) else { continue };
        let Ok(doc) = std::fs::read_to_string(e.path()).map_err(anyhow::Error::from).and_then(|t| Ok(serde_json::from_str::<Json>(&t)?))
        else {
            continue;
        };
        out.push((resref.to_string(), name_of(ext, &doc)));
    }
    out.sort();
    out
}

fn loc_text(v: &Json) -> String {
    v["value"]["0"].as_str().unwrap_or("").to_string()
}

fn name_of(ext: &str, doc: &Json) -> String {
    if ext == "dlg" {
        return String::new();
    }
    if ext == "utp" || ext == "utd" || ext == "utm" {
        return loc_text(&doc["LocName"]);
    }
    if ext == "utc" {
        format!("{} {}", loc_text(&doc["FirstName"]), loc_text(&doc["LastName"])).trim().to_string()
    } else {
        loc_text(&doc["LocalizedName"])
    }
}

/// `resref` if it is one ([`resref_ok`]), else the error that refuses it. A name from a request becomes a file name: an
/// absolute path or a `../` in it would read any `.json` on the server, other builders' work included.
fn named(resref: &str) -> Result<&str> {
    anyhow::ensure!(resref_ok(resref), "{resref:?} isn't a blueprint name (1 to 16 lower case letters, digits or _)");
    Ok(resref)
}

/// The module's own blueprint `resref` of `ext` as GFF JSON text (None: the module has none): from the review service on a
/// team server that has one, else the module checkout. A name that isn't a resref is refused before anything is read.
fn module_blueprint(ext: &str, resref: &str) -> Result<Option<String>> {
    let resref = named(resref)?;
    if crate::review::active() {
        return crate::review::blueprint(ext, resref);
    }
    Ok(std::fs::read_to_string(crate::config::Layout::from_env().blueprint_dir(ext).join(format!("{resref}.{ext}.json"))).ok())
}

/// Whether the module or the game already has a blueprint by this name (a new one may not reuse it).
pub(crate) fn taken(game: &GameData, ext: &str, resref: &str) -> Result<bool> {
    let resref = named(resref)?;
    if module_blueprint(ext, resref)?.is_some() {
        return Ok(true);
    }
    let restype = ResType::from_extension(ext).context("unknown blueprint type")?;
    Ok(game.resman.contains(&ResKey { resref: mg_core::ResRef::from_str(resref)?, restype }))
}

/// Any blueprint's GFF JSON: the builder's, the module's, else the game's.
pub(crate) fn source(game: &GameData, ext: &str, resref: &str) -> Result<Json> {
    let resref = named(resref)?;
    if let Some(t) = read(ext, resref) {
        return Ok(serde_json::from_str(&t)?);
    }
    if let Some(t) = module_blueprint(ext, resref)? {
        return Ok(serde_json::from_str(&t)?);
    }
    let restype = ResType::from_extension(ext).context("unknown blueprint type")?;
    let bytes = game
        .resman
        .get(&ResKey { resref: mg_core::ResRef::from_str(resref)?, restype })
        .with_context(|| format!("no {ext} blueprint {resref}"))?;
    crate::publish::gff_to_json(&bytes, ext)
}

/// Writes the builder's own blueprint as GFF JSON, and forgets any cached copy so the next read sees it.
pub(crate) fn write(ext: &str, resref: &str, doc: &Json) -> Result<()> {
    // Every caller checked these already; checked again here because they become a file name.
    anyhow::ensure!(EXTS.contains(&ext) && resref_ok(resref), "bad blueprint name {resref:?}");
    std::fs::create_dir_all(dir())?;
    std::fs::write(path(ext, resref), serde_json::to_string_pretty(doc)? + "\n")?;
    crate::things::forget_blueprint(ext, resref);
    Ok(())
}

/// A new blueprint, copied from `from` (any blueprint of the same kind), with `resref`, `name` and a tag to match.
pub fn create(game: &GameData, ext: &str, resref: &str, from: &str, name: &str) -> Result<Json> {
    check_ext(ext)?;
    anyhow::ensure!(resref_ok(resref), "a blueprint name is 1 to 16 lower case letters, digits or _");
    anyhow::ensure!(read(ext, resref).is_none() && !taken(game, ext, resref)?, "{resref} is already a {ext} blueprint: pick another name");
    anyhow::ensure!(list(ext).len() < 200, "that's a lot of new blueprints: publish or delete some first");
    let mut doc = source(game, ext, &from.to_ascii_lowercase())?;
    set(&mut doc, ext, resref_label(ext), &json!(resref), 'r')?;
    let label = match ext {
        "utc" => "FirstName",
        "utp" | "utd" | "utm" => "LocName",
        _ => "LocalizedName",
    };
    if ext == "utc" {
        set(&mut doc, ext, "LastName", &json!(""), 'l')?;
    }
    set(&mut doc, ext, label, &json!(name), 'l')?;
    set(&mut doc, ext, "Tag", &json!(resref.to_ascii_uppercase()), 's')?;
    // (The module's own palette ids and the copied blueprint's comment mean nothing for a new one.)
    if let Some(o) = doc.as_object_mut() {
        o.remove("PaletteID");
        o.remove("Comment");
    }
    write(ext, resref, &doc)?;
    Ok(describe(ext, &doc))
}

fn node(ty: char, v: &Json, label: &str) -> Result<Json> {
    let num = || {
        v.as_f64()
            .or_else(|| v.as_bool().map(|b| f64::from(u8::from(b))))
            .or_else(|| v.as_str().and_then(|t| t.trim().parse().ok()))
            .with_context(|| format!("{label}: a number"))
    };
    Ok(match ty {
        'b' => json!({ "type": "byte", "value": num()?.clamp(0.0, 255.0) as u8 }),
        'h' => json!({ "type": "short", "value": num()?.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16 }),
        'i' => json!({ "type": "int", "value": num()? as i32 }),
        'w' => json!({ "type": "word", "value": num()?.clamp(0.0, 65535.0) as u16 }),
        'd' => json!({ "type": "dword", "value": num()?.max(0.0) as u32 }),
        'f' => json!({ "type": "float", "value": num()? as f32 }),
        's' => json!({ "type": "cexostring", "value": v.as_str().unwrap_or("") }),
        'r' => {
            let t = v.as_str().unwrap_or("").to_ascii_lowercase();
            anyhow::ensure!(t.is_empty() || resref_ok(&t), "{label}: '{t}' isn't a resref (up to 16 letters, digits or _)");
            json!({ "type": "resref", "value": t })
        }
        'l' => {
            let t = v.as_str().unwrap_or("");
            json!({ "type": "cexolocstring", "value": if t.is_empty() { json!({}) } else { json!({ "0": t }) } })
        }
        _ => anyhow::bail!("{label}: unknown field type"),
    })
}

fn set(doc: &mut Json, ext: &str, label: &str, v: &Json, ty: char) -> Result<()> {
    anyhow::ensure!(doc.is_object(), "not a blueprint");
    doc[label] = node(ty, v, label)?;
    let _ = ext;
    Ok(())
}

/// Sets `fields` (label to value) on a new blueprint of the builder's; a label that isn't one of the kind's is refused.
pub fn edit(ext: &str, resref: &str, fields: &serde_json::Map<String, Json>) -> Result<Json> {
    check_ext(ext)?;
    let mut doc: Json = serde_json::from_str(&read(ext, resref).with_context(|| format!("no new {ext} blueprint {resref}"))?)?;
    for (label, v) in fields {
        let (_, _, ty) = FIELDS.iter().find(|(e, l, _)| *e == ext && l == label).with_context(|| format!("{label} can't be set here"))?;
        set(&mut doc, ext, label, v, *ty)?;
    }
    if ext == "uti" && fields.keys().any(|k| ["AddCost", "StackSize", "Charges"].contains(&k.as_str())) {
        crate::itemprops::recost(crate::models::game()?, &mut doc, resref)?;
    }
    write(ext, resref, &doc)?;
    Ok(describe(ext, &doc))
}

/// Deletes the builder's own blueprint `resref` (the module's and the game's are never touched); none there is no error.
pub fn delete(ext: &str, resref: &str) -> Result<()> {
    check_ext(ext)?;
    anyhow::ensure!(resref_ok(resref), "bad blueprint name");
    let _ = std::fs::remove_file(path(ext, resref));
    crate::things::forget_blueprint(ext, resref);
    Ok(())
}

/// A new blueprint as the editor's form needs it: its settable fields with their current values.
pub fn describe(ext: &str, doc: &Json) -> Json {
    let fields: Vec<Json> = FIELDS
        .iter()
        .filter(|(e, _, _)| *e == ext)
        .map(|(_, label, ty)| {
            let v = &doc[*label]["value"];
            let value = if *ty == 'l' { json!(v["0"].as_str().unwrap_or("")) } else { v.clone() };
            json!({ "label": label, "type": ty.to_string(), "value": value })
        })
        .collect();
    json!({ "ext": ext, "resref": doc[resref_label(ext)]["value"], "name": name_of(ext, doc), "fields": fields })
}

/// The builder's own blueprint `resref` as the editor's form shows it.
pub fn get(ext: &str, resref: &str) -> Result<Json> {
    check_ext(ext)?;
    let doc: Json = serde_json::from_str(&read(ext, resref).with_context(|| format!("no new {ext} blueprint {resref}"))?)?;
    Ok(describe(ext, &doc))
}

/// The builder's new blueprints that `texts` (GFF JSON documents) name anywhere as an instance, an item carried or worn, or
/// a creature spawned, and those blueprints' own: (`<resref>.<ext>.json`, text). What Publish sends along with an area.
pub fn used_by(texts: &[&Json]) -> Vec<(String, String)> {
    const KEYS: [&str; 5] = ["TemplateResRef", "InventoryRes", "EquippedRes", "ResRef", "Conversation"];
    fn walk(v: &Json, out: &mut std::collections::BTreeSet<String>) {
        match v {
            Json::Object(m) => {
                for (k, c) in m {
                    if KEYS.contains(&k.as_str())
                        && let Some(r) = c["value"].as_str().filter(|r| !r.is_empty())
                    {
                        out.insert(r.to_ascii_lowercase());
                    }
                    walk(c, out);
                }
            }
            Json::Array(a) => a.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    let mut wanted = std::collections::BTreeSet::new();
    texts.iter().for_each(|t| walk(t, &mut wanted));
    let mut files: std::collections::BTreeMap<String, String> = Default::default();
    // A new item a new creature wears is as new as the creature.
    loop {
        let before = files.len();
        for ext in EXTS {
            for (resref, _) in list(ext) {
                if wanted.contains(&resref)
                    && !files.contains_key(&format!("{resref}.{ext}.json"))
                    && let Some(t) = read(ext, &resref)
                {
                    if let Ok(doc) = serde_json::from_str::<Json>(&t) {
                        walk(&doc, &mut wanted);
                    }
                    let t = if ext == "dlg" { crate::dialog::compacted(&t).unwrap_or(t) } else { t };
                    files.insert(format!("{resref}.{ext}.json"), t);
                }
            }
        }
        if files.len() == before {
            break;
        }
    }
    // Scripts of the builder's that any of it names (a conversation's condition, a creature's or area's script field), and the
    // ones those include. (Every text in the documents is a candidate: a script is named by a resref value.)
    fn leaves(v: &Json, out: &mut std::collections::BTreeSet<String>) {
        match v {
            Json::String(s) if s.len() <= 16 => {
                out.insert(s.to_ascii_lowercase());
            }
            Json::Object(m) => m.values().for_each(|c| leaves(c, out)),
            Json::Array(a) => a.iter().for_each(|c| leaves(c, out)),
            _ => {}
        }
    }
    let mut names = std::collections::BTreeSet::new();
    texts.iter().for_each(|t| leaves(t, &mut names));
    for t in files.values().filter_map(|t| serde_json::from_str::<Json>(t).ok()) {
        leaves(&t, &mut names);
    }
    let mut scripts: std::collections::BTreeMap<String, String> = Default::default();
    let mut todo: Vec<String> = crate::newscript::list().into_iter().filter(|n| names.contains(n)).collect();
    while let Some(n) = todo.pop() {
        if scripts.contains_key(&format!("{n}.nss")) {
            continue;
        }
        let Some(text) = crate::newscript::read(&n) else { continue };
        todo.extend(crate::newscript::includes(&text).into_iter().filter(|i| crate::newscript::read(i).is_some()));
        scripts.insert(format!("{n}.nss"), text);
    }
    files.extend(scripts);
    files.into_iter().collect()
}

/// Sets what a new creature carries and wears (either list, when given): the blueprint's own lists, laid out as Aurora does.
pub fn set_gear(
    game: &GameData,
    ext: &str,
    resref: &str,
    inventory: Option<Vec<(String, bool)>>,
    equipment: Option<Vec<(String, String)>>,
) -> Result<Json> {
    anyhow::ensure!(["utc", "utp", "utm"].contains(&ext), "only a creature, a placeable or a store holds things");
    let kind = match ext {
        "utc" => mg_rules::ObjectKind::Creature,
        "utm" => mg_rules::ObjectKind::Store,
        _ => mg_rules::ObjectKind::Placeable,
    };
    let mut doc: Json = serde_json::from_str(&read(ext, resref).with_context(|| format!("no new {ext} {resref}"))?)?;
    let mut bp = crate::things::from_json(&serde_json::to_string(&doc)?, ext, resref)?;
    if let Some(inv) = &inventory {
        crate::things::set_inventory(game, kind, &mut bp, inv)?;
    }
    if let Some(eq) = &equipment {
        anyhow::ensure!(ext == "utc", "only a creature wears things");
        crate::things::set_equipment(game, &mut bp, eq)?;
    }
    let file_type = match ext {
        "utc" => *b"UTC ",
        "utm" => *b"UTM ",
        _ => *b"UTP ",
    };
    let gff = mg_gff::Gff { file_type, version: mg_gff::Gff::VERSION, root: bp };
    let back = crate::publish::gff_to_json(&gff.to_bytes()?, ext)?;
    let held = if ext == "utm" { "StoreList" } else { "ItemList" };
    for key in [held, "Equip_ItemList"] {
        if (key == held && inventory.is_none()) || (key == "Equip_ItemList" && equipment.is_none()) {
            continue;
        }
        doc[key] = back[key].clone();
    }
    write(ext, resref, &doc)?;
    Ok(describe(ext, &doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_is_written_as_the_gff_json_node_of_its_type() {
        let mut d = json!({ "Tag": { "type": "cexostring", "value": "A" } });
        set(&mut d, "utc", "Str", &json!("14"), 'b').unwrap();
        set(&mut d, "utc", "FirstName", &json!("Bob"), 'l').unwrap();
        set(&mut d, "utc", "Conversation", &json!("Bob_Talk"), 'r').unwrap();
        assert_eq!(d["Str"], json!({ "type": "byte", "value": 14 }));
        assert_eq!(d["FirstName"]["value"]["0"], "Bob");
        assert_eq!(d["Conversation"]["value"], "bob_talk");
        assert!(set(&mut d, "utc", "Conversation", &json!("way_too_long_for_a_resref"), 'r').is_err());
        assert_eq!(describe("utc", &d)["fields"].as_array().unwrap().iter().find(|f| f["label"] == "FirstName").unwrap()["value"], "Bob");
    }

    #[test]
    fn a_module_blueprint_is_named_by_a_resref_never_a_path() {
        // Another builder's blueprint, by its absolute path and by `../`: refused, never read.
        let dir = std::env::temp_dir().join(format!("ls-newbp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("secret.utc.json"), r#"{"Description":"BOBS PRIVATE NOTES"}"#).unwrap();
        let absolute = dir.join("secret").display().to_string().to_ascii_lowercase();
        for name in [absolute.as_str(), "../../workspaces/bob/blueprints/secret", "a/b", "secret.utc", ""] {
            let e = module_blueprint("utc", name).unwrap_err();
            assert!(e.to_string().contains("isn't a blueprint name"), "{name}: {e}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_are_resrefs() {
        assert!(resref_ok("my_goblin_2") && resref_ok("a"));
        assert!(!resref_ok("") && !resref_ok("Upper") && !resref_ok("seventeen_chars_xx") && !resref_ok("a b"));
    }
}
