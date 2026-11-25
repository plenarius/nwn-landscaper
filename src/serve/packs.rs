// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The texture packs' routes (`/api/textures/packs`): what is installed and what the index offers, installing one (in the
//! background, its progress asked for by the editor) and removing one. On a team server the library is everyone's, so only
//! admins install or remove; on your own machine you are the admin.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tiny_http::Method;

use super::{Answer, Ask, fail, json_response, refuse};
use crate::textures::packs;

/// How long the index is kept before it is read again.
const INDEX_FOR: Duration = Duration::from_secs(300);

/// The install under way or last finished: what the editor's progress bar reads.
#[derive(Clone, Default)]
struct Job {
    id: u64,
    pack: String,
    /// The pack's name, as the index gives it.
    name: String,
    /// `downloading`, `checking`, `installing`, then `done` or `failed`.
    step: &'static str,
    done: u64,
    total: u64,
    error: Option<String>,
    result: Option<Value>,
}

impl Job {
    fn running(&self) -> bool {
        self.id > 0 && !matches!(self.step, "done" | "failed")
    }

    fn json(&self) -> Value {
        json!({
            "job": self.id, "pack": self.pack, "name": self.name, "step": self.step, "done": self.done, "total": self.total,
            "running": self.running(), "error": self.error, "result": self.result,
        })
    }
}

fn job() -> std::sync::MutexGuard<'static, Job> {
    static JOB: Mutex<Job> =
        Mutex::new(Job { id: 0, pack: String::new(), name: String::new(), step: "", done: 0, total: 0, error: None, result: None });
    JOB.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The index as last read (or why it couldn't be), kept [`INDEX_FOR`].
fn index(fresh: bool) -> Result<Vec<Value>, String> {
    type Kept = Option<(Instant, String, Result<Vec<Value>, String>)>;
    static KEPT: Mutex<Kept> = Mutex::new(None);
    let url = packs::index_url();
    if !fresh
        && let Some((at, u, r)) = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref()
        && *u == url
        && at.elapsed() < INDEX_FOR
    {
        return r.clone();
    }
    let r = packs::fetch_index(&url).map_err(|e| crate::workspace::scrub(&format!("{e:#}")));
    *KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((Instant::now(), url, r.clone()));
    r
}

/// Answers the packs' routes; `None` for any other request.
pub(super) fn route(cx: &mut Ask<'_>) -> Option<Answer> {
    let admin = cx.me.admin;
    // (A name that can't be a pack's is refused before the index is read or the library opened.)
    if cx.path.starts_with("/api/textures/packs/") && cx.method == Method::Post && !packs::valid_pack_id(&cx.query.text("id")) {
        return Some(refuse(400, "that is not a pack's id"));
    }
    Some(match (&cx.method, cx.path) {
        (Method::Get, "/api/textures/packs") => {
            let installed = packs::installed();
            let (available, index_error) = match index(cx.query.text("fresh") == "1") {
                Ok(list) => (list, None),
                Err(e) => (Vec::new(), Some(e)),
            };
            let available: Vec<Value> = available
                .into_iter()
                .map(|mut p| {
                    let have = installed.iter().find(|i| i["id"] == p["id"]);
                    p["installed"] = have.and_then(|i| i["version"].as_str()).map(str::to_string).into();
                    // (Where it downloads from is the server's business.)
                    if let Some(o) = p.as_object_mut() {
                        o.remove("url");
                    }
                    p
                })
                .collect();
            json_response(
                200,
                &json!({ "installed": installed, "available": available, "index_error": index_error, "manage": admin, "job": job().json() }),
            )
        }
        (Method::Get, "/api/textures/packs/job") => json_response(200, &job().json()),
        (Method::Post, "/api/textures/packs/install" | "/api/textures/packs/remove") if !admin => {
            refuse(403, "only an admin installs or removes texture packs: the library is the whole team's")
        }
        (Method::Post, "/api/textures/packs/install") => {
            let id = cx.query.text("id");
            let found = index(false).map_err(anyhow::Error::msg).and_then(|i| {
                let name = i.iter().find(|p| p["id"] == id.as_str()).and_then(|p| p["name"].as_str()).unwrap_or_default().to_string();
                packs::find_in_index(&i, &id).map(|f| (f, name))
            });
            let ((source, expect), name) = match found {
                Ok(found) => found,
                Err(e) => return Some(fail(404, &e)),
            };
            let mut j = job();
            if j.running() {
                return Some(refuse(409, &format!("the pack {} is being installed: wait for it to finish", j.pack)));
            }
            *j = Job { id: j.id + 1, pack: id.clone(), name, step: "downloading", ..Job::default() };
            let n = j.id;
            drop(j);
            let (user, is_admin) = (cx.me.user.clone(), cx.me.admin);
            std::thread::spawn(move || {
                // (The audit log names who asked.)
                crate::workspace::set_acting(&user, is_admin);
                let step = |s: &'static str, done: u64, total: u64| {
                    let mut j = job();
                    if j.id == n {
                        (j.step, j.done, j.total) = (s, done, total);
                    }
                };
                let r = packs::install(&source, &expect, &step);
                let mut j = job();
                match r {
                    Ok(r) => {
                        crate::audit::log("texture_pack", None, true, &format!("installed {} {}", r.id, r.version));
                        j.step = "done";
                        j.result = Some(json!({
                            "id": r.id, "name": r.name, "version": r.version, "textures": r.textures, "replaced": r.replaced,
                            "unchanged": r.unchanged,
                        }));
                    }
                    Err(e) => {
                        crate::audit::log("texture_pack", None, false, &format!("installing {id}: {e:#}"));
                        j.step = "failed";
                        j.error = Some(crate::workspace::scrub(&format!("{e:#}")));
                    }
                }
            });
            json_response(202, &job().json())
        }
        (Method::Post, "/api/textures/packs/remove") => {
            let id = cx.query.text("id");
            if job().running() {
                return Some(refuse(409, "a pack is being installed: wait for it to finish"));
            }
            // (An export under way reads the library's pictures: removing waits for it.)
            let _turn = super::job();
            match packs::remove(&id, false) {
                Ok(r) => {
                    crate::audit::log("texture_pack", None, true, &format!("removed {id}"));
                    json_response(200, &json!({ "removed": id, "name": r.name, "textures": r.textures }))
                }
                Err(e) => {
                    crate::audit::log("texture_pack", None, false, &format!("removing {id}: {e:#}"));
                    match e.downcast_ref::<packs::InUse>() {
                        Some(u) => json_response(409, &json!({ "error": crate::workspace::scrub(&u.to_string()), "areas": u.areas })),
                        None => fail(400, &e),
                    }
                }
            }
        }
        _ => return None,
    })
}
