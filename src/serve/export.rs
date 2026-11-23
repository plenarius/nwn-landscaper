// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What leaves the editor: an export (or the editor's Check), publishing an area or a module through the review service, a
//! package to download instead, a version in Gitea, and a test on the dev server.

use std::path::Path;

use anyhow::{Context, Result};
use tiny_http::{Method, Response};

use super::areas::{guard_assignment, guard_new_name, imports_in};
use super::query::{Query, area_name_ok, checked_area_name, short_name_ok};
use super::{Answer, Ask, fail, job, json_response, read_body, refuse, response_header};
use crate::workspace::{areas_dir, exports_dir};

/// Answers the routes of exports, publishing and testing; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let (me, query) = (&cx.me, cx.query);
    Some(match (&cx.method, cx.path) {
        (Method::Post, p @ ("/api/export" | "/api/check")) => {
            let check = p == "/api/check";
            let body = match read_body(cx.req, super::body_max::AREA) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let area = query.text("name");
            // (A team server's disk quota, crate::quota: a check writes its files too, in exports/.check.)
            if let Err(full) = crate::quota::room(&area, body.len() as u64) {
                return Some(refuse(413, &full.to_string()));
            }
            let done = {
                let _job = job();
                export_request(query.as_str(), &body, check)
            };
            // (An export writes a module and a hak, of sizes known only now: measured again. A check is measured with the
            // next walk.)
            if check {
                crate::quota::wrote(body.len() as u64);
            } else {
                crate::quota::forget();
            }
            match done {
                Ok(mut report) => {
                    // (Where the module was built and installed, as the builder knows it: crate::workspace::shown.)
                    for key in ["module", "installed_module"] {
                        if let Some(path) = report[key].as_str().map(std::path::PathBuf::from) {
                            report[key] = crate::workspace::shown(&path).into();
                        }
                    }
                    // (Checks run all the time as someone edits: only exports are recorded.)
                    if !check {
                        crate::audit::log("export", Some(&area), true, &format!("{} routes work", report["routes"]["ok"]));
                    }
                    json_response(200, &report)
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    eprintln!("export failed: {msg}");
                    // Being turned away (an ended assignment, a taken name) is recorded either way.
                    if e.downcast_ref::<super::areas::Denied>().is_some() {
                        crate::audit::log("denied", Some(&area), false, &format!("{}: {msg}", if check { "check" } else { "export" }));
                    } else if !check {
                        crate::audit::log("export", Some(&area), false, &msg);
                    }
                    refuse(400, &msg)
                }
            }
        }
        (Method::Post, "/api/publish") => {
            let name = query.text("name");
            let dry = query.one("dry");
            match publish_request(&name, &query.text("note"), dry) {
                Ok(v) => {
                    crate::audit::log(
                        if dry { "publish-check" } else { "publish" },
                        Some(&name),
                        true,
                        v["pr_urls"]["nwn_module"].as_str().unwrap_or(""),
                    );
                    json_response(200, &v)
                }
                Err(e) => {
                    crate::audit::log("publish", Some(&name), false, &format!("{e:#}"));
                    fail(400, &e)
                }
            }
        }
        // Publishing without a review service: what a package of the area would hold, and the package as a zip.
        (Method::Post, "/api/package") => match package_request(&query.text("name"), query.one("whole")) {
            Ok(v) => json_response(200, &v),
            Err(e) => fail(400, &e),
        },
        (Method::Get, "/api/package-download") => {
            let name = query.text("name");
            match package_download(&name, query.one("whole")) {
                Ok((zip, file)) => {
                    crate::audit::log("package", Some(&name), true, &file);
                    Response::from_data(zip)
                        .with_header(response_header("Content-Type", "application/zip"))
                        .with_header(response_header("Content-Disposition", format!("attachment; filename=\"{file}\"")))
                }
                Err(e) => fail(400, &e),
            }
        }
        // Testing the builder's module on the dev server (one holder at a time; the review service keeps the claim).
        (Method::Get, "/api/dev-test") => match crate::devtest::status(query.flag("touch")) {
            Ok(v) => {
                json_response(200, &serde_json::json!({ "status": v, "me": me.user, "admin": me.admin, "join": crate::config::dev_join() }))
            }
            Err(e) => fail(502, &e),
        },
        (Method::Get, "/api/dev-test/waypoints") => match crate::devtest::waypoints(&query.text("area")) {
            Ok(v) => json_response(200, &v),
            Err(e) => fail(400, &e),
        },
        (Method::Post, "/api/dev-test") => {
            let (module, area, wp) = (query.text("module"), query.text("area"), query.text("waypoint"));
            match crate::devtest::start(&module, &area, &wp, &query.text("entry"), query.one("takeover")) {
                Ok(v) => {
                    crate::audit::log("dev-test", Some(&module), true, &format!("from {area}/{wp}"));
                    json_response(200, &v)
                }
                Err(e) => {
                    crate::audit::log("dev-test", Some(&module), false, &format!("{e:#}"));
                    fail(400, &e)
                }
            }
        }
        (Method::Post, "/api/dev-test/release") => match crate::devtest::release() {
            Ok(v) => {
                crate::audit::log("dev-release", None, true, "");
                json_response(200, &v)
            }
            Err(e) => fail(400, &e),
        },
        // Try on dev: the areas Publish would send, on the dev server with no restart (one trial per builder).
        (Method::Get, "/api/dev-trial") => match crate::devtrial::status(query.flag("touch")) {
            Ok(mut v) => {
                v["join"] = crate::config::dev_join().into();
                json_response(200, &v)
            }
            Err(e) => fail(502, &e),
        },
        (Method::Post, "/api/dev-trial") => {
            let name = query.text("name");
            match trial_request(&name, query.one("whole"), &query.text("note")) {
                Ok(v) => {
                    let areas: Vec<&str> = v["revs"].as_object().into_iter().flatten().map(|(a, _)| a.as_str()).collect();
                    crate::audit::log("dev-trial", Some(&name), true, &areas.join(", "));
                    json_response(200, &v)
                }
                Err(e) => {
                    crate::audit::log("dev-trial", Some(&name), false, &format!("{e:#}"));
                    fail(400, &e)
                }
            }
        }
        (Method::Post, "/api/dev-trial/release") => match crate::devtrial::release() {
            Ok(v) => {
                crate::audit::log("dev-trial-release", None, true, "");
                json_response(200, &v)
            }
            Err(e) => fail(400, &e),
        },
        (Method::Post, "/api/publish-module") => {
            let module = query.text("module");
            let dry = query.one("dry");
            match publish_module_request(&module, &query.text("note"), dry) {
                Ok(v) => {
                    crate::audit::log(
                        if dry { "publish-check" } else { "publish" },
                        Some(&module),
                        true,
                        &format!("module: {}", v["pr_urls"]["nwn_module"].as_str().unwrap_or("")),
                    );
                    json_response(200, &v)
                }
                Err(e) => {
                    crate::audit::log("publish", Some(&module), false, &format!("module: {e:#}"));
                    fail(400, &e)
                }
            }
        }
        (Method::Post, "/api/version") => match version_request(&query.text("name"), query.get("note")) {
            Ok(v) => {
                crate::audit::log("version", query.get("name").as_deref(), true, v["commit"].as_str().unwrap_or(""));
                json_response(200, &v)
            }
            Err(e) => {
                crate::audit::log("version", query.get("name").as_deref(), false, &format!("{e:#}"));
                fail(400, &e)
            }
        },
        _ => return None,
    })
}

/// What an export's query string may no longer carry: lists and objects, which come in its body's JSON (see [`export_request`]).
/// They made the request's line grow with the area (an area's entrances, its water's shores), up to the 1 MB a request's head
/// may take.
const IN_BODY: [&str; 4] = ["palette", "water", "entrances", "atmosphere"];

/// Export (or with `check`, only analyse: the editor's Check paths, in a scratch folder so the last export's files, which
/// re-exports read, stay).
///
/// The query string has the area's name and its simple settings: `tiles`, `tilesY`, `shadow`, `module`, `kind`, `roof` and
/// `clearance`. The body is the heights (f32 a sample, rows from the south), then the paint (one byte a sample: an index into the
/// palette), then JSON: `palette` (the paint's texture ids), `water`, `entrances`, `atmosphere`, `objects`, `things`,
/// `settings` and `originals`. A query string that still has one of [`IN_BODY`] (an editor older than this engine) is refused.
pub(super) fn export_request(query: &str, body: &[u8], check: bool) -> Result<serde_json::Value> {
    let query = Query::new(query);
    if let Some(key) = IN_BODY.iter().find(|k| query.raw(k).is_some()) {
        anyhow::bail!(
            "the export's {key} goes in the request's JSON body now, not its query string: this editor is older than the engine, \
             reload the page"
        );
    }
    let param = |k: &str| query.get(k);
    // Checked before anything is written: the name becomes file names in the exports folder.
    let name = checked_area_name(&query.required("name")?)?;
    // Tiles across, and up (as many as across when not given).
    let tiles: usize = query.required("tiles")?.parse().context("tiles")?;
    let tiles_y: usize = match param("tilesY") {
        Some(v) => v.parse().context("tilesY")?,
        None => tiles,
    };
    anyhow::ensure!(TILES.contains(&tiles) && TILES.contains(&tiles_y), "an area is 3 to 32 tiles each way, not {tiles} by {tiles_y}");
    let (nx, ny) = (tiles * 16 + 1, tiles_y * 16 + 1);
    let samples = nx * ny;
    // Heights, then the paint and the JSON, when there are (see above).
    let (heights, rest) = body.split_at(body.len().min(samples * 4));
    let (materials, tail) = if rest.len() >= samples { rest.split_at(samples) } else { (rest, &[][..]) };
    let extra: serde_json::Value = if tail.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(tail).context("the JSON after the heights and materials")?
    };
    anyhow::ensure!(
        heights.len() == samples * 4,
        "expected {} bytes of heights for {tiles} by {tiles_y} tiles, got {}",
        samples * 4,
        body.len()
    );
    // On a team server: a new area can't take an existing area's name, and
    // anything built from an existing area needs the assignment to stand.
    guard_new_name(&name)?;
    let palette: Vec<String> = match extra.get("palette").filter(|p| !p.is_null()) {
        Some(p) => serde_json::from_value(p.clone()).context("palette: a list of texture ids")?,
        None => Vec::new(),
    };
    // (guard_assignment also refuses an import named with anything but a plain name.)
    guard_assignment(&name, &imports_in(&palette, &extra))?;
    let dir = if check { exports_dir().join(".check") } else { exports_dir() };
    std::fs::create_dir_all(&dir)?;
    // The ground the installed module was last built on, so its objects can follow the terrain's changes.
    let old = previous_heights(&dir, &name);
    // (Freshly imported, not exported since: the installed module has never had this light, whatever the header says.)
    let old_header =
        std::fs::read_to_string(dir.join(format!("{name}.json"))).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let just_imported = old_header.as_ref().is_some_and(|h| h["imported"].as_bool() == Some(true));
    // The entrances (each with its id) and the area's next entrance id, which never goes down: an id is never given twice.
    let entrances = extra.get("entrances").cloned().unwrap_or_default();
    let entrance_next = crate::heightmap::next_entrance_id(&entrances, old_header.as_ref())?;
    let old_settings = old_header.as_ref().filter(|_| !just_imported).map(|h| h["settings"].clone());
    let old_originals = old_header.as_ref().map(|h| h["originals"].clone()).filter(serde_json::Value::is_object);
    let old_atmosphere = old_header.filter(|_| !just_imported).map(|h| h["atmosphere"].clone());
    // (A check works in its own folder and never installs anything: nothing of it needs putting back.)
    let rollback = (!check).then(|| Rollback::of(&dir, &name));
    crate::atomic::write(&dir.join(format!("{name}.f32")), heights)?;
    let mut header = serde_json::json!({
        "name": name, "tiles": tiles, "tilesY": tiles_y, "spacing": 0.625, "width": nx, "height": ny, "data": format!("{name}.f32"),
    });
    if !materials.is_empty() {
        crate::atomic::write(&dir.join(format!("{name}.materials")), materials)?;
        header["materials"] = format!("{name}.materials").into();
        header["palette"] = palette.into();
    }
    if let Some(v) = query.number::<f64>("shadow") {
        header["shadow"] = v.into();
    }
    // The module it goes in (another area's, to link them), else its own.
    let module = param("module").filter(|m| short_name_ok(m)).map_or_else(|| name.clone(), |m| m.to_ascii_lowercase());
    header["module"] = module.into();
    if let Some(kind) = param("kind").filter(|k| k == "underground" || k == "interior") {
        header["kind"] = kind.into();
        if let Some(r) = query.number::<f64>("roof") {
            header["roof"] = r.into();
        }
        if let Some(c) = query.number::<f64>("clearance").filter(|c| *c > 0.0) {
            header["clearance"] = c.into();
        }
    }
    if let Some(objects) = extra.get("objects") {
        header["objects"] = objects.clone();
    }
    // The things the editor placed (creatures, triggers...) and the area's own settings.
    for key in ["things", "settings", "originals"] {
        if let Some(v) = extra.get(key).filter(|v| !v.is_null()) {
            header[key] = v.clone();
        }
    }
    if !entrances.is_null() {
        header["entrances"] = entrances;
    }
    header["entranceNextId"] = entrance_next.into();
    if let Some(water) = extra.get("water") {
        header["water"] = water.clone();
    }
    // Without one (an export not from the editor), the last one carries on.
    if let Some(a) = extra.get("atmosphere").or(old_atmosphere.as_ref().filter(|a| a.is_object())) {
        header["atmosphere"] = a.clone();
    }
    let json = dir.join(format!("{name}.json"));
    crate::atomic::write(&json, serde_json::to_string_pretty(&header)?.as_bytes())?;
    if check {
        return crate::generator::check(&json);
    }
    let before = crate::generator::Before {
        heights: old,
        atmosphere: old_atmosphere,
        settings: old_settings,
        originals: old_originals,
        just_imported,
        things: extra.get("things").is_some_and(serde_json::Value::is_array),
    };
    let report = crate::generator::export(&json, &before)?;
    if let Some(r) = rollback {
        r.keep();
    }
    Ok(report)
}

/// What an export replaces before it knows whether it will work, held so a failure can put it back: the last export's header,
/// heights and paint (what the next export compares against: `old_atmosphere`, `old_settings`, `previous_heights`) and the
/// area's hak. Without this, an export that failed (a missing model, say) left its header behind as "the last export", the next
/// one saw no change in light, fog or settings and didn't write them to the module, and objects followed the failed attempt's
/// ground. Put back when dropped (an error or a panic) before [`Rollback::keep`].
///
/// Each file is held by a hard link beside it (`.<file>.rollback`), not read into memory: a 32 by 32 area's hak is over 100 MB.
/// That works because an export replaces each of them by a rename ([`crate::atomic::write`]), which leaves the link holding the
/// old file; a file written in place would change under the link too.
struct Rollback {
    /// Each file, and its link (None: it wasn't there).
    files: Vec<(std::path::PathBuf, Option<std::path::PathBuf>)>,
    kept: bool,
}

impl Rollback {
    fn of(dir: &Path, name: &str) -> Rollback {
        let mut paths: Vec<std::path::PathBuf> = ["json", "f32", "materials"].iter().map(|e| dir.join(format!("{name}.{e}"))).collect();
        paths.extend(crate::workspace::nwn_user_dir().map(|u| u.join("hak").join(format!("{name}.hak"))));
        Rollback::holding(paths)
    }

    /// A rollback of these files.
    fn holding(paths: Vec<std::path::PathBuf>) -> Rollback {
        let files = paths
            .into_iter()
            .map(|p| {
                let held = Rollback::link_of(&p);
                // (One left by a server that stopped in the middle of an export: what is in place now is newer.)
                let _ = std::fs::remove_file(&held);
                // (Without hard links, a copy.)
                let was = p.is_file() && (std::fs::hard_link(&p, &held).is_ok() || std::fs::copy(&p, &held).is_ok());
                (p, was.then_some(held))
            })
            .collect();
        Rollback { files, kept: false }
    }

    /// Where `path`'s old file is held: a hidden name beside it.
    fn link_of(path: &Path) -> std::path::PathBuf {
        let name = path.file_name().map_or_default(|n| n.to_string_lossy().into_owned());
        path.with_file_name(format!(".{name}.rollback"))
    }

    /// The export worked: what it wrote stays.
    fn keep(mut self) {
        self.kept = true;
    }
}

impl Drop for Rollback {
    fn drop(&mut self) {
        for (path, held) in &self.files {
            if self.kept {
                if let Some(h) = held {
                    let _ = std::fs::remove_file(h);
                }
                continue;
            }
            let put_back = match held {
                Some(h) => std::fs::rename(h, path),
                None => std::fs::remove_file(path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
            };
            if let Err(e) = put_back {
                eprintln!("a failed export couldn't put back {}: {e:#}", path.display());
            }
        }
    }
}

/// The heights of the last export of `name`, if its files are complete.
fn previous_heights(dir: &Path, name: &str) -> Option<crate::heightmap::Heightmap> {
    let header: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.json"))).ok()?).ok()?;
    let tiles = header["tiles"].as_u64()? as usize;
    let tiles_y = header["tilesY"].as_u64().map_or(tiles, |v| v as usize);
    let (nx, ny) = (tiles * 16 + 1, tiles_y * 16 + 1);
    let bytes = std::fs::read(dir.join(format!("{name}.f32"))).ok()?;
    if bytes.len() != nx * ny * 4 {
        return None;
    }
    Some(crate::heightmap::Heightmap {
        name: name.to_string(),
        tiles,
        tiles_y,
        spacing: 0.625,
        width: nx,
        height: ny,
        data: bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        materials: Vec::new(),
        palette: Vec::new(),
        atmosphere: None,
        things: Vec::new(),
        settings: None,
        originals: None,
        water: Vec::new(),
        shadow: None,
        entrances: Vec::new(),
        objects: Vec::new(),
        underground: false,
        interior: false,
        roof: None,
        ceiling: Vec::new(),
        serial: 0,
    })
}

/// How many tiles an area may be each way (as `heightmap::load` checks once the files are written).
const TILES: std::ops::RangeInclusive<usize> = 3..=32;

/// The header of `name`'s last export (`<name>.json` in the exports folder), the file a publish or a package is built from.
fn exported(name: &str) -> Result<serde_json::Value> {
    std::fs::read(exports_dir().join(format!("{name}.json")))
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .with_context(|| format!("{name} hasn't been exported yet"))
}

/// Refuses `header`'s area unless the acting builder may build on it and on every import it uses (see [`guard_assignment`]).
fn guard_area(name: &str, header: &serde_json::Value) -> Result<()> {
    let palette: Vec<String> = header["palette"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
    guard_assignment(name, &imports_in(&palette, header))
}

/// The last export of `name`, once the name, the production name and the assignment checks pass: the one gate that a publish,
/// a package and a module publish all go through, so they can't drift apart.
fn guarded_export(name: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(area_name_ok(name), "bad area name {name:?}");
    guard_new_name(name)?;
    let header = exported(name)?;
    guard_area(name, &header)?;
    Ok(header)
}

/// Every area exported into `module`, sorted by name: an area's module is its header's `module`, else its own name.
fn areas_in_module(module: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for e in std::fs::read_dir(exports_dir()).into_iter().flatten().flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some(name) = file.strip_suffix(".json").filter(|n| area_name_ok(n)) else { continue };
        let Some(h) = std::fs::read(e.path()).ok().and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok()) else { continue };
        if h["tiles"].is_u64() && h["module"].as_str().unwrap_or(name) == module {
            names.push(name.to_string());
        }
    }
    names.sort();
    names
}

/// Publishes the last export of `name` as PRs (or, with `dry`, only checks what they would hold).
fn publish_request(name: &str, note: &str, dry: bool) -> Result<serde_json::Value> {
    guarded_export(name)?;
    let findings = crate::publish::findings(name)?;
    refuse_errors(&findings, dry)?;
    let mut v = crate::publish::publish(name, note, dry)?;
    v["findings"] = serde_json::to_value(&findings)?;
    Ok(v)
}

/// What the check found wrong stops a real publish (a check on its own only reports it).
fn refuse_errors(findings: &[crate::validate::Finding], dry: bool) -> Result<()> {
    let errors: Vec<String> = findings
        .iter()
        .filter(|f| f.level == "error")
        .map(|f| if f.what.is_empty() { f.msg.clone() } else { format!("{}: {}", f.what, f.msg) })
        .collect();
    anyhow::ensure!(
        dry || errors.is_empty(),
        "fix these first ({} problem{}): {}",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" },
        errors.iter().take(5).cloned().collect::<Vec<_>>().join("; ")
    );
    Ok(())
}

/// Tries `name` (or with `whole` its module) on dev: the areas a publish would send, checked as a publish is.
fn trial_request(name: &str, whole: bool, note: &str) -> Result<serde_json::Value> {
    let (names, findings) = package_areas(name, whole, false)?;
    let mut v = crate::devtrial::start(&names, note)?;
    v["findings"] = serde_json::to_value(&findings)?;
    Ok(v)
}

/// The areas a package, a publish or a trial on dev covers: `name`, or with `whole` every area exported into its module; each checked
/// (name guard, assignment, the pre-publish findings). Errors stop a package (not its check, `dry`).
fn package_areas(name: &str, whole: bool, dry: bool) -> Result<(Vec<String>, Vec<crate::validate::Finding>)> {
    checked_area_name(name)?;
    let names: Vec<String> = if whole {
        let header = exported(name)?;
        areas_in_module(header["module"].as_str().unwrap_or(name))
    } else {
        vec![name.to_string()]
    };
    anyhow::ensure!(!names.is_empty(), "no area has been exported");
    let mut findings = Vec::new();
    for n in &names {
        guarded_export(n)?;
        for mut f in crate::publish::findings(n)? {
            if names.len() > 1 {
                f.msg = format!("{n}: {}", f.msg);
            }
            findings.push(f);
        }
    }
    refuse_errors(&findings, dry)?;
    Ok((names, findings))
}

/// What a package of `name` (or its module) would hold, and what the check found.
fn package_request(name: &str, whole: bool) -> Result<serde_json::Value> {
    let (names, findings) = package_areas(name, whole, true)?;
    let (_, mut plan) = crate::publish::package(&names)?;
    plan["findings"] = serde_json::to_value(&findings)?;
    Ok(serde_json::json!({ "plan": plan, "findings": findings }))
}

/// The package itself as a zip, and a name for the file.
fn package_download(name: &str, whole: bool) -> Result<(Vec<u8>, String)> {
    let (names, _) = package_areas(name, whole, false)?;
    let (files, _) = crate::publish::package(&names)?;
    let file = if whole { format!("{}-package.zip", names.first().map_or(name, String::as_str)) } else { format!("{name}-package.zip") };
    Ok((crate::publish::package_zip(&files)?, file))
}

/// Every area exported into `module` (this workspace's), published as one set. Each area is checked as for a
/// single publish; one that isn't ready stops the lot, named.
fn publish_module_request(module: &str, note: &str, dry: bool) -> Result<serde_json::Value> {
    anyhow::ensure!(short_name_ok(module), "bad module name {module:?}");
    let names = areas_in_module(module);
    anyhow::ensure!(!names.is_empty(), "no area has been exported into module {module}");
    for name in &names {
        guarded_export(name)?;
    }
    let mut findings = Vec::new();
    for name in &names {
        for mut f in crate::publish::findings(name)? {
            f.msg = format!("{name}: {}", f.msg);
            findings.push(f);
        }
    }
    refuse_errors(&findings, dry)?;
    let mut v = crate::publish::publish_module(&names, note, dry)?;
    v["findings"] = serde_json::to_value(&findings)?;
    Ok(v)
}

/// Saves the editor's copy of `name` as a version in Gitea (a commit on
/// `build/<user>/<name>`). Autosave never does this; the builder asks.
fn version_request(name: &str, note: Option<String>) -> Result<serde_json::Value> {
    anyhow::ensure!(area_name_ok(name), "bad area name {name:?}");
    guard_new_name(name)?;
    let user = crate::workspace::acting_user();
    let body = std::fs::read(areas_dir().join(format!("{name}.json"))).with_context(|| format!("{name} hasn't been saved yet"))?;
    let v: serde_json::Value = serde_json::from_slice(&body).context("the saved area isn't JSON")?;
    // The working copy, not the export: a version saves what the builder has now, so it is what must stand.
    guard_area(name, &v)?;
    let mut files = vec![(format!("{user}/{name}/area.json"), body)];
    if let Ok(src) = std::fs::read(exports_dir().join(format!("{name}.import")).join("source.json")) {
        files.push((format!("{user}/{name}/source.json"), src));
    }
    let note = note.map(|n| n.trim().chars().take(200).collect::<String>()).filter(|n| !n.is_empty());
    let message = format!("{user}: {name}{}", note.map_or_default(|n| format!(" - {n}")));
    crate::gitea::commit(&format!("build/{user}/{name}"), &files, &message, &user)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder with a last export's header and heights and its hak (`area.hak`), as an export finds them; no paint.
    fn last_export(test: &str) -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
        let dir = std::env::temp_dir().join(format!("landscaper-rollback-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<std::path::PathBuf> = ["area.json", "area.f32", "area.materials", "area.hak"].iter().map(|f| dir.join(f)).collect();
        for p in paths.iter().filter(|p| !p.ends_with("area.materials")) {
            std::fs::write(p, format!("last {}", p.display())).unwrap();
        }
        (dir, paths)
    }

    /// What the folder holds: each file and its content.
    fn contents(dir: &Path) -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read_to_string(e.path()).unwrap()))
            .collect();
        all.sort();
        all
    }

    /// What an export does to the files: each replaced whole (as `atomic::write` does), the paint made.
    fn export_over(paths: &[std::path::PathBuf]) {
        for p in paths {
            crate::atomic::write(p, b"new").unwrap();
        }
    }

    #[test]
    fn a_failed_export_puts_back_the_hak_and_the_last_exports_files() {
        let (dir, paths) = last_export("failed");
        let before = contents(&dir);
        let rollback = Rollback::holding(paths.clone());
        // The hak is held by a second link, not a copy in memory.
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::MetadataExt::nlink(&std::fs::metadata(&paths[3]).unwrap()), 2);
        export_over(&paths);
        drop(rollback);
        // The hak and the header and heights as they were, the paint that wasn't there gone, nothing held left behind.
        assert_eq!(contents(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_finished_export_keeps_what_it_wrote() {
        let (dir, paths) = last_export("kept");
        let rollback = Rollback::holding(paths.clone());
        export_over(&paths);
        rollback.keep();
        let names = ["area.f32", "area.hak", "area.json", "area.materials"];
        assert_eq!(contents(&dir), names.map(|n| (n.to_string(), "new".to_string())));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
