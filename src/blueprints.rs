// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The blueprints a builder can place: the skeleton module's (its `core/`, through the review service on a team
//! server, else a checkout) and the game's own (base game and haks), searched by name or resref.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use mg_core::{ResType, StrRef};
use mg_gff::Struct;
use mg_resman::ResKey;
use mg_rules::{GameData, ObjectKind};
use serde_json::{Value as Json, json};

use crate::things::{ext, kind_name, name_label};

#[derive(Clone, Debug)]
struct Entry {
    resref: String,
    name: String,
}

type Index = std::sync::Arc<Vec<Entry>>;

/// The module's index is re-read after this many seconds (blueprints change); the game's never does.
const MODULE_SECS: u64 = 600;
static MODULE: Mutex<Option<HashMap<String, (std::time::Instant, Index)>>> = Mutex::new(None);
static GAME: Mutex<Option<HashMap<String, Index>>> = Mutex::new(None);

/// A localized string from a blueprint's JSON (`{"0": "text"}` and/or `{"id": strref}`).
fn json_name(game: &GameData, v: &Json) -> String {
    let v = &v["value"];
    if let Some(t) = v["0"].as_str().filter(|t| !t.trim().is_empty()) {
        return t.to_string();
    }
    v["id"].as_u64().and_then(|id| game.string(StrRef(id as u32))).unwrap_or_default()
}

/// A blueprint's name in English, from its localized string `label` (its text, else the talk table's); empty when none.
pub fn struct_name(game: &GameData, s: &Struct, label: &str) -> String {
    let Some(l) = s.locstring(label) else { return String::new() };
    l.text(mg_core::Language::ENGLISH, mg_core::Gender::Male)
        .map(std::borrow::Cow::into_owned)
        .filter(|t| !t.trim().is_empty())
        .or_else(|| game.string(l.strref))
        .unwrap_or_default()
}

/// The checkout's blueprints of a kind (read once).
fn module_index(game: &GameData, kind: ObjectKind) -> Index {
    let key = ext(kind);
    if let Some((at, i)) = MODULE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|m| m.get(&key))
        && at.elapsed().as_secs() < MODULE_SECS
    {
        return i.clone();
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(crate::config::Layout::from_env().blueprint_dir(&key)).into_iter().flatten().flatten() {
        let fname = e.file_name().to_string_lossy().into_owned();
        let Some(resref) = fname.strip_suffix(&format!(".{key}.json")) else { continue };
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(j) = serde_json::from_str::<Json>(&text) else { continue };
        let label = name_label(kind);
        let mut name = json_name(game, &j[label]);
        if kind == ObjectKind::Creature {
            let last = json_name(game, &j["LastName"]);
            if !last.is_empty() {
                name = format!("{name} {last}");
            }
        }
        out.push(Entry { resref: resref.to_string(), name });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.resref.cmp(&b.resref)));
    let out = Index::new(out);
    MODULE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(key, (std::time::Instant::now(), out.clone()));
    out
}

/// The game's (base game and haks) blueprints of a kind (read once).
fn game_index(game: &GameData, kind: ObjectKind) -> Index {
    let key = ext(kind);
    if let Some(i) = GAME.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|m| m.get(&key)) {
        return i.clone();
    }
    let restype: ResType = kind.restype();
    let mut out = Vec::new();
    for r in game.resman.list(restype) {
        let Ok(bytes) = game.resman.get(&ResKey { resref: r, restype }) else { continue };
        let Ok(g) = mg_gff::Gff::read(&bytes) else { continue };
        let mut name = struct_name(game, &g.root, name_label(kind));
        if kind == ObjectKind::Creature {
            let last = struct_name(game, &g.root, "LastName");
            if !last.is_empty() {
                name = format!("{name} {last}");
            }
        }
        out.push(Entry { resref: r.to_string().to_ascii_lowercase(), name });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.resref.cmp(&b.resref)));
    let out = Index::new(out);
    GAME.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert_with(HashMap::new).insert(key, out.clone());
    out
}

fn matches(e: &Entry, q: &str) -> bool {
    q.is_empty() || e.resref.contains(q) || e.name.to_lowercase().contains(q)
}

/// Blueprints of `kind` whose name or resref has `q` (names first that start with it), at most `limit`:
/// `[{ resref, name, source }]`.
pub fn search(game: &GameData, kind: ObjectKind, q: &str, limit: usize) -> Result<Vec<Json>> {
    let q = q.trim().to_lowercase();
    let mut out: Vec<(bool, Json)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |e: &Entry, source: &str, out: &mut Vec<(bool, Json)>| {
        if seen.insert(e.resref.clone()) {
            let starts = e.name.to_lowercase().starts_with(&q) || e.resref.starts_with(&q);
            out.push((
                starts,
                json!({ "resref": e.resref, "name": if e.name.is_empty() { e.resref.clone() } else { e.name.clone() }, "source": source }),
            ));
        }
    };
    // The builder's own new blueprints first.
    let ext_name = ext(kind);
    for (resref, name) in
        crate::newbp::list(&ext_name).into_iter().filter(|(r, n)| q.is_empty() || r.contains(&q) || n.to_lowercase().contains(&q))
    {
        push(&Entry { resref, name }, "mine", &mut out);
    }
    if crate::review::active() {
        for b in crate::review::blueprint_list(&ext(kind), &q, limit)? {
            if let Some(r) = b["resref"].as_str() {
                push(&Entry { resref: r.to_string(), name: b["name"].as_str().unwrap_or("").to_string() }, "module", &mut out);
            }
        }
    } else {
        for e in module_index(game, kind).iter().filter(|e| matches(e, &q)) {
            push(e, "module", &mut out);
        }
    }
    for e in game_index(game, kind).iter().filter(|e| matches(e, &q)) {
        push(e, "game", &mut out);
    }
    out.sort_by_key(|(starts, _)| !*starts);
    Ok(out.into_iter().take(limit).map(|(_, j)| j).collect())
}

/// What the editor needs to build its forms: each kind's settable fields, and the new-area template.
pub fn meta() -> Json {
    let kinds: Vec<Json> = ObjectKind::ALL
        .iter()
        .map(|k| {
            json!({
                "kind": kind_name(*k),
                "fields": crate::things::fields_of(*k)
                    .into_iter()
                    .map(|(l, t)| json!({ "label": l, "type": t.to_string() }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "kinds": kinds, "template": crate::settings::template() })
}
