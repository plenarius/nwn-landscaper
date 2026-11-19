// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Previews the review service bakes and the editor shows: an item's icon, a creature's scene and its textures, passed through
//! from the service (review.rs). Their files are named by what they are made of, so the browser keeps them for good.

use tiny_http::{Header, Method, Response};

use super::query::percent_decode;
use super::{Answer, Ask, fail, json_response, refuse};

/// Answers the preview routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let query = cx.query;
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/preview/item") => {
            let resref = query.lower("resref");
            if !crate::newbp::resref_ok(&resref) {
                refuse(400, "which item?")
            } else {
                match crate::review::call("GET", &format!("/api/v1/blueprints/uti/{resref}/icon"), None) {
                    Ok(v) => json_response(200, &v),
                    Err(e) => fail(502, &e),
                }
            }
        }
        (Method::Get, p) if p.starts_with("/api/preview-icon/") => {
            // /api/preview-icon/<key>.png: the key says what the icon is made of, so it never changes.
            let key = percent_decode(p.trim_start_matches("/api/preview-icon/")).trim_end_matches(".png").to_string();
            if key.is_empty()
                || key.len() > 300
                || !key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '+' | '|' | '~' | '^' | '-' | '.'))
            {
                refuse(400, "bad icon")
            } else {
                let enc: String =
                    key.chars().map(|c| if matches!(c, '|' | '^') { format!("%{:02X}", c as u8) } else { c.to_string() }).collect();
                match crate::review::fetch_bytes(&format!("/api/v1/icons/{enc}.png"), "image/png") {
                    Ok(Some(body)) => kept_for_good(body, "image/png"),
                    Ok(None) => refuse(404, "not there"),
                    Err(e) => fail(502, &e),
                }
            }
        }
        (Method::Get, "/api/preview/creature") => {
            let resref = query.lower("resref");
            if !crate::newbp::resref_ok(&resref) {
                refuse(400, "which creature?")
            } else {
                match crate::review::call("GET", &format!("/api/v1/blueprints/utc/{resref}/preview"), None) {
                    Ok(v) => json_response(200, &v),
                    Err(e) => fail(502, &e),
                }
            }
        }
        (Method::Get, p) if p.starts_with("/api/preview-file/") => {
            // /api/preview-file/<64 hex>/scene.json or /tex/<name>.webp (or .png, from an older bake): named by content, so
            // kept for good. Each is served as the type its name says, and the review service must have sent it as that.
            let rest = p.trim_start_matches("/api/preview-file/");
            let ok_key = |k: &str| k.len() == 64 && k.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
            let file = rest.split_once('/').and_then(|(k, f)| {
                ok_key(k)
                    .then(|| match f {
                        "scene.json" => Some((format!("/api/v1/render/{k}/scene.json"), "application/json")),
                        _ => f
                            .strip_prefix("tex/")
                            .filter(|t| preview_tex_ok(t))
                            .map(|t| (format!("/api/v1/render/{k}/tex/{t}"), preview_tex_type(t))),
                    })
                    .flatten()
            });
            match file.map(|(path, want)| (crate::review::fetch_bytes(&path, want), want)) {
                Some((Ok(Some(body)), want)) => kept_for_good(body, want),
                Some((Ok(None), _)) => refuse(404, "not there"),
                Some((Err(e), _)) => fail(502, &e),
                None => refuse(400, "bad preview file"),
            }
        }
        _ => return None,
    })
}

/// A file named by what it is made of, as `ctype` (one of this file's own, never one the review service sent): the browser
/// keeps it for a year.
fn kept_for_good(body: Vec<u8>, ctype: &'static str) -> Answer {
    let headers = [("Content-Type", ctype), ("Cache-Control", "private, max-age=31536000, immutable")];
    headers.into_iter().filter_map(|(k, v)| Header::from_bytes(k, v).ok()).fold(Response::from_data(body), Response::with_header)
}

/// The content type of a preview's texture, by its name ([`preview_tex_ok`]).
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "preview_tex_ok allows lower case names only")]
pub(super) fn preview_tex_type(t: &str) -> &'static str {
    if t.ends_with(".png") { "image/png" } else { "image/webp" }
}

/// A baked preview's texture name the editor may ask the review service for: lower case letters, digits, `_ . -`, ending in `.webp`
/// (what the baker writes now) or `.png` (older bakes).
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the name must be lower case anyway (the check below)")]
pub(super) fn preview_tex_ok(t: &str) -> bool {
    (t.ends_with(".webp") || t.ends_with(".png"))
        && (1..=124).contains(&t.len())
        && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'))
}
