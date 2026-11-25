// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Steep ground cut where its paint changes, so each piece takes one sample's texture, as the editor draws it.
//!
//! The ground's mesh has a vertex every 0.5 m ([`super::RENDER_STEP`]), the paint a sample every 0.625 m. Walkable ground takes the
//! tile's baked texture, which blends the samples as the editor does. A steep face takes one library texture whole: the sample's
//! nearest its middle (tile.rs). The editor gives each point of steep ground its nearest sample's texture (js/view.js), so its
//! paint changes on the lines halfway between two samples, where a whole face in the game crossed them. At a river, faces in the
//! bed's mud reached up past the water out of the bank's sand: a row of dark teeth along the waterline.
//!
//! [`split`] cuts each steep face over samples of different paints along those halfway lines, where the samples either side of
//! one differ. The faces beside a cut take the new vertices on their shared edges too, the neighbouring tile's included, so the
//! ground stays closed: a vertex in the middle of a neighbour's edge (a T-junction) lets the sky through as specks.

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

/// A cut: along x (0: the line x = c) or y (1), between samples `k` and `k` + 1.
type Cut = (usize, i64);

impl Area<'_> {
    fn n(&self) -> i64 {
        (self.tile / self.step).round() as i64
    }

    /// The line halfway between samples `k` and `k` + 1 (area metres).
    fn line(&self, k: i64) -> f32 {
        (k as f32 + 0.5) * self.spacing
    }

    /// The grid vertex (gi, gj) of the whole area: in tile (tx, ty)'s own coordinates, as [`super::grid`] makes it (so a vertex two
    /// tiles share is the same in both), and in the area's, with its height read again (the same in every tile).
    fn vertex(&self, (tx, ty): (usize, usize), (gi, gj): (i64, i64)) -> ([f32; 3], [f32; 3]) {
        let (i, j) = (gi - tx as i64 * self.n(), gj - ty as i64 * self.n());
        let (ox, oy) = (tx as f32 * self.tile, ty as f32 * self.tile);
        let (lx, ly) = (i as f32 * self.step, j as f32 * self.step);
        let h = (self.height)(ox + lx, oy + ly);
        ([lx - self.tile / 2.0, ly - self.tile / 2.0, h - (self.base)(tx, ty)], [ox + lx, oy + ly, h])
    }

    /// The cuts face `t` (area coordinates) needs: each halfway line across it whose samples either side differ somewhere under
    /// it. Between two cuts every sample under the face then has the same paint.
    fn cuts(&self, t: [[f32; 3]; 3]) -> Vec<Cut> {
        let s = |axis: usize| {
            let r = |v: [f32; 3]| (v[axis] / self.spacing).round() as i64;
            (t.iter().map(|&v| r(v)).min().unwrap_or(0), t.iter().map(|&v| r(v)).max().unwrap_or(0))
        };
        let ((i0, i1), (j0, j1)) = (s(0), s(1));
        let m = self.material;
        let mut cuts = Vec::new();
        cuts.extend((i0..i1).filter(|&k| (j0..=j1).any(|j| m(k, j) != m(k + 1, j))).map(|k| (0, k)));
        cuts.extend((j0..j1).filter(|&k| (i0..=i1).any(|i| m(i, k) != m(i, k + 1))).map(|k| (1, k)));
        cuts
    }
}

/// The two triangles of grid cell (i, j), counter-clockwise from above, as [`super::grid`] makes them.
fn cell(i: i64, j: i64) -> [[(i64, i64); 3]; 2] {
    [[(i, j), (i + 1, j), (i + 1, j + 1)], [(i, j), (i + 1, j + 1), (i, j + 1)]]
}

/// A point of a piece: its vertex in the grid and the cuts it lies on (by construction, not by measure).
#[derive(Clone, Copy)]
struct Point {
    id: u32,
    at: [f32; 3],
    on: [Option<i64>; 2],
}

/// Cuts the faces of `parts[steep]` (tile (tx, ty)'s steep ground, in grid `g`, which [`super::grid`] made) where their paint
/// changes, and puts the new vertices into every face of `parts` that shares an edge with a cut face, the next tile's included.
/// Every face stays in its part.
pub(super) fn split(g: &mut Grid, (tx, ty): (usize, usize), parts: &mut [Vec<[u32; 3]>], steep: usize, area: &Area<'_>) {
    let n = area.n();
    let (gx, gy) = (tx as i64 * n, ty as i64 * n);
    let global = |v: u32| (gx + i64::from(v) % (n + 1), gy + i64::from(v) / (n + 1));
    let world = |v: (i64, i64)| area.vertex((tx, ty), v).1;
    let crosses = |a: [f32; 3], b: [f32; 3], (axis, k): Cut| (a[axis] - area.line(k)) * (b[axis] - area.line(k)) < 0.0;
    // Each edge's cuts (grid vertices in the area, the lower first), from this tile's steep faces and the next tiles' across its
    // border.
    let mut on_edge: HashMap<[(i64, i64); 2], BTreeSet<Cut>> = HashMap::new();
    let mut add = |face: [(i64, i64); 3], cuts: &[Cut], only: Option<[(i64, i64); 2]>| {
        for e in 0..3 {
            let mut edge = [face[e], face[(e + 1) % 3]];
            edge.sort_unstable();
            if only.is_some_and(|o| o != edge) {
                continue;
            }
            for &c in cuts.iter().filter(|&&c| crosses(world(edge[0]), world(edge[1]), c)) {
                on_edge.entry(edge).or_default().insert(c);
            }
        }
    };
    let mut cut: HashMap<[u32; 3], Vec<Cut>> = HashMap::new();
    for &t in &parts[steep] {
        let cuts = area.cuts(t.map(|v| world(global(v))));
        if !cuts.is_empty() {
            add(t.map(global), &cuts, None);
            cut.insert(t, cuts);
        }
    }
    // The faces across the border: each border edge's other face, in the tile beside, if it is steep there.
    let (w, h) = (area.tiles.0 as i64, area.tiles.1 as i64);
    for k in 0..n {
        for (dx, dy, cell_at, which, edge) in [
            (-1, 0, (gx - 1, gy + k), 0, [(gx, gy + k), (gx, gy + k + 1)]),
            (1, 0, (gx + n, gy + k), 1, [(gx + n, gy + k), (gx + n, gy + k + 1)]),
            (0, -1, (gx + k, gy - 1), 1, [(gx + k, gy), (gx + k + 1, gy)]),
            (0, 1, (gx + k, gy + n), 0, [(gx + k, gy + n), (gx + k + 1, gy + n)]),
        ] {
            let (nx, ny) = (tx as i64 + dx, ty as i64 + dy);
            if !(0..w).contains(&nx) || !(0..h).contains(&ny) {
                continue;
            }
            let beside = (nx as usize, ny as usize);
            let face = cell(cell_at.0, cell_at.1)[which];
            if (area.steep)(beside.0, beside.1, face.map(|v| area.vertex(beside, v).0)) {
                let cuts = area.cuts(face.map(|v| area.vertex(beside, v).1));
                add(face, &cuts, Some(edge));
            }
        }
    }
    if on_edge.is_empty() {
        return;
    }
    // The points on the edges, from the edge's ends in the area's coordinates, the lower first: the same in both tiles of a border.
    // Two cuts can cross on an edge (a diagonal, every fourth sample): one point, on both.
    let origin = [tx as f32 * area.tile + area.tile / 2.0, ty as f32 * area.tile + area.tile / 2.0, (area.base)(tx, ty)];
    let mut points: HashMap<([(i64, i64); 2], Cut), Point> = HashMap::new();
    for (edge, cuts) in &on_edge {
        let (a, b) = (world(edge[0]), world(edge[1]));
        // Each point along the edge (its share of the way) and the cuts it is on.
        let mut made: Vec<(f32, Point, Vec<Cut>)> = Vec::new();
        for &(axis, k) in cuts {
            let c = area.line(k);
            let s = (c - a[axis]) / (b[axis] - a[axis]);
            if let Some((_, p, on)) = made.iter_mut().find(|(t, _, _)| (t - s).abs() < 1e-5) {
                p.on[axis] = Some(k);
                on.push((axis, k));
                continue;
            }
            let at: [f32; 3] = std::array::from_fn(|d| if d == axis { c } else { a[d] + s * (b[d] - a[d]) } - origin[d]);
            let mut on = [None; 2];
            on[axis] = Some(k);
            made.push((s, Point { id: g.verts.len() as u32, at, on }, vec![(axis, k)]));
            g.verts.push(at);
        }
        for (_, p, on) in made {
            points.extend(on.into_iter().map(|c| ((*edge, c), p)));
        }
    }
    for (part, faces) in parts.iter_mut().enumerate() {
        let mut out = Vec::with_capacity(faces.len());
        for &t in faces.iter() {
            let corners = t.map(global);
            // The polygon: the corners, each followed by the points along its edge to the next.
            let mut poly = Vec::new();
            for e in 0..3 {
                let (a, b) = (corners[e], corners[(e + 1) % 3]);
                poly.push(Point { id: t[e], at: g.verts[t[e] as usize], on: [None; 2] });
                let mut edge = [a, b];
                edge.sort_unstable();
                let Some(cuts) = on_edge.get(&edge) else { continue };
                let from = g.verts[t[e] as usize];
                let mut along: Vec<Point> = cuts.iter().map(|&c| points[&(edge, c)]).collect();
                along.sort_by(|p, q| dist2(p.at, from).total_cmp(&dist2(q.at, from)));
                along.dedup_by_key(|p| p.id);
                poly.extend(along);
            }
            if poly.len() == 3 {
                out.push(t);
                continue;
            }
            let own = if part == steep { cut.get(&t).cloned().unwrap_or_default() } else { Vec::new() };
            let plane = t.map(|v| g.verts[v as usize]);
            let mut inner: HashMap<(i64, i64), Point> = HashMap::new();
            let mut pieces = vec![poly];
            // Along x first, then y: a cut along y only meets an x cut's line inside the face.
            for &(axis, k) in &own {
                let c = area.line(k) - origin[axis];
                let mut next = Vec::new();
                for piece in pieces {
                    let (a, b) = halves(&piece, axis, k, c, &mut |p, q| {
                        let o = 1 - axis;
                        let Some(ko) = p.on[o].filter(|&ko| q.on[o] == Some(ko)) else {
                            // Every edge of the face's own that a cut crosses has its point already: only another cut's line is
                            // left. (Were one missed: where the edge crosses, a piece with no hole, if not a vertex shared.)
                            debug_assert!(false, "a cut crossed an edge with no point on it");
                            let s = (c - p.at[axis]) / (q.at[axis] - p.at[axis]);
                            let at = [0, 1, 2].map(|d| p.at[d] + s * (q.at[d] - p.at[d]));
                            g.verts.push(at);
                            let mut on = [None; 2];
                            on[axis] = Some(k);
                            return Point { id: g.verts.len() as u32 - 1, at, on };
                        };
                        let (kx, ky) = if axis == 0 { (k, ko) } else { (ko, k) };
                        *inner.entry((kx, ky)).or_insert_with(|| {
                            let (x, y) = (area.line(kx) - origin[0], area.line(ky) - origin[1]);
                            let at = [x, y, on_plane(plane, x, y)];
                            g.verts.push(at);
                            Point { id: g.verts.len() as u32 - 1, at, on: [Some(kx), Some(ky)] }
                        })
                    });
                    next.extend([a, b].into_iter().filter(|p| p.len() >= 3));
                }
                pieces = next;
            }
            for piece in pieces {
                fan(g, &piece, &mut out);
            }
        }
        *faces = out;
    }
}

/// The two sides of convex polygon `poly` across cut (`axis`, `k`) at `c` (tile coordinates). A point on the cut goes to both; an
/// edge that crosses it is a line of another cut, and `meet` gives the point where the two cuts cross.
fn halves(poly: &[Point], axis: usize, k: i64, c: f32, meet: &mut dyn FnMut(&Point, &Point) -> Point) -> (Vec<Point>, Vec<Point>) {
    let side = |p: &Point| {
        // (A point a hair from the cut is on it: two cuts crossing on an edge, met from the other cut's side.)
        if p.on[axis] == Some(k) || (p.at[axis] - c).abs() < 1e-5 {
            0
        } else if p.at[axis] < c {
            -1
        } else {
            1
        }
    };
    let (mut below, mut above) = (Vec::new(), Vec::new());
    for (e, p) in poly.iter().enumerate() {
        let q = &poly[(e + 1) % poly.len()];
        match side(p) {
            -1 => below.push(*p),
            1 => above.push(*p),
            _ => {
                below.push(*p);
                above.push(*p);
            }
        }
        if side(p) * side(q) < 0 {
            let m = meet(p, q);
            below.push(m);
            above.push(m);
        }
    }
    (below, above)
}

/// Convex polygon `poly` (counter-clockwise from above) as triangles: itself if it is one, else a fan around its middle, a new
/// vertex (fanned from a corner, a point on one of the corner's edges would be left in the middle of a face's side).
fn fan(g: &mut Grid, poly: &[Point], out: &mut Vec<[u32; 3]>) {
    if poly.len() == 3 {
        out.push([poly[0].id, poly[1].id, poly[2].id]);
        return;
    }
    let k = poly.len() as f32;
    let mid = poly.iter().fold([0.0; 3], |m, p| [m[0] + p.at[0] / k, m[1] + p.at[1] / k, m[2] + p.at[2] / k]);
    let id = g.verts.len() as u32;
    g.verts.push(mid);
    for e in 0..poly.len() {
        out.push([id, poly[e].id, poly[(e + 1) % poly.len()].id]);
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// The height at (x, y) of the plane through triangle `t`.
fn on_plane(t: [[f32; 3]; 3], x: f32, y: f32) -> f32 {
    let [a, b, c] = t;
    let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let l1 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
    let l2 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
    l1 * a[2] + l2 * b[2] + (1.0 - l1 - l2) * c[2]
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

    /// Tile (tx, ty) as tile.rs makes it: the grid, its faces in two parts (ground, steep), split.
    fn tile(area: &Area<'_>, tx: usize, ty: usize) -> (Grid, [Vec<[u32; 3]>; 2]) {
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
        split(&mut g, (tx, ty), &mut parts, 1, area);
        (g, parts)
    }

    fn twice_area(t: [[f32; 3]; 3]) -> f32 {
        (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])
    }

    #[test]
    fn every_piece_of_steep_ground_lies_over_one_paint() {
        let area = area_with(&paint, &steep);
        let (mut cut, mut whole) = (0, 0);
        for (tx, ty) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (g, parts) = tile(&area, tx, ty);
            let near = |p: [f32; 3]| {
                let (x, y) = (p[0] + tx as f32 * 10.0 + 5.0, p[1] + ty as f32 * 10.0 + 5.0);
                paint((x / 0.625).round() as i64, (y / 0.625).round() as i64)
            };
            for t in &parts[1] {
                let v = t.map(|v| g.verts[v as usize]);
                let mid = [0, 1, 2].map(|d| (v[0][d] + v[1][d] + v[2][d]) / 3.0);
                // (Each corner pulled a hair in, off the halfway lines it may lie on.)
                let paints: BTreeSet<u8> =
                    v.iter().map(|c| near([0, 1, 2].map(|d| c[d] + (mid[d] - c[d]) * 0.01))).chain([near(mid)]).collect();
                assert_eq!(paints.len(), 1, "tile ({tx}, {ty}): a steep face over {paints:?}: {v:?}");
                if i64::from(*t.iter().max().unwrap()) >= (N + 1) * (N + 1) { cut += 1 } else { whole += 1 }
            }
        }
        assert!(cut > 50 && whole > 500, "{cut} pieces, {whole} faces whole");
    }

    #[test]
    fn cut_ground_stays_closed_inside_a_tile_and_across_its_borders() {
        let area = area_with(&paint, &steep);
        let mut border: HashMap<(usize, usize), Vec<[f32; 2]>> = HashMap::new();
        for (tx, ty) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (g, parts) = tile(&area, tx, ty);
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
        let (g, parts) = tile(&area, 1, 0);
        assert_eq!(g.verts.len(), 21 * 21);
        assert_eq!(parts[0].len() + parts[1].len(), 800);
    }
}
