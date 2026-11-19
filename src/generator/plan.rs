// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The area plan: each tile's walkmesh and PathNode layout, worked out together so the engine's pathing can cross every
//! tile edge, and the in-tile crossings too costly for it cut.

use super::{
    CLIFF_OVERRIDE, FLAT_OVERRIDE, MAT_GRASS, MAT_NONWALK, TILE, TRANSITION, WALKABLE_NZ, blocked, flat, height, normal_z, rugged,
    tile_level, tiles_x, tiles_y,
};
use crate::{heightmap, mincut, pathnode, rtin, seams, sim};

/// One tile's walkmesh, as written to its `.wok`.
#[derive(Clone)]
pub(crate) struct Walk {
    /// The vertices, tile-centred (m).
    pub(crate) verts: Vec<[f32; 3]>,
    /// The faces, each with its surfacemat.2da row.
    pub(crate) faces: Vec<([u32; 3], u32)>,
    /// Faces made non-walkable on purpose (untangle's cuts): hole filling must leave them alone. Empty: none.
    locked: Vec<bool>,
}

/// Walkmesh height error allowed (m) when simplifying.
const WALK_ERROR: f32 = 0.25;
/// Walkmesh samples a tile side (2^k + 1): 0.625 m apart.
const WALK_SAMPLES: usize = 17;
/// LS_SAMPLES overrides WALK_SAMPLES (2^k + 1), for experiments.
fn walk_samples() -> usize {
    std::env::var("LS_SAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(WALK_SAMPLES)
}

/// A tile's walkmesh: an RTIN over the terrain (see rtin.rs), split where the height error exceeds WALK_ERROR or walkability changes.
/// A tile's walkmesh, from the area plan (see `area_plan`).
pub(crate) fn walkmesh(tx: usize, ty: usize) -> Walk {
    area_plan().0[ty * tiles_x() + tx].clone()
}

/// The whole area's walkmeshes and PathNode layouts, worked out together and
/// kept for the terrain variant last asked for (a new load of the terrain is a
/// new variant, so an older plan is never asked for again):
/// 1. each tile's walkmesh on its own (RTIN);
/// 2. shared edges reconciled (seams.rs: small pieces pruned, edge segments
///    walkable only if walkable on both sides);
/// 3. layouts chosen (pathnode.rs);
/// 4. walkable regions touching a tile edge that no layout node owns are
///    made non-walkable (the engine has no node for them, so a crossing
///    landing there fails), then back to 2, until nothing changes.
pub(crate) fn area_plan() -> std::sync::Arc<(Vec<Walk>, Vec<(char, u8)>)> {
    use std::sync::atomic::Ordering::Relaxed;
    type Plan = std::sync::Arc<(Vec<Walk>, Vec<(char, u8)>)>;
    type Variant = (u32, u8, bool, bool, usize, u64);
    static CACHE: std::sync::Mutex<Option<(Variant, Plan)>> = std::sync::Mutex::new(None);
    let terrain = heightmap::current().map_or(0, |h| h.serial);
    // (Not LS_WALK_ERROR: an environment variable stays the same for the whole process.)
    let variant = (CLIFF_OVERRIDE.load(Relaxed), FLAT_OVERRIDE.load(Relaxed), flat(), rugged(), walk_samples(), terrain);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((v, plan)) = cache.as_ref()
        && *v == variant
    {
        return plan.clone();
    }
    // (The plan before goes first: a 32 by 32 one holds a few megabytes of walkmeshes.)
    *cache = None;
    let plan: Plan = {
        let t0 = std::time::Instant::now();
        let mut walks: Vec<Walk> = (0..tiles_x() * tiles_y()).map(|i| walkmesh_raw(i % tiles_x(), i / tiles_x())).collect();
        if std::env::var_os("LS_TIMING").is_some() {
            eprintln!("  walkmeshes {:.2}s", t0.elapsed().as_secs_f32());
        }
        let mut nodes = plan_rounds(&mut walks);
        // Experimental, opt-in: its cuts are undone by hole filling and it is slow (FINDINGS "Time budget").
        if std::env::var_os("LS_OLD_SEAMS").is_none() && std::env::var_os("LS_UNTANGLE").is_some() {
            nodes = untangle(&mut walks, nodes);
        }
        if let Ok(spec) = std::env::var("LS_DEBUG_SEAM") {
            // "a,b": tiles a and b; print each one's faces touching their shared edge.
            let ts: Vec<usize> = spec.split(',').filter_map(|v| v.parse().ok()).collect();
            for &t in &ts {
                let w = &walks[t];
                let mut segs: Vec<String> = Vec::new();
                for (f, m) in &w.faces {
                    for k in 0..3 {
                        let (p, q) = (w.verts[f[k] as usize], w.verts[f[(k + 1) % 3] as usize]);
                        if (p[0] - q[0]).abs() < 1e-3 && p[0].abs() >= 4.999 {
                            let ox = (t % tiles_x()) as f32 * TILE + TILE / 2.0;
                            let oy = (t / tiles_x()) as f32 * TILE + TILE / 2.0;
                            segs.push(format!(
                                "x{:.2} y{:.2}-{:.2}:{}",
                                p[0] + ox,
                                p[1].min(q[1]) + oy,
                                p[1].max(q[1]) + oy,
                                if *m == MAT_NONWALK { "." } else { "#" }
                            ));
                        }
                    }
                }
                segs.sort();
                eprintln!("tile {t}: {}", segs.join(" "));
            }
        }
        print_path_nodes(&nodes);
        std::sync::Arc::new((walks, nodes))
    };
    *cache = Some((variant, plan.clone()));
    plan
}

/// Tile `i`'s walkmesh placed in the area, for the checks that look across tile edges.
pub(super) fn tile_mesh(i: usize, w: &Walk) -> seams::TileMesh {
    seams::TileMesh {
        locked: w.locked.clone(),
        origin: ((i % tiles_x()) as f32 * TILE + TILE / 2.0, (i / tiles_x()) as f32 * TILE + TILE / 2.0),
        verts: w.verts.clone(),
        faces: w.faces.iter().map(|(f, m)| (*f, *m != MAT_NONWALK)).collect(),
    }
}

/// Steps 2-4 of `area_plan` until stable; returns the layouts.
fn plan_rounds(walks: &mut [Walk]) -> Vec<(char, u8)> {
    let old = std::env::var_os("LS_OLD_SEAMS").is_some();
    let mut nodes = Vec::new();
    for round in 0..8 {
        if !old {
            let mut meshes: Vec<seams::TileMesh> = walks.iter().enumerate().map(|(i, w)| tile_mesh(i, w)).collect();
            seams::reconcile(&mut meshes, TILE);
            for (w, m) in walks.iter_mut().zip(&meshes) {
                for (face, (_, walk)) in w.faces.iter_mut().zip(&m.faces) {
                    face.1 = if *walk { MAT_GRASS } else { MAT_NONWALK };
                }
            }
        }
        let t0 = std::time::Instant::now();
        let (picked, pruned) = choose_path_nodes_for(walks, !old && std::env::var_os("LS_NO_PRUNE").is_none());
        if std::env::var_os("LS_TIMING").is_some() {
            eprintln!("  round {round}: choose {:.2}s, pruned {pruned}", t0.elapsed().as_secs_f32());
        }
        nodes = picked;
        if std::env::var_os("LS_DEBUG_PLAN").is_some() {
            eprintln!("  plan round {round}: pruned {pruned} faces");
        }
        if pruned == 0 || round == 7 {
            break;
        }
    }
    nodes
}

thread_local! {
    /// Whether this thread is running an export ([`as_export`]).
    static EXPORTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `f`, run as an export: [`in_parallel`] on this thread works on [`crate::config::export_threads`] threads (every core
/// unless `--jobs` says otherwise) rather than a check's [`crate::config::threads`].
pub(super) fn as_export<R>(f: impl FnOnce() -> R) -> R {
    /// Puts back what the thread was doing before, even when `f` panics.
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            EXPORTING.set(self.0);
        }
    }
    let _restore = Restore(EXPORTING.replace(true));
    f()
}

/// `f` of each item, in order, worked on `--jobs` threads: by default every core in an export ([`as_export`]), half the cores
/// otherwise (a check).
pub(super) fn in_parallel<T: Sync + Copy, R: Send>(items: &[T], f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let threads = if EXPORTING.get() { crate::config::export_threads() } else { crate::config::threads() };
    in_parallel_on(threads, items, f)
}

/// `f` of each item, in order, worked on at most `threads` threads (started for this call and gone when it returns).
pub(crate) fn in_parallel_on<T: Sync + Copy, R: Send>(threads: usize, items: &[T], f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let threads = threads.max(1).min(items.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&item) = items.get(k) else { break };
                        mine.push((k, f(item)));
                    }
                    mine
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("a worker thread panicked (its message is above)")).collect()
    });
    out.sort_by_key(|(k, _)| *k);
    out.into_iter().map(|(_, r)| r).collect()
}

/// The simulator's view of a plan.
pub(super) fn sim_area_of(walks: &[Walk], nodes: &[(char, u8)]) -> sim::Area {
    let tiles = (0..tiles_x() * tiles_y())
        .map(|i| sim::TileInput {
            verts: walks[i].verts.clone(),
            faces: walks[i].faces.iter().map(|(f, m)| (*f, *m != MAT_NONWALK)).collect(),
            letter: nodes[i].0,
            pn_orient: nodes[i].1,
            orient: 0,
            level: tile_level(i % tiles_x(), i / tiles_x()),
        })
        .collect();
    sim::Area::new(tiles_x() as i32, tiles_y() as i32, tiles, TRANSITION)
}

/// In-tile crossings between exits of one node, with their simulated cost: (units, tile, from, to), area coordinates, worst first.
/// In-tile crossings of the given tiles (see `untangle`).
pub(super) fn tile_crossings_for(walks: &[Walk], nodes: &[(char, u8)], tiles: &[usize]) -> Vec<(i64, usize, (f32, f32), (f32, f32))> {
    let area = sim_area_of(walks, nodes);
    let layouts = pathnode::layouts();
    let n = tiles_x();
    // Each tile's crossings on its own: spread over the cores.
    let one = |t: usize| -> Vec<(i64, usize, (f32, f32), (f32, f32))> {
        let mut rows = Vec::new();
        let (tx, ty) = ((t % n) as f32 * TILE, (t / n) as f32 * TILE);
        let Some(l) = layouts.iter().find(|l| (l.letter, l.orientation) == nodes[t]) else { return rows };
        for node in &l.nodes {
            for &a in node {
                for &b in node {
                    if a == b {
                        continue;
                    }
                    let inset = |p: (f32, f32)| (p.0.clamp(0.4, 9.6) + tx, p.1.clamp(0.4, 9.6) + ty);
                    let (from, to) = (inset(a), inset(b));
                    if !area.on_walkmesh(from.0, from.1) || !area.on_walkmesh(to.0, to.1) {
                        continue;
                    }
                    let r = sim::area::simulate(&area, &sim::Query::new(from, to), false);
                    rows.push((r.stats.intra_used, t, from, to));
                }
            }
        }
        rows
    };
    let mut rows: Vec<_> = in_parallel(tiles, one).into_iter().flatten().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    rows
}

/// In-tile crossings costing more than this (simulated work units) get cut.
pub(super) const CROSSING_LIMIT: i64 = 200_000;

/// Breaks U-shaped regions the engine's in-tile search can't cross in time: for the most expensive in-tile crossing, removes the
/// fewest walkable faces separating its two ends (mincut.rs) and re-plans, keeping the cut only if the two ends stay connected through
/// other tiles and that crossing is no longer over the limit. Repeats until nothing is left to fix; crossings it can't fix are left
/// for the export check to report.
fn untangle(walks: &mut Vec<Walk>, mut nodes: Vec<(char, u8)>) -> Vec<(char, u8)> {
    let debug = std::env::var_os("LS_DEBUG_PLAN").is_some();
    let mut given_up: Vec<(usize, (i32, i32), (i32, i32))> = Vec::new();
    let key = |p: (f32, f32)| ((p.0 * 10.0).round() as i32, (p.1 * 10.0).round() as i32);
    let tiles = tiles_x() * tiles_y();
    // Crossing costs per tile, recomputed only for tiles that change.
    let mut cache: Vec<Vec<(i64, usize, (f32, f32), (f32, f32))>> = vec![Vec::new(); tiles];
    let mut stale: Vec<usize> = (0..tiles).collect();
    for _ in 0..60 {
        let fresh = tile_crossings_for(walks, &nodes, &stale);
        for &t in &stale {
            cache[t].clear();
        }
        for row in fresh {
            cache[row.1].push(row);
        }
        let mut rows: Vec<_> = cache.iter().flatten().copied().collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        let worst = rows.into_iter().filter(|r| r.0 > CROSSING_LIMIT).find(|r| !given_up.contains(&(r.1, key(r.2), key(r.3))));
        let Some((units, t, from, to)) = worst else { break };
        stale.clear();
        let (ox, oy) = ((t % tiles_x()) as f32 * TILE + TILE / 2.0, (t / tiles_x()) as f32 * TILE + TILE / 2.0);
        let w = &walks[t];
        let faces: Vec<([u32; 3], bool)> = w.faces.iter().map(|(f, m)| (*f, *m != MAT_NONWALK)).collect();
        let face_at = |p: (f32, f32)| -> Option<usize> {
            let (x, y) = (p.0 - ox, p.1 - oy);
            faces.iter().position(|(f, walk)| {
                *walk && {
                    let [a, b, c] = f.map(|i| w.verts[i as usize]);
                    let s = |u: [f32; 3], v: [f32; 3]| (v[0] - u[0]) * (y - u[1]) - (v[1] - u[1]) * (x - u[0]);
                    let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
                    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
                }
            })
        };
        let cut = match (face_at(from), face_at(to)) {
            (Some(a), Some(b)) => mincut::min_cut(&faces, &[a], &[b]),
            _ => None,
        };
        let Some(cut) = cut else {
            given_up.push((t, key(from), key(to)));
            continue;
        };
        let mut trial = walks.clone();
        if trial[t].locked.is_empty() {
            trial[t].locked = vec![false; trial[t].faces.len()];
        }
        for &f in &cut {
            trial[t].faces[f].1 = MAT_NONWALK;
            trial[t].locked[f] = true;
        }
        let trial_nodes = plan_rounds(&mut trial);
        let meshes: Vec<seams::TileMesh> = trial.iter().enumerate().map(|(i, w)| tile_mesh(i, w)).collect();
        let conn = seams::Connectivity::new(&meshes, TILE);
        let still_connected = matches!((conn.component(from.0, from.1), conn.component(to.0, to.1)), (Some(a), Some(b)) if a == b);
        let area = sim_area_of(&trial, &trial_nodes);
        let after = sim::area::simulate(&area, &sim::Query::new(from, to), false).stats.intra_used;
        if debug {
            eprintln!(
                "  untangle tile ({},{}) ({:.1},{:.1})->({:.1},{:.1}) {units} units: cut {} faces; connected {still_connected}, \
                 now {after} units",
                t % tiles_x(),
                t / tiles_x(),
                from.0,
                from.1,
                to.0,
                to.1,
                cut.len()
            );
        }
        if still_connected && after <= CROSSING_LIMIT {
            // Tiles whose walkmesh or layout changed need their costs again.
            stale = (0..tiles)
                .filter(|&i| trial_nodes[i] != nodes[i] || trial[i].faces.iter().zip(&walks[i].faces).any(|(a, b)| a.1 != b.1))
                .collect();
            *walks = trial;
            nodes = trial_nodes;
        } else {
            given_up.push((t, key(from), key(to)));
        }
    }
    nodes
}

/// One tile's walkmesh on its own (before the edges are reconciled).
pub(super) fn walkmesh_raw(tx: usize, ty: usize) -> Walk {
    let n = walk_samples();
    let step = TILE / (n - 1) as f32;
    let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
    let base = tile_level(tx, ty) as f32 * TRANSITION;
    let h = |i: f32, j: f32| height(ox + i * step, oy + j * step);
    let mut heights = Vec::with_capacity(n * n);
    let mut walkable = Vec::with_capacity(n * n);
    let max_rise = step * (1.0 - WALKABLE_NZ * WALKABLE_NZ).sqrt() / WALKABLE_NZ;
    const N4: [(f32, f32); 4] = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];
    // Walkable if no neighbour (one sample away, even across the tile edge) is steeper than the slope limit.
    let raw = |fi: f32, fj: f32| -> bool {
        let z = h(fi, fj);
        !N4.iter().any(|(di, dj)| (h(fi + di, fj + dj) - z).abs() > max_rise) && !blocked(ox + fi * step, oy + fj * step)
    };
    // Morphological opening (erode, then dilate, by one sample): removes bridges and spurs under ~2 samples wide. A one-sample ledge
    // across a cliff is no use to a creature, and sends the engine's in-tile search exploring for millions of steps. Computed on the
    // global sample grid, so neighbouring tiles agree.
    let open = std::env::var_os("LS_NO_OPENING").is_none();
    let eroded = |fi: f32, fj: f32| raw(fi, fj) && N4.iter().all(|(di, dj)| raw(fi + di, fj + dj));
    for j in 0..n {
        for i in 0..n {
            let (fi, fj) = (i as f32, j as f32);
            heights.push(h(fi, fj));
            let w = if open { raw(fi, fj) && (eroded(fi, fj) || N4.iter().any(|(di, dj)| eroded(fi + di, fj + dj))) } else { raw(fi, fj) };
            walkable.push(w);
        }
    }
    let samples = rtin::Samples { size: n, height: heights.clone(), walkable: walkable.clone() };
    let seams = std::env::var_os("LS_OLD_SEAMS").is_none();
    let max_error = std::env::var("LS_WALK_ERROR").ok().and_then(|v| v.parse().ok()).unwrap_or(WALK_ERROR);
    let mesh = rtin::build(&samples, max_error, seams);
    let verts: Vec<[f32; 3]> =
        mesh.verts.iter().map(|&(i, j)| [i as f32 * step - TILE / 2.0, j as f32 * step - TILE / 2.0, heights[j * n + i] - base]).collect();
    let walk_at = |v: u32| {
        let (i, j) = mesh.verts[v as usize];
        walkable[j * n + i]
    };
    // The edge of `t` lying on the tile border, if any.
    let border_edge = |t: [u32; 3]| -> Option<(u32, u32)> {
        (0..3).map(|k| (t[k], t[(k + 1) % 3])).find(|&(a, b)| {
            let (p, q) = (mesh.verts[a as usize], mesh.verts[b as usize]);
            (p.0 == q.0 && (p.0 == 0 || p.0 == n - 1)) || (p.1 == q.1 && (p.1 == 0 || p.1 == n - 1))
        })
    };
    let faces = mesh
        .tris
        .iter()
        .map(|&t| {
            let walk = match border_edge(t).filter(|_| seams) {
                // A face on the tile border is walkable as its border edge is, so the tiles either side agree on every point of the
                // edge (the server picks a crossing from one side's walkmesh and starts the next tile there).
                Some((a, b)) => walk_at(a) && walk_at(b),
                None => t.iter().all(|&v| walk_at(v)) && normal_z(&verts, t) >= WALKABLE_NZ,
            };
            (t, if walk { MAT_GRASS } else { MAT_NONWALK })
        })
        .collect();
    Walk { verts, faces, locked: Vec::new() }
}

/// Each tile's PathNode, from its walkmesh (see pathnode.rs).
pub(super) fn choose_path_nodes() -> Vec<(char, u8)> {
    area_plan().1.clone()
}

/// Prints the plan's path nodes, once a plan (LS_QUIET: never).
fn print_path_nodes(nodes: &[(char, u8)]) {
    if std::env::var_os("LS_QUIET").is_some() {
        return;
    }
    let rows: Vec<String> = (0..tiles_y())
        .rev()
        .map(|ty| {
            (0..tiles_x())
                .map(|tx| {
                    let (l, o) = nodes[ty * tiles_x() + tx];
                    format!("{l}{o}")
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    println!("path nodes (north row first): {}", rows.join(" | "));
}

/// Chooses each tile's layout from its walkmesh; with `prune`, also makes non-walkable the border-touching regions no chosen node
/// owns. Returns the layouts and how many faces were pruned.
fn choose_path_nodes_for(walks: &mut [Walk], prune: bool) -> (Vec<(char, u8)>, usize) {
    let layouts: std::borrow::Cow<'_, [pathnode::Layout]> = if std::env::var_os("LS_UPPER").is_some() {
        pathnode::layouts().iter().filter(|l| l.letter.is_ascii_uppercase()).cloned().collect()
    } else {
        pathnode::layouts().into()
    };
    let points = pathnode::edge_points(&layouts);
    let regions: Vec<pathnode::Regions> = walks
        .iter()
        .map(|w| {
            let verts: Vec<[f32; 3]> = w.verts.iter().map(|v| [v[0] + TILE / 2.0, v[1] + TILE / 2.0, v[2]]).collect();
            let faces: Vec<([u32; 3], bool)> = w.faces.iter().map(|(f, m)| (*f, *m != MAT_NONWALK)).collect();
            if std::env::var_os("LS_LOOSE_EXITS").is_some() {
                pathnode::Regions::new(&verts, &faces, &points)
            } else {
                pathnode::Regions::strict(&verts, &faces, &points)
            }
        })
        .collect();
    if std::env::var_os("LS_TIMING").is_some() {
        eprintln!("    regions done");
    }
    let picks = pathnode::choose(&layouts, &regions, tiles_x(), tiles_y());
    let mut pruned = 0;
    if prune {
        for (i, p) in picks.iter().enumerate() {
            if let Some(j) = p {
                for f in regions[i].unowned_border_faces(&layouts[*j]) {
                    walks[i].faces[f].1 = MAT_NONWALK;
                    pruned += 1;
                }
            }
        }
        if std::env::var_os("LS_KEEP_UNREPRESENTED").is_none() {
            for (t, f) in pathnode::unrepresented_crossings(&layouts, &regions, &picks, tiles_x(), tiles_y()) {
                if walks[t].faces[f].1 != MAT_NONWALK {
                    walks[t].faces[f].1 = MAT_NONWALK;
                    pruned += 1;
                }
            }
        }
    }
    let out = picks
        .iter()
        .enumerate()
        .map(|(i, p)| match p {
            Some(j) => (layouts[*j].letter, layouts[*j].orientation),
            None => {
                eprintln!("  tile {i}: no PathNode layout fits its walkmesh; using A");
                ('A', 0)
            }
        })
        .collect();
    (out, pruned)
}

#[cfg(test)]
mod tests {
    use super::{EXPORTING, as_export};

    #[test]
    fn an_export_is_marked_only_while_it_runs() {
        assert!(!EXPORTING.get());
        assert!(as_export(|| as_export(|| EXPORTING.get()) && EXPORTING.get()));
        assert!(!EXPORTING.get());
        // Put back after a panic too, and only on the thread that exported.
        assert!(std::panic::catch_unwind(|| as_export(|| panic!("in an export"))).is_err());
        assert!(!EXPORTING.get());
        assert!(as_export(|| !std::thread::scope(|s| s.spawn(|| EXPORTING.get()).join().unwrap())));
    }
}
