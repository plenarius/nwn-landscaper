// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! nwn-bake lives in its own repository (<https://github.com/plenarius/nwn-bake>) and comes in as a git dependency at a
//! release tag. These checks keep the Landscaper in step with that release:
//!
//! - Moonglow pinned to the same commit by the Landscaper and by nwn-bake (MOONGLOW.md);
//! - the files shared word for word by every repository that follows STYLE.md are the same as nwn-bake's at that tag, and so is
//!   the shared lint block.
//!
//! nwn-bake's files are read from Cargo's git checkout of the locked revision (`cargo metadata` names it), so the checks need
//! no network once the dependencies are fetched, and always compare against exactly the release the Landscaper builds with.

use std::path::{Path, PathBuf};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Moonglow's commits named in a manifest (`moonglow-toolset", rev = "..."`) or a lock file (`moonglow-toolset?rev=...#...`).
fn moonglow_revs(text: &str) -> Vec<String> {
    let mut revs = Vec::new();
    for (i, _) in text.match_indices("moonglow-toolset") {
        let rest = &text[i + "moonglow-toolset".len()..];
        let rest = rest.strip_prefix("\", rev = \"").or_else(|| rest.strip_prefix("?rev="));
        if let Some(rev) = rest {
            revs.push(rev.chars().take_while(char::is_ascii_hexdigit).collect());
        }
    }
    revs
}

/// The top of nwn-bake's repository as Cargo checked it out for the locked revision.
fn nwn_bake_checkout() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root())
        .output()
        .expect("cargo metadata");
    assert!(out.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let pkg = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "nwn-bake")
        .expect("nwn-bake is not among the Landscaper's dependencies");
    let source = pkg["source"].as_str().unwrap_or_default();
    assert!(
        source.starts_with("git+https://github.com/plenarius/nwn-bake?tag=v"),
        "nwn-bake must come from a release tag of its repository, not {source:?}"
    );
    // `<checkout>/nwn-bake/Cargo.toml`: the library sits at the top of nwn-bake's workspace.
    let manifest = PathBuf::from(pkg["manifest_path"].as_str().unwrap());
    let top = manifest.parent().and_then(Path::parent).unwrap().to_path_buf();
    assert!(top.join("STYLE.md").is_file(), "{} does not look like nwn-bake's repository", top.display());
    top
}

#[test]
fn moonglow_is_pinned_alike() {
    let bake = nwn_bake_checkout();
    let mut seen = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (name, path) in [
        ("Cargo.toml", root().join("Cargo.toml")),
        ("Cargo.lock", root().join("Cargo.lock")),
        ("nwn-bake's Cargo.toml", bake.join("Cargo.toml")),
    ] {
        let text = std::fs::read_to_string(&path).unwrap();
        let revs = moonglow_revs(&text);
        assert!(!revs.is_empty(), "{name} names no Moonglow commit");
        for r in revs {
            assert_eq!(r.len(), 40, "{name}: Moonglow's rev must be a whole commit hash, not {r}");
            seen.entry(r).or_default().push(name.to_string());
        }
    }
    assert_eq!(
        seen.len(),
        1,
        "the Landscaper must pin Moonglow to the commit its nwn-bake release pins (MOONGLOW.md); found: {:?}",
        seen.iter()
            .map(|(r, f)| (r, {
                let mut f = f.clone();
                f.dedup();
                f
            }))
            .collect::<Vec<_>>()
    );
}

/// The files STYLE.md ("The checks") says every repository keeps word for word, nwn-bake's copy being the master.
const SHARED: &[&str] = &[
    "STYLE.md",
    "scripts/check.sh",
    ".editorconfig",
    "_typos.toml",
    ".vale.ini",
    ".markdownlint.jsonc",
    "lychee.toml",
    "rustfmt.toml",
    "deny.toml",
    "about.toml",
];

/// Every file under `dir`, relative to `base`, sorted.
fn files(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::path);
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            files(base, &p, out);
        } else {
            out.push(p.strip_prefix(base).unwrap().to_path_buf());
        }
    }
}

/// The `[workspace.lints.*]` tables of a manifest without their comments, `missing_docs` left out (STYLE.md: only nwn-bake
/// has it).
fn lint_block(manifest: &str) -> String {
    let mut block = String::new();
    let mut inside = false;
    for line in manifest.lines() {
        if line.starts_with('[') {
            inside = line.starts_with("[workspace.lints.");
        }
        let line = line.trim_end();
        if inside && !line.is_empty() && !line.starts_with('#') && !line.starts_with("missing_docs") {
            block.push_str(line);
            block.push('\n');
        }
    }
    block.trim_end().to_string()
}

#[test]
fn shared_files_match_nwn_bake() {
    let bake = nwn_bake_checkout();
    let mut names: Vec<PathBuf> = SHARED.iter().map(PathBuf::from).collect();
    let plain = Path::new(".vale/styles/Plain");
    let (mut ours, mut theirs) = (Vec::new(), Vec::new());
    files(root(), &root().join(plain), &mut ours);
    files(&bake, &bake.join(plain), &mut theirs);
    assert_eq!(ours, theirs, "the Vale styles in {} differ from nwn-bake's", plain.display());
    names.extend(ours);
    let mut differ = Vec::new();
    for n in &names {
        let a = std::fs::read(root().join(n)).unwrap_or_else(|e| panic!("{}: {e}", n.display()));
        let b = std::fs::read(bake.join(n)).unwrap_or_else(|e| panic!("nwn-bake's {}: {e}", n.display()));
        if a != b {
            differ.push(n.display().to_string());
        }
    }
    assert!(
        differ.is_empty(),
        "these shared files differ from nwn-bake's at the tag the Landscaper uses ({}); copy nwn-bake's: {differ:?}",
        bake.display()
    );
    let ours = lint_block(&std::fs::read_to_string(root().join("Cargo.toml")).unwrap());
    let theirs = lint_block(&std::fs::read_to_string(bake.join("Cargo.toml")).unwrap());
    assert!(ours.contains("unsafe_code"), "no [workspace.lints] in the Landscaper's Cargo.toml");
    assert_eq!(ours, theirs, "the [workspace.lints] block differs from nwn-bake's (STYLE.md, \"Lints\")");
}
