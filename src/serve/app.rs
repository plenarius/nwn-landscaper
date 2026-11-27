// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The desktop app's own routes ([`crate::AppRoutes`]):
//!
//! - `POST /api/app/open` with `{"what": "data" | "log" | "folders"}` opens the data folder or the log with the system, or shows
//!   the app's game-folder page. It answers 204, or `{"error": ...}` with 409 when the app can't.
//! - `GET /api/app/update` answers the app's updater as JSON (`{"state": "checking" | "none" | "available" | "installing", ...}`),
//!   and `POST /api/app/update` installs the update it offers: 202, or `{"error": ...}` with 409.
//!
//! Only the app has them: `serve_local` registers what to do ([`set_routes`]); `landscaper serve` and team servers never do, so
//! for them the routes are unknown, like any other.

use std::sync::OnceLock;

use tiny_http::{Method, Response};

use super::{Answer, Ask, json_response, read_body, refuse};

static ROUTES: OnceLock<crate::AppRoutes> = OnceLock::new();

/// Registers the app's handlers, once, before it serves.
pub(crate) fn set_routes(routes: crate::AppRoutes) {
    let _ = ROUTES.set(routes);
}

/// Answers `/api/app/open` and `/api/app/update` when the app registered its handlers; `None` for anything else, and for those
/// routes without them.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    if !matches!(cx.path, "/api/app/open" | "/api/app/update") || crate::workspace::team() {
        return None;
    }
    let routes = ROUTES.get()?;
    match (cx.path, &cx.method) {
        ("/api/app/update", Method::Get) => Some(json_response(200, &(routes.update)())),
        ("/api/app/update", Method::Post) => Some(done((routes.install)(), 202)),
        ("/api/app/update", _) => Some(refuse(405, "GET or POST /api/app/update")),
        (_, Method::Post) => Some(match read_body(cx.req, 4096) {
            Ok(body) => open(&routes.open, &body),
            Err(answer) => answer,
        }),
        _ => Some(refuse(405, "POST /api/app/open")),
    }
}

/// `status` with no body when the app did it, else 409 with why not.
fn done(outcome: Result<(), String>, status: u16) -> Answer {
    match outcome {
        Ok(()) => Response::from_data(Vec::new()).with_status_code(status),
        Err(why) => json_response(409, &serde_json::json!({ "error": why })),
    }
}

fn open(open: &crate::AppOpen, body: &[u8]) -> Answer {
    let what = serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v["what"].as_str().map(str::to_string));
    match what.as_deref() {
        Some(w @ ("data" | "log" | "folders")) => done(open(w), 204),
        _ => refuse(400, "what: data, log or folders"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_opens_the_three_things_and_says_why_not() {
        let open: crate::AppOpen = Box::new(|w| if w == "log" { Err("no log".into()) } else { Ok(()) });
        assert_eq!(super::open(&open, br#"{"what":"data"}"#).status_code().0, 204);
        assert_eq!(super::open(&open, br#"{"what":"folders"}"#).status_code().0, 204);
        assert_eq!(super::open(&open, br#"{"what":"log"}"#).status_code().0, 409);
        assert_eq!(super::open(&open, br#"{"what":"/etc"}"#).status_code().0, 400);
        assert_eq!(super::open(&open, b"not json").status_code().0, 400);
    }

    #[test]
    fn an_install_answers_202_or_why_not() {
        assert_eq!(super::done(Ok(()), 202).status_code().0, 202);
        assert_eq!(super::done(Err("nothing to install".into()), 202).status_code().0, 409);
    }
}
