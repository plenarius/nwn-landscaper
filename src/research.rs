// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The research commands (`landscaper --help` lists them last): tools for working on the Landscaper itself. They check the
//! path node table against the game's tilesets, run routes on a real server (the oracle) and compare the route planner's
//! simulator with what the server did. They print what they find.

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_resman::ResKey;
use mg_set::Tileset;

use crate::generator::{
    CLIFF_OVERRIDE, FLAT_OVERRIDE, MAT_NONWALK, RUGGED_OVERRIDE, TILE, WALKABLE_NZ, area_plan, blocked, describe, generate, hak_name,
    height, sim_area, tiles_x, tiles_y, walkmesh,
};
use crate::{models, oracle, pathnode, seams, sim};

/// Checks the PathNode matcher against a BioWare tileset: each tile's own (letter, orientation) should be one its walkmesh allows.
/// Tried under a few coordinate frames, to confirm which one the layouts use.
pub(crate) fn validate_pathnodes(set_name: &str) -> Result<()> {
    let game = models::open_game(nwn_bake::game::Haks::None, nwn_bake::game::Tlk::None)?;
    let set_data = game.resman.get(&ResKey { resref: ResRef::from_str(set_name)?, restype: ResType::SET })?;
    let set = Tileset::parse(&set_data, game.codepage())?;
    let surf = game.table("surfacemat")?;
    let walk_col = surf.column("Walk").context("Walk column")?;
    let walkable = |m: u32| surf.cell(m as usize, walk_col) == Some("1");
    let layouts = pathnode::layouts();
    let points = pathnode::edge_points(layouts);
    let frames: [(&str, fn(f32, f32) -> (f32, f32)); 4] = [
        ("x east, y north", |x, y| (x, y)),
        ("y flipped", |x, y| (x, 10.0 - y)),
        ("x flipped", |x, y| (10.0 - x, y)),
        ("x/y swapped", |x, y| (y, x)),
    ];
    let mut ok = [0usize; 4];
    let mut checked = 0;
    let mut misses: Vec<String> = Vec::new();
    for t in &set.tiles {
        let Some(letter) = t.path_node.as_ref().and_then(|p| p.chars().next()) else { continue };
        let Ok(data) = game.resman.get_named(&t.model, ResType::WOK) else { continue };
        let wm = mg_mdl::walkmesh::Walkmesh::read(&data, mg_mdl::walkmesh::WalkmeshKind::Tile)?;
        // .set orientations are degrees; layouts count quarter turns.
        let quarter = (t.orientation.rem_euclid(360) / 90) as u8;
        let turn = if std::env::var_os("LS_CW").is_some() { (4 - quarter) % 4 } else { quarter };
        if std::env::var_os("LS_ROTATED").is_some() && (quarter == 0 || quarter == 2) {
            continue;
        }
        let Some(layout) = layouts.iter().find(|l| l.letter == letter && l.orientation == turn) else { continue };
        let mut verts = Vec::new();
        let mut faces = Vec::new();
        for sface in wm.surfaces_for(None) {
            let Some(mesh) = wm.model.nodes[sface.node].mesh() else { continue };
            let base = verts.len() as u32;
            verts.extend(mesh.vertices.iter().copied());
            faces.extend(mesh.faces.iter().map(|f| (f.vertices.map(|v| v + base), walkable(f.material))));
        }
        if faces.is_empty() {
            continue;
        }
        checked += 1;
        for (i, (_, frame)) in frames.iter().enumerate() {
            let vs: Vec<[f32; 3]> = verts
                .iter()
                .map(|v| {
                    let (x, y) = frame(v[0] + 5.0, v[1] + 5.0);
                    [x, y, v[2]]
                })
                .collect();
            let regions = pathnode::Regions::new(&vs, &faces, &points);
            if regions.allows(layout) {
                ok[i] += 1;
            } else if i == 0 && misses.len() < 8 {
                misses.push(format!("{}={}{}", t.model, letter, t.orientation));
            }
        }
    }
    println!("{set_name}: {checked} tiles with walkmeshes");
    for (i, (name, _)) in frames.iter().enumerate() {
        println!("  {name:16} {:4} consistent ({:.0}%)", ok[i], 100.0 * ok[i] as f32 / checked as f32);
    }
    println!("  inconsistent under the first frame, e.g.: {}", misses.join(" "));
    Ok(())
}

/// Generates the area with `n` random start/goal pairs, walks a creature
/// between each in the server, and compares with true reachability.
pub(crate) fn oracle_run(n: usize, seed: u64, out: &std::path::Path) -> Result<()> {
    let (aw, ah) = (tiles_x() as f32 * TILE, tiles_y() as f32 * TILE);
    let max_slope = (1.0 - WALKABLE_NZ * WALKABLE_NZ).sqrt() / WALKABLE_NZ;
    let reach = oracle::Reach::new(aw, ah, 0.5, max_slope, height, blocked);
    // LS_PAIRS="x0,y0,x1,y1;...": these routes instead of random ones.
    let pairs = match std::env::var("LS_PAIRS") {
        Ok(text) => text
            .split(';')
            .filter(|p| !p.trim().is_empty())
            .map(|p| {
                let v: Vec<f32> = p.split(',').map(|x| x.trim().parse().context("LS_PAIRS")).collect::<Result<_>>()?;
                anyhow::ensure!(v.len() == 4, "LS_PAIRS: x0,y0,x1,y1 each");
                Ok(oracle::Pair { from: (v[0], v[1]), to: (v[2], v[3]), reachable: true })
            })
            .collect::<Result<Vec<_>>>()?,
        Err(_) => oracle::pairs(&reach, aw, ah, TILE, n, seed),
    };
    // Long enough for a route across an 8 x 8 area (LS_ORACLE_SECONDS for big mazes).
    let seconds: f32 = std::env::var("LS_ORACLE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(40.0);
    std::fs::create_dir_all(out)?;
    generate(out, Some(&oracle::script(&pairs, seconds)))?;
    // Moonglow's `mg` runs the server: MG names it, else the one on the PATH.
    let mg = std::env::var("MG").unwrap_or_else(|_| "mg".to_string());
    let run = std::process::Command::new(&mg)
        .args(["test", &format!("{}.mod", hak_name()), "--settle", &format!("{}", seconds + 3.0), "--timeout", "200", "--log"])
        .current_dir(out)
        .output()
        .with_context(|| format!("can't run Moonglow's `{mg}` (set MG to its path)"))?;
    let log = String::from_utf8_lossy(&run.stderr);
    std::fs::write(out.join("server.log"), log.as_bytes())?;
    let results = oracle::results(&log, pairs.len());
    let (mut tp, mut fneg, mut fpos, mut tn, mut missing) = (0, 0, 0, 0, 0);
    let mut rows = Vec::new();
    for (i, (p, r)) in pairs.iter().zip(&results).enumerate() {
        let Some(end) = r else {
            missing += 1;
            continue;
        };
        let arrived = ((end.0 - p.to.0).powi(2) + (end.1 - p.to.1).powi(2)).sqrt() < 1.5;
        match (p.reachable, arrived) {
            (true, true) => tp += 1,
            (true, false) => {
                fneg += 1;
                println!(
                    "  #{i}: ({:.1},{:.1}) -> ({:.1},{:.1}) reachable, stopped at ({:.1},{:.1})",
                    p.from.0, p.from.1, p.to.0, p.to.1, end.0, end.1
                );
            }
            (false, true) => {
                fpos += 1;
                println!("  #{i}: ({:.1},{:.1}) -> ({:.1},{:.1}) NOT reachable, yet arrived", p.from.0, p.from.1, p.to.0, p.to.1);
            }
            (false, false) => tn += 1,
        }
        rows.push(serde_json::json!({
            "from": [p.from.0, p.from.1], "to": [p.to.0, p.to.1], "reachable": p.reachable, "end": [end.0, end.1], "arrived": arrived,
        }));
    }
    std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&rows)?)?;
    println!(
        "oracle seed {seed}: {n} pairs: reachable+arrived {tp}, reachable+stuck {fneg}, unreachable+arrived {fpos}, \
         unreachable+stuck {tn}, no result {missing}"
    );
    Ok(())
}

/// Simulates one walk, from (`v[0]`, `v[1]`) to (`v[2]`, `v[3]`) on the built-in test terrain, and prints its trace.
pub(crate) fn sim_one(v: [f32; 4]) -> Result<()> {
    let area = sim_area();
    let q = sim::Query::new((v[0], v[1]), (v[2], v[3]));
    let r = sim::area::simulate(&area, &q, true);
    for line in &r.trace {
        println!("{line}");
    }
    println!("direct line from the start: {}", r.direct_line);
    println!("outcome: {}", describe(&r));
    let w: Vec<String> = r.waypoints.iter().map(|p| format!("({:.2},{:.2})", p.0, p.1)).collect();
    println!("waypoints: {}", w.join(" "));
    println!("start: ({:.2},{:.2})", r.start.0, r.start.1);
    println!(
        "end: ({:.2},{:.2}); {} intra-tile calls, {} work units, {} line tests, {} replans",
        r.end.0, r.end.1, r.stats.intra_calls, r.stats.intra_used, r.stats.intra_lines, r.stats.replans
    );
    Ok(())
}

/// Runs the simulator on every pair of an oracle results file and compares with what the server did. The area is regenerated from the
/// current code; a file name containing `cliff<h>` or `flat` selects that variant (each file on its own: nothing carries over between
/// files).
pub(crate) fn sim_check(path: &str) -> Result<()> {
    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("");
    // Each file selects its own variant; nothing carries over to the next.
    use std::sync::atomic::Ordering::Relaxed;
    // `cliff<h>` or `flat` anywhere in the name (e.g. holdout-cliff5.0-seed8.json).
    let cliff = name.find("cliff").and_then(|i| name[i + 5..].split('-').next()).and_then(|h| h.parse::<f32>().ok());
    CLIFF_OVERRIDE.store(cliff.map_or(0, f32::to_bits), Relaxed);
    FLAT_OVERRIDE.store(if name.contains("flat") { 1 } else { 2 }, Relaxed);
    RUGGED_OVERRIDE.store(if name.contains("rugged") { 1 } else { 2 }, Relaxed);
    if cliff.is_none() && !name.contains("flat") && !name.contains("rugged") {
        eprintln!("{name}: no cliff<h>, flat or rugged in the name; using the current configuration");
    }
    let rows: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let area = sim_area();
    let pt = |v: &serde_json::Value| -> Result<(f32, f32)> {
        let n = |k: usize| v[k].as_f64().map(|x| x as f32).with_context(|| format!("{path}: a point that isn't [x, y]: {v}"));
        Ok((n(0)?, n(1)?))
    };
    let (mut agree, mut total, mut close) = (0, 0, 0);
    let (mut fail_server, mut fail_both) = (0, 0);
    for (i, row) in rows.iter().enumerate() {
        let (from, to, end) = (pt(&row["from"])?, pt(&row["to"])?, pt(&row["end"])?);
        let arrived = row["arrived"].as_bool().unwrap_or(false);
        let r = sim::area::simulate(&area, &sim::Query::new(from, to), false);
        let off: Vec<&str> = [(from, "start"), (to, "goal")].iter().filter(|(p, _)| !area.on_walkmesh(p.0, p.1)).map(|(_, n)| *n).collect();
        let note = if off.is_empty() { String::new() } else { format!(" [{} off the walkmesh]", off.join(" and ")) };
        total += 1;
        let predicted = ((r.end.0 - to.0).powi(2) + (r.end.1 - to.1).powi(2)).sqrt() < 1.5;
        if !arrived {
            fail_server += 1;
        }
        let miss = ((r.end.0 - end.0).powi(2) + (r.end.1 - end.1).powi(2)).sqrt();
        if miss <= 0.5 {
            close += 1;
        }
        let pair = format!("#{i}: ({:.1},{:.1}) -> ({:.1},{:.1})", from.0, from.1, to.0, to.1);
        if std::env::var_os("LS_SIM_ALL").is_some() {
            println!(
                "  ALL {pair}: {} units {} calls {} replans {} grid {} | server {} ({:.2},{:.2}) sim ({:.2},{:.2})",
                describe(&r),
                r.stats.intra_used,
                r.stats.intra_calls,
                r.stats.replans,
                r.stats.grid_lines,
                if arrived { "arrived" } else { "stopped" },
                end.0,
                end.1,
                r.end.0,
                r.end.1
            );
        }
        if predicted == arrived {
            agree += 1;
            if !arrived {
                fail_both += 1;
            }
            if miss > 0.5 {
                println!(
                    "  {pair}: agree ({}), but sim ends ({:.2},{:.2}), server ({:.2},{:.2}){note}",
                    describe(&r),
                    r.end.0,
                    r.end.1,
                    end.0,
                    end.1
                );
            }
        } else {
            println!(
                "  {pair}: DISAGREE sim {} ends ({:.2},{:.2}); server {} at ({:.2},{:.2}){note}",
                describe(&r),
                r.end.0,
                r.end.1,
                if arrived { "arrived" } else { "stopped" },
                end.0,
                end.1
            );
        }
    }
    println!(
        "{name}: arrival agrees on {agree}/{total}; server failures reproduced {fail_both}/{fail_server}; end point within 0.5 m \
         on {close}/{total}"
    );
    Ok(())
}

/// Simulator-only check of the current generator: for each terrain variant and seed, the oracle's random pairs, and how many reachable
/// ones the simulated server walks to the end. No server needed.
pub(crate) fn sim_predict(n: usize, seeds: std::ops::RangeInclusive<u64>, verbose: bool) -> Result<()> {
    use std::sync::atomic::Ordering::Relaxed;
    let max_slope = (1.0 - WALKABLE_NZ * WALKABLE_NZ).sqrt() / WALKABLE_NZ;
    let only = std::env::var("LS_VARIANTS").ok();
    let variants: Vec<(String, Option<f32>)> = [3.0f32, 3.5, 4.0, 5.0, 6.0]
        .iter()
        .map(|h| (format!("cliff{h:.1}"), Some(*h)))
        .chain([("flat".to_string(), None), ("rugged".to_string(), None)])
        .filter(|(n, _)| only.as_ref().is_none_or(|o| o.split(',').any(|v| v == n)))
        .collect();
    let (mut all_ok, mut all_n) = (0, 0);
    for (name, h) in &variants {
        CLIFF_OVERRIDE.store(h.map_or(0, f32::to_bits), Relaxed);
        FLAT_OVERRIDE.store(if name == "flat" { 1 } else { 2 }, Relaxed);
        RUGGED_OVERRIDE.store(if name == "rugged" { 1 } else { 2 }, Relaxed);
        let (aw, ah) = (tiles_x() as f32 * TILE, tiles_y() as f32 * TILE);
        let area = sim_area();
        let reach = oracle::Reach::new(aw, ah, 0.5, max_slope, height, blocked);
        let (mut ok, mut reachable, mut faces) = (0, 0, 0);
        let mut calls: Vec<i32> = Vec::new();
        for ty in 0..tiles_y() {
            for tx in 0..tiles_x() {
                faces = faces.max(walkmesh(tx, ty).faces.len());
            }
        }
        // Truth from the walkmeshes the server gets (pruning can remove ground).
        let meshes: Vec<seams::TileMesh> = (0..tiles_x() * tiles_y())
            .map(|i| {
                let w = walkmesh(i % tiles_x(), i / tiles_x());
                seams::TileMesh {
                    locked: Vec::new(),
                    origin: ((i % tiles_x()) as f32 * TILE + TILE / 2.0, (i / tiles_x()) as f32 * TILE + TILE / 2.0),
                    verts: w.verts,
                    faces: w.faces.iter().map(|(f, m)| (*f, *m != MAT_NONWALK)).collect(),
                }
            })
            .collect();
        let conn = seams::Connectivity::new(&meshes, TILE);
        // Walkable coverage: of the truly walkable ground (0.5 m grid), how much lies on walkable walkmesh faces.
        let (mut truly, mut covered) = (0usize, 0usize);
        for j in 0..reach.h {
            for i in 0..reach.w {
                if reach.comp[j * reach.w + i] >= 0 {
                    let (x, y) = ((i as f32 * reach.step).clamp(0.01, aw - 0.01), (j as f32 * reach.step).clamp(0.01, ah - 0.01));
                    truly += 1;
                    if conn.component(x, y).is_some() {
                        covered += 1;
                    }
                }
            }
        }
        let mut lost = 0;
        for seed in seeds.clone() {
            for p in oracle::pairs(&reach, aw, ah, TILE, n, seed) {
                if !p.reachable {
                    continue;
                }
                let (a, b) = (conn.component(p.from.0, p.from.1), conn.component(p.to.0, p.to.1));
                if a.is_none() || a != b {
                    lost += 1; // no longer connected on the walkmesh
                    if verbose {
                        let why = |c: Option<usize>| if c.is_none() { "off the walkmesh" } else { "on it" };
                        println!(
                            "  {name} seed {seed}: LOST ({:.1},{:.1}) {} -> ({:.1},{:.1}) {}",
                            p.from.0,
                            p.from.1,
                            why(a),
                            p.to.0,
                            p.to.1,
                            why(b)
                        );
                    }
                    continue;
                }
                reachable += 1;
                let r = sim::area::simulate(&area, &sim::Query::new(p.from, p.to), false);
                calls.push(r.stats.calls);
                if ((r.end.0 - p.to.0).powi(2) + (r.end.1 - p.to.1).powi(2)).sqrt() < 1.5 {
                    ok += 1;
                } else if verbose {
                    println!(
                        "  {name} seed {seed}: ({:.1},{:.1}) -> ({:.1},{:.1}) {} ends ({:.1},{:.1})",
                        p.from.0,
                        p.from.1,
                        p.to.0,
                        p.to.1,
                        describe(&r),
                        r.end.0,
                        r.end.1
                    );
                }
            }
        }
        calls.sort_unstable();
        let pct = |q: f32| calls.get(((calls.len().max(1) - 1) as f32 * q) as usize).copied().unwrap_or(0);
        println!(
            "{name:9} routes {ok}/{reachable}  coverage {:.1}%  faces <= {faces}/tile  calls p50 {} p95 {} max {} of 100  \
             ({lost} off/disconnected)",
            100.0 * covered as f32 / truly.max(1) as f32,
            pct(0.5),
            pct(0.95),
            pct(1.0)
        );
        all_ok += ok;
        all_n += reachable;
    }
    CLIFF_OVERRIDE.store(0, Relaxed);
    FLAT_OVERRIDE.store(0, Relaxed);
    RUGGED_OVERRIDE.store(0, Relaxed);
    println!("total {all_ok}/{all_n}");
    Ok(())
}

/// Export-time check of the current area: for each tile, the simulated cost of crossing it between every pair of its exits in the same
/// region (the engine's in-tile search explodes on U-shaped regions), worst first.
pub(crate) fn check_tiles(limit_units: i64) -> Result<()> {
    let area = sim_area();
    let nodes = area_plan().1.clone();
    let layouts = pathnode::layouts();
    let n = tiles_x();
    let mut rows = Vec::new();
    #[expect(clippy::needless_range_loop, reason = "`t` is the tile number: it indexes nodes and gives the tile's position")]
    for t in 0..n * tiles_y() {
        let (tx, ty) = ((t % n) as f32 * TILE, (t / n) as f32 * TILE);
        let Some(l) = layouts.iter().find(|l| (l.letter, l.orientation) == nodes[t]) else { continue };
        for node in &l.nodes {
            for &a in node {
                for &b in node {
                    if a == b {
                        continue;
                    }
                    // 0.4 m inside the tile at each exit.
                    let inset = |p: (f32, f32)| (p.0.clamp(0.4, 9.6) + tx, p.1.clamp(0.4, 9.6) + ty);
                    let (from, to) = (inset(a), inset(b));
                    if !area.on_walkmesh(from.0, from.1) || !area.on_walkmesh(to.0, to.1) {
                        continue;
                    }
                    let r = sim::area::simulate(&area, &sim::Query::new(from, to), false);
                    rows.push((r.stats.intra_used, r.stats.calls, t, from, to, describe(&r)));
                }
            }
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let bad = rows.iter().filter(|r| r.0 > limit_units).count();
    println!("{} in-tile crossings checked; {bad} over {limit_units} work units", rows.len());
    for (units, calls, t, from, to, what) in rows.iter().take(8) {
        println!(
            "  tile ({},{}) ({:.1},{:.1}) -> ({:.1},{:.1}): {units} units, {calls} calls, {what}",
            t % n,
            t / n,
            from.0,
            from.1,
            to.0,
            to.1
        );
    }
    Ok(())
}

/// `landscaper sim-line`: walkmesh coverage along a segment, every 0.25 m: '#' walkable.
pub(crate) fn sim_line(x0: f32, y0: f32, x1: f32, y1: f32) {
    let area = sim_area();
    let n = (((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt() / 0.25).round() as usize;
    let line: String = (0..=n)
        .map(|k| {
            let t = k as f32 / n.max(1) as f32;
            if area.on_walkmesh(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t) { '#' } else { '.' }
        })
        .collect();
    println!("{line}");
}

/// `landscaper wok-stats <tileset>...`: walkmesh faces per tile.
pub(crate) fn wok_stats(tilesets: &[String]) -> Result<()> {
    let game = models::open_game(nwn_bake::game::Haks::None, nwn_bake::game::Tlk::None)?;
    for name in tilesets {
        let data = game.resman.get(&ResKey { resref: ResRef::from_str(name)?, restype: ResType::SET })?;
        let set = Tileset::parse(&data, game.codepage())?;
        let mut counts: Vec<usize> = Vec::new();
        for t in &set.tiles {
            let Ok(d) = game.resman.get_named(&t.model, ResType::WOK) else { continue };
            let wm = mg_mdl::walkmesh::Walkmesh::read(&d, mg_mdl::walkmesh::WalkmeshKind::Tile)?;
            let n: usize = wm.surfaces_for(None).filter_map(|s| wm.model.nodes[s.node].mesh()).map(|m| m.faces.len()).sum();
            if n > 0 {
                counts.push(n);
            }
        }
        anyhow::ensure!(!counts.is_empty(), "{name}: no tile has a walkmesh");
        counts.sort_unstable();
        let pct = |q: f32| counts[((counts.len() - 1) as f32 * q) as usize];
        println!(
            "{name}: {} tiles, faces per tile: min {} median {} p90 {} max {}",
            counts.len(),
            counts[0],
            pct(0.5),
            pct(0.9),
            counts[counts.len() - 1]
        );
    }
    Ok(())
}
