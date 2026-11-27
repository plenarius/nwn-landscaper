// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bringing existing areas in: the search over the module's areas, importing one (or one of a builder's own uploaded module or
//! ERF), importing again from the kept original, the pictures an import made, and picking up a texture from an imported area's
//! ground.

use anyhow::{Context, Result};
use tiny_http::{Method, Response};

use super::areas::{Denied, Taken, area_exists, area_lock, guard_assignment};
use super::query::{Query, area_name_ok, percent_decode, short_name_ok};
use super::{Answer, Ask, Paced, fail, job, json_response, refuse, response_header, stalled};
use crate::workspace::{areas_dir, exports_dir};

/// Answers the import routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let (me, query) = (&cx.me, cx.query);
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/import-sources") => {
            let q = query.text("q");
            let searching = q.trim().chars().count() >= 3 && !me.admin;
            if searching && !crate::audit::search_allowed(&me.user) {
                crate::audit::log("search", None, false, &format!("rate limited: {q}"));
                crate::audit::notify_throttled(
                    &format!("limit:{}", me.user),
                    format!("**{}** hit the search limit (last search {:?}); more are in the activity log", me.user, q),
                );
                refuse(429, "that is a lot of searching: wait a few minutes")
            } else {
                match import_search(query.as_str()) {
                    Ok(r) => {
                        if searching {
                            crate::audit::log(
                                "search",
                                None,
                                true,
                                &format!("{q} -> {} of {}", r["sources"].as_array().map_or(0, Vec::len), r["total"]),
                            );
                        }
                        json_response(200, &r)
                    }
                    // (The area list or the assignments can't be read: not an empty list, which would read as "no such area".)
                    Err(e) => fail(503, &e),
                }
            }
        }
        // A builder's own module or ERF: kept in their workspace, listed with its areas, imported by area.
        (Method::Get, "/api/uploads") => {
            json_response(200, &serde_json::json!({ "uploads": crate::upload::list(), "max_mb": crate::upload::MAX_BYTES >> 20 }))
        }
        (Method::Post, "/api/upload") => {
            let file = query.text("file");
            // (A body that doesn't say its length is refused: the limits are checked before it is read.)
            match cx
                .req
                .body_length()
                .map(|n| n as u64)
                .ok_or((413, format!("a file up to {} MB, please", crate::upload::MAX_BYTES >> 20)))
                .and_then(|n| crate::upload::begin(&file, n))
            {
                Err((status, why)) => refuse(status, &why),
                // (Paced: an upload that stops arriving gives its turn back, see serve.rs.)
                Ok(turn) => match crate::upload::store(&turn, &file, &mut Paced::new(cx.req.as_reader())) {
                    Ok((areas, len)) => {
                        crate::audit::log("upload", Some(&file), true, &format!("{len} bytes, {} areas", areas.len()));
                        json_response(200, &serde_json::json!({ "file": file, "areas": areas }))
                    }
                    Err(e) => {
                        crate::audit::log("upload", Some(&file), false, &format!("{e:#}"));
                        let slow = e.chain().any(|c| c.downcast_ref::<std::io::Error>().is_some_and(stalled));
                        fail(if slow { 408 } else { 400 }, &e)
                    }
                },
            }
        }
        (Method::Post, "/api/upload-delete") => match crate::upload::delete(&query.text("file")) {
            Ok(()) => json_response(200, &serde_json::json!({ "ok": true })),
            Err(e) => fail(400, &e),
        },
        (Method::Post, "/api/reimport") => {
            let name = query.text("name");
            // (For the log; the re-import itself stops on a note that can't be read.)
            let from = crate::import::recorded(&name).ok().flatten();
            let res = {
                let _job = job();
                reimport_request(&name)
            };
            match res {
                Ok(r) => {
                    crate::audit::log("reimport", from.as_deref(), true, &format!("as {name}"));
                    json_response(200, &r)
                }
                Err(e) => {
                    crate::audit::log("reimport", from.as_deref(), false, &format!("{name}: {e:#}"));
                    fail(400, &e)
                }
            }
        }
        (Method::Post, "/api/import") => {
            let (area, name) = (query.get("area").or_else(|| query.get("source")).unwrap_or_default(), query.text("name"));
            // (A team server's disk quota, crate::quota; what the import writes is measured with the next request.)
            if let Err(full) = crate::quota::room(&name, 0) {
                return Some(refuse(413, &full.to_string()));
            }
            let res = {
                let _job = job();
                import_request(query.get("area"), query.get("source"), query.text("name"), query.get("upload"))
            };
            crate::quota::forget();
            match res {
                Ok(r) => {
                    crate::audit::log("import", Some(&area), true, &format!("as {name}"));
                    if !me.admin {
                        crate::audit::notify_throttled(
                            &format!("import:{}", me.user),
                            format!("**{}** imported `{area}` as `{name}` (more imports in the activity log)", me.user),
                        );
                    }
                    json_response(200, &r)
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    crate::audit::log("import", Some(&area), false, &msg);
                    if !me.admin && matches!(e.downcast_ref::<Denied>(), Some(Denied::NotAssigned(_))) {
                        crate::audit::notify_throttled(
                            &format!("denied:{}", me.user),
                            format!("**{}** tried to import `{area}`, which isn't assigned to them", me.user),
                        );
                    }
                    // (The same answer as a save's: the editor shows `error` either way.)
                    if e.downcast_ref::<Taken>().is_some() {
                        json_response(409, &serde_json::json!({ "error": msg, "exists": name }))
                    } else {
                        refuse(400, &msg)
                    }
                }
            }
        }
        (Method::Get, p) if p.starts_with("/api/import-file/") => {
            // A file an import made for the editor (a ground patch): /api/import-file/<name>/<decal_....png>.
            let rest = percent_decode(p.trim_start_matches("/api/import-file/"));
            let ok = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !t.contains("..");
            #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the import writes these files as `decal_<key>.png`")]
            let wanted = rest.split_once('/').filter(|(n, f)| ok(n) && ok(f) && f.starts_with("decal_") && f.ends_with(".png"));
            match wanted {
                Some((name, file)) => match std::fs::read(crate::paint::import_dir(name).join(file)) {
                    Ok(png) => Response::from_data(png)
                        .with_header(response_header("Content-Type", "image/png"))
                        .with_header(response_header("Cache-Control", "no-cache")),
                    Err(_) => refuse(404, "no such file"),
                },
                None => refuse(400, "bad import file"),
            }
        }
        // Picks up the texture under a spot of an imported area's ground: it joins the library (once) and the editor paints with it.
        (Method::Post, "/api/pick-texture") => {
            match pick_texture_request(&query.text("name"), query.number("x").unwrap_or(f32::NAN), query.number("y").unwrap_or(f32::NAN)) {
                Ok(v) => {
                    if v["added"].as_bool() == Some(true) {
                        crate::audit::log("pick-texture", Some(&query.text("name")), true, v["id"].as_str().unwrap_or(""));
                    }
                    json_response(200, &v)
                }
                Err(e) => fail(400, &e),
            }
        }
        (Method::Get, p) if p.starts_with("/api/import/") => {
            // An imported area's picture, for the editor.
            let name = percent_decode(p.trim_start_matches("/api/import/"));
            match short_name_ok(&name).then(|| std::fs::read(exports_dir().join(format!("{name}.import.view.png")))) {
                Some(Ok(png)) => Response::from_data(png).with_header(response_header("Content-Type", "image/png")),
                _ => refuse(404, &format!("no imported area {name}")),
            }
        }
        _ => return None,
    })
}

/// Imports an NWN area (one `import::sources` lists) as a new landscaper area. Never over an area that exists (a working
/// copy, an export or an import): an import starts a new one. (Over a working copy alone, the editor would open the working
/// copy, and its next export would run as the import's first, taking in the import's objects.)
fn import_request(area: Option<String>, source: Option<String>, name: String, upload: Option<String>) -> Result<serde_json::Value> {
    anyhow::ensure!(area_name_ok(&name), "the name must be 1-9 letters, digits or _ (it prefixes every resref)");
    let dir = exports_dir();
    if area_exists(&name) {
        return Err(Taken(name).into());
    }
    // Their own file: theirs to import, no assignment. Its name can't be one of the module's areas' (it would collide when published).
    if let Some(file) = upload {
        let resref = area.context("which area of the file?")?;
        if crate::import::production_name(&name)? {
            return Err(Denied::ProductionName(name).into());
        }
        // (`import_to` refuses an area too big, or whose tile list doesn't match its size, before it opens the game.)
        let mut src = crate::import::Source::module(&crate::upload::path_of(&file)?, &resref)?;
        // Not a production area, so nothing ties it to an assignment. (Named as the builder knows it, not by its path.)
        src.area = String::new();
        src.label = format!("{file}:{resref}");
        let r = crate::import::import_to(&src, &name, &dir)?;
        let note = crate::paint::import_dir(&name).join("source.json");
        if let Some(mut j) = std::fs::read(&note).ok().and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok()) {
            j["upload"] = serde_json::json!({ "file": file, "area": resref });
            std::fs::write(&note, serde_json::to_vec(&j)?)?;
        }
        return Ok(r);
    }
    let admin = crate::workspace::acting_admin();
    let list = crate::import::sources()?;
    // A builder names the area (its resref); only an admin (or your own machine) may point at a file.
    let entry = match (&area, &source) {
        (Some(a), _) => list.iter().find(|s| s["area"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(a))),
        (None, Some(src)) if admin => list.iter().find(|s| s["source"].as_str() == Some(src.as_str())),
        _ => None,
    }
    .context("not an area I can import")?;
    let resref = entry["area"].as_str().unwrap_or_default().to_ascii_lowercase();
    if !admin {
        if !crate::assignments::allows(&crate::workspace::acting_user(), &resref)? {
            return Err(Denied::NotAssigned(resref).into());
        }
        // Their copy's name can't be another of the module's areas'.
        if crate::import::production_name(&name)? {
            return Err(Denied::ProductionName(name).into());
        }
    }
    let src = crate::import::Source::parse(entry["source"].as_str().context("no source")?)?;
    crate::import::import_to(&src, &name, &dir)
}

/// Whether every search word (lower case) is in the area's name, tag, resref or tileset: builders know an area by the name
/// they see in the game ("Forest of Fog"), which is not the tag or resref ("ForestofMists1").
pub(super) fn source_matches(words: &[&str], s: &serde_json::Value) -> bool {
    let text = format!(
        "{} {} {} {}",
        s["name"].as_str().unwrap_or(""),
        s["tag"].as_str().unwrap_or(""),
        s["area"].as_str().unwrap_or(""),
        s["tileset"].as_str().unwrap_or("")
    )
    .to_lowercase();
    words.iter().all(|w| text.contains(w))
}

/// The import search. An admin (or your own machine) gets what matches, everything about it. A builder gets, for what they typed
/// (3 letters at least; without, only what is assigned to them), at most 25 matches with only its name, size and kind, and whether
/// it is theirs: no folder paths, no way to list the whole module.
fn import_search(query: &str) -> Result<serde_json::Value> {
    let q = Query::new(query).text("q").to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let all = crate::import::sources()?;
    let matches = |s: &serde_json::Value| source_matches(&words, s);
    if crate::workspace::acting_admin() {
        let mut found: Vec<serde_json::Value> = all.into_iter().filter(|s| matches(s)).collect();
        let total = found.len();
        found.truncate(150);
        return Ok(serde_json::json!({ "sources": found, "total": total }));
    }
    let me = crate::workspace::acting_user();
    let mine: Vec<String> = crate::assignments::active_for(&me)?.into_iter().map(|a| a.area).collect();
    let searching = q.chars().filter(|c| !c.is_whitespace()).count() >= 3;
    let kind = |flags: i64| {
        if flags & 1 != 0 {
            "interior"
        } else if flags & 2 != 0 {
            "underground"
        } else {
            "outdoors"
        }
    };
    let mut out: Vec<serde_json::Value> = all
        .iter()
        .filter(|s| {
            let assigned = s["area"].as_str().is_some_and(|a| mine.iter().any(|m| crate::assignments::covers(m, a)));
            if searching { matches(s) } else { assigned }
        })
        .map(|s| {
            let area = s["area"].as_str().unwrap_or_default();
            serde_json::json!({
                "area": area, "tag": s["tag"], "name": s["name"], "tileset": s["tileset"], "width": s["width"], "height": s["height"],
                "kind": kind(s["flags"].as_i64().unwrap_or(0)), "assigned": mine.iter().any(|m| crate::assignments::covers(m, area)),
            })
        })
        .collect();
    // Theirs first.
    out.sort_by_key(|s| !s["assigned"].as_bool().unwrap_or(false));
    let total = out.len();
    out.truncate(25);
    let note = if searching { "" } else { "Type at least 3 letters to search; these are the areas assigned to you." };
    Ok(serde_json::json!({ "sources": out, "total": total, "note": note }))
}

/// The tileset texture under (x, y) metres of the imported area `name`, put in the library; what it is called there.
fn pick_texture_request(name: &str, x: f32, y: f32) -> Result<serde_json::Value> {
    anyhow::ensure!(short_name_ok(name), "bad area name");
    anyhow::ensure!(x.is_finite() && y.is_finite(), "no spot given");
    let dir = crate::paint::import_dir(name);
    anyhow::ensure!(dir.is_dir(), "{name} wasn't imported");
    let (resref, surface) = crate::import::pick_at(&dir, x, y)?;
    // The module's haks, where its tileset's textures may be.
    let haks: Vec<String> = std::fs::read(dir.join("haks.json")).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
    let p = crate::textures::add_picked(&haks, &resref, surface)?;
    Ok(serde_json::json!({ "id": p.id, "name": p.name, "added": p.added }))
}

/// Imports area `name` again from the original its import kept (with
/// whatever the importer does better now): its ground, pictures and objects
/// start over, and the editor's working copy with them (the one before
/// kept as .bak). The next export puts the original's objects and light into
/// the module again, as after any import. If the import fails, the earlier
/// import and the working copy stay as they were. It holds the area's lock ([`area_lock`]) throughout, as a save does: another
/// tab's save at that moment was moved aside unseen, or landed right after the move and put the old area back over the import.
fn reimport_request(name: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(area_name_ok(name), "bad area name {name:?}");
    guard_assignment(name, &[])?;
    let saved = areas_dir().join(format!("{name}.json"));
    let lock = area_lock(&saved);
    let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let src = crate::import::Source::kept(name)?;
    // What the earlier import left (pictures, objects, textures) goes, once the new import has worked.
    let r = crate::import::reimport_to(&src, name, &exports_dir())?;
    if saved.exists() {
        std::fs::rename(&saved, areas_dir().join(format!("{name}.json.bak")))?;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{area_lock, areas_dir, reimport_request, source_matches};

    #[test]
    fn a_reimport_waits_for_a_save_of_its_area() {
        // A save of `relock` under way (it holds the area's lock): the re-import waits for it, then runs (and fails here: there is
        // no kept original). Before, it went ahead under the save.
        let lock = area_lock(&areas_dir().join("relock.json"));
        let saving = lock.lock().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || done.send(reimport_request("relock").is_ok()).unwrap());
        assert!(finished.recv_timeout(Duration::from_millis(300)).is_err(), "it waits for the save");
        drop(saving);
        assert_eq!(finished.recv_timeout(Duration::from_secs(10)), Ok(false), "then runs");
    }

    #[test]
    fn area_search_finds_the_name_the_tag_the_resref_and_the_tileset() {
        let a = serde_json::json!({
            "area": "forestofmists1", "tag": "ForestofMists1", "name": "Forest of Fog - East of Oakhurst", "tileset": "ttf02",
        });
        for q in [vec!["fog"], vec!["oakhurst", "fog"], vec!["mists"], vec!["forestofmists1"], vec!["ttf02"]] {
            assert!(source_matches(&q, &a), "{q:?}");
        }
        assert!(!source_matches(&["fog", "clover"], &a));
        // An area with no name (a talk table name the server could not read) is still found by the rest.
        assert!(source_matches(&["mists"], &serde_json::json!({ "area": "forestofmists1", "tag": "ForestofMists1", "tileset": "ttf02" })));
    }
}
