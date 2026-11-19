// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The builder's own conversations, edited as a tree of lines: the list, one to read (theirs, or with `any=1` the module's or
//! the game's), and making, saving and deleting one.

use tiny_http::Method;

use super::{Answer, Ask, fail, json_response, read_body};

/// Answers the conversation routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let query = cx.query;
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/dialogs") => json_response(
            200,
            &serde_json::json!({ "dialogs": crate::dialog::list(), "found": crate::dialog::search(&query.text("q"), 40) }),
        ),
        (Method::Get, "/api/dialog") => {
            let resref = query.text("resref");
            let any = query.flag("any");
            match crate::models::game().and_then(|g| if any { crate::dialog::get_any(g, &resref) } else { crate::dialog::get(g, &resref) })
            {
                Ok(v) => json_response(200, &v),
                Err(e) => fail(404, &e),
            }
        }
        (Method::Post, p @ ("/api/dialog" | "/api/dialog-save" | "/api/dialog-delete")) => {
            let body = match read_body(cx.req, super::body_max::DIALOG) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let resref = query.text("resref");
            let r = crate::models::game().and_then(|g| match p {
                "/api/dialog" => crate::dialog::create(g, &resref, &query.text("from")),
                "/api/dialog-save" => crate::dialog::save(g, &resref, &serde_json::from_slice(&body)?, query.one("compact")),
                _ => crate::dialog::delete(&resref).map(|()| serde_json::json!({})),
            });
            match r {
                Ok(v) => {
                    crate::audit::log(p.trim_start_matches("/api/"), Some(&resref), true, "");
                    json_response(200, &v)
                }
                Err(e) => fail(400, &e),
            }
        }
        _ => return None,
    })
}
