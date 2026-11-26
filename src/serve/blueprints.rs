// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the editor places and how it looks: the game's blueprints and what they hold, a builder's own new blueprints (their
//! fields, a creature's gear, an item's properties), the placeables and door looks there are, and a model or texture from the
//! game's data (read through Moonglow).

use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use tiny_http::{Method, Response};

use super::query::percent_decode;
use super::{Answer, Ask, fail, json_response, read_body, refuse, response_header};

/// Answers the blueprint, model and texture routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let query = cx.query;
    Some(match (&cx.method, cx.path) {
        // What the editor needs to place things: each kind's fields, and the new-area template.
        (Method::Get, "/api/things-meta") => json_response(200, &crate::blueprints::meta()),
        // A creature blueprint's look: the review service bakes it; the editor fetches the scene and its textures through here.
        // What a placeable's or door's blueprint looks like: the model its appearance names (the editor draws it
        // from /api/model); an item's: its inventory icon (the review service says which, and draws it).
        // What a blueprint holds now (a container's, creature's or store's items, an encounter's creatures), for the
        // editor to start a change from.
        (Method::Get, "/api/blueprint-contents") => {
            let (kind, resref) = (query.lower("kind"), query.lower("resref"));
            match (
                crate::things::kind_of(&kind).filter(|_| matches!(kind.as_str(), "placeable" | "creature" | "store" | "encounter")),
                crate::newbp::resref_ok(&resref),
            ) {
                (Some(k), true) => match crate::models::game().and_then(|g| crate::things::contents_of(g, k, &resref)) {
                    Ok(v) => json_response(200, &v),
                    Err(e) => fail(404, &e),
                },
                _ => refuse(400, "a placeable, creature, store or encounter resref, please"),
            }
        }
        // A blueprint's own values for the settings the editor may change on a thing made from it, so its form can say what
        // "the blueprint's" is (Locked: the blueprint's (on)).
        (Method::Get, "/api/blueprint-fields") => {
            let (kind, resref) = (query.lower("kind"), query.lower("resref"));
            match (crate::things::kind_of(&kind), crate::newbp::resref_ok(&resref)) {
                (Some(k), true) => match crate::models::game().and_then(|g| crate::things::blueprint_fields(g, k, &resref)) {
                    Ok(v) => json_response(200, &v),
                    Err(e) => fail(404, &e),
                },
                _ => refuse(400, "a kind (creature, placeable, door, trigger, waypoint, encounter, store or item) and a resref, please"),
            }
        }
        (Method::Get, "/api/blueprint-model") => {
            let (kind, resref) = (query.lower("kind"), query.lower("resref"));
            match (
                crate::things::kind_of(&kind).filter(|_| matches!(kind.as_str(), "placeable" | "door")),
                crate::newbp::resref_ok(&resref),
            ) {
                (Some(k), true) => match crate::models::game().and_then(|g| crate::things::model_of(g, k, &resref)) {
                    Ok(m) => json_response(200, &serde_json::json!({ "model": m })),
                    Err(e) => fail(404, &e),
                },
                _ => refuse(400, "a placeable or door resref, please"),
            }
        }
        (Method::Get, "/api/area-lists") => match crate::models::game() {
            Ok(g) => json_response(200, &crate::settings::lists(g)),
            Err(e) => fail(500, &e),
        },
        // The builder's own new blueprints (a creature or item copied from an existing one, then changed); `/api/blueprints`
        // lists them with the game's.
        (Method::Get, "/api/new-blueprint") => match crate::newbp::get(&query.text("ext"), &query.text("resref")) {
            Ok(v) => json_response(200, &v),
            Err(e) => fail(404, &e),
        },
        (Method::Post, p @ ("/api/new-blueprint" | "/api/new-blueprint-edit" | "/api/new-blueprint-delete")) => {
            let body = match read_body(cx.req, super::body_max::FIELDS) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let (ext, resref) = (query.text("ext"), query.text("resref"));
            let r = crate::models::game().and_then(|g| match p {
                "/api/new-blueprint" => crate::newbp::create(g, &ext, &resref, &query.text("from"), &query.text("name")),
                "/api/new-blueprint-edit" => {
                    let fields: serde_json::Value = serde_json::from_slice(&body)?;
                    crate::newbp::edit(&ext, &resref, fields.as_object().context("fields as an object")?)
                }
                _ => crate::newbp::delete(&ext, &resref).map(|()| serde_json::json!({})),
            });
            match r {
                Ok(v) => {
                    crate::audit::log(p.trim_start_matches("/api/"), Some(&resref), true, &ext);
                    json_response(200, &v)
                }
                Err(e) => fail(400, &e),
            }
        }
        // What a new creature of the builder's carries and wears.
        (Method::Post, "/api/new-blueprint-gear") => {
            let resref = query.text("resref");
            let body = match read_body(cx.req, super::body_max::FIELDS) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let r = crate::models::game().and_then(|g| {
                let v: serde_json::Value = serde_json::from_slice(&body)?;
                let inv = v["inventory"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|i| Some((i["resref"].as_str()?.to_ascii_lowercase(), i["infinite"].as_bool().unwrap_or(false))))
                        .collect()
                });
                let eq = v["equipment"].as_array().map(|a| {
                    a.iter().filter_map(|i| Some((i["slot"].as_str()?.to_string(), i["resref"].as_str()?.to_ascii_lowercase()))).collect()
                });
                let ext = query.raw("ext").unwrap_or("utc");
                crate::newbp::set_gear(g, ext, &resref, inv, eq)
            });
            match r {
                Ok(v) => json_response(200, &v),
                Err(e) => fail(400, &e),
            }
        }
        // The properties of a new item of the builder's.
        (Method::Get, p @ ("/api/itemprops" | "/api/itemprops-types" | "/api/itemprops-choices")) => {
            let num = |k: &str| query.number::<u16>(k).unwrap_or(0);
            let r = crate::models::game().and_then(|g| match p {
                "/api/itemprops" => crate::itemprops::get(g, &query.text("resref")),
                "/api/itemprops-types" => crate::itemprops::types(g, &query.text("resref")),
                _ => crate::itemprops::choices(g, &query.text("resref"), num("property"), num("subtype")),
            });
            match r {
                Ok(v) => json_response(200, &v),
                Err(e) => fail(400, &e),
            }
        }
        (Method::Post, p @ ("/api/itemprops-add" | "/api/itemprops-remove")) => {
            let body = match read_body(cx.req, super::body_max::FIELDS) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let r = crate::models::game().and_then(|g| match p {
                "/api/itemprops-add" => crate::itemprops::add(g, &query.text("resref"), &serde_json::from_slice(&body)?),
                _ => crate::itemprops::remove(g, &query.text("resref"), query.text("index").parse()?),
            });
            match r {
                Ok(v) => json_response(200, &v),
                Err(e) => fail(400, &e),
            }
        }
        (Method::Get, "/api/blueprints") => match query.get("kind").as_deref().and_then(crate::things::kind_of) {
            None => refuse(400, "which kind of blueprint?"),
            Some(kind) => {
                let limit = query.number::<usize>("limit").unwrap_or(60).clamp(1, 200);
                match crate::models::game().and_then(|g| crate::blueprints::search(g, kind, &query.text("q"), limit)) {
                    Ok(list) => json_response(200, &serde_json::json!({ "blueprints": list })),
                    Err(e) => fail(502, &e),
                }
            }
        },
        (Method::Get, "/api/placeables") => match crate::models::game().and_then(crate::models::placeables) {
            Ok(list) => json_response(200, &serde_json::json!({ "placeables": &*list })),
            Err(e) => fail(500, &e),
        },
        // The plants among them, measured once: the Place and Scatter panels' Small plants and Large plants (plants.rs).
        (Method::Get, "/api/placeable-groups") => match crate::models::game().and_then(crate::plants::plants) {
            Ok(list) => json_response(
                200,
                &serde_json::json!({ "large_from": crate::plants::LARGE_FROM, "max_width": crate::plants::MAX_WIDTH, "plants": &*list }),
            ),
            Err(e) => fail(500, &e),
        },
        // The particle pack's kinds: the Ambience tool's palette, colours and live counts.
        (Method::Get, "/api/particles") => match particle_pack() {
            Ok(pack) => json_response(200, &pack),
            Err(e) => fail(404, &e),
        },
        // Every door look there is: the generic doors (genericdoors.2da) and the tile doors (doortypes.2da).
        (Method::Get, "/api/door-appearances") => match crate::models::game().and_then(crate::models::door_appearances) {
            Ok(list) => json_response(200, &serde_json::json!({ "doors": list })),
            Err(e) => fail(500, &e),
        },
        (Method::Get, p) if p.starts_with("/api/model/") || p.starts_with("/api/texture/") => {
            let name = percent_decode(p.rsplit('/').next().unwrap_or("")).to_ascii_lowercase();
            // A resref, or an import's standing things (@import:<name>:<xxyy>).
            let imported = name
                .strip_prefix(crate::paint::IMPORT)
                .is_some_and(|r| r.len() <= 24 && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':'));
            if !imported && !super::query::resource_name_ok(&name) {
                refuse(400, "bad resref")
            } else if p.starts_with("/api/model/") {
                match crate::models::game().and_then(|g| crate::models::flat(g, &name)) {
                    Ok(f) => json_response(200, &serde_json::to_value(&*f).unwrap_or_default()),
                    Err(e) => fail(404, &e),
                }
            } else {
                match crate::models::game().and_then(|g| crate::models::texture_png(g, &name)) {
                    // The game's textures don't change while it runs.
                    Ok(png) => Response::from_data(png)
                        .with_header(response_header("Content-Type", "image/png"))
                        .with_header(response_header("Cache-Control", "max-age=3600")),
                    Err(e) => fail(404, &e),
                }
            }
        }
        _ => return None,
    })
}

/// Where the particle pack is when not beside the program: a folder holding its `pack.json`.
const PARTICLES_ENV: &str = "LANDSCAPER_PARTICLES";

/// The particle pack's `pack.json`, from the folder [`PARTICLES_ENV`] names, else `packs/particles` beside the program (and, in a
/// debug build, in the source tree): where the export reads it from.
fn particle_pack() -> anyhow::Result<serde_json::Value> {
    let named = std::env::var_os(PARTICLES_ENV).filter(|d| !d.is_empty()).map(PathBuf::from);
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("packs").join("particles")));
    let source = cfg!(debug_assertions).then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("packs").join("particles"));
    let dir = named
        .or_else(|| beside.into_iter().chain(source).find(|d| d.join("pack.json").is_file()))
        .with_context(|| format!("no particle pack: set {PARTICLES_ENV} to its folder, or keep packs/particles beside the program"))?;
    read_particle_pack(&dir)
}

/// The `pack.json` in `dir`, as it is, once it reads as a pack of format 1 with kinds.
fn read_particle_pack(dir: &Path) -> anyhow::Result<serde_json::Value> {
    let path = dir.join("pack.json");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let pack: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
    ensure!(pack["format"].as_u64() == Some(1), "{}: format {} (this Landscaper reads format 1)", path.display(), pack["format"]);
    ensure!(pack["kinds"].as_array().is_some_and(|k| !k.is_empty()), "{}: no kinds", path.display());
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_particle_pack_is_read_as_it_is_and_only_in_its_format() {
        let dir = std::env::temp_dir().join(format!("ls-particles-route-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |j: &str| std::fs::write(dir.join("pack.json"), j).unwrap();
        write(r#"{ "format": 1, "id": "p", "kinds": [{ "id": "mist", "shows": "always", "live": { "day": 18, "night": 18 } }] }"#);
        let pack = read_particle_pack(&dir).unwrap();
        assert_eq!(pack["kinds"][0]["live"]["night"], 18);
        write(r#"{ "format": 2, "kinds": [{ "id": "mist" }] }"#);
        assert!(format!("{:#}", read_particle_pack(&dir).unwrap_err()).contains("format 2"));
        write(r#"{ "format": 1, "kinds": [] }"#);
        assert!(format!("{:#}", read_particle_pack(&dir).unwrap_err()).contains("no kinds"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(read_particle_pack(&dir).is_err());
    }
}
