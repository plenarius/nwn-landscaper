// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Making neighbouring tiles' walkmeshes agree at their shared edges.
//!
//! The server picks where to cross into the next tile using only the tile it is leaving (GenerateBestIntraTileExit), then starts the
//! next tile's search from that point. A crossing that is walkable on one side but off the walkmesh, or on a cut-off sliver, on the
//! other gets denied, and the route may be lost (research/FINDINGS.md). So, over all tiles together: walkable pieces too small to lead
//! anywhere are made non-walkable, and a border segment stays walkable only if it is walkable on both sides, until nothing changes.

use std::collections::HashMap;

/// One tile's walkmesh, in area coordinates for the border bookkeeping: vertices (x, y) and faces with their walkability.
pub struct TileMesh {
    /// Faces never to be made walkable (see generator.rs `Walk::locked`); empty: none.
    pub locked: Vec<bool>,
    /// The tile's centre in the area (m): its vertices are about it.
    pub origin: (f32, f32),
    /// The vertices (tile-centred, m).
    pub verts: Vec<[f32; 3]>,
    /// The faces, each with whether it is walkable.
    pub faces: Vec<([u32; 3], bool)>,
}

/// Smallest walkable piece kept (m²): one sample cell's triangle is ~0.8.
const MIN_PIECE: f32 = 2.0;
/// Non-walkable holes smaller than this (m²), inside a tile, become walkable: a creature can step over a small steep rock, and every
/// hole multiplies the engine's in-tile search (IntraTileDFS explores around it).
const MAX_HOLE: f32 = 4.0;

/// Faces of each non-walkable piece (faces joined by a shared edge).
fn holes(m: &TileMesh) -> Vec<Vec<usize>> {
    let flipped =
        TileMesh { locked: Vec::new(), origin: m.origin, verts: m.verts.clone(), faces: m.faces.iter().map(|(f, w)| (*f, !w)).collect() };
    pieces(&flipped)
}

fn key(x: f32, y: f32) -> (i64, i64) {
    ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64)
}

/// Faces of each walkable piece (faces joined by a shared edge).
fn pieces(m: &TileMesh) -> Vec<Vec<usize>> {
    let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (fi, (f, walk)) in m.faces.iter().enumerate() {
        if *walk {
            for k in 0..3 {
                let (a, b) = (f[k].min(f[(k + 1) % 3]), f[k].max(f[(k + 1) % 3]));
                edge_faces.entry((a, b)).or_default().push(fi);
            }
        }
    }
    let mut seen = vec![false; m.faces.len()];
    let mut out = Vec::new();
    for start in 0..m.faces.len() {
        if seen[start] || !m.faces[start].1 {
            continue;
        }
        let mut stack = vec![start];
        let mut piece = Vec::new();
        seen[start] = true;
        while let Some(fi) = stack.pop() {
            piece.push(fi);
            let f = m.faces[fi].0;
            for k in 0..3 {
                let (a, b) = (f[k].min(f[(k + 1) % 3]), f[k].max(f[(k + 1) % 3]));
                for &g in &edge_faces[&(a, b)] {
                    if !seen[g] {
                        seen[g] = true;
                        stack.push(g);
                    }
                }
            }
        }
        out.push(piece);
    }
    out
}

fn area(m: &TileMesh, fi: usize) -> f32 {
    let [a, b, c] = m.faces[fi].0.map(|i| m.verts[i as usize]);
    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0
}

/// Border segments of a tile: (segment key in area coordinates) -> face.
fn border_faces(m: &TileMesh, tile: f32) -> HashMap<((i64, i64), (i64, i64)), usize> {
    let mut out = HashMap::new();
    let on_border = |v: [f32; 3]| v[0].abs() >= tile / 2.0 - 1e-3 || v[1].abs() >= tile / 2.0 - 1e-3;
    for (fi, (f, _)) in m.faces.iter().enumerate() {
        for k in 0..3 {
            let (p, q) = (m.verts[f[k] as usize], m.verts[f[(k + 1) % 3] as usize]);
            let same_side = ((p[0] - q[0]).abs() < 1e-3 && p[0].abs() >= tile / 2.0 - 1e-3)
                || ((p[1] - q[1]).abs() < 1e-3 && p[1].abs() >= tile / 2.0 - 1e-3);
            if on_border(p) && on_border(q) && same_side {
                let (kp, kq) = (key(p[0] + m.origin.0, p[1] + m.origin.1), key(q[0] + m.origin.0, q[1] + m.origin.1));
                out.insert(if kp < kq { (kp, kq) } else { (kq, kp) }, fi);
            }
        }
    }
    out
}

/// Prunes small pieces and syncs shared edges until stable. Returns how many faces were made non-walkable.
pub fn reconcile(tiles: &mut [TileMesh], tile: f32) -> usize {
    let borders: Vec<_> = tiles.iter().map(|m| border_faces(m, tile)).collect();
    let mut changed_total = 0;
    for pass in 0.. {
        if pass == 200 {
            eprintln!("seams: not stable after 200 passes");
            break;
        }
        let mut changed = 0;
        for m in tiles.iter_mut() {
            if std::env::var_os("LS_KEEP_HOLES").is_none() {
                let half = tile / 2.0 - 1e-3;
                for hole in holes(m) {
                    let a: f32 = hole.iter().map(|&fi| area(m, fi)).sum();
                    let touches_border = hole.iter().any(|&fi| {
                        m.faces[fi].0.iter().any(|&v| m.verts[v as usize][0].abs() >= half || m.verts[v as usize][1].abs() >= half)
                    });
                    let locked = hole.iter().any(|&fi| m.locked.get(fi).copied().unwrap_or(false));
                    if a < MAX_HOLE && !touches_border && !locked {
                        for fi in hole {
                            m.faces[fi].1 = true;
                            changed += 1;
                        }
                    }
                }
            }
            for piece in pieces(m) {
                let a: f32 = piece.iter().map(|&fi| area(m, fi)).sum();
                if a < MIN_PIECE {
                    for fi in piece {
                        m.faces[fi].1 = false;
                        changed += 1;
                    }
                }
            }
        }
        // A segment shared by two tiles stays walkable only if both agree.
        let mut by_segment: HashMap<_, Vec<(usize, usize)>> = HashMap::new();
        for (ti, b) in borders.iter().enumerate() {
            for (seg, &fi) in b {
                by_segment.entry(*seg).or_default().push((ti, fi));
            }
        }
        for sides in by_segment.values() {
            if sides.len() == 2 {
                let (a, b) = (sides[0], sides[1]);
                let (wa, wb) = (tiles[a.0].faces[a.1].1, tiles[b.0].faces[b.1].1);
                if wa != wb {
                    tiles[a.0].faces[a.1].1 = false;
                    tiles[b.0].faces[b.1].1 = false;
                    changed += 1;
                }
            }
        }
        changed_total += changed;
        if changed == 0 {
            if std::env::var_os("LS_DEBUG_PLAN").is_some() {
                eprintln!("  seams: stable after {pass} passes, {changed_total} faces changed");
            }
            break;
        }
    }
    changed_total
}

/// Connectivity of the whole area's walkable faces, as a creature could walk them: within a tile through shared edges, across tiles
/// through border segments walkable on both sides. `component(x, y)` is that of the walkable face under an area point.
pub struct Connectivity {
    face_comp: Vec<Vec<Option<usize>>>,
    tiles_origin: Vec<(f32, f32)>,
    /// The tiles by the `tile`-sized square their origin is in ([`Connectivity::cell`]), so a point's tiles are found
    /// without looking at every tile: `cells[(cy - cell0.1) * cells_x + cx - cell0.0]`.
    cells: Vec<Vec<usize>>,
    cell0: (i64, i64),
    cells_x: usize,
    verts: Vec<Vec<[f32; 3]>>,
    faces: Vec<Vec<([u32; 3], bool)>>,
    tile: f32,
}

impl Connectivity {
    /// The walkable pieces of every tile's walkmesh joined across the tiles' shared edges (`tile` is a tile's side, m).
    pub fn new(tiles: &[TileMesh], tile: f32) -> Connectivity {
        let offsets: Vec<usize> = tiles
            .iter()
            .scan(0, |acc, m| {
                let o = *acc;
                *acc += m.faces.len();
                Some(o)
            })
            .collect();
        let total: usize = tiles.iter().map(|m| m.faces.len()).sum();
        let mut parent: Vec<usize> = (0..total).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        for (ti, m) in tiles.iter().enumerate() {
            for piece in pieces(m) {
                for w in piece.windows(2) {
                    let (a, b) = (find(&mut parent, offsets[ti] + w[0]), find(&mut parent, offsets[ti] + w[1]));
                    parent[a] = b;
                }
            }
        }
        let mut by_segment: HashMap<_, Vec<usize>> = HashMap::new();
        for (ti, m) in tiles.iter().enumerate() {
            for (seg, fi) in border_faces(m, tile) {
                if m.faces[fi].1 {
                    by_segment.entry(seg).or_default().push(offsets[ti] + fi);
                }
            }
        }
        for sides in by_segment.values() {
            if sides.len() == 2 {
                let (a, b) = (find(&mut parent, sides[0]), find(&mut parent, sides[1]));
                parent[a] = b;
            }
        }
        let face_comp = tiles
            .iter()
            .enumerate()
            .map(|(ti, m)| (0..m.faces.len()).map(|fi| m.faces[fi].1.then(|| find(&mut parent, offsets[ti] + fi))).collect())
            .collect();
        let cell = |o: (f32, f32)| Connectivity::cell(o.0, o.1, tile);
        let at: Vec<(i64, i64)> = tiles.iter().map(|m| cell(m.origin)).collect();
        let lo = at.iter().copied().reduce(|a, b| (a.0.min(b.0), a.1.min(b.1))).unwrap_or((0, 0));
        let hi = at.iter().copied().reduce(|a, b| (a.0.max(b.0), a.1.max(b.1))).unwrap_or((0, 0));
        let cells_x = (hi.0 - lo.0 + 1) as usize;
        let mut cells = vec![Vec::new(); cells_x * (hi.1 - lo.1 + 1) as usize];
        for (ti, c) in at.into_iter().enumerate() {
            cells[(c.1 - lo.1) as usize * cells_x + (c.0 - lo.0) as usize].push(ti);
        }
        Connectivity {
            face_comp,
            tiles_origin: tiles.iter().map(|m| m.origin).collect(),
            cells,
            cell0: lo,
            cells_x,
            verts: tiles.iter().map(|m| m.verts.clone()).collect(),
            faces: tiles.iter().map(|m| m.faces.clone()).collect(),
            tile,
        }
    }

    /// The `tile`-sized square (x, y) is in.
    fn cell(x: f32, y: f32, tile: f32) -> (i64, i64) {
        ((x / tile).floor() as i64, (y / tile).floor() as i64)
    }

    /// The component of the first tile (in tile order) whose square holds (x, y) and has a walkable face under it. A tile
    /// whose origin is within half a tile of the point has its origin in the point's square or one beside it, so only the 3
    /// by 3 squares around it are looked at.
    pub fn component(&self, x: f32, y: f32) -> Option<usize> {
        let (cx, cy) = Connectivity::cell(x, y, self.tile);
        // (The tile and its component; a later tile is only looked at when no earlier one has been found.)
        let mut found: Option<(usize, Option<usize>)> = None;
        for gy in cy - 1..=cy + 1 {
            for gx in cx - 1..=cx + 1 {
                let (i, j) = (gx - self.cell0.0, gy - self.cell0.1);
                if i < 0 || j < 0 || i >= self.cells_x as i64 {
                    continue;
                }
                let Some(tiles) = self.cells.get(j as usize * self.cells_x + i as usize) else { continue };
                for &ti in tiles {
                    if found.is_some_and(|(earlier, _)| earlier < ti) {
                        continue;
                    }
                    if let Some(c) = self.in_tile(ti, x, y) {
                        found = Some((ti, c));
                    }
                }
            }
        }
        found.and_then(|(_, c)| c)
    }

    /// Whether tile `ti`'s square holds (x, y) with a walkable face under it, and if so that face's component.
    fn in_tile(&self, ti: usize, x: f32, y: f32) -> Option<Option<usize>> {
        let o = self.tiles_origin[ti];
        let (lx, ly) = (x - o.0, y - o.1);
        if lx.abs() > self.tile / 2.0 || ly.abs() > self.tile / 2.0 {
            return None;
        }
        for (fi, (f, walk)) in self.faces[ti].iter().enumerate() {
            if !walk {
                continue;
            }
            let [a, b, c] = f.map(|i| self.verts[ti][i as usize]);
            let s = |u: [f32; 3], v: [f32; 3]| (v[0] - u[0]) * (ly - u[1]) - (v[1] - u[1]) * (lx - u[0]);
            let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
            if !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0)) {
                return Some(self.face_comp[ti][fi]);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_finds_the_same_component_as_a_search_of_every_tile() {
        // 3 by 2 tiles of 10 m, each two triangles; the middle tile's south-east half is not walkable, so points on it and
        // on its edges fall to a neighbour or to nothing.
        let tile = 10.0;
        let tiles: Vec<TileMesh> = (0..6)
            .map(|i| TileMesh {
                locked: Vec::new(),
                origin: ((i % 3) as f32 * tile + 5.0, (i / 3) as f32 * tile + 5.0),
                verts: vec![[-5.0, -5.0, 0.0], [5.0, -5.0, 0.0], [5.0, 5.0, 0.0], [-5.0, 5.0, 0.0]],
                faces: vec![([0, 1, 2], i != 1), ([0, 2, 3], true)],
            })
            .collect();
        let conn = Connectivity::new(&tiles, tile);
        let every_tile = |x: f32, y: f32| (0..tiles.len()).find_map(|ti| conn.in_tile(ti, x, y)).flatten();
        let mut on_mesh = 0;
        for j in -4..=84 {
            for i in -4..=124 {
                let (x, y) = (i as f32 * 0.25, j as f32 * 0.25);
                assert_eq!(conn.component(x, y), every_tile(x, y), "({x}, {y})");
                on_mesh += usize::from(conn.component(x, y).is_some());
            }
        }
        assert!(on_mesh > 100 * 80, "{on_mesh}");
    }
}
