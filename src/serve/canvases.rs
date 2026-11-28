// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editor's canvases (docs/DESIGN-canvas-cut.md, step 3; the files are `crate::canvas`): the list, making one, loading its
//! header and its blocks, saving the header and the blocks that changed, and deleting one.
//!
//! - `GET /api/canvases`: `{ canvases: [{ name, tiles, tilesY, saved }] }`; one that can't be read has `damaged` instead.
//! - `POST /api/canvas-new?name=` with the header as the body: the canvas, every block flat at its `ground`. Answers the versions.
//! - `GET /api/canvas?name=`: `{ header, version, blocks: { b03_02: version } }` (blocks without a file left out: `none`).
//! - `GET /api/canvas-block?name=&block=b03_02`: the block file (see `crate::canvas`), its version in `X-Landscaper-Version`.
//! - `PUT /api/canvas-save?name=` with `{ header?, base?, blocks: [{ block, base, heights, paint, chasm?, blend? }] }`: `header`
//!   with `base`, the version it started from; each block with its `base` and its heights (f32), paint, chasm mask and blend
//!   layer (bytes, two a sample: soft paint's second texture and weight) in base64. Answers `{ saved, version, blocks }`: the header's version and each saved block's. 409 with `moved` when a base
//!   is not the version there now: `{ moved: { canvas.json | b03_02: version there now }, version }`.
//! - `DELETE /api/canvas?name=` with `{ base, blocks }`, the versions the tab holds of the header and of every block it has
//!   loaded or saved (one left out is `none`).
//!
//! - `GET /api/canvas-cut?name=`: the canvas cut, nothing built: `{ canvas, pieces: [{ name, id, things, objects, entrances,
//!   water } | { name, id, refused }], crossings: [...], warnings }` (`crate::cut`).
//! - `POST /api/canvas-check?name=&pieces=a,b` and `POST /api/canvas-export?name=&pieces=a,b`: each piece (`pieces` left out:
//!   all of them) checked, or exported into one module named after the canvas, then the route check across the canvas:
//!   `{ canvas, module, pieces: [{ name, report }], crossings, routes: { start, reached, problems }, warnings }`.
//!
//! On a team server a canvas is one more item of the builder's workspace, as an area is (the same quota, an admin reads it
//! without changing it). A builder needs, as for an area, an assignment that still stands for every existing area the canvas
//! is built from: its name's import, the imports its paint and objects use, and any piece named like an existing area.

use anyhow::{Context, Result};
use base64::Engine;
use tiny_http::Method;

use super::areas::{Denied, area_exists, guard_assignment, guard_new_name, imports_in};
use super::{Answer, Ask, fail, json_response, read_body, response_header};
use crate::canvas::{self, BlockId, Canvas, Damaged, Moved, NoCanvas, Seam};
use crate::cut;

/// Answers the canvas routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let query = cx.query;
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/canvases") => json_response(200, &list()),
        (Method::Post, "/api/canvas-new") => {
            let name = query.text("name");
            let body = match read_body(cx.req, super::body_max::AREA) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let made = create(&name, &body);
            crate::audit::log("canvas_new", Some(&name), made.is_ok(), &made.as_ref().map_or_else(|e| format!("{e:#}"), |_| String::new()));
            match made {
                Ok(v) => json_response(200, &serde_json::to_value(v).unwrap_or_default()),
                Err(e) => answer_error(&e),
            }
        }
        (Method::Get, "/api/canvas") => match load(&query.text("name")) {
            Ok((header, v)) => json_response(200, &serde_json::json!({ "header": header, "version": v.version, "blocks": v.blocks }))
                .with_header(response_header("X-Landscaper-Version", v.version.as_str())),
            Err(e) => answer_error(&e),
        },
        (Method::Get, "/api/canvas-block") => match load_block(&query.text("name"), &query.text("block")) {
            Ok((bytes, version)) => tiny_http::Response::from_data(bytes)
                .with_header(response_header("Content-Type", "application/octet-stream"))
                .with_header(response_header("X-Landscaper-Version", version.as_str())),
            Err(e) => answer_error(&e),
        },
        (Method::Post | Method::Put, "/api/canvas-save") => {
            let name = query.text("name");
            let body = match read_body(cx.req, super::body_max::CANVAS) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            match save(&name, &body) {
                Ok(v) => {
                    json_response(200, &serde_json::json!({ "saved": crate::assignments::now(), "version": v.version, "blocks": v.blocks }))
                }
                Err(e) => answer_error(&e),
            }
        }
        (Method::Get, "/api/canvas-cut") => match cut_summary(&query.text("name")) {
            Ok(v) => json_response(200, &v),
            Err(e) => answer_error(&e),
        },
        (Method::Post, p @ ("/api/canvas-export" | "/api/canvas-check")) => {
            let check = p == "/api/canvas-check";
            let name = query.text("name");
            let only: Vec<String> = query.text("pieces").split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
            let done = {
                let _job = super::job();
                export(&name, &only, check)
            };
            if !check {
                crate::quota::forget();
                crate::audit::log(
                    "canvas_export",
                    Some(&name),
                    done.is_ok(),
                    &done.as_ref().map_or_else(|e| format!("{e:#}"), |_| String::new()),
                );
            }
            match done {
                Ok(v) => json_response(200, &v),
                Err(e) => answer_error(&e),
            }
        }
        (Method::Delete, "/api/canvas") => {
            let name = query.text("name");
            let body = match read_body(cx.req, super::body_max::FIELDS) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let deleted = delete(&name, &body);
            crate::audit::log(
                "canvas_delete",
                Some(&name),
                deleted.is_ok(),
                &deleted.as_ref().map_or_else(|e| format!("{e:#}"), |kept| format!("set aside in {kept}")),
            );
            match deleted {
                Ok(_) => json_response(200, &serde_json::json!({ "deleted": name })),
                Err(e) => answer_error(&e),
            }
        }
        _ => return None,
    })
}

/// The answer to a refused canvas request: 409 with `moved` for [`Moved`] (and the versions there now), 404 for
/// [`NoCanvas`], 403 for an assignment, 413 for a full workspace, 500 for a [`Damaged`] file, 400 for the rest.
fn answer_error(e: &anyhow::Error) -> Answer {
    if let Some(m) = e.downcast_ref::<Moved>() {
        let error = crate::workspace::scrub(&format!("{e:#}"));
        return json_response(409, &serde_json::json!({ "error": error, "moved": m.moved, "version": m.version }));
    }
    let status = if e.downcast_ref::<NoCanvas>().is_some() {
        404
    } else if e.downcast_ref::<Denied>().is_some() {
        403
    } else if e.downcast_ref::<crate::quota::Full>().is_some() {
        413
    } else if e.downcast_ref::<Damaged>().is_some() {
        500
    } else if e.downcast_ref::<Seam>().is_some() {
        409
    } else {
        400
    };
    fail(status, e)
}

/// Canvas `name` of the workspace, its name checked.
fn canvas(name: &str) -> Result<Canvas> {
    anyhow::ensure!(canvas::name_ok(name), "bad canvas name {name:?} (1-7 lower case letters, digits or _)");
    Ok(Canvas::at(&crate::workspace::canvases_dir(), name))
}

/// Whether the workspace has a canvas called `name`.
pub(super) fn canvas_exists(name: &str) -> bool {
    canvas::name_ok(name) && Canvas::at(&crate::workspace::canvases_dir(), name).exists()
}

/// What a person may do with a canvas whose header is `header`: as for an area ([`guard_assignment`]), every existing area it
/// is built from needs their assignment (its name's import, the imports its paint and objects use); and each piece named like
/// an existing area (which its publish would replace) needs that area's. Admins and your own machine: everything.
fn guard(name: &str, header: &serde_json::Value) -> Result<()> {
    let palette: Vec<String> = header["palette"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
    guard_assignment(name, &imports_in(&palette, header))?;
    if crate::workspace::acting_admin() {
        return Ok(());
    }
    let me = crate::workspace::acting_user();
    for piece in header["pieces"].as_array().into_iter().flatten().filter_map(|p| p["name"].as_str()) {
        if crate::import::production_name(piece)? && !crate::assignments::allows(&me, piece)? {
            return Err(Denied::NotAssigned(piece.to_string()).into());
        }
    }
    Ok(())
}

/// The canvases of the workspace.
fn list() -> serde_json::Value {
    let rows: Vec<serde_json::Value> = canvas::list(&crate::workspace::canvases_dir())
        .into_iter()
        .map(|(name, header, saved)| match header {
            Ok(h) => serde_json::json!({ "name": name, "tiles": h.size.tiles_x, "tilesY": h.size.tiles_y, "saved": saved }),
            Err(e) => serde_json::json!({ "name": name, "saved": saved, "damaged": crate::workspace::scrub(&format!("{e:#}")) }),
        })
        .collect();
    serde_json::json!({ "canvases": rows })
}

/// Makes canvas `name` from the header in `body`. Refused when the workspace has an area or a canvas of that name, when the
/// name is an existing area's (as a new area's is, unless it is that area's import), and without the assignments it needs.
fn create(name: &str, body: &[u8]) -> Result<canvas::Versions> {
    let c = canvas(name)?;
    let header: serde_json::Value = serde_json::from_slice(body).context("the canvas header isn't JSON")?;
    guard_new_name(name)?;
    guard(name, &header)?;
    let lock = canvas::lock(&c.dir);
    let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    anyhow::ensure!(!c.exists() && !area_exists(name), "there is already an area or a canvas called {name}: pick another name");
    crate::quota::room(name, body.len() as u64)?;
    let made = c.create(header)?;
    crate::quota::wrote(body.len() as u64);
    Ok(made)
}

/// Canvas `name`'s header and the versions of it and of every block that has a file.
fn load(name: &str) -> Result<(serde_json::Value, canvas::Versions)> {
    let c = canvas(name)?;
    let header = c.header()?;
    let blocks = c.block_versions(&header)?;
    Ok((header.json, canvas::Versions { version: header.version, blocks }))
}

/// Block `key` of canvas `name`: its file's bytes, and its version. A block with no file: flat ground at the header's height
/// in the same form, and version `none`.
fn load_block(name: &str, key: &str) -> Result<(Vec<u8>, String)> {
    let c = canvas(name)?;
    let id = BlockId::parse(key).with_context(|| format!("bad block name {key:?} (b03_02)"))?;
    let header = c.header()?;
    anyhow::ensure!(header.size.has(id), "canvas {name} has no block {key}");
    Ok(match c.block_bytes(&header, id)? {
        Some(found) => found,
        None => {
            let (w, h) = header.size.samples(id);
            (canvas::Block::flat(w, h, header.ground).encode(), canvas::NONE.to_string())
        }
    })
}

/// A save as the editor sends it (see the module comment).
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveBody {
    #[serde(default)]
    header: Option<serde_json::Value>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    blocks: Vec<BlockBody>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockBody {
    block: String,
    base: String,
    heights: String,
    paint: String,
    #[serde(default)]
    chasm: Option<String>,
    #[serde(default)]
    blend: Option<String>,
}

/// Saves what `body` changes of canvas `name`: all of it or, when a base is stale, a block damaged, a shared row would
/// differ, or the workspace is full, none of it. The versions after.
fn save(name: &str, body: &[u8]) -> Result<canvas::Versions> {
    let c = canvas(name)?;
    let sent: SaveBody = serde_json::from_slice(body).context("the save isn't a canvas save")?;
    guard_new_name(name)?;
    let lock = canvas::lock(&c.dir);
    let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let old = c.header()?;
    let size = old.size;
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut blocks = Vec::new();
    for b in sent.blocks {
        let id =
            BlockId::parse(&b.block).filter(|id| size.has(*id)).with_context(|| format!("canvas {name} has no block {:?}", b.block))?;
        let (w, h) = size.samples(id);
        let bytes = |what: &str, s: &str| b64.decode(s).with_context(|| format!("block {}: its {what} aren't base64", b.block));
        let heights = bytes("heights", &b.heights)?;
        anyhow::ensure!(heights.len() == w * h * 4, "block {}: {} bytes of heights, {} expected", b.block, heights.len(), w * h * 4);
        let heights = heights.as_chunks::<4>().0.iter().map(|q| f32::from_le_bytes(*q)).collect();
        let chasm = b.chasm.as_deref().map(|s| bytes("chasm mask", s)).transpose()?;
        let blend = b.blend.as_deref().map(|s| bytes("blend layer", s)).transpose()?;
        let block = canvas::Block::new(w, h, heights, bytes("paint", &b.paint)?, chasm)
            .and_then(|b| b.with_blend(blend.as_deref()))
            .with_context(|| format!("block {}", b.block))?;
        blocks.push((id, b.base, block));
    }
    let header = match sent.header {
        Some(h) => Some((h, sent.base.context("base missing: the version of the header the editor started from")?)),
        None => None,
    };
    // (Checked against the header as it will be: a save can't add an import or a piece name its builder isn't assigned.)
    guard(name, header.as_ref().map_or(&old.json, |h| &h.0))?;
    let plan = c.plan(canvas::Save { header, blocks })?;
    crate::quota::room(name, plan.adding)?;
    let adding = plan.adding;
    let saved = c.write(plan)?;
    crate::quota::wrote(adding);
    Ok(saved)
}

/// A delete as the editor sends it: the versions it holds.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBody {
    base: String,
    #[serde(default)]
    blocks: std::collections::BTreeMap<String, String>,
}

/// Deletes canvas `name` when `body`'s versions are those on disk ([`Canvas::check_unchanged`]): its folder is set aside whole
/// in `deleted/<name>-<Unix seconds>/canvases/`, for an admin to bring back, as a deleted area's files are. Where, as the builder
/// knows it.
fn delete(name: &str, body: &[u8]) -> Result<String> {
    let c = canvas(name)?;
    let sent: DeleteBody = serde_json::from_slice(body).context("base missing: the versions of the canvas the editor holds")?;
    let lock = canvas::lock(&c.dir);
    let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let header = c.header()?;
    guard(name, &header.json)?;
    c.check_unchanged(&sent.base, &sent.blocks)?;
    let deleted = crate::workspace::deleted_dir();
    std::fs::create_dir_all(&deleted)?;
    let now = crate::assignments::now();
    // (One folder a delete: when the same name was deleted within the same second, the next second's.)
    let aside = (now..now + 60)
        .map(|at| deleted.join(format!("{name}-{at}")))
        .find(|d| std::fs::create_dir(d).is_ok())
        .with_context(|| format!("no folder for {name} in the deleted areas"))?;
    std::fs::create_dir_all(aside.join("canvases"))?;
    std::fs::rename(&c.dir, aside.join("canvases").join(name)).with_context(|| format!("setting canvas {name} aside"))?;
    crate::quota::prune_deleted(&deleted, name);
    crate::quota::forget();
    Ok(crate::workspace::shown(&aside))
}

/// Canvas `name` cut, nothing built ([`cut::summary`]).
fn cut_summary(name: &str) -> Result<serde_json::Value> {
    let c = canvas(name)?;
    let lock = canvas::lock(&c.dir);
    let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cut::summary(&c)
}

/// Checks (`check`) or exports canvas `name`'s pieces (those named in `only`, else all), each as an area of the module named after
/// the canvas, then checks the routes across them. Every piece is cut before any is built, so a refused one (a trigger across a
/// cut) stops the export before anything is written. The start piece goes first: the module starts in the first area exported
/// into it.
pub(crate) fn export(name: &str, only: &[String], check: bool) -> Result<serde_json::Value> {
    let c = canvas(name)?;
    let (header, ground) = {
        let lock = canvas::lock(&c.dir);
        let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let header = c.header()?;
        guard(name, &header.json)?;
        let ground = cut::Ground::load(&c, &header)?;
        (header, ground)
    };
    let list = cut::pieces(&header);
    anyhow::ensure!(!list.is_empty(), "canvas {name} has no pieces yet: cut it into pieces first");
    let start = header.json["startPiece"].as_str();
    let mut chosen = cut::chosen(&list, only)?;
    chosen.sort_by_key(|p| Some(p.name.as_str()) != start);
    let (crossings, mut warnings) = cut::crossings(name, &header, &ground, &list);
    warnings.extend(cut::in_gaps(&header, &list));
    let areas = chosen.iter().map(|p| cut::area(name, &header, &ground, p, &crossings, cut::HALO)).collect::<Result<Vec<_>>>()?;
    let palette: Vec<String> = serde_json::from_value(header.json["palette"].clone()).unwrap_or_default();
    let imports = imports_in(&palette, &header.json);
    let mut reports = std::collections::BTreeMap::new();
    let mut rows = Vec::new();
    for a in areas {
        let mut report = super::export::export_area(&a.name, a.header, &a.heights, &a.materials, &a.blend, &imports, check)
            .with_context(|| format!("piece {}", a.name))?;
        for key in ["module", "installed_module"] {
            if let Some(path) = report[key].as_str().map(std::path::PathBuf::from) {
                report[key] = crate::workspace::shown(&path).into();
            }
        }
        rows.push(serde_json::json!({ "name": a.name, "report": report }));
        reports.insert(a.name, report);
    }
    let routes = cut::canvas_routes(&list, &crossings, &reports, start);
    Ok(serde_json::json!({
        "canvas": name, "module": name, "pieces": rows, "crossings": cut::crossings_json(&crossings), "routes": routes,
        "warnings": warnings,
    }))
}
