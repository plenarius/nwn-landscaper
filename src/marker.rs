// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A hidden mark in what a builder exports, so a leaked area can be traced.
//!
//! On a team server every export gets a short id that belongs to the builder and the area (kept in `markers.json` beside the
//! workspaces, and written to the audit log). The id goes into the area's `VarTable` (`ls_ref`), its Comments, and the `VarTable` of
//! every creature, placeable and door in its GIT. It deters; it does not prevent: someone who knows can strip it.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_gff::{Struct, Value};
use mg_module::Module;
use mg_resman::ResKey;

use crate::paint;

/// The variable the id is stored in.
pub const VAR: &str = "ls_ref";

static LOCK: Mutex<()> = Mutex::new(());

/// The file the marks are kept in.
pub fn file() -> PathBuf {
    paint::data_dir().join("markers.json")
}

/// Whether `markers.json` reads (a team server checks it at start).
pub fn check() -> Result<()> {
    crate::records::read::<HashMap<String, String>>(&file()).map(drop).with_context(|| file().display().to_string())
}

/// The id for `user`'s work on `area`: made once, then the same every time. An error when `markers.json` is there but doesn't
/// parse (read as empty, it would be written back with this one id, and every earlier mark would lose its builder), or when
/// the new id can't be written (the export would carry a mark nothing traces).
pub fn id(user: &str, area: &str) -> Result<String> {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = format!("{user}/{}", area.to_ascii_lowercase());
    let mut map: HashMap<String, String> = crate::records::read(&file())?;
    if let Some(id) = map.get(&key) {
        return Ok(id.clone());
    }
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write(key.as_bytes());
    h.write_u64(crate::assignments::now());
    let id = format!("{:012x}", h.finish() & 0xffff_ffff_ffff);
    map.insert(key, id.clone());
    // (Written whole or not at all: a cut-off file would stop every export until it is restored.)
    crate::atomic::write(&file(), &serde_json::to_vec_pretty(&map)?).context("markers.json")?;
    Ok(id)
}

/// Which builder and area an id belongs to, as `<user>/<area>` (`landscaper marker <id>`).
pub fn who(id: &str) -> Result<Option<String>> {
    let map: HashMap<String, String> = crate::records::read(&file())?;
    Ok(map.into_iter().find(|(_, v)| v == id).map(|(k, _)| k))
}

/// Sets (or replaces) the string variable `ls_ref` in a `VarTable` list.
fn set_var(s: &mut Struct, id: &str) {
    let mut var = Struct::new(0);
    var.set("Name", Value::String(VAR.as_bytes().to_vec()));
    var.set("Type", Value::Dword(3));
    var.set("Value", Value::String(id.as_bytes().to_vec()));
    match s.list_mut("VarTable") {
        Some(list) => {
            list.retain(|v| v.string("Name") != Some(VAR.as_bytes()));
            list.push(var);
        }
        None => s.set("VarTable", Value::List(vec![var])),
    }
}

/// Marks the areas of the module file `module` that are on `tileset`.
pub fn stamp_module(module: &Path, tileset: &str, id: &str) -> Result<usize> {
    let mut m = Module::open(module).with_context(|| format!("opening {}", module.display()))?;
    let mut marked = 0;
    for area in m.areas()? {
        let are_key = ResKey { resref: area, restype: ResType::ARE };
        let Some(Ok(mut are)) = m.gff(&are_key) else { continue };
        if !are.root.resref("Tileset").is_some_and(|t| t.to_string().eq_ignore_ascii_case(tileset)) {
            continue;
        }
        set_var(&mut are.root, id);
        let tag = format!("ref:{id}");
        let comments = are.root.string("Comments").map_or_default(|b| String::from_utf8_lossy(b).into_owned());
        if !comments.contains(&tag) {
            let text = if comments.is_empty() { tag } else { format!("{comments}\n{tag}") };
            are.root.set("Comments", Value::String(text.into_bytes()));
        }
        m.set_gff(are_key, &are)?;
        let git_key = ResKey { resref: area, restype: ResType::GIT };
        if let Some(Ok(mut git)) = m.gff(&git_key) {
            for list in ["Creature List", "Placeable List", "Door List"] {
                if let Some(items) = git.root.list_mut(list) {
                    for item in items {
                        set_var(item, id);
                    }
                }
            }
            m.set_gff(git_key, &git)?;
        }
        marked += 1;
    }
    crate::atomic::write(module, &m.to_archive_bytes()?)?;
    Ok(marked)
}
