// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editor's own files (landscaper-web) and the texture library (`/textures/`), for any GET no API route answered; any
//! other method that reached no route is refused with 405. `/changelog.d/` lists the editor's changelog entries waiting for
//! its next release, by which it says `+dev` beside its version (its js/version.js).

use std::path::{Component, Path, PathBuf};

use tiny_http::{Method, Response};

use super::{Answer, Ask, json_response, refuse, response_header};
use crate::paint::texture_dir;

/// Answers a GET with a file from `web` (or the texture library), and anything else with 405.
pub(super) fn route(cx: &mut Ask<'_>, web: &Path) -> Answer {
    let path = cx.path;
    match (&cx.method, path) {
        (Method::Get, _) => {
            let rel = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
            let (root, rel) = match rel.strip_prefix("textures/") {
                Some(t) => (texture_dir(), PathBuf::from(t)),
                None if rel.trim_end_matches('/') == "changelog.d" => {
                    let waiting = crate::version::fragments(&web.join("changelog.d"));
                    return json_response(200, &serde_json::json!({ "fragments": waiting }));
                }
                None => (web.to_path_buf(), PathBuf::from(rel)),
            };
            // Only plain relative paths inside the web folder / texture library, and no dot files (a clone's `.git`).
            if rel.as_os_str().is_empty()
                || rel.components().any(|c| !matches!(c, Component::Normal(n) if !n.to_string_lossy().starts_with('.')))
            {
                refuse(403, "forbidden")
            } else {
                let file = root.join(&rel);
                let if_none_match = cx.req.headers().iter().find(|h| h.field.equiv("If-None-Match")).map(|h| h.value.to_string());
                match std::fs::read(&file) {
                    // A team server: the browser keeps its copy and asks whether it changed (a 304 when it
                    // hasn't, so a page load is a few small answers, not 1.6 MB); the libraries, which
                    // change rarely, are kept a day. Private: everything is behind the sign-in.
                    Ok(data) if crate::workspace::team() => {
                        let tag = format!("\"{:016x}\"", fnv1a(&data));
                        let control = if rel.starts_with("vendor") { "private, max-age=86400" } else { "private, no-cache" };
                        let r = if if_none_match.as_deref() == Some(tag.as_str()) {
                            Response::from_data(Vec::new()).with_status_code(304)
                        } else {
                            Response::from_data(data).with_header(response_header("Content-Type", content_type(&file)))
                        };
                        r.with_header(response_header("ETag", tag)).with_header(response_header("Cache-Control", control))
                    }
                    // On your own machine: no-store, the editor changes often; never run a stale copy.
                    Ok(data) => Response::from_data(data)
                        .with_header(response_header("Content-Type", content_type(&file)))
                        .with_header(response_header("Cache-Control", "no-store")),
                    Err(_) => refuse(404, "not found"),
                }
            }
        }
        _ => refuse(405, "method not allowed"),
    }
}

/// The `Content-Type` of a file of the editor, by its extension.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// FNV-1a, 64 bits: a content tag that is the same on every run (std's hashers change between runs).
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}
