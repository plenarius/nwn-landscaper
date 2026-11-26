// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The skies' routes, for the Sky & fog panel: what it offers (`/api/skies`) and a sky's pictures
//! (`/api/sky-picture/<sky>?time=day&kind=strip`), which the panel and the 3D view load as pictures. Installing and removing sky
//! packs stays on the command line (docs/SKY-PACKS.md).

use tiny_http::{Method, Response};

use super::{Answer, Ask, fail, json_response, refuse, response_header};
use crate::skies::picture::Kind;

/// Answers the skies' routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/skies") => match crate::skies::offered() {
            Ok(v) => json_response(200, &v),
            Err(e) => fail(500, &e),
        },
        (Method::Get, p) if p.starts_with("/api/sky-picture/") => {
            let sky = p.trim_start_matches("/api/sky-picture/");
            let time = cx.query.get("time").unwrap_or_else(|| "day".into());
            let kind = cx.query.get("kind").unwrap_or_else(|| "strip".into());
            // (A sky's id or row-<n>: lower-case letters, digits and -; nothing else reaches a path.)
            if sky.is_empty() || sky.len() > 40 || !sky.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return Some(refuse(400, "that is not a sky"));
            }
            let Some(kind) = Kind::parse(&kind) else { return Some(refuse(400, "kind is strip or panorama")) };
            match crate::skies::picture_of(sky, &time, kind) {
                // (A sky keeps its id when its pack is updated: kept a short while only.)
                Ok(png) => Response::from_data(png)
                    .with_header(response_header("Content-Type", "image/png"))
                    .with_header(response_header("Cache-Control", "max-age=300")),
                Err(e) => fail(404, &e),
            }
        }
        _ => return None,
    })
}
