// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The debugging switches: environment variables named `LS_*` that the research behind the Landscaper used, and still uses, to
//! turn a step off, print what a step did or try a variant. None is needed to use the Landscaper. Some change what an export
//! writes, so the program says at start which are set ([`describe_set`]); docs/DEBUGGING.md lists them all, from [`SWITCHES`].
//! A test checks that every `LS_*` name the code reads is in the table and in docs/DEBUGGING.md.

/// What setting a switch does to an export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The exported area or its files differ.
    ChangesExport,
    /// Prints more, writes files beside the export, or only changes a research command: the export is the same.
    Report,
}

/// Every switch: its name, what it does, and its [`Effect`]. In the order docs/DEBUGGING.md lists them.
pub const SWITCHES: &[(&str, &str, Effect)] = {
    use Effect::{ChangesExport as X, Report as R};
    &[
        // The test terrain (research commands, and an export with no heightmap).
        ("LS_HEIGHTMAP", "a heightmap file (the editor's JSON) the research commands work on, instead of the built-in test terrain", R),
        ("LS_CLIFF", "the test terrain's cliff height in metres", X),
        ("LS_FLAT", "the test terrain without height: its cliff becomes a strip of non-walkable ground", X),
        ("LS_RUGGED", "the rugged test terrain: terraced noise over 8 x 8 tiles", X),
        ("LS_CONTROL", "`generate` adds a control area made of the game's own rural tiles (ttr01)", X),
        // Walkmesh and path nodes.
        ("LS_SAMPLES", "height samples along a tile's side for its walkmesh (2^k + 1; default 17)", X),
        ("LS_WALK_ERROR", "how far the walkmesh may stray from the terrain, in metres", X),
        ("LS_NOLEVEL", "no tile height levels: every tile at level 0", X),
        ("LS_NO_OPENING", "keep thin walkable slivers that the walkmesh normally closes", X),
        ("LS_OLD_SEAMS", "the older way of matching tile edges and choosing path nodes", X),
        ("LS_UNTANGLE", "an experimental pass that untangles path node choices (slow)", X),
        ("LS_NO_PRUNE", "keep walkable faces that no path node layout reaches", X),
        ("LS_KEEP_UNREPRESENTED", "keep walkable faces at tile edges that the chosen path node layouts don't represent", X),
        ("LS_KEEP_HOLES", "keep small holes in tile walkmeshes instead of filling them", X),
        ("LS_UPPER", "choose only from the upper case path node layouts", X),
        ("LS_LOOSE_EXITS", "match tile exits to walkable regions loosely", X),
        ("LS_IGNORE_POS", "accept a path node layout even when a node lies off its region", X),
        ("LS_NO_SPREAD", "score the path node layouts without the penalty for crossings that break", X),
        ("LS_NO_UNBREAK", "skip the pass that removes crossings that break", X),
        ("LS_NO_SURFACES", "walkmesh surfaces as grass and non-walkable only, not the painted surfaces", X),
        // The tiles' files.
        ("LS_BAKE", "pixels a side of each tile's baked texture (a power of two; default 512)", X),
        ("LS_SHADOW", "how dark baked shadows are, 0 to 1, over the editor's setting", X),
        ("LS_SHADOW_DEBUG", "`half` or `corner`: test patterns in the shadow maps, to see how the game lays them", X),
        ("LS_MINIMAP", "`terrain`: the minimaps from the terrain alone, not the area drawn from above with what stands on it", X),
        ("LS_REUSE", "every tile uses one of the game's models and minimaps (ttr01_o01_01): to test the tileset alone", X),
        ("LS_NO_EDGE", "no scenery beyond the area's edge", X),
        ("LS_NO_ORIGINAL", "an imported area without the original's identity, scripts and objects: to test pathing alone", X),
        ("LS_ONLOAD", "the test module runs its route test from OnModuleLoad (for tracing under a debugger)", X),
        ("LS_CASE", "with LS_ONLOAD: the one route the test runs (default `detour`)", X),
        ("LS_DUMP", "also write each tile's baked texture as a PNG and each tile's model as text (`.mdl.txt`) beside the export", R),
        // Imports.
        ("LS_IMPORT_TIMES", "print how long each stage of an import took", R),
        ("LS_IMPORT_WATER", "print the water faces an import finds", R),
        ("LS_IMPORT_MATS", "print the walkable area of each surface an import finds", R),
        // Printing what a step did.
        ("LS_TIMING", "print how long each step of an export took", R),
        ("LS_DEBUG_PLAN", "print the path node plan, round by round", R),
        ("LS_DEBUG_SEAM", "`a,b`: print the walkmesh faces of tiles a and b along their shared edge", R),
        ("LS_DEBUG_TILE", "a tile number: print its path node regions", R),
        ("LS_DEBUG_BREAKS", "print the crossings that break, before and after they are removed", R),
        ("LS_DEBUG_ROUTES", "`export` and `check`: print each route the report tries", R),
        ("LS_CHECK_MAP", "`export` and `check`, `x0,y0,x1,y1`: print the ground in that box every 0.5 m (walkable, joined, lost)", R),
        ("LS_CHECK_PAIR", "`export` and `check`, `x0,y0,x1,y1`: print which rules join the two points", R),
        // The route simulator and the oracle (research commands only).
        ("LS_QUIET", "don't print the path nodes each plan chose (less printed)", R),
        ("LS_SIM_ALL", "`sim-check`: print every route, not only the ones that differ", R),
        ("LS_SIM_DEBUG", "the simulator prints its in-tile search", R),
        ("LS_SIM_SUB", "the simulator smooths the sub-tile path first", R),
        ("LS_SIM_LINE_US", "the simulator's cost of a line test, in microseconds (default 0.7)", R),
        ("LS_SIM_GRID_US", "the simulator's cost of a grid line test, in microseconds (default 0.7)", R),
        ("LS_VARIANTS", "`sim-predict`: only these terrain variants, comma separated (cliff3.0 ... cliff6.0, flat, rugged)", R),
        ("LS_VERBOSE", "`sim-predict`: print every route", R),
        ("LS_PAIRS", "`oracle`, `x0,y0,x1,y1;...`: these routes instead of random ones", R),
        ("LS_ORACLE_SECONDS", "`oracle`: how long the server walks each route (default 40)", R),
        ("LS_CW", "`validate-pathnodes`: read tile orientations clockwise", R),
        ("LS_ROTATED", "`validate-pathnodes`: only tiles turned a quarter or three quarters", R),
    ]
};

/// What to say at start about the `LS_*` variables in `set`: one line each, the ones that change an export first.
pub fn describe_set(set: &[String]) -> Vec<String> {
    let mut changes = Vec::new();
    let mut other = Vec::new();
    for name in set {
        match SWITCHES.iter().find(|(n, _, _)| n == name) {
            Some((n, what, Effect::ChangesExport)) => changes.push(format!("debugging switch {n} is set: {what} (exports differ)")),
            Some((n, what, Effect::Report)) => other.push(format!("debugging switch {n} is set: {what}")),
            None => other.push(format!("{name} is set, but the Landscaper has no such switch (docs/DEBUGGING.md lists them)")),
        }
    }
    changes.sort();
    other.sort();
    changes.extend(other);
    changes
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    /// Every `"LS_..."` name in the Rust files under `dir`.
    fn read_by_code(dir: &Path, out: &mut BTreeSet<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                read_by_code(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") && p.file_name().is_some_and(|f| f != "switches.rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                for piece in text.split("\"LS_").skip(1) {
                    let name: String = piece.chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
                    if !name.is_empty() && piece[name.len()..].starts_with('"') {
                        out.insert(format!("LS_{name}"));
                    }
                }
            }
        }
    }

    #[test]
    fn every_switch_the_code_reads_is_listed_here_and_in_the_debugging_document() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut read = BTreeSet::new();
        read_by_code(&root.join("src"), &mut read);
        let listed: BTreeSet<String> = SWITCHES.iter().map(|(n, _, _)| (*n).to_string()).collect();
        let missing: Vec<_> = read.difference(&listed).collect();
        assert!(missing.is_empty(), "switches the code reads that are not in SWITCHES: {missing:?}");
        let unused: Vec<_> = listed.difference(&read).collect();
        assert!(unused.is_empty(), "switches in SWITCHES that no code reads: {unused:?}");
        let doc = std::fs::read_to_string(root.join("docs/DEBUGGING.md")).unwrap();
        for (n, _, _) in SWITCHES {
            assert!(doc.contains(&format!("`{n}`")), "docs/DEBUGGING.md doesn't list {n}");
        }
    }

    #[test]
    fn the_start_names_the_switches_that_change_an_export_first() {
        let lines = describe_set(&["LS_TIMING".into(), "LS_BAKE".into(), "LS_NOPE".into()]);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains("LS_BAKE") && lines[0].contains("exports differ"));
        assert!(lines.iter().any(|l| l.contains("LS_NOPE") && l.contains("no such switch")));
    }
}
