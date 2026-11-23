// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Try on dev: what Publish sends goes to the review service's `/api/v1/dev/trial`, which puts it on dev with no restart.

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// A name the service takes as a trial's owner (also a folder name on dev): letters, digits, `_` and `-`, up to 64.
fn user_ok(u: &str) -> bool {
    (1..=64).contains(&u.len()) && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// The acting builder, if the service can take the name.
fn me() -> Result<String> {
    let me = crate::workspace::acting_user();
    anyhow::ensure!(user_ok(&me), "your user name {me:?} has characters the dev server can't take: ask an admin");
    Ok(me)
}

/// Whether a form carries the shared texture library: without it the trial's floors show magenta on dev.
fn has_library(fields_files: &(Vec<(&'static str, String)>, Vec<(String, String, Vec<u8>)>)) -> bool {
    fields_files.1.iter().any(|(field, _, data)| field == "library_files" && !data.is_empty())
}

/// Puts `names`' last exports on dev as this builder's trial (replacing their last one). The service's answer, as it is.
pub fn start(names: &[String], note: &str) -> Result<Value> {
    me()?;
    let form = crate::publish::module_form(names, note)?;
    anyhow::ensure!(has_library(&form), "the shared texture library ({}.hak) is missing: export again", crate::shared::HAK);
    crate::publish::post_form("/api/v1/dev/trial", &form, "the dev server's service")
}

/// This builder's trial as `{enabled, trial}` (`trial` null when none); `touch` pushes its expiry back.
pub fn status(touch: bool) -> Result<Value> {
    let me = me()?;
    match crate::review::call("GET", &format!("/api/v1/dev/trial?user={me}&touch={touch}"), None) {
        Ok(all) => Ok(mine(&all, &me)),
        Err(e) if e.downcast_ref::<crate::review::Refused>().is_some_and(|r| r.status == 503 && r.body["available"] == false) => {
            Ok(json!({ "enabled": false, "trial": null, "why": e.to_string() }))
        }
        Err(e) => Err(e),
    }
}

/// `me`'s entry of the service's list of every trial, as [`status`] answers.
fn mine(all: &Value, me: &str) -> Value {
    let mut trial = all["trials"].get(me).cloned().unwrap_or(Value::Null);
    if trial.is_object() {
        let commands: serde_json::Map<String, Value> = trial["akeys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|a| (a.to_string(), json!(format!(";forge {a}"))))
            .collect();
        trial["commands"] = commands.into();
    }
    json!({ "enabled": all["enabled"] != false, "trial": trial })
}

/// Ends this builder's trial: their areas go back to live on dev, then their files leave it.
pub fn release() -> Result<Value> {
    let me = me()?;
    crate::review::call("POST", "/api/v1/dev/trial/release", Some(&json!({ "user": me }))).context("releasing the trial")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_service_takes() {
        for ok in ["alice", "Bob_2", "a-b"] {
            assert!(user_ok(ok), "{ok}");
        }
        for bad in ["", "a.b", "a b", "a@b.c", "a/b", &"x".repeat(65)] {
            assert!(!user_ok(bad), "{bad}");
        }
    }

    #[test]
    fn status_is_the_builders_own_trial_with_its_commands() {
        let all = json!({ "enabled": true, "trials": {
            "alice": { "akeys": ["bay1", "cave2"], "files": 40, "expires_in": 3000 },
            "bob": { "akeys": ["town"], "files": 3, "expires_in": 10 } } });
        let v = mine(&all, "alice");
        assert_eq!(v["enabled"], true);
        assert_eq!(v["trial"]["expires_in"], 3000);
        assert_eq!(v["trial"]["commands"], json!({ "bay1": ";forge bay1", "cave2": ";forge cave2" }));
        assert!(!v.to_string().contains("town"), "another builder's trial isn't shown");
        assert_eq!(mine(&all, "carol"), json!({ "enabled": true, "trial": null }));
    }

    #[test]
    fn a_trial_needs_the_texture_library() {
        let zip = |f: &str, d: &[u8]| (f.to_string(), "x.zip".to_string(), d.to_vec());
        assert!(!has_library(&(vec![], vec![zip("module_bay1", b"z"), zip("hak_bay1", b"z")])));
        assert!(!has_library(&(vec![], vec![zip("library_files", b"")])));
        assert!(has_library(&(vec![], vec![zip("hak_bay1", b"z"), zip("library_files", b"z")])));
    }
}
