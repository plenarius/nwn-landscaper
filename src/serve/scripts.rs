// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scripts: anyone may pick one by name; reading, writing and compiling them is for admins (a script runs on the live server).

use tiny_http::Method;

use super::{Answer, Ask, fail, json_response, read_body, refuse};

/// Answers the script routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let (me, query) = (&cx.me, cx.query);
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/scripts") => {
            json_response(200, &serde_json::json!({ "scripts": crate::newscript::search(&query.text("q"), 40) }))
        }
        (Method::Get, "/api/script") | (Method::Post, "/api/script" | "/api/script-save" | "/api/script-delete") if !me.admin => {
            refuse(403, "writing scripts is for admins for now")
        }
        (Method::Get, "/api/script") => {
            let resref = query.text("resref");
            // `any=1`: to look at: yours, else the module's, else the game's (read only; saving only touches your own).
            let any = query.flag("any");
            let mine = crate::newscript::read(&resref);
            let found = match (&mine, any) {
                (Some(t), _) => Some(t.clone()),
                (None, true) => crate::models::game()
                    .ok()
                    .and_then(|g| crate::newscript::source(g, &resref.to_ascii_lowercase()))
                    .and_then(|b| String::from_utf8(b).ok()),
                (None, false) => None,
            };
            match found {
                Some(source) => json_response(200, &serde_json::json!({ "resref": resref, "source": source, "mine": mine.is_some() })),
                None => refuse(404, &format!("no script {resref} {}", if any { "in the module or the game" } else { "of yours" })),
            }
        }
        (Method::Post, p @ ("/api/script" | "/api/script-save" | "/api/script-delete")) => {
            let body = match read_body(cx.req, super::body_max::SCRIPT) {
                Ok(b) => b,
                Err(refused) => return Some(refused),
            };
            let resref = query.text("resref");
            let r = crate::models::game().and_then(|g| match p {
                "/api/script" => crate::newscript::create(g, &resref, &query.text("from"), &query.text("kind")),
                "/api/script-save" => crate::newscript::save(g, &resref, &String::from_utf8(body.clone())?),
                _ => crate::newscript::delete(&resref).map(|()| serde_json::json!({})),
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
