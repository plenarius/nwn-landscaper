// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Steep ground cut where its paint changes, on a smooth line, so each piece takes one texture and a bank's paint changes at one
//! height.
//!
//! The ground's mesh has a vertex every 0.5 m ([`super::RENDER_STEP`]), the paint a sample every 0.625 m. Walkable ground takes the
//! tile's baked texture, which blends the four samples around each point bilinearly, as the editor does. A steep face takes one
//! library texture whole (tile.rs). Until 0.6.1 that was the sample's nearest the face's middle; 0.6.1 cut steep faces on the lines
//! halfway between two samples, so each point took its nearest sample's, as the editor draws steep ground. Either way the paint
//! changed in steps of a sample along any line not parallel to the grid. On a river's bank, where the ground climbs 1.6 m in one
//! sample, each step stood up the face as a tooth: the bed's mud reached up to the water's surface and the bank's sand down into
//! the water, a sawtooth seen through it.
//!
//! [`split`] gives each vertex of steep ground the texture that weighs most in that blend, and cuts each edge whose ends differ where
//! their two textures weigh the same (taken as linear along the edge). Across a bank the height and the blend both change from one
//! sample to the next, so that line keeps to one height, under the water. The faces beside a cut take the new vertices on their
//! shared edges too, the neighbouring tile's included, so the ground stays closed: a vertex in the middle of a neighbour's edge (a
//! T-junction) lets the sky through as specks.

use std::collections::{BTreeSet, HashMap};

use super::Grid;

/// What [`split`] reads of the area.
pub(super) struct Area<'a> {
    /// The grid's step and a tile's side (m).
    pub(super) step: f32,
    pub(super) tile: f32,
    /// The area's size in tiles.
    pub(super) tiles: (usize, usize),
    /// The paint's sample spacing (m) and sample (i, j)'s paint.
    pub(super) spacing: f32,
    pub(super) material: &'a dyn Fn(i64, i64) -> u8,
    /// The ground's height at an area point.
    pub(super) height: &'a dyn Fn(f32, f32) -> f32,
    /// Tile (tx, ty)'s base height (its level).
    pub(super) base: &'a dyn Fn(usize, usize) -> f32,
    /// Whether tile (tx, ty) draws a face (its own coordinates, as [`super::grid`] makes them) as steep ground.
    pub(super) steep: &'a dyn Fn(usize, usize, [[f32; 3]; 3]) -> bool,
}

/// A grid vertex of the whole area, and an edge between two (the lower first).
type Vertex = (i64, i64);
type Edge = [Vertex; 2];

impl Area<'_> {
    fn n(&self) -> i64 {
        (self.tile / self.step).round() as i64
    }

    /// The grid vertex (gi, gj) of the whole area: in tile (tx, ty)'s own coordinates, as [`super::grid`] makes it (so a vertex two
    /// tiles share is the same in both), and in the area's, with its height read again (the same in every tile).
    fn vertex(&self, (tx, ty): (usize, usize), (gi, gj): Vertex) -> ([f32; 3], [f32; 3]) {
        let (i, j) = (gi - tx as i64 * self.n(), gj - ty as i64 * self.n());
        let (ox, oy) = (tx as f32 * self.tile, ty as f32 * self.tile);
        let (lx, ly) = (i as f32 * self.step, j as f32 * self.step);
        let h = (self.height)(ox + lx, oy + ly);
        ([lx - self.tile / 2.0, ly - self.tile / 2.0, h - (self.base)(tx, ty)], [ox + lx, oy + ly, h])
    }

    /// Each texture's weight at area point (x, y) in the bilinear blend of the four samples around it.
    fn blend(&self, x: f32, y: f32) -> Vec<(u8, f32)> {
        let (fx, fy) = ((x / self.spacing).max(0.0), (y / self.spacing).max(0.0));
        let (i, j) = (fx.floor(), fy.floor());
        let (sx, sy) = (fx - i, fy - j);
        let (i, j) = (i as i64, j as i64);
        let mut w: Vec<(u8, f32)> = Vec::with_capacity(4);
        for ((di, dj), share) in
            [((0, 0), (1.0 - sx) * (1.0 - sy)), ((1, 0), sx * (1.0 - sy)), ((0, 1), (1.0 - sx) * sy), ((1, 1), sx * sy)]
        {
            let m = (self.material)(i + di, j + dj);
            match w.iter_mut().find(|(k, _)| *k == m) {
                Some(e) => e.1 += share,
                None => w.push((m, share)),
            }
        }
        w
    }

    /// The texture steep ground takes at area point (x, y): the one weighing most in the blend (in a tie, the lower palette index).
    fn paint(&self, x: f32, y: f32) -> u8 {
        self.blend(x, y).into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map_or(0, |(m, _)| m)
    }

    /// How much more texture `a` weighs than texture `b` at area point (x, y).
    fn lead(&self, x: f32, y: f32, a: u8, b: u8) -> f32 {
        let w = self.blend(x, y);
        let of = |m: u8| w.iter().find(|(k, _)| *k == m).map_or(0.0, |(_, s)| *s);
        of(a) - of(b)
    }
}

/// The two triangles of grid cell (i, j), counter-clockwise from above, as [`super::grid`] makes them.
fn cell(i: i64, j: i64) -> [[Vertex; 3]; 2] {
    [[(i, j), (i + 1, j), (i + 1, j + 1)], [(i, j), (i + 1, j + 1), (i, j + 1)]]
}

fn edge(a: Vertex, b: Vertex) -> Edge {
    if a <= b { [a, b] } else { [b, a] }
}

/// Cuts the faces of `parts[steep]` (tile (tx, ty)'s steep ground, in grid `g`, which [`super::grid`] made) where their paint
/// changes, and puts the new vertices into every face of `parts` that shares an edge with a cut face, the next tile's included.
/// Every face stays in its part. Gives the texture of each face of `parts[steep]`, in order.
pub(super) fn split(g: &mut Grid, (tx, ty): (usize, usize), parts: &mut [Vec<[u32; 3]>], steep: usize, area: &Area<'_>) -> Vec<u8> {
    let n = area.n();
    let (gx, gy) = (tx as i64 * n, ty as i64 * n);
    let global = |v: u32| (gx + i64::from(v) % (n + 1), gy + i64::from(v) / (n + 1));
    let world = |v: Vertex| area.vertex((tx, ty), v).1;
    let mut paints: HashMap<Vertex, u8> = HashMap::new();
    let mut paint = |v: Vertex| {
        *paints.entry(v).or_insert_with(|| {
            let p = world(v);
            area.paint(p[0], p[1])
        })
    };
    // The edges whose ends differ, of this tile's steep faces and of the next tiles' across its border. (A BTreeSet: the new
    // vertices come in the same order every run.)
    let mut edges: BTreeSet<Edge> = BTreeSet::new();
    for &t in &parts[steep] {
        let c = t.map(global);
        for e in 0..3 {
            let (a, b) = (c[e], c[(e + 1) % 3]);
            if paint(a) != paint(b) {
                edges.insert(edge(a, b));
            }
        }
    }
    let (w, h) = (area.tiles.0 as i64, area.tiles.1 as i64);
    for k in 0..n {
        for (dx, dy, cell_at, which, border) in [
            (-1, 0, (gx - 1, gy + k), 0, [(gx, gy + k), (gx, gy + k + 1)]),
            (1, 0, (gx + n, gy + k), 1, [(gx + n, gy + k), (gx + n, gy + k + 1)]),
            (0, -1, (gx + k, gy - 1), 1, [(gx + k, gy), (gx + k + 1, gy)]),
            (0, 1, (gx + k, gy + n), 0, [(gx + k, gy + n), (gx + k + 1, gy + n)]),
        ] {
            let (nx, ny) = (tx as i64 + dx, ty as i64 + dy);
            if !(0..w).contains(&nx) || !(0..h).contains(&ny) || paint(border[0]) == paint(border[1]) {
                continue;
            }
            let beside = (nx as usize, ny as usize);
            let face = cell(cell_at.0, cell_at.1)[which];
            if (area.steep)(beside.0, beside.1, face.map(|v| area.vertex(beside, v).0)) {
                edges.insert(border);
            }
        }
    }
    // Each edge's point, from its ends in the area's coordinates: the same in both tiles of a border. Kept a hair from the ends, so
    // no piece is flat.
    let origin = [tx as f32 * area.tile + area.tile / 2.0, ty as f32 * area.tile + area.tile / 2.0, (area.base)(tx, ty)];
    let mut points: HashMap<Edge, u32> = HashMap::new();
    for &[a, b] in &edges {
        let (pa, pb, ma, mb) = (world(a), world(b), paint(a), paint(b));
        let (la, lb) = (area.lead(pa[0], pa[1], ma, mb), area.lead(pb[0], pb[1], ma, mb));
        let s = if la - lb > 1e-6 { (la / (la - lb)).clamp(0.01, 0.99) } else { 0.5 };
        points.insert([a, b], g.verts.len() as u32);
        g.verts.push(std::array::from_fn(|d| pa[d] + s * (pb[d] - pa[d]) - origin[d]));
    }
    let mut out_paints = Vec::with_capacity(parts[steep].len());
    for (part, faces) in parts.iter_mut().enumerate() {
        let mut out = Vec::with_capacity(faces.len());
        for &t in faces.iter() {
            let c = t.map(global);
            let mid: [Option<u32>; 3] = std::array::from_fn(|e| points.get(&edge(c[e], c[(e + 1) % 3])).copied());
            if part == steep {
                pieces(g, t, c.map(&mut paint), mid, &mut out, &mut out_paints);
            } else if mid.iter().all(Option::is_none) {
                out.push(t);
            } else {
                let poly: Vec<u32> = (0..3).flat_map(|e| std::iter::once(t[e]).chain(mid[e])).collect();
                fan(g, &poly, &mut out);
            }
        }
        *faces = out;
    }
    out_paints
}

/// Steep face `t` as pieces of one texture each, into `out` and their textures into `paints`: `p` is its corners' textures and
/// `mid[e]` the point on its edge from corner e to the next, where those two differ.
fn pieces(g: &mut Grid, t: [u32; 3], p: [u8; 3], mid: [Option<u32>; 3], out: &mut Vec<[u32; 3]>, paints: &mut Vec<u8>) {
    match mid {
        [Some(m01), Some(m12), Some(m20)] => {
            // Three textures: each corner's piece reaches to the middle of the three points.
            let at = [m01, m12, m20].map(|m| g.verts[m as usize]);
            let centre = g.verts.len() as u32;
            g.verts.push(std::array::from_fn(|d| (at[0][d] + at[1][d] + at[2][d]) / 3.0));
            for (corner, after, before) in [(0, m01, m20), (1, m12, m01), (2, m20, m12)] {
                out.extend([[t[corner], after, centre], [t[corner], centre, before]]);
                paints.extend([p[corner]; 2]);
            }
        }
        _ => {
            // Two textures: the corner whose both edges are cut stands alone; the rest is a four-sided piece.
            let alone = (0..3).find_map(|a| Some((a, mid[a]?, mid[(a + 2) % 3]?)));
            let Some((a, after, before)) = alone else {
                // One texture. (An edge is cut only where its ends differ: no face has one cut edge.)
                debug_assert!(mid.iter().all(Option::is_none), "a steep face with one cut edge");
                out.push(t);
                paints.push(p[0]);
                return;
            };
            let (b, c) = ((a + 1) % 3, (a + 2) % 3);
            out.extend([[t[a], after, before], [after, t[b], t[c]], [after, t[c], before]]);
            paints.extend([p[a], p[b], p[b]]);
        }
    }
}

/// Convex polygon `poly` (vertex ids, counter-clockwise from above) as triangles: a fan around its middle, a new vertex (fanned
/// from a corner, a point on one of the corner's edges would make a flat piece).
fn fan(g: &mut Grid, poly: &[u32], out: &mut Vec<[u32; 3]>) {
    let k = poly.len() as f32;
    let mid = poly.iter().fold([0.0; 3], |m, &v| {
        let p = g.verts[v as usize];
        [m[0] + p[0] / k, m[1] + p[1] / k, m[2] + p[2] / k]
    });
    let id = g.verts.len() as u32;
    g.verts.push(mid);
    for e in 0..poly.len() {
        out.push([id, poly[e], poly[(e + 1) % poly.len()]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: i64 = 20;

    /// A 2 x 2 tile area: rolling ground, painted in two by a curve with a few stray samples (a river's bank, a fleck), steep
    /// where a band crosses it.
    fn area_with<'a>(material: &'a dyn Fn(i64, i64) -> u8, steep: &'a dyn Fn(usize, usize, [[f32; 3]; 3]) -> bool) -> Area<'a> {
        Area {
            step: 0.5,
            tile: 10.0,
            tiles: (2, 2),
            spacing: 0.625,
            material,
            height: &|x, y| 3.0 * (x * 0.31).sin() + 2.0 * (y * 0.23).cos() + 0.1 * x,
            base: &|tx, ty| [0.0, 5.0, 5.0, 10.0][ty * 2 + tx],
            steep,
        }
    }

    fn paint(i: i64, j: i64) -> u8 {
        let (x, y) = (i as f32 * 0.625, j as f32 * 0.625);
        if (i, j) == (12, 20) || (i, j) == (17, 9) { 2 } else { u8::from(y > 9.0 + 4.0 * (x * 0.4).sin()) }
    }

    fn steep(tx: usize, ty: usize, f: [[f32; 3]; 3]) -> bool {
        let x = f.iter().map(|v| v[0]).sum::<f32>() / 3.0 + tx as f32 * 10.0 + 5.0;
        let y = f.iter().map(|v| v[1]).sum::<f32>() / 3.0 + ty as f32 * 10.0 + 5.0;
        (x + 0.5 * y + 2.0) % 7.0 < 4.5
    }

    /// Tile (tx, ty) as tile.rs makes it: the grid, its faces in two parts (ground, steep), split, and each steep face's texture.
    fn tile(area: &Area<'_>, tx: usize, ty: usize) -> (Grid, [Vec<[u32; 3]>; 2], Vec<u8>) {
        let at = |i: i64, j: i64| (j * (N + 1) + i) as u32;
        let mut verts = Vec::new();
        for j in 0..=N {
            for i in 0..=N {
                verts.push(area.vertex((tx, ty), (tx as i64 * N + i, ty as i64 * N + j)).0);
            }
        }
        let mut parts: [Vec<[u32; 3]>; 2] = Default::default();
        for j in 0..N {
            for i in 0..N {
                for t in [[at(i, j), at(i + 1, j), at(i + 1, j + 1)], [at(i, j), at(i + 1, j + 1), at(i, j + 1)]] {
                    parts[usize::from((area.steep)(tx, ty, t.map(|v| verts[v as usize])))].push(t);
                }
            }
        }
        let mut g = Grid { verts, tris: Vec::new() };
        let paints = split(&mut g, (tx, ty), &mut parts, 1, area);
        assert_eq!(paints.len(), parts[1].len());
        (g, parts, paints)
    }

    fn twice_area(t: [[f32; 3]; 3]) -> f32 {
        (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])
    }

    #[test]
    fn every_piece_of_steep_ground_takes_the_paint_that_weighs_most_there() {
        let area = area_with(&paint, &steep);
        let (mut cut, mut whole, mut off) = (0, 0, 0);
        for (tx, ty) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (g, parts, paints) = tile(&area, tx, ty);
            let world = |p: [f32; 3]| (p[0] + tx as f32 * 10.0 + 5.0, p[1] + ty as f32 * 10.0 + 5.0);
            for (t, &m) in parts[1].iter().zip(&paints) {
                let v = t.map(|v| g.verts[v as usize]);
                // Its corners of the grid have its paint. (The rest are on cuts, where two weigh the same.)
                for (&id, &c) in t.iter().zip(&v) {
                    if i64::from(id) < (N + 1) * (N + 1) {
                        let (x, y) = world(c);
                        assert_eq!(area.paint(x, y), m, "tile ({tx}, {ty}): a corner of a steep face: {v:?}");
                    }
                }
                // Its middle too, but in slivers along a cut: the cut takes the blend as linear between two vertices, which it isn't.
                let (x, y) = world([0, 1, 2].map(|d| (v[0][d] + v[1][d] + v[2][d]) / 3.0));
                if area.paint(x, y) != m {
                    let size = twice_area(v) / 2.0;
                    assert!(size < 0.04, "tile ({tx}, {ty}): a piece of {size} m² with another paint in its middle: {v:?}");
                    off += 1;
                }
                if i64::from(*t.iter().max().unwrap()) >= (N + 1) * (N + 1) { cut += 1 } else { whole += 1 }
            }
        }
        assert!(cut > 50 && whole > 500, "{cut} pieces, {whole} faces whole");
        assert!(off * 10 < cut, "{off} of {cut} pieces have another paint in their middle");
    }

    #[test]
    fn a_bank_s_paint_changes_at_one_height() {
        // A river's bank across the grid at 25 degrees: the bed's mud at 0 m, the bank's sand 1.6 m up, one sample on.
        let sand = |i: i64, j: i64| (j as f32 * 0.625) > 4.0 + 0.466 * (i as f32 * 0.625);
        let material = move |i: i64, j: i64| u8::from(sand(i, j));
        let height = move |x: f32, y: f32| {
            let (fx, fy) = (x / 0.625, y / 0.625);
            let (i, j) = (fx.floor(), fy.floor());
            let (sx, sy) = (fx - i, fy - j);
            let h = |di: i64, dj: i64| if sand(i as i64 + di, j as i64 + dj) { 1.6 } else { 0.0 };
            h(0, 0) * (1.0 - sx) * (1.0 - sy) + h(1, 0) * sx * (1.0 - sy) + h(0, 1) * (1.0 - sx) * sy + h(1, 1) * sx * sy
        };
        let steep = |_: usize, _: usize, f: [[f32; 3]; 3]| {
            let (u, v) = (std::array::from_fn::<f32, 3, _>(|d| f[1][d] - f[0][d]), std::array::from_fn::<f32, 3, _>(|d| f[2][d] - f[0][d]));
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            n[2] / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() < 0.64
        };
        let area = Area {
            step: 0.5,
            tile: 10.0,
            tiles: (2, 2),
            spacing: 0.625,
            material: &material,
            height: &height,
            base: &|_, _| 0.0,
            steep: &steep,
        };
        let mut faces = [0; 2];
        for (tx, ty) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (g, parts, paints) = tile(&area, tx, ty);
            for (t, &m) in parts[1].iter().zip(&paints) {
                let z = t.map(|v| g.verts[v as usize][2]);
                // Mud under the line halfway up, sand over it: no teeth. (A cut is kept a hair from a vertex: 1% of an edge.)
                let (low, high) = (z.iter().copied().fold(f32::MAX, f32::min), z.iter().copied().fold(f32::MIN, f32::max));
                if m == 0 {
                    assert!(high <= 0.8 + 0.02, "tile ({tx}, {ty}): mud up to {high} m");
                } else {
                    assert!(low >= 0.8 - 0.02, "tile ({tx}, {ty}): sand down to {low} m");
                }
                faces[usize::from(m)] += usize::from(high - low > 0.5);
            }
        }
        assert!(faces[0] > 20 && faces[1] > 20, "the bank's faces: {faces:?}");
    }

    #[test]
    fn cut_ground_stays_closed_inside_a_tile_and_across_its_borders() {
        let area = area_with(&paint, &steep);
        let mut border: HashMap<(usize, usize), Vec<[f32; 2]>> = HashMap::new();
        for (tx, ty) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (g, parts, _) = tile(&area, tx, ty);
            let faces: Vec<[u32; 3]> = parts.concat();
            // The tile is covered once, counter-clockwise: 100 m².
            let sum: f32 = faces.iter().map(|t| twice_area(t.map(|v| g.verts[v as usize])) / 2.0).sum();
            assert!((sum - 100.0).abs() < 1e-3, "tile ({tx}, {ty}) covers {sum} m²");
            assert!(faces.iter().all(|t| twice_area(t.map(|v| g.verts[v as usize])) > 1e-7), "no face is flat or turned over");
            // Every edge inside is two faces', one each way; one face's only on the tile's border.
            let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
            for t in &faces {
                for e in 0..3 {
                    *edges.entry((t[e], t[(e + 1) % 3])).or_default() += 1;
                }
            }
            for (&(a, b), &k) in &edges {
                let (p, q) = (g.verts[a as usize], g.verts[b as usize]);
                let on_border = |d: usize| (p[d].abs() - 5.0).abs() < 1e-4 && (q[d].abs() - 5.0).abs() < 1e-4 && p[d] == q[d];
                if on_border(0) || on_border(1) {
                    assert!(!edges.contains_key(&(b, a)));
                    let world = |v: [f32; 3]| [v[0] + tx as f32 * 10.0 + 5.0, v[1] + ty as f32 * 10.0 + 5.0];
                    border.entry((tx, ty)).or_default().extend([world(p), world(q)]);
                } else {
                    assert_eq!(
                        (k, edges.get(&(b, a))),
                        (1, Some(&1)),
                        "tile ({tx}, {ty}): an edge with a vertex in the middle: {p:?} {q:?}"
                    );
                }
            }
        }
        // The vertices along a border between two tiles are the same in both.
        let along = |t: (usize, usize), keep: &dyn Fn([f32; 2]) -> bool| {
            let mut v: Vec<[i64; 2]> = border[&t].iter().filter(|p| keep(**p)).map(|p| p.map(|c| (c * 1e4).round() as i64)).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let x10 = |p: [f32; 2]| (p[0] - 10.0).abs() < 1e-4;
        let y10 = |p: [f32; 2]| (p[1] - 10.0).abs() < 1e-4;
        assert_eq!(along((0, 0), &x10), along((1, 0), &x10));
        assert_eq!(along((0, 1), &x10), along((1, 1), &x10));
        assert_eq!(along((0, 0), &y10), along((0, 1), &y10));
        assert_eq!(along((1, 0), &y10), along((1, 1), &y10));
        assert!(along((0, 0), &x10).len() > 21 && along((0, 0), &y10).len() > 21, "the test crosses cuts at the borders");
    }

    #[test]
    fn one_paint_leaves_the_ground_as_it_was() {
        let area = area_with(&|_, _| 4, &steep);
        let (g, parts, paints) = tile(&area, 1, 0);
        assert_eq!(g.verts.len(), 21 * 21);
        assert_eq!(parts[0].len() + parts[1].len(), 800);
        assert!(paints.iter().all(|&m| m == 4));
    }
}
