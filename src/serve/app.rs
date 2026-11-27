// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The desktop app's own route: `POST /api/app/open` with `{"what": "data" | "log" | "folders"}` opens the data folder or the
//! log with the system, or shows the app's game-folder page. It answers 204, or `{"error": ...}` with 409 when the app can't.
//! Only the app has it: `serve_local` registers what to do ([`set_open`]); `landscaper serve` and team servers never do, so for
//! them the route is unknown, like any other.

use std::sync::OnceLock;

use tiny_http::{Method, Response};

use super::{Answer, Ask, json_response, read_body, refuse};

/// What the app does for each `what` (`crate::AppOpen`).
pub type Open = crate::AppOpen;

static OPEN: OnceLock<Open> = OnceLock::new();

/// Registers the app's handler, once, before it serves.
pub(crate) fn set_open(open: Open) {
    let _ = OPEN.set(open);
}

/// Answers `/api/app/open` when the app registered a handler; `None` for anything else, and for that route without one.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    if cx.path != "/api/app/open" || crate::workspace::team() {
        return None;
    }
    let open = OPEN.get()?;
    if cx.method != Method::Post {
        return Some(refuse(405, "POST /api/app/open"));
    }
    let body = match read_body(cx.req, 4096) {
        Ok(b) => b,
        Err(answer) => return Some(answer),
    };
    Some(answer(open, &body))
}

fn answer(open: &Open, body: &[u8]) -> Answer {
    let what = serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v["what"].as_str().map(str::to_string));
    match what.as_deref() {
        Some(w @ ("data" | "log" | "folders")) => match open(w) {
            Ok(()) => Response::from_data(Vec::new()).with_status_code(204),
            Err(why) => json_response(409, &serde_json::json!({ "error": why })),
        },
        _ => refuse(400, "what: data, log or folders"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_opens_the_three_things_and_says_why_not() {
        let open: super::Open = Box::new(|w| if w == "log" { Err("no log".into()) } else { Ok(()) });
        assert_eq!(super::answer(&open, br#"{"what":"data"}"#).status_code().0, 204);
        assert_eq!(super::answer(&open, br#"{"what":"folders"}"#).status_code().0, 204);
        assert_eq!(super::answer(&open, br#"{"what":"log"}"#).status_code().0, 409);
        assert_eq!(super::answer(&open, br#"{"what":"/etc"}"#).status_code().0, 400);
        assert_eq!(super::answer(&open, b"not json").status_code().0, 400);
    }
}
