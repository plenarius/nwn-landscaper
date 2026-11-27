// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The read-only routes a trusted service calls to list and fetch a builder's exports (`/api/service/*`, GET only;
//! docs/REVIEW-SERVICE.md, "Fetching exports").
//!
//! Off (404) unless this is a team server with `LANDSCAPER_SERVICE_KEY` set. Every call carries `Authorization: Bearer <key>`
//! and names the builder in `user`; the routes then act as that builder (never an admin by group), so an export's name and
//! assignment are checked exactly as Publish checks them. The key isn't the proxy secret: it opens these routes and nothing else.
//! They come before the request gate, which guards against browsers: a page can't add the key to a request.

use std::io::Read;

use tiny_http::{Method, Response, ResponseBox};

use super::export::guarded_export;
use super::query::{Query, area_name_ok};
use super::{Reply, json_response, refuse, response_header};
use crate::export_id;
use crate::workspace::{clean_user, exports_dir, nwn_user_dir, same_secret, workspaces_dir};

/// The routes' common start.
const PREFIX: &str = "/api/service/";

/// The most files streamed at once: a download holds an answer thread while it lasts.
const STREAMS: usize = 8;

/// Answers a request for `/api/service/*`; `None` for any other.
pub(super) fn route(req: &tiny_http::Request) -> Option<Reply> {
    let url = req.url();
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let route = path.strip_prefix(PREFIX).or_else(|| (path == "/api/service").then_some(""))?;
    let Some(key) = crate::config::service_key().filter(|_| crate::workspace::team()) else {
        return Some(refuse(404, "not found").into());
    };
    crate::audit::set_address(req.remote_addr().map_or_default(|a| a.ip().to_string()));
    let header =
        |name: &str| req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.to_string());
    let sent = header("Authorization").and_then(|a| a.trim().strip_prefix("Bearer ").map(|k| k.trim().to_string()));
    if !sent.is_some_and(|k| same_secret(&k, &key)) {
        crate::audit::log_refused(None, 401, "a service request without the service key");
        return Some(refuse(401, "the service key is missing or wrong").into());
    }
    if *req.method() != Method::Get {
        return Some(refuse(405, "the service routes are GET only").into());
    }
    let query = Query::new(query);
    let Some(user) = clean_user(&query.text("user")) else {
        return Some(refuse(400, "bad or missing user").into());
    };
    if !workspaces_dir().join(&user).is_dir() {
        return Some(refuse(404, &format!("{user} has no workspace")).into());
    }
    crate::workspace::set_user(Some(user.clone()));
    // An admin by name only: the groups that would make one aren't sent to these routes.
    crate::workspace::set_acting(&user, crate::config::AuthConfig::from_env().admin_users.contains(&user));
    let name = query.text("name");
    let answer = match route {
        "exports" => Ok(list()),
        "export" => export(&name),
        "hak" => checked(&name).and_then(|()| stream(&format!("{name} has no hak"), export_id::hak_path(&name), header("If-None-Match"))),
        "library" => stream(
            "the texture library has no hak",
            nwn_user_dir().map(|d| d.join("hak").join(format!("{}.hak", crate::shared::HAK))),
            header("If-None-Match"),
        ),
        "skies" => stream(
            &format!("no area here has a Landscaper sky: there is no {}.hak", crate::skies::HAK),
            nwn_user_dir().map(|d| d.join("hak").join(format!("{}.hak", crate::skies::HAK))),
            header("If-None-Match"),
        ),
        _ => Err(refuse(404, "no such service route").into()),
    };
    let ok = answer.as_ref().is_ok_and(|r| r.status() < 400);
    crate::audit::log("service-export", (!name.is_empty()).then_some(name.as_str()), ok, route);
    Some(answer.unwrap_or_else(|r| r))
}

/// Refuses `name` unless it is an export this builder may fetch: 400 for a bad name, 404 for none, 403 when Publish would refuse it.
fn checked(name: &str) -> Result<(), Reply> {
    if !area_name_ok(name) {
        return Err(refuse(400, &format!("bad area name {name:?}")).into());
    }
    if !exports_dir().join(format!("{name}.json")).is_file() {
        return Err(refuse(404, &format!("{name} hasn't been exported")).into());
    }
    guarded_export(name).map(drop).map_err(|e| refuse(403, &format!("{e:#}")).into())
}

/// Every export this builder may fetch, by name.
fn list() -> Reply {
    let mut names: Vec<String> = std::fs::read_dir(exports_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|f| f.strip_suffix(".json")).filter(|n| area_name_ok(n)).map(str::to_string))
        .collect();
    names.sort();
    let library = nwn_user_dir().map(|d| d.join("hak").join(format!("{}.hak", crate::shared::HAK)));
    let library =
        library.and_then(|p| export_id::sha256_file(&p).ok()).map(|(sha, bytes)| serde_json::json!({ "bytes": bytes, "sha256": sha }));
    let skies = nwn_user_dir().map(|d| d.join("hak").join(format!("{}.hak", crate::skies::HAK)));
    let skies =
        skies.and_then(|p| export_id::sha256_file(&p).ok()).map(|(sha, bytes)| serde_json::json!({ "bytes": bytes, "sha256": sha }));
    let out: Vec<serde_json::Value> = names.iter().filter_map(|n| entry(n, library.as_ref(), skies.as_ref())).collect();
    json_response(200, &serde_json::Value::Array(out)).into()
}

/// One export of the list, or `None` when it isn't one this builder may fetch (an area not exported to NWN, an ended assignment).
fn entry(name: &str, library: Option<&serde_json::Value>, skies: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let header_bytes = crate::atomic::read(&exports_dir().join(format!("{name}.json"))).ok()?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes).ok()?;
    header["tiles"].as_u64()?;
    guarded_export(name).ok()?;
    let hak = export_id::hak_path(name)?;
    let (sha, bytes) = export_id::sha256_file(&hak).ok()?;
    let exported_at = std::fs::metadata(&hak).and_then(|m| m.modified()).ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let rework = crate::import::recorded(name).ok()?;
    Some(serde_json::json!({
        "name": name, "area": rework.clone().unwrap_or_else(|| name.to_string()), "new": rework.is_none(),
        "module": header["module"].as_str().unwrap_or(name), "exported_at": exported_at, "export": export_id::of(&header_bytes, &sha),
        "hak": { "bytes": bytes, "sha256": sha }, "library": library, "skies": skies,
    }))
}

/// One export's area files, new blueprints and the pre-publish findings.
fn export(name: &str) -> Result<Reply, Reply> {
    checked(name)?;
    let made = (|| {
        let id = export_id::export_id(name)?;
        let mut v = crate::publish::service_export(name)?;
        v["export"] = id.into();
        v["findings"] = serde_json::to_value(crate::publish::findings(name)?)?;
        anyhow::Ok(v)
    })();
    match made {
        Ok(v) => Ok(json_response(200, &v).into()),
        Err(e) => Err(refuse(422, &format!("{e:#}")).into()),
    }
}

/// The file at `path` (a hak), streamed, with its SHA-256 as the `ETag`; 304 when `if_none_match` names it. `missing` is the refusal when there is no such file.
fn stream(missing: &str, path: Option<std::path::PathBuf>, if_none_match: Option<String>) -> Result<Reply, Reply> {
    let path = path.ok_or_else(|| refuse(404, "no module folder"))?;
    let Ok(mut file) = crate::atomic::open(&path) else {
        return Err(refuse(404, missing).into());
    };
    let (sha, len) = export_id::sha256_of(&path, &mut file).map_err(|e| refuse(500, &format!("{e:#}")))?;
    let etag = format!("\"{sha}\"");
    let headers = || [response_header("ETag", etag.as_str()), response_header("Cache-Control", "private, no-store")];
    if if_none_match.is_some_and(|m| m.split(',').any(|t| t.trim().trim_start_matches("W/") == etag)) {
        return Ok(Reply::Stream(headers().into_iter().fold(Response::empty(304).boxed(), Response::with_header)));
    }
    let turn = Turn::begin().ok_or_else(|| refuse(503, "too many downloads at once: try again shortly"))?;
    let len = usize::try_from(len).map_err(|_| refuse(500, "the file is too big"))?;
    let body: ResponseBox = Response::new(tiny_http::StatusCode(200), Vec::new(), Streamed { file, _turn: turn }, Some(len), None).boxed();
    // (A length, not chunks: the service checks the size as it arrives.)
    let body = body.with_header(response_header("Content-Type", "application/octet-stream")).with_chunked_threshold(usize::MAX);
    Ok(Reply::Stream(headers().into_iter().fold(body, Response::with_header)))
}

/// A file being streamed; its [`Turn`] ends when the answer is sent and this is dropped.
struct Streamed {
    file: std::fs::File,
    _turn: Turn,
}

impl Read for Streamed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

/// One of the [`STREAMS`] downloads that may run at once.
struct Turn;

static NOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Turn {
    fn begin() -> Option<Turn> {
        use std::sync::atomic::Ordering;
        if NOW.fetch_add(1, Ordering::SeqCst) < STREAMS {
            return Some(Turn);
        }
        NOW.fetch_sub(1, Ordering::SeqCst);
        None
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        NOW.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
