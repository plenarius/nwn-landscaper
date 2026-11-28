// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds the particle pack (`packs/particles`) into the program, so a released Landscaper bakes ambience with nothing beside
//! it: a table of the pack's files (`OUT_DIR/particles.rs`), which `src/ambience.rs` reads. And counts the changelog entries
//! waiting in changelog.d/ for the next release (scripts/changelog.sh), for the engine's version (src/version.rs): a build
//! with any says `+dev`. And digests the source, for the files kept between exports (src/generator/painter/kept.rs).

use std::fmt::Write as _;
use std::hash::Hasher as _;
use std::path::{Path, PathBuf};

fn main() {
    changes_waiting();
    source_digest();
    let root = Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join("packs").join("particles");
    // (A folder: cargo looks at everything in it.)
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    for sub in ["", "models", "textures"] {
        let Ok(entries) = std::fs::read_dir(root.join(sub)) else { continue };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(ext.as_str(), "json" | "mdl" | "tga") {
                let rel = if sub.is_empty() { name.to_string() } else { format!("{sub}/{name}") };
                files.push((rel, path));
            }
        }
    }
    files.sort();
    let mut out = String::from(
        "/// The particle pack's files: each one's path in the pack, and its contents.\npub(super) const FILES: &[(&str, &[u8])] = &[\n",
    );
    for (rel, path) in &files {
        let _ = writeln!(out, "    ({rel:?}, include_bytes!({:?})),", path.display().to_string());
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("particles.rs");
    std::fs::write(dest, out).expect("writing the particle pack's table");
}

/// `LANDSCAPER_CHANGES_WAITING`: how many changelog entries changelog.d/ holds (the same rule as `version::fragments`: its
/// `.md` files, not its dot files such as the folder's markdownlint settings).
fn changes_waiting() {
    let dir = Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join("changelog.d");
    println!("cargo:rerun-if-changed={}", dir.display());
    let fragment = |n: &str| !n.starts_with('.') && Path::new(n).extension().is_some_and(|x| x == "md");
    let waiting = std::fs::read_dir(&dir).map_or(0, |d| d.flatten().filter(|e| e.file_name().to_str().is_some_and(fragment)).count());
    println!("cargo:rustc-env=LANDSCAPER_CHANGES_WAITING={waiting}");
}

/// `LANDSCAPER_SOURCE_DIGEST`: a digest of `src/`, the locked dependencies, the manifest, the target and the features, so a
/// cached bake from any other code is never read. (A release's version alone would let a build between releases read a bake
/// made by other code.)
fn source_digest() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    let mut dirs = vec![root.join("src")];
    while let Some(dir) = dirs.pop() {
        for path in std::fs::read_dir(&dir).expect("reading src/").flatten().map(|e| e.path()) {
            if path.is_dir() { dirs.push(path) } else { files.push(path) }
        }
    }
    files.sort();
    files.extend([root.join("Cargo.lock"), root.join("Cargo.toml")]);
    // (SipHash with fixed keys: the same digest of the same source on every build.)
    let mut h = std::hash::DefaultHasher::new();
    for path in &files {
        h.write(path.strip_prefix(&root).unwrap_or(path).to_string_lossy().as_bytes());
        h.write(&std::fs::read(path).unwrap_or_default());
    }
    let mut features: Vec<String> = std::env::vars().map(|(k, _)| k).filter(|k| k.starts_with("CARGO_FEATURE_")).collect();
    features.sort();
    features.push(std::env::var("TARGET").unwrap_or_default());
    for v in &features {
        h.write(v.as_bytes());
    }
    // (A folder: cargo looks at everything in it.)
    for watched in ["src", "Cargo.lock", "Cargo.toml"] {
        println!("cargo:rerun-if-changed={}", root.join(watched).display());
    }
    println!("cargo:rustc-env=LANDSCAPER_SOURCE_DIGEST={:016x}", h.finish());
}
