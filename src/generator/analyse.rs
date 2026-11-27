// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The analysis an export or a check runs: the simulator's routes over the plan, coverage, the warnings and the report.

use anyhow::Result;

use super::{
    CROSSING_LIMIT, TILE, WALKABLE_NZ, area_plan, blocked, generate, hak_name, height, in_parallel, plan_digest, sim_area_of, simulate_all,
    tile_crossings_for, tile_mesh, tiles_x, tiles_y, walkmesh_raw,
};
use crate::{heightmap, objects, oracle, seams, sim};

/// The generated area as the server's planner sees it.
pub(crate) fn sim_area() -> sim::Area {
    let plan = area_plan();
    sim_area_of(&plan.0, &plan.1)
}

/// A simulated route's outcome in a few words, for the route check's report and `sim-check`.
pub(crate) fn describe(r: &sim::SimResult) -> String {
    match &r.outcome {
        sim::Outcome::Direct => "direct line".into(),
        sim::Outcome::Complete => format!("complete, {} waypoints", r.waypoints.len()),
        sim::Outcome::Partial { why } => format!("partial line ({why})"),
        sim::Outcome::Failed { why } => format!("FAILED ({why})"),
        sim::Outcome::Grid => format!("grid path, {} waypoints", r.waypoints.len()),
    }
}

/// Where the module starts players: the built-in test terrain's spot, or for painted terrain
/// the walkable point nearest the area's centre.
pub(super) fn start_point() -> [f32; 3] {
    if heightmap::current().is_none() {
        return [6.0, 20.0, 0.0];
    }
    let area = sim_area();
    let (cx, cy) = (tiles_x() as f32 * TILE / 2.0, tiles_y() as f32 * TILE / 2.0);
    for r in 0..(cx.max(cy) as i32 * 2) {
        for k in 0..(8 * r.max(1)) {
            let a = k as f32 / (8 * r.max(1)) as f32 * std::f32::consts::TAU;
            let (x, y) = (cx + r as f32 * 0.5 * a.cos(), cy + r as f32 * 0.5 * a.sin());
            if area.on_walkmesh(x, y) {
                return [x, y, height(x, y)];
            }
        }
    }
    [cx, cy, height(cx, cy)]
}

/// The export warning's words for a tile with `count` crossings over [`CROSSING_LIMIT`].
fn crossing_why(count: usize) -> String {
    let which = if count == 1 { "one way across it costs".to_string() } else { format!("{count} ways across it each cost") };
    format!(
        "the walkable ground inside this tile is too winding for NWN's in-tile search (a U-shaped or narrow passage): {which} the \
         search over {} units of work, more than it allows; straighten or widen it",
        thousands(CROSSING_LIMIT)
    )
}

/// What the one-line summaries add for `n` routes too long for one click ("" for none).
pub(crate) fn long_note(n: usize) -> String {
    if n == 0 { String::new() } else { format!(" (and {n} in shorter clicks)") }
}

/// A report's routes as the editor counts them: one that shorter clicks complete works ("400 of 400 routes work (68 of them in
/// shorter clicks)").
pub(crate) fn routes_work(report: &serde_json::Value) -> String {
    let n = |k: &str| report["routes"][k].as_u64().unwrap_or(0);
    let (ok, long, total) = (n("ok"), n("long"), n("total"));
    let clicks = if long == 0 { String::new() } else { format!(" ({long} of them in shorter clicks)") };
    format!("{} of {total} routes work{clicks}", ok + long)
}

/// `n` with commas between thousands.
fn thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Where planning closed a passage (see `export`): places where ground joined before path nodes were fitted is apart after. Either the
/// ground between was made unwalkable (a patch bordering two now-separate pieces), or both sides are still walkable but no longer
/// joined across a tile edge. One point a place (cuts within 3 m merged).
fn closed_passages(before: &seams::Connectivity, conn: &seams::Connectivity, aw: f32, ah: f32) -> Vec<(f32, f32)> {
    let step = 0.5;
    let (w, h) = ((aw / step) as usize, (ah / step) as usize);
    let at = |i: usize, j: usize| (((i as f32 + 0.5) * step).min(aw - 0.01), ((j as f32 + 0.5) * step).min(ah - 0.01));
    let was: Vec<Option<usize>> = (0..w * h)
        .map(|k| {
            let (x, y) = at(k % w, k / w);
            before.component(x, y)
        })
        .collect();
    let now: Vec<Option<usize>> = (0..w * h)
        .map(|k| {
            let (x, y) = at(k % w, k / w);
            conn.component(x, y)
        })
        .collect();
    let mut cuts: Vec<(f32, f32)> = Vec::new();
    // Neighbours still walkable, joined before, apart now.
    for k in 0..w * h {
        let (i, j) = (k % w, k / w);
        for q in [(i + 1 < w).then(|| k + 1), (j + 1 < h).then(|| k + w)].into_iter().flatten() {
            if let (Some(a), Some(b), Some(c), Some(d)) = (was[k], was[q], now[k], now[q])
                && a == b
                && c != d
            {
                let ((x0, y0), (x1, y1)) = (at(i, j), at(q % w, q / w));
                cuts.push(((x0 + x1) / 2.0, (y0 + y1) / 2.0));
            }
        }
    }
    // Patches made unwalkable between two pieces now apart.
    let removed: Vec<bool> = (0..w * h).map(|k| was[k].is_some() && now[k].is_none()).collect();
    let mut seen = vec![false; w * h];
    for start in 0..w * h {
        if !removed[start] || seen[start] {
            continue;
        }
        let (mut patch, mut stack, mut beside) = (Vec::new(), vec![start], std::collections::BTreeSet::new());
        seen[start] = true;
        while let Some(k) = stack.pop() {
            patch.push(k);
            let (i, j) = (k % w, k / w);
            let around = [(i > 0).then(|| k - 1), (i + 1 < w).then(|| k + 1), (j > 0).then(|| k - w), (j + 1 < h).then(|| k + w)];
            for q in around.into_iter().flatten() {
                if removed[q] {
                    if !seen[q] {
                        seen[q] = true;
                        stack.push(q);
                    }
                } else if let (Some(c), Some(r)) = (now[q], was[q]) {
                    beside.insert((r, c));
                }
            }
        }
        let parts: Vec<&(usize, usize)> = beside.iter().collect();
        if patch.len() >= 4 && parts.iter().any(|a| parts.iter().any(|b| a.0 == b.0 && a.1 != b.1)) {
            let (sx, sy) = patch.iter().fold((0.0, 0.0), |(sx, sy), &k| {
                let (x, y) = at(k % w, k / w);
                (sx + x, sy + y)
            });
            cuts.push((sx / patch.len() as f32, sy / patch.len() as f32));
        }
    }
    // One point a place.
    let mut places: Vec<(f32, f32, usize)> = Vec::new();
    for (x, y) in cuts {
        match places.iter_mut().find(|p| ((p.0 / p.2 as f32 - x).powi(2) + (p.1 / p.2 as f32 - y).powi(2)).sqrt() < 3.0) {
            Some(p) => {
                p.0 += x;
                p.1 += y;
                p.2 += 1;
            }
            None => places.push((x, y, 1)),
        }
    }
    places.into_iter().map(|(x, y, c)| (x / c as f32, y / c as f32)).collect()
}

/// Generates a painted terrain's hak and module (next to the heightmap, in `<name>_nwn/`) and checks it with the simulator, writing
/// report.json: how many random routes the engine completes, walkable coverage, and warnings with area coordinates (routes the engine
/// fails, in-tile crossings too expensive for it). Then installs the module in the user folder's `modules/` (see [`super::install`]);
/// the report says where.
pub(crate) fn export(json: &std::path::Path, before: &super::install::Before) -> Result<serde_json::Value> {
    let mut report = super::plan::as_export(|| analyse(json, true))?;
    super::install::install(json, before, &mut report)?;
    if let Some(m) = report["installed_module"].as_str() {
        println!("  installed {m}");
    }
    Ok(report)
}

/// The export's analysis alone (the editor's Check paths): the same planning, route simulation and warnings, no files built.
pub(crate) fn check(json: &std::path::Path) -> Result<serde_json::Value> {
    analyse(json, false)
}

/// Loads a painted terrain, builds its hak and module when `build`, and
/// analyses its pathing: the report (also written as report.json when built).
fn analyse(json: &std::path::Path, build: bool) -> Result<serde_json::Value> {
    let clock = std::time::Instant::now();
    let lap = |what: &str| {
        if std::env::var_os("LS_TIMING").is_some() {
            eprintln!("{:>7.2}s {what}", clock.elapsed().as_secs_f32());
        }
    };
    let hm = heightmap::load(json)?;
    // (Its import, if it was made by one, may be beside it rather than in the exports folder: `paint::import_dir`.)
    crate::paint::set_area_folder(json.parent().map(std::path::Path::to_path_buf));
    lap("load");
    // Every baked object's model must be there (else say which).
    objects::check()?;
    let out = json.parent().unwrap_or(std::path::Path::new(".")).join(format!("{}_nwn", hm.name));
    if build {
        std::fs::create_dir_all(&out)?;
        generate(&out, Some(""))?;
    }
    let (aw, ah) = (tiles_x() as f32 * TILE, tiles_y() as f32 * TILE);
    let area = sim_area();
    lap("setup");
    let plan = area_plan();
    lap("plan");
    // Truth: the walkmesh's own connectivity (what a creature could walk).
    let meshes: Vec<seams::TileMesh> = plan.0.iter().enumerate().map(|(i, w)| tile_mesh(i, w)).collect();
    let conn = seams::Connectivity::new(&meshes, TILE);
    // And before path nodes were fitted: what the builder drew.
    let raw: Vec<seams::TileMesh> = (0..tiles_x() * tiles_y()).map(|i| tile_mesh(i, &walkmesh_raw(i % tiles_x(), i / tiles_x()))).collect();
    let before = seams::Connectivity::new(&raw, TILE);
    lap("connectivity");
    let max_slope = (1.0 - WALKABLE_NZ * WALKABLE_NZ).sqrt() / WALKABLE_NZ;
    let reach = oracle::Reach::new(aw, ah, 0.5, max_slope, height, blocked);
    let (mut truly, mut covered) = (0usize, 0usize);
    for j in 0..reach.h {
        for i in 0..reach.w {
            if reach.comp[j * reach.w + i] >= 0 {
                truly += 1;
                let (x, y) = ((i as f32 * reach.step).clamp(0.01, aw - 0.01), (j as f32 * reach.step).clamp(0.01, ah - 0.01));
                covered += usize::from(conn.component(x, y).is_some());
            }
        }
    }
    lap("reach");
    let mut warnings = Vec::new();
    let (mut ok, mut total) = (0, 0);
    let pairs_n = (tiles_x() * tiles_y() * 4).clamp(40, 400);
    let wide = reach.wide(1);
    if let Ok(v) = std::env::var("LS_CHECK_MAP") {
        // "x0,y0,x1,y1": the ground there, every 0.5 m (rows north first): '#' not walkable, '.' walkable and joined in the walkmesh,
        // 'o' walkable but not in it (lost by the generator).
        let q: Vec<f32> = v.split(',').filter_map(|t| t.trim().parse().ok()).collect();
        if q.len() == 4 {
            let mut y = q[3];
            while y >= q[1] {
                let mut row = format!("{y:6.1} ");
                let mut x = q[0];
                while x <= q[2] {
                    row.push(if reach.at(x, y) < 0 {
                        '#'
                    } else if before.component(x, y).is_some() {
                        '.'
                    } else {
                        'o'
                    });
                    x += 0.5;
                }
                eprintln!("{row}");
                y -= 0.5;
            }
        }
    }
    if let Ok(v) = std::env::var("LS_CHECK_PAIR") {
        // "x0,y0,x1,y1": which of the pairing rules join the two points.
        let q: Vec<f32> = v.split(',').filter_map(|t| t.trim().parse().ok()).collect();
        if q.len() == 4 {
            eprintln!(
                "pair: reach {} {} | before {:?} {:?} | wide {} {} | after {:?} {:?}",
                reach.at(q[0], q[1]),
                reach.at(q[2], q[3]),
                before.component(q[0], q[1]),
                before.component(q[2], q[3]),
                wide.at(q[0], q[1]),
                wide.at(q[2], q[3]),
                conn.component(q[0], q[1]),
                conn.component(q[2], q[3])
            );
        }
    }
    let pairs: Vec<_> = oracle::pairs(&reach, aw, ah, TILE, pairs_n, 1)
        .into_iter()
        // Ground the builder joined (by its slopes and what blocks it, through a passage a creature fits, not a sliver along a wall or
        // the area's edge), however the tiles came out: a room the walkmesh or the planning cut off fails here, as in the game.
        .filter(|p| {
            let (a, b) = (wide.at(p.from.0, p.from.1), wide.at(p.to.0, p.to.1));
            a >= 0 && a == b
        })
        .filter(|p| conn.component(p.from.0, p.from.1).is_some() && conn.component(p.to.0, p.to.1).is_some())
        .collect();
    let routes = if std::env::var_os("LS_DEBUG_ROUTES").is_some() {
        in_parallel(&pairs.iter().collect::<Vec<_>>(), |p| {
            eprintln!("route ({:.2},{:.2}) -> ({:.2},{:.2})", p.from.0, p.from.1, p.to.0, p.to.1);
            let r = sim::area::simulate(&area, &sim::Query::new(p.from, p.to), false);
            eprintln!("done  ({:.2},{:.2}) -> ({:.2},{:.2})", p.from.0, p.from.1, p.to.0, p.to.1);
            r
        })
    } else {
        let ends: Vec<_> = pairs.iter().map(|p| (p.from, p.to)).collect();
        simulate_all(&area, plan_digest(&plan.0, &plan.1), &ends, i64::MAX)
    };
    let mut failed = Vec::new();
    for (p, r) in pairs.iter().zip(routes) {
        total += 1;
        if ((r.end.0 - p.to.0).powi(2) + (r.end.1 - p.to.1).powi(2)).sqrt() < 1.5 {
            ok += 1;
        } else {
            failed.push((p, r));
        }
    }
    lap("routes");
    // A failed route that shorter clicks along the way complete is too long for one click, not blocked: its own list, apart from
    // the warnings, with how many clicks.
    let clicks = in_parallel(&failed.iter().map(|(p, _)| (p.from, p.to)).collect::<Vec<_>>(), |(from, to)| {
        super::hops::hops(&area, &wide, &conn, from, to)
    });
    let mut long_routes = Vec::new();
    for ((p, r), clicks) in failed.into_iter().zip(clicks) {
        let mut route = serde_json::json!({
            "kind": "failed_route", "from": [p.from.0, p.from.1], "to": [p.to.0, p.to.1],
            "stops_at": [r.end.0, r.end.1], "why": describe(&r),
        });
        match clicks {
            Some(n) => {
                route["kind"] = "long_route".into();
                route["clicks"] = n.into();
                route["why"] = format!(
                    "too long for one click ({}): NWN's route search gives up when the way is about 20 tiles longer than going \
                     straight there, so one click stops here; {n} shorter clicks get there",
                    describe(&r)
                )
                .into();
                long_routes.push(route);
            }
            None => warnings.push(route),
        }
    }
    lap("long routes");
    // One warning per tile with crossings over the limit: one of them and how many. Each search stops at the limit.
    let all: Vec<usize> = (0..tiles_x() * tiles_y()).collect();
    let mut per_tile: std::collections::BTreeMap<usize, ((f32, f32), (f32, f32), usize)> = Default::default();
    for (units, t, from, to) in tile_crossings_for(&plan.0, &plan.1, &all, CROSSING_LIMIT) {
        if units > CROSSING_LIMIT {
            per_tile.entry(t).or_insert((from, to, 0)).2 += 1;
        }
    }
    for (t, (from, to, count)) in per_tile {
        warnings.push(serde_json::json!({
            "kind": "expensive_crossing", "tile": [t % tiles_x(), t / tiles_x()],
            "from": [from.0, from.1], "to": [to.0, to.1], "over": CROSSING_LIMIT, "crossings": count,
            "why": crossing_why(count),
        }));
    }
    lap("crossings");
    // Passages planning closed: ground walkable before path nodes were fitted, made unwalkable by them, that alone joined ground on
    // either side (a gap between obstacles at a tile corner, say). The routes above can't show it: they only pair points the final
    // ground joins.
    for at in closed_passages(&before, &conn, aw, ah) {
        warnings.push(serde_json::json!({
            "kind": "closed_passage", "at": [at.0, at.1],
            "why": "the game's tile pathfinding has no way through here, so the export made it unwalkable: this gap is closed (move \
                    or widen it, away from tile corners and edges)",
        }));
    }
    lap("closed passages");
    // The particle placements: what was skipped, and the live particle count the editor shows.
    let ambience = super::ambience_plan();
    warnings.extend(ambience.warnings());
    let faces_max = plan.0.iter().map(|w| w.faces.len()).max().unwrap_or(0);
    let report = serde_json::json!({
        "name": hm.name,
        "hak": format!("{}.hak", hak_name()),
        "module": out.join(format!("{}.mod", hak_name())),
        "tiles": tiles_x(),
        "tilesY": tiles_y(),
        "faces_max": faces_max,
        "coverage": covered as f32 / truly.max(1) as f32,
        "routes": { "ok": ok, "total": total, "long": long_routes.len() },
        "long_routes": long_routes,
        "start": start_point(),
        "warnings": warnings,
        "ambience": ambience.report(),
    });
    if !build {
        return Ok(report);
    }
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&report)?)?;
    println!(
        "export {}: {}x{} tiles, routes {ok}/{total}{}, coverage {:.1}%, {} warnings -> {}",
        hm.name,
        tiles_x(),
        tiles_y(),
        long_note(long_routes.len()),
        100.0 * covered as f32 / truly.max(1) as f32,
        report["warnings"].as_array().map_or(0, Vec::len),
        out.join("report.json").display()
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{crossing_why, routes_work, thousands};

    #[test]
    fn the_crossing_warning_names_the_limit() {
        assert_eq!(thousands(200_000), "200,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(999), "999");
        assert!(crossing_why(1).contains(": one way across it costs the search over 200,000 units of work, more than"));
        assert!(crossing_why(3).contains(": 3 ways across it each cost the search over 200,000 units of work, more than"));
    }

    #[test]
    fn routes_too_long_for_one_click_work() {
        let r = |v: serde_json::Value| routes_work(&serde_json::json!({ "routes": v }));
        assert_eq!(r(serde_json::json!({ "ok": 332, "total": 400, "long": 68 })), "400 of 400 routes work (68 of them in shorter clicks)");
        assert_eq!(r(serde_json::json!({ "ok": 390, "total": 400, "long": 0 })), "390 of 400 routes work");
        assert_eq!(r(serde_json::json!({ "ok": 40, "total": 40 })), "40 of 40 routes work");
    }
}
