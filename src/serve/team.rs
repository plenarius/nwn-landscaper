// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The team's routes: who is asking (`/api/me`), the people with a workspace, an admin looking at someone's workspace, the
//! activity log, and assignments (who may work on which existing area).

use anyhow::{Context, Result};
use tiny_http::Method;

use super::{Answer, Ask, fail, json_response, refuse, response_header};

/// Answers the team's routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let (me, query) = (&cx.me, cx.query);
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/me") => json_response(
            200,
            &serde_json::json!({
                "version": crate::version::text(),
                "team": crate::workspace::team(),
                "user": me.user,
                "name": me.name,
                "admin": me.admin,
                "groups": me.groups,
                "viewing": cx.viewing,
                "versions": crate::gitea::enabled(),
                "publish": crate::review::active(),
                "package": crate::review::config().is_none(),
                "reviewers": std::env::var("LANDSCAPER_REVIEWERS").unwrap_or_default(),
            }),
        ),
        (Method::Get, "/api/users") if me.admin => json_response(
            200,
            &serde_json::json!({
                "users": crate::workspace::list_users()
                    .into_iter()
                    .map(|(user, areas, last)| serde_json::json!({ "user": user, "areas": areas, "last": last }))
                    .collect::<Vec<_>>(),
            }),
        ),
        (Method::Get, "/api/users") => refuse(403, "admins only"),
        // An admin looks at someone's workspace (reading only: the cookie
        // counts for GET requests alone); an empty user is back to their own.
        (Method::Post, "/api/view-as") if me.admin => {
            let user = query.text("user");
            let cookie = match crate::workspace::clean_user(&user) {
                Some(u) if crate::workspace::workspaces_dir().join(&u).is_dir() => format!("ls_as={u}; Path=/; HttpOnly; SameSite=Strict"),
                _ => "ls_as=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict".to_string(),
            };
            if cookie.starts_with("ls_as=") && !cookie.starts_with("ls_as=;") {
                crate::audit::log("view_as", None, true, &format!("looks at {user}'s workspace"));
            }
            json_response(200, &serde_json::json!({ "ok": true })).with_header(response_header("Set-Cookie", cookie))
        }
        (Method::Post, "/api/view-as") => refuse(403, "admins only"),
        // Admins read the audit log.
        (Method::Get, "/api/audit") if me.admin => {
            let limit = query.number::<usize>("limit").unwrap_or(200).min(1000);
            json_response(
                200,
                &serde_json::json!({
                    "events": crate::audit::read(limit, query.nonempty("user").as_deref(), query.nonempty("action").as_deref()),
                }),
            )
        }
        (Method::Get, "/api/audit") => refuse(403, "admins only"),
        // Assignments: who may work on which existing area. Everyone sees their own; admins see all and grant and revoke.
        (Method::Get, "/api/assignments") => match crate::assignments::all() {
            Ok(all) => {
                let t = crate::assignments::now();
                let rows: Vec<serde_json::Value> = all
                    .into_iter()
                    .filter(|a| me.admin || a.user == me.user)
                    .map(|a| {
                        let active = a.active_at(t);
                        let mut v = serde_json::to_value(&a).unwrap_or_default();
                        v["active"] = active.into();
                        v
                    })
                    .collect();
                json_response(200, &serde_json::json!({ "assignments": rows }))
            }
            Err(e) => fail(500, &e),
        },
        (Method::Post, "/api/assignments") if me.admin => {
            let (who_for, area) = (query.text("user"), query.text("area").to_ascii_lowercase());
            match assign_request(&me.user, query.get("user"), query.get("area"), query.get("days"), query.get("note")) {
                Ok(v) => {
                    crate::audit::log(
                        "assign",
                        Some(&area),
                        true,
                        &format!("to {who_for}{}", query.get("days").map_or(String::new(), |d| format!(" for {d} days"))),
                    );
                    json_response(200, &v)
                }
                Err(e) => {
                    crate::audit::log("assign", Some(&area), false, &format!("to {who_for}: {e:#}"));
                    fail(400, &e)
                }
            }
        }
        (Method::Post, "/api/assignments/revoke") if me.admin => {
            let (who_for, area) = (query.text("user"), query.text("area").to_ascii_lowercase());
            match revoke_request(query.get("user"), query.get("area")) {
                Ok(v) => {
                    crate::audit::log(
                        "revoke",
                        Some(&area),
                        true,
                        &format!("from {who_for}: {} ended, {} removed", v["ended"], v["removed_imports"].as_array().map_or(0, Vec::len)),
                    );
                    json_response(200, &v)
                }
                Err(e) => fail(400, &e),
            }
        }
        (Method::Post, "/api/assignments" | "/api/assignments/revoke") => refuse(403, "admins only"),
        _ => return None,
    })
}

/// Grants a builder an area (an admin's action).
fn assign_request(
    by: &str,
    user: Option<String>,
    area: Option<String>,
    days: Option<String>,
    note: Option<String>,
) -> Result<serde_json::Value> {
    let user = user.and_then(|u| crate::workspace::clean_user(&u)).context("which user?")?;
    let area = area.map(|a| a.to_ascii_lowercase()).filter(|a| !a.is_empty()).context("which area?")?;
    anyhow::ensure!(
        area == crate::assignments::ALL
            || crate::import::sources()?.iter().any(|s| s["area"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(&area))),
        "no existing area called {area}"
    );
    // (A `days` that isn't a whole number is refused: read as no end, it gave the area for good.)
    let days = days
        .filter(|d| !d.trim().is_empty())
        .map(|d| d.trim().parse::<u64>().with_context(|| format!("days: {d:?} isn't a whole number of days")))
        .transpose()?;
    let a = crate::assignments::grant(&user, &area, by, days, note)?;
    Ok(serde_json::to_value(&a)?)
}

/// Ends a builder's assignment of an area and removes what they built from the areas it gave them and no other assignment
/// still does, in their workspace (the kept original, pictures, build and hak). When it ended nothing, nothing is removed.
fn revoke_request(user: Option<String>, area: Option<String>) -> Result<serde_json::Value> {
    let user = user.and_then(|u| crate::workspace::clean_user(&u)).context("which user?")?;
    let area = area.map(|a| a.to_ascii_lowercase()).filter(|a| !a.is_empty()).context("which area?")?;
    let ended = crate::assignments::revoke(&user, &area)?;
    let removed = if ended > 0 { crate::import::remove_imports_of(&user, &area)? } else { Vec::new() };
    Ok(serde_json::json!({ "ended": ended, "removed_imports": removed }))
}
