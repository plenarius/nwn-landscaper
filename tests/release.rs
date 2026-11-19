// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A release's two halves agree: the engine and the editor (landscaper-web) are tagged with the same `vX.Y.Z`, and the
//! self-host image builds the editor of the engine's own version by default (README, "Releases").

use std::path::Path;

/// `name = "value"` in the TOML table `table` of `text` (the first such line after the table's header).
fn table_value(text: &str, table: &str, name: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == format!("[{table}]");
        } else if inside && let Some(rest) = line.strip_prefix(name).map(str::trim_start).and_then(|r| r.strip_prefix('=')) {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// The default of `ARG <name>=...` in a Dockerfile.
fn arg_default(dockerfile: &str, name: &str) -> Option<String> {
    dockerfile.lines().find_map(|l| l.trim().strip_prefix(&format!("ARG {name}="))).map(|v| v.trim().trim_matches('"').to_string())
}

#[test]
fn the_self_host_image_builds_the_editor_of_the_engines_own_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let version =
        table_value(&std::fs::read_to_string(root.join("Cargo.toml")).unwrap(), "package", "version").expect("Cargo.toml's version");
    let dockerfile = std::fs::read_to_string(root.join("deploy/self-host/Dockerfile")).expect("deploy/self-host/Dockerfile");
    let web_ref = arg_default(&dockerfile, "WEB_REF").expect("the Dockerfile has no ARG WEB_REF=... default");
    assert_eq!(
        web_ref,
        format!("v{version}"),
        "deploy/self-host/Dockerfile builds the editor at {web_ref}, but the engine is {version}: set WEB_REF to v{version} \
         (and tag both repositories v{version} when releasing)"
    );
}

#[test]
fn the_readers_find_what_they_look_for() {
    let toml = "[workspace]\nversion = \"9\"\n\n[package]\nname = \"x\"\nversion = \"1.2.3\"\n[dependencies]\nversion = \"0\"\n";
    assert_eq!(table_value(toml, "package", "version").as_deref(), Some("1.2.3"));
    assert_eq!(table_value(toml, "lib", "version"), None);
    assert_eq!(arg_default("FROM x\nARG WEB_REPO=r\nARG WEB_REF=v1.2.3\n", "WEB_REF").as_deref(), Some("v1.2.3"));
    assert_eq!(arg_default("ARG WEB_REF_X=y\n", "WEB_REF"), None);
}
