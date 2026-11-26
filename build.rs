// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Builds the particle pack (`packs/particles`) into the program, so a released Landscaper bakes ambience with nothing beside
//! it: a table of the pack's files (`OUT_DIR/particles.rs`), which `src/ambience.rs` reads.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
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
