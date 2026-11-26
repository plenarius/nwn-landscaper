// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An area's underlay: a picture the editor lays under the ground to trace over (a battlemat, a scanned map), kept as
//! `areas/<name>.underlay` beside the working copy, so it is there on another computer and for an admin looking. Where it lies
//! (its place, size, turn, how see-through) is in the area's JSON; only the picture is here. Nothing exports it.

use anyhow::Result;
use tiny_http::Method;

use super::areas::{area_exists, guard_assignment};
use super::query::area_name_ok;
use super::{Answer, Ask, fail, json_response, read_body, refuse, response_header};
use crate::workspace::areas_dir;

/// Answers `/api/underlay`; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    if cx.path != "/api/underlay" {
        return None;
    }
    let name = cx.query.text("name");
    if !area_name_ok(&name) {
        return Some(refuse(400, &format!("bad area name {name:?}")));
    }
    if let Err(e) = guard_assignment(&name, &[]) {
        return Some(fail(403, &e));
    }
    let path = areas_dir().join(format!("{name}.underlay"));
    Some(match cx.method {
        Method::Get => match crate::atomic::read(&path) {
            Ok(picture) => match kind(&picture) {
                Some(mime) => tiny_http::Response::from_data(picture)
                    .with_header(response_header("Content-Type", mime))
                    .with_header(response_header("Cache-Control", "no-store")),
                None => refuse(500, &format!("{name}'s underlay isn't a picture")),
            },
            Err(_) => refuse(404, &format!("{name} has no underlay")),
        },
        Method::Put => {
            let body = match read_body(cx.req, super::body_max::UNDERLAY) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            match save(&name, &body) {
                Ok(()) => json_response(200, &serde_json::json!({ "saved": body.len() })),
                Err(e) if e.downcast_ref::<crate::quota::Full>().is_some() => fail(413, &e),
                Err(e) if e.downcast_ref::<NotPicture>().is_some() => fail(415, &e),
                Err(e) => fail(404, &e),
            }
        }
        Method::Delete => {
            let _ = std::fs::remove_file(&path);
            crate::quota::forget();
            json_response(200, &serde_json::json!({ "removed": name }))
        }
        _ => refuse(405, "GET, PUT or DELETE"),
    })
}

/// A body that isn't one of the pictures a browser draws ([`kind`]), answered 415.
#[derive(Debug)]
struct NotPicture;

impl std::fmt::Display for NotPicture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "that isn't a PNG, JPEG, WebP, GIF or BMP picture")
    }
}

impl std::error::Error for NotPicture {}

/// The media type of picture `bytes`, from its first bytes: PNG, JPEG, WebP, GIF or BMP; `None` for anything else. It is
/// answered as that type, never as what the upload claimed, so a page or a script can't be stored and served as one.
pub(super) fn kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        Some("image/bmp")
    } else {
        None
    }
}

/// Keeps `picture` as area `name`'s underlay, written whole or not at all. The area must be saved first (its working copy, an
/// export or an import): an underlay of no area would be a file nothing removes.
fn save(name: &str, picture: &[u8]) -> Result<()> {
    if kind(picture).is_none() {
        return Err(NotPicture.into());
    }
    anyhow::ensure!(area_exists(name), "there is no area called {name}: save it first");
    let dir = areas_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.underlay"));
    let before = std::fs::metadata(&path).map_or(0, |m| m.len());
    crate::quota::room(name, (picture.len() as u64).saturating_sub(before))?;
    crate::atomic::write(&path, picture)?;
    crate::quota::wrote((picture.len() as u64).saturating_sub(before));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_told_by_their_first_bytes() {
        assert_eq!(kind(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(kind(b"\xff\xd8\xff\xe0...."), Some("image/jpeg"));
        assert_eq!(kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(kind(b"GIF89a.."), Some("image/gif"));
        assert_eq!(kind(&[b"BM".as_slice(), &[0; 30]].concat()), Some("image/bmp"));
        for not in [&b"<html><script>"[..], b"%PDF-1.7", b"", b"BM", b"RIFF\0\0\0\0WAVE"] {
            assert_eq!(kind(not), None, "{not:?}");
        }
    }
}
