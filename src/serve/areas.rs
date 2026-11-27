// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editor's areas: the list for its Open, saving and loading one, the waypoints and doors other areas can lead to, and an
//! imported area's original objects. Also the checks every route that works on an area makes: the assignment it needs, and
//! that a new area doesn't take an existing one's name.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tiny_http::Method;

use super::query::area_name_ok;
use super::{Answer, Ask, fail, json_response, read_body, refuse, response_header};
use crate::workspace::{areas_dir, exports_dir};

/// Answers the routes of the editor's areas; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let query = cx.query;
    Some(match (&cx.method, cx.path) {
        // The editor's Open: exported areas, one area to load back, and the entrance doors of a module (for Leads to).
        (Method::Get, "/api/areas") => json_response(200, &list_areas()),
        (Method::Post, "/api/save") | (Method::Put, "/api/save") => {
            let name = query.text("name");
            let body = match read_body(cx.req, super::body_max::AREA) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let base = query.nonempty("base");
            match save_area(&name, &body, query.one("new"), query.one("replace"), base.as_deref()) {
                Ok((at, version)) => json_response(200, &serde_json::json!({ "saved": at, "version": version })),
                Err(e) if e.downcast_ref::<crate::quota::Full>().is_some() => fail(413, &e),
                Err(e) => conflict(&name, &e).unwrap_or_else(|| fail(400, &e)),
            }
        }
        // Deleting an area (see `delete_area`); a heavy job's turn, so no export or import of it is under way meanwhile.
        (Method::Delete, "/api/area") => {
            let name = query.text("name");
            let deleted = {
                let _job = super::job();
                delete_area(&name, query.nonempty("base").as_deref())
            };
            crate::audit::log(
                "delete",
                Some(&name),
                deleted.is_ok(),
                &deleted.as_ref().map_or_else(|e| format!("{e:#}"), |kept| format!("set aside in {kept}")),
            );
            match deleted {
                Ok(_) => json_response(200, &serde_json::json!({ "deleted": name })),
                Err(e) if e.downcast_ref::<Denied>().is_some() => fail(403, &e),
                Err(e) if e.downcast_ref::<NoArea>().is_some() => fail(404, &e),
                Err(e) => match e.downcast_ref::<InUse>() {
                    Some(u) => {
                        json_response(409, &serde_json::json!({ "error": crate::workspace::scrub(&format!("{e:#}")), "used_by": u.by }))
                    }
                    None => conflict(&name, &e).unwrap_or_else(|| fail(400, &e)),
                },
            }
        }
        // An imported area's original objects, for the editor to draw and edit.
        (Method::Get, "/api/originals") => {
            let name = query.text("name");
            if !area_name_ok(&name) {
                refuse(400, &format!("bad area name {name:?}"))
            } else if let Err(e) = guard_assignment(&name, &[]) {
                fail(403, &e)
            } else {
                let list = match crate::models::game() {
                    Ok(g) => {
                        crate::import::original(&name).and_then(|o| o.git).map_or_default(|git| crate::things::describe_originals(g, &git))
                    }
                    _ => Vec::new(),
                };
                json_response(200, &serde_json::json!({ "originals": list }))
            }
        }
        // The waypoints and doors the areas of a module have, for a transition or a door to lead to.
        (Method::Get, "/api/links") => json_response(200, &serde_json::json!({ "links": module_links(&query.text("module")) })),
        // The version (`X-Landscaper-Version`) goes with the area, for its next save to send as `base`.
        (Method::Get, "/api/area") => match load_area(&query.text("name")) {
            Ok((a, version)) => tiny_http::Response::from_data(a)
                .with_header(response_header("Content-Type", "application/json"))
                .with_header(response_header("X-Landscaper-Version", version.as_str())),
            Err(e) => fail(404, &e),
        },
        (Method::Get, "/api/doors") => json_response(200, &module_doors(&query.text("module"))),
        // An existing area's waypoints, for a transition to lead to (the review service has them).
        (Method::Get, "/api/waypoints") => match area_waypoints(&query.text("area")) {
            Ok(v) => json_response(200, &v),
            Err(e) => fail(400, &e),
        },
        _ => return None,
    })
}

/// A builder turned away from an area: what the activity log records as `denied`, and the import route tells the admins about.
#[derive(Debug)]
pub(super) enum Denied {
    /// An import of a production area (its resref) that isn't assigned to them.
    NotAssigned(String),
    /// Work on an import whose assignment (of this resref) has ended.
    Ended(String),
    /// A new area (or a copy of an uploaded one) named like one of the module's areas.
    ProductionName(String),
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denied::NotAssigned(resref) => write!(f, "{resref} isn't assigned to you: ask an admin for it"),
            Denied::Ended(resref) => write!(f, "your assignment of {resref} has ended: ask an admin"),
            Denied::ProductionName(name) => {
                write!(f, "{name} is the name of an existing area of {}: pick another name", crate::config::project())
            }
        }
    }
}

impl std::error::Error for Denied {}

/// What a person may do with an area built from an existing one (or referring to an import): their assignment must still stand. A
/// builder's export, check or re-import of one whose assignment ended is refused.
pub(super) fn guard_assignment(name: &str, referenced: &[String]) -> Result<()> {
    // A name that isn't plain (`../`, `/`...) is refused for everyone: it would name a folder outside the workspace.
    for n in std::iter::once(name).chain(referenced.iter().map(String::as_str)) {
        anyhow::ensure!(crate::workspace::plain_name(n), "bad area name {n:?}");
    }
    if crate::workspace::acting_admin() {
        return Ok(());
    }
    let me = crate::workspace::acting_user();
    for n in std::iter::once(name.to_string()).chain(referenced.iter().cloned()) {
        if let Some(area) = crate::import::recorded(&n)
            && !crate::assignments::allows(&me, &area)?
        {
            return Err(Denied::Ended(area).into());
        }
    }
    Ok(())
}

/// A new (not imported) area can't take an existing area's name or tag. Refused, too, when the module's area list isn't
/// available (a review service that has never answered): the name can't be checked.
pub(super) fn guard_new_name(name: &str) -> Result<()> {
    if !crate::workspace::acting_admin() && crate::import::recorded(name).is_none() && crate::import::production_name(name)? {
        return Err(Denied::ProductionName(name.to_string()).into());
    }
    Ok(())
}

/// The imports an area refers to (`@import:<name>` in its paint or objects).
pub(super) fn imports_in(palette: &[String], objects: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> =
        palette.iter().filter_map(|p| p.strip_prefix(crate::paint::IMPORT)).map(|r| r.split(':').next().unwrap_or(r).to_string()).collect();
    for o in objects["objects"].as_array().into_iter().flatten() {
        if let Some(r) = o["model"].as_str().and_then(|m| m.strip_prefix(crate::paint::IMPORT)) {
            out.push(r.split(':').next().unwrap_or(r).to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A save that would replace a different area of the same name, or an import named like an area there is (answered 409,
/// see [`save_area`]).
#[derive(Debug)]
pub(super) struct Taken(pub(super) String);

impl std::fmt::Display for Taken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "there is already an area called {}: open it, pick another name, or replace it", self.0)
    }
}

impl std::error::Error for Taken {}

/// A save that started from a version of the working copy that isn't the one on disk now: another tab or computer saved the
/// area since (answered 409 with `"moved": true`, see [`save_area`]).
#[derive(Debug)]
pub(super) struct Moved {
    name: String,
    /// When the working copy there now was saved (Unix seconds); `None` when there is none (the area was imported again).
    saved: Option<u64>,
    /// The version there now ([`version`]).
    version: String,
}

impl std::fmt::Display for Moved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} was saved from another tab or computer since you opened it: keep yours or open that one", self.name)
    }
}

impl std::error::Error for Moved {}

/// The 409 that answers a refusal of area `name` as [`Taken`] (`{ error, exists }`) or [`Moved`] (with `moved`, `saved` and
/// `version` too), which the editor tells apart; `None` for any other error.
fn conflict(name: &str, e: &anyhow::Error) -> Option<Answer> {
    let error = crate::workspace::scrub(&format!("{e:#}"));
    if let Some(m) = e.downcast_ref::<Moved>() {
        return Some(json_response(
            409,
            &serde_json::json!({ "error": error, "exists": name, "moved": true, "saved": m.saved, "version": m.version }),
        ));
    }
    e.downcast_ref::<Taken>().is_some().then(|| json_response(409, &serde_json::json!({ "error": error, "exists": name })))
}

/// A delete of an area the workspace hasn't got (answered 404).
#[derive(Debug)]
struct NoArea(String);

impl std::fmt::Display for NoArea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "there is no area called {}", self.0)
    }
}

impl std::error::Error for NoArea {}

/// A delete of an area whose import other areas of the workspace use (`@import:<name>` in their paint or objects), answered
/// 409 with `used_by`: they would lose their ground and objects.
#[derive(Debug)]
struct InUse {
    name: String,
    /// The areas that use it, by name.
    by: Vec<String>,
}

impl std::fmt::Display for InUse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is imported, and {} use{} its ground or objects: delete {} first",
            self.name,
            self.by.join(", "),
            if self.by.len() == 1 { "s" } else { "" },
            if self.by.len() == 1 { "it" } else { "them" }
        )
    }
}

impl std::error::Error for InUse {}

/// The version of a working copy, as the editor sends it back as `base`: 16 hex digits of the file's FNV-1a hash, or
/// [`NO_COPY`] when there is no working copy (an area opened from its export or a fresh import). Taken from the bytes, not
/// the file's time: two saves within one second (or one clock tick of the file system) still differ, and a save of the same
/// bytes changes nothing anyone could lose.
/// The area saved at `path` (`name`'s), or None when there is none. An error when it is there but can't be read: taken as
/// none, the save would replace it with no copy kept, past the check that the editor holds its latest version.
fn saved_copy(path: &std::path::Path, name: &str) -> Result<Option<Vec<u8>>> {
    match crate::atomic::read(path) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => anyhow::bail!("{name}'s saved copy ({name}.json) can't be read ({e}): nothing was saved; fix or restore it"),
    }
}

pub(super) fn version(saved: Option<&[u8]>) -> String {
    let Some(bytes) = saved else { return NO_COPY.to_string() };
    // FNV-1a, 64 bits (its published offset basis and prime): stable across Rust versions, unlike `DefaultHasher`, so a
    // restart or an upgrade never makes every open tab's version stale.
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    format!("{hash:016x}")
}

/// The version of an area with no working copy (see [`version`]).
pub(super) const NO_COPY: &str = "none";

/// Whether the workspace has an area called `name`: a working copy, an export or an import.
pub(super) fn area_exists(name: &str) -> bool {
    areas_dir().join(format!("{name}.json")).exists()
        || exports_dir().join(format!("{name}.json")).exists()
        || exports_dir().join(format!("{name}.import")).exists()
}

/// The lock of one working copy (its path): two saves of the same area take turns, so they never interleave their renames, and
/// a delete or a re-import, which move it aside, take it too.
pub(super) fn area_lock(path: &std::path::Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    type Locks = std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>;
    static LOCKS: std::sync::Mutex<Option<Locks>> = std::sync::Mutex::new(None);
    let mut g = LOCKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    g.get_or_insert_with(Default::default).entry(path.to_path_buf()).or_default().clone()
}

/// Saves the editor's area `name` (its JSON). Returns when (Unix seconds).
///
/// - Written whole or not at all ([`crate::atomic::write`]), and saves of one area take turns ([`area_lock`]): a team server
///   answers on many threads, and two overlapping saves (two tabs, an autosave during a slow one) used to leave no
///   `name.json` at all, only `.bak`.
/// - The copy before is kept as `name.json.bak` (a hard link, so `name.json` itself is never moved away).
/// - `new` (`/api/save?new=1`, the first save of an area the editor's New made): refused with [`Taken`] (409) when the
///   workspace already has an area called `name` (a working copy, an export or an import), unless `replace` (`&replace=1`).
/// - A save whose JSON has a `created` value (the editor's id for the area, set when it was made) is refused the same way when
///   the working copy has a different one, unless `replace`: it is another area that happens to have the name.
/// - `base` (`&base=<version>`): the [`version`] of the working copy the editor's area started from (its open, or its last
///   save). Refused with [`Moved`] (409) when the working copy on disk isn't that one any more, unless `replace`: two tabs on
///   one area used to save over each other, the last save winning. A save without `base` (an older editor) isn't checked.
/// - A replaced working copy is kept as `name.json.replaced-<Unix seconds>`, which the next autosave doesn't overwrite. A new
///   area (`new`, or another `created`) that replaces one removes that one's underlay (`underlay.rs`).
///
/// Returns when it was saved (Unix seconds) and its new [`version`].
fn save_area(name: &str, body: &[u8], new: bool, replace: bool, base: Option<&str>) -> Result<(u64, String)> {
    anyhow::ensure!(area_name_ok(name), "bad area name {name:?}");
    guard_new_name(name)?;
    let v: serde_json::Value = serde_json::from_slice(body).context("the area isn't JSON")?;
    anyhow::ensure!(
        v["name"].as_str() == Some(name) && v["tiles"].as_u64().is_some() && v["heights"].is_string(),
        "that isn't area {name}"
    );
    let dir = areas_dir();
    std::fs::create_dir_all(&dir)?;
    let (path, bak) = (dir.join(format!("{name}.json")), dir.join(format!("{name}.json.bak")));
    let lock = area_lock(&path);
    let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let created = |v: &serde_json::Value| v.get("created").filter(|c| !c.is_null() && c.as_str() != Some("")).cloned();
    let old = saved_copy(&path, name)?;
    let other = || {
        let theirs = old.as_deref().and_then(|d| serde_json::from_slice::<serde_json::Value>(d).ok()).and_then(|o| created(&o));
        matches!((created(&v), theirs), (Some(a), Some(b)) if a != b)
    };
    // (A canvas is never replaced by an area: `replace` doesn't reach it.)
    anyhow::ensure!(!(new && super::canvases::canvas_exists(name)), "there is a canvas called {name}: pick another name");
    if !replace && ((new && area_exists(name)) || other()) {
        return Err(Taken(name.to_string()).into());
    }
    let there = version(old.as_deref());
    let moved = base.is_some_and(|b| b != there);
    if !replace && moved {
        let saved = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
        return Err(Moved { name: name.to_string(), saved: old.as_ref().and(saved.map(|d| d.as_secs())), version: there }.into());
    }
    // (A team server's disk quota, crate::quota: the save, and the copy of the area it replaces.)
    let replacing = old.as_ref().filter(|_| replace && (new || other() || moved)).map_or(0, Vec::len);
    crate::quota::room(name, (body.len() + replacing) as u64)?;
    // A new area that replaces another (`new`, or another `created`) doesn't take over its underlay: the picture was traced for
    // that one.
    if replace && (new || other()) {
        let _ = std::fs::remove_file(dir.join(format!("{name}.underlay")));
    }
    if let Some(old) = &old {
        if replacing > 0 {
            crate::atomic::write(&dir.join(format!("{name}.json.replaced-{now}")), old)?;
            crate::quota::prune_replaced(&dir, name);
        }
        let _ = std::fs::remove_file(&bak);
        if std::fs::hard_link(&path, &bak).is_err() {
            crate::atomic::write(&bak, old)?;
        }
    }
    crate::atomic::write(&path, body)?;
    crate::quota::wrote((body.len() + replacing) as u64);
    Ok((now, version(Some(body))))
}

/// Deletes area `name` of the workspace, as the editor's Delete does. `base`: the [`version`] of the working copy the editor
/// holds; refused with [`Moved`] (409) when the one on disk isn't that one any more, so a tab never deletes what another tab
/// has saved since. Refused, too, when the builder's assignment of the area has ended ([`guard_assignment`], 403), when the
/// workspace has no such area ([`NoArea`], 404), and when other areas use its import ([`InUse`], 409).
///
/// What it has goes as follows; the area's name is free again at once, and it no longer counts as one of the workspace's areas
/// ([`crate::quota`]):
/// - its working copy (`areas/<name>.json`, and its underlay `areas/<name>.underlay`) and its import (`exports/<name>.import/`
///   and its picture) are moved, as they are,
///   into a folder of their own in `deleted/` (`<name>-<Unix seconds>/`, see [`crate::workspace::deleted_dir`]), for an admin
///   to bring back. On a team server, the newest 3 deleted copies of each name stay, and they count towards the 5 GB.
/// - what can be made again from them is removed: the copy before the last save (`.bak`), the last export (its header,
///   heights, paint and build folder) and what the editor's Check left of it.
/// - the installed module and the area's hak stay: the module is the builder's own work and may hold other areas, and it
///   needs the hak to load this one. Assignments stay too: they are an admin's, of the production area, not of this copy.
///
/// Returns where the area was set aside, as the builder knows it ([`crate::workspace::shown`]).
fn delete_area(name: &str, base: Option<&str>) -> Result<String> {
    anyhow::ensure!(area_name_ok(name), "bad area name {name:?}");
    let base = base.context("base missing: the version of the area the editor holds")?;
    guard_assignment(name, &[])?;
    let (areas, exports) = (areas_dir(), exports_dir());
    let path = areas.join(format!("{name}.json"));
    let lock = area_lock(&path);
    let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !area_exists(name) {
        return Err(NoArea(name.to_string()).into());
    }
    let old = saved_copy(&path, name)?;
    let there = version(old.as_deref());
    if base != there {
        let saved = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
        return Err(Moved { name: name.to_string(), saved: old.as_ref().and(saved.map(|d| d.as_secs())), version: there }.into());
    }
    let import = exports.join(format!("{name}.import"));
    if import.is_dir() {
        let by = import_users(name);
        if !by.is_empty() {
            return Err(InUse { name: name.to_string(), by }.into());
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let deleted = crate::workspace::deleted_dir();
    std::fs::create_dir_all(&deleted)?;
    // (One folder a delete: when the same name was deleted within the same second, the next second's.)
    let aside = (now..now + 60)
        .map(|at| deleted.join(format!("{name}-{at}")))
        .find(|d| std::fs::create_dir(d).is_ok())
        .with_context(|| format!("no folder for {name} in {}", deleted.display()))?;
    for (from, dir) in [
        (path.clone(), "areas"),
        (areas.join(format!("{name}.underlay")), "areas"),
        (import, "exports"),
        (exports.join(format!("{name}.import.view.png")), "exports"),
    ] {
        if from.exists() {
            let to = aside.join(dir);
            std::fs::create_dir_all(&to)?;
            std::fs::rename(&from, to.join(from.file_name().context("a file name")?))
                .with_context(|| format!("setting {} aside", crate::workspace::shown(&from)))?;
        }
    }
    crate::quota::prune_deleted(&deleted, name);
    // What can be made again: the copy before the last save, the last export and the Check's scratch copy.
    let _ = std::fs::remove_file(areas.join(format!("{name}.json.bak")));
    for dir in [exports.clone(), exports.join(".check")] {
        for ext in ["json", "f32", "materials", "import.png"] {
            let file = format!("{name}.{ext}");
            let _ = std::fs::remove_file(dir.join(&file));
            let _ = std::fs::remove_file(dir.join(format!(".{file}.rollback")));
        }
        let _ = std::fs::remove_dir_all(dir.join(format!("{name}_nwn")));
    }
    crate::quota::forget();
    Ok(crate::workspace::shown(&aside))
}

/// The other areas of the workspace that use area `name`'s import (`@import:<name>` in their paint or objects): working copies
/// and exports, by name. Only what [`imports_in`] reads of each file is kept, not its heights.
fn import_users(name: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Uses {
        #[serde(default)]
        palette: Vec<serde_json::Value>,
        #[serde(default)]
        objects: serde_json::Value,
    }
    let mut by = std::collections::BTreeSet::new();
    for dir in [areas_dir(), exports_dir()] {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            let Some(other) = file.strip_suffix(".json").filter(|n| area_name_ok(n) && *n != name) else { continue };
            let Some(uses) = crate::atomic::read(&e.path()).ok().and_then(|d| serde_json::from_slice::<Uses>(&d).ok()) else { continue };
            let palette: Vec<String> = uses.palette.iter().filter_map(|p| p.as_str().map(str::to_string)).collect();
            if imports_in(&palette, &serde_json::json!({ "objects": uses.objects })).iter().any(|i| i == name) {
                by.insert(other.to_string());
            }
        }
    }
    by.into_iter().collect()
}

/// What the area list and the links read of a saved area or an export's header. Its heights and paint, by far the biggest
/// part of a saved area, are skipped without being kept. Each field is as the file has it (null when it has none).
#[derive(serde::Deserialize)]
struct Header {
    #[serde(default)]
    name: serde_json::Value,
    #[serde(default)]
    tiles: serde_json::Value,
    #[serde(default, rename = "tilesY")]
    tiles_y: serde_json::Value,
    #[serde(default)]
    kind: serde_json::Value,
    #[serde(default)]
    module: serde_json::Value,
    #[serde(default)]
    imported: serde_json::Value,
    #[serde(default)]
    things: serde_json::Value,
}

impl Header {
    /// The header of the area at `path`, if it is one.
    fn read(path: &std::path::Path) -> Option<Header> {
        serde_json::from_slice(&crate::atomic::read(path).ok()?).ok()
    }

    /// The area's row in the list (without when it was saved or exported).
    fn row(&self, name: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name, "tiles": self.tiles, "tilesY": self.tiles_y.as_u64().or(self.tiles.as_u64()),
            "kind": self.kind.as_str().unwrap_or("surface"), "module": self.module.as_str().unwrap_or(name),
        })
    }
}

/// The areas there are: saved by the editor, exported, or both; newest first.
fn list_areas() -> serde_json::Value {
    let secs = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut by_name: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
    for e in std::fs::read_dir(areas_dir()).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(h) = Header::read(&p) else { continue };
        let Some(name) = h.name.as_str().filter(|n| area_name_ok(n)) else { continue };
        let mut row = h.row(name);
        row["saved"] = e.metadata().and_then(|m| m.modified()).map_or(0, secs).into();
        by_name.insert(name.to_string(), row);
    }
    let mut out: Vec<(std::time::SystemTime, serde_json::Value)> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(exports_dir()) {
        for e in dir.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json")
                || p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.contains('.'))
            {
                continue;
            }
            let Some(h) = Header::read(&p) else { continue };
            let Some(name) = h.name.as_str().filter(|_| h.tiles.as_u64().is_some()) else { continue };
            let when = secs(e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH));
            match by_name.get_mut(name) {
                // Freshly imported (its export header written by the import): not exported yet, nothing to note but that it exists.
                Some(_) if h.imported.as_bool() == Some(true) => {}
                Some(a) => a["exported"] = when.into(),
                None => {
                    let mut row = h.row(name);
                    row[if h.imported.as_bool() == Some(true) { "saved" } else { "exported" }] = when.into();
                    by_name.insert(name.to_string(), row);
                }
            }
        }
    }
    out.extend(by_name.into_values().map(|mut a| {
        let latest = a["saved"].as_u64().max(a["exported"].as_u64()).unwrap_or(0);
        // Imported: it can be imported again from its kept original.
        a["imported"] = a["name"].as_str().is_some_and(|n| crate::paint::import_dir(n).join("area.are").exists()).into();
        (std::time::UNIX_EPOCH + std::time::Duration::from_secs(latest), a)
    }));
    out.sort_by_key(|a| std::cmp::Reverse(a.0));
    serde_json::json!({ "areas": out.into_iter().map(|(_, v)| v).collect::<Vec<_>>() })
}

/// An area as the editor keeps one (Area.toJSON), as JSON: its saved working copy as it was saved, else its last export (header,
/// heights and paint in base64). With the working copy's [`version`] ([`NO_COPY`] for an export).
fn load_area(name: &str) -> Result<(Vec<u8>, String)> {
    use base64::Engine;
    anyhow::ensure!(area_name_ok(name), "bad area name");
    // The editor's working copy (sent as it is, once it is known to be JSON), else the last export.
    if let Ok(saved) = crate::atomic::read(&areas_dir().join(format!("{name}.json"))) {
        serde_json::from_slice::<serde::de::IgnoredAny>(&saved)?;
        let version = version(Some(&saved));
        return Ok((saved, version));
    }
    let dir = exports_dir();
    let mut h: serde_json::Value =
        serde_json::from_str(&crate::atomic::read_to_string(&dir.join(format!("{name}.json"))).context("no such exported area")?)?;
    let b64 = base64::engine::general_purpose::STANDARD;
    h["heights"] = b64.encode(crate::atomic::read(&dir.join(format!("{name}.f32")))?).into();
    if let Ok(m) = crate::atomic::read(&dir.join(format!("{name}.materials"))) {
        h["materials"] = b64.encode(m).into();
    }
    if h["palette"].is_null() {
        h["palette"] = serde_json::json!(["grassMedium"]);
    }
    Ok((serde_json::to_vec(&h)?, NO_COPY.to_string()))
}

/// The tags of the waypoints and doors placed in the editor's areas of `module`: [{ tag, area, kind }].
fn module_links(module: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if module.is_empty() || !module.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return out;
    }
    for e in std::fs::read_dir(areas_dir()).into_iter().flatten().flatten() {
        if e.path().extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(a) = Header::read(&e.path()) else { continue };
        if a.module.as_str().unwrap_or(a.name.as_str().unwrap_or("")) != module {
            continue;
        }
        for t in a.things.as_array().into_iter().flatten() {
            let (kind, tag) = (t["kind"].as_str().unwrap_or(""), t["tag"].as_str().unwrap_or(""));
            if !tag.is_empty() && matches!(kind, "waypoint" | "door") {
                out.push(serde_json::json!({ "tag": tag, "area": a.name, "kind": kind }));
            }
        }
    }
    out
}

/// The landscaper entrance doors (tags `<area>_ent<n>`) in the installed module `module`: [{ tag, area }].
fn module_doors(module: &str) -> serde_json::Value {
    let mut out = Vec::new();
    let path = crate::workspace::nwn_user_dir().map(|u| u.join("modules").join(format!("{module}.mod")));
    if let Some(m) = path
        .filter(|_| !module.is_empty() && module.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .and_then(|p| mg_module::Module::open(&p).ok())
    {
        for a in m.areas().unwrap_or_default() {
            let Some(Ok(git)) = m.gff(&mg_resman::ResKey { resref: a, restype: mg_core::ResType::GIT }) else { continue };
            for d in git.root.list("Door List").unwrap_or(&[]) {
                let tag = d.string("Tag").map_or_default(|t| String::from_utf8_lossy(t).into_owned());
                if tag.rsplit_once("_ent").is_some_and(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
                    out.push(serde_json::json!({ "tag": tag, "area": a.to_string() }));
                }
            }
        }
    }
    serde_json::json!({ "doors": out })
}

/// An existing area's waypoints (tags and places) from the review service, for a transition to lead to.
fn area_waypoints(area: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(resref_ok(area), "which area?");
    crate::review::call("GET", &format!("/api/v1/areas/{}/waypoints", area.to_ascii_lowercase()), None)
}

/// An existing area's resref as the service's path may carry it: letters, digits, `_ . -`, up to 64, not starting with `.`.
fn resref_ok(n: &str) -> bool {
    (1..=64).contains(&n.len()) && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resrefs_that_may_be_passed_on() {
        for ok in ["bay1", "WP_gate", "a-b.c"] {
            assert!(resref_ok(ok), "{ok}");
        }
        for bad in ["", ".x", "a b", "a/b", "a;b", &"x".repeat(65)] {
            assert!(!resref_ok(bad), "{bad}");
        }
    }
}
