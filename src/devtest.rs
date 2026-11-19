// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Testing a builder's module on the dev server: the team server checks who is asking and that the module is
//! theirs and exported, and passes it to the review service, which keeps the claim (one test at a time) and starts the
//! load (`/api/v1/dev/test`).

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// A name that can go to the service and on to a workflow input: letters, digits, `_ . -`, up to 64.
fn name_ok(n: &str) -> bool {
    (1..=64).contains(&n.len()) && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The dev server's state: who holds it, how long they have, what is running.
pub fn status(touch: bool) -> Result<Value> {
    let me = crate::workspace::acting_user();
    crate::review::call("GET", &format!("/api/v1/dev/test?user={me}&touch={touch}"), None)
}

/// An existing area's waypoints (tags and places), to choose where the test enters from.
pub fn waypoints(area: &str) -> Result<Value> {
    anyhow::ensure!(name_ok(area), "which area?");
    crate::review::call("GET", &format!("/api/v1/areas/{}/waypoints", area.to_ascii_lowercase()), None)
}

/// The areas of the builder's exported module `module`: their resrefs in the installed module.
fn module_areas(module: &str) -> Result<Vec<String>> {
    let path = crate::workspace::nwn_user_dir().context("no module folder")?.join("modules").join(format!("{module}.mod"));
    anyhow::ensure!(path.is_file(), "module {module} hasn't been exported yet: export it first");
    let m = mg_module::Module::open(&path)?;
    Ok(m.areas()?.iter().map(|a| a.to_string().to_ascii_lowercase()).collect())
}

/// Puts `module` (its last export) on the dev server, entering from `area`'s waypoint, leading to `entry`.
pub fn start(module: &str, area: &str, waypoint: &str, entry: &str, takeover: bool) -> Result<Value> {
    let me = crate::workspace::acting_user();
    anyhow::ensure!(
        name_ok(module) && name_ok(area) && name_ok(waypoint) && (entry.is_empty() || name_ok(entry)),
        "a name here has characters I can't pass on"
    );
    anyhow::ensure!(!takeover || crate::workspace::acting_admin(), "only an admin can take the dev server from someone");
    let areas = module_areas(module)?;
    anyhow::ensure!(!areas.is_empty(), "module {module} has no area");
    anyhow::ensure!(entry.is_empty() || areas.iter().any(|a| a == &entry.to_ascii_lowercase()), "module {module} has no area {entry}");
    crate::review::call(
        "POST",
        "/api/v1/dev/test",
        Some(&json!({ "user": me, "module": module, "area": area, "waypoint": waypoint, "entry": entry, "takeover": takeover })),
    )
}

/// Gives the dev server back (the holder, or an admin for anyone's).
pub fn release() -> Result<Value> {
    let me = crate::workspace::acting_user();
    crate::review::call("POST", "/api/v1/dev/test/release", Some(&json!({ "user": me, "admin": crate::workspace::acting_admin() })))
        .context("releasing the dev server")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_may_be_passed_on() {
        for ok in ["bay1", "WP_gate", "a-b.c"] {
            assert!(name_ok(ok), "{ok}");
        }
        for bad in ["", ".x", "a b", "a/b", "a;b", &"x".repeat(65)] {
            assert!(!name_ok(bad), "{bad}");
        }
    }
}
