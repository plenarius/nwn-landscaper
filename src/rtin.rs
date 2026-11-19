// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Walkmesh simplification: a right-triangulated irregular network (RTIN, a port of Mapbox's "Martini") over one tile.
//!
//! The engine's in-tile path search (CNWTileSurfaceMesh::IntraTileDFS) is a depth-first search over triangles under a cost bound that
//! gives up past TILE_SIZE * 4, in time slices; on dense walkmeshes it never finishes. BioWare's tiles have a median of ~45 walkmesh
//! faces. RTIN keeps big triangles where the ground is simple and splits only where the height error or the walkability changes, and
//! its triangles always meet vertex to vertex, which the engine needs (it joins triangles by shared vertices).

/// A tile's sampled ground: `size` = 2^k + 1 samples a side, row-major from the south-west, with each sample's height and whether
/// ground there is walkable.
pub struct Samples {
    /// Samples a side: 2^k + 1.
    pub size: usize,
    /// Each sample's height (m).
    pub height: Vec<f32>,
    /// Whether the ground at each sample is walkable.
    pub walkable: Vec<bool>,
}

/// The triangles RTIN keeps for a tile, by sample: few where the ground is plain, more where it changes.
pub struct Mesh {
    /// Sample (x, y) of each vertex.
    pub verts: Vec<(usize, usize)>,
    /// Counter-clockwise seen from above.
    pub tris: Vec<[u32; 3]>,
}

type P = (usize, usize);

/// Triangulates with height error at most `max_error`, also splitting any triangle whose samples disagree on walkability (down to the
/// sample spacing). With `full_border`, the tile's border is split at every sample, so neighbouring tiles built from the same samples
/// share their edge vertices exactly.
pub fn build(s: &Samples, max_error: f32, full_border: bool) -> Mesh {
    let size = s.size;
    let tile = size - 1;
    assert!(tile.is_power_of_two(), "RTIN needs 2^k + 1 samples a side");
    let idx = |p: P| p.1 * size + p.0;
    let num_triangles = tile * tile * 2 - 2;
    let num_parents = num_triangles - tile * tile;

    // Each split point's error: its own, and its children's (bottom-up).
    let mut errors = vec![0f32; size * size];
    for i in (0..num_triangles).rev() {
        let (a, b, c) = coords(i, tile);
        let m = ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
        let interpolated = (s.height[idx(a)] + s.height[idx(b)]) / 2.0;
        let mut e = (interpolated - s.height[idx(m)]).abs();
        let w = [s.walkable[idx(a)], s.walkable[idx(b)], s.walkable[idx(c)], s.walkable[idx(m)]];
        if w.iter().any(|&v| v != w[0]) {
            e = f32::INFINITY;
        }
        // A hypotenuse lying on the tile border: split it while it is longer than one sample step.
        let on_border = (a.0 == b.0 && (a.0 == 0 || a.0 == tile)) || (a.1 == b.1 && (a.1 == 0 || a.1 == tile));
        if full_border && on_border {
            e = f32::INFINITY;
        }
        let mi = idx(m);
        errors[mi] = errors[mi].max(e);
        if i < num_parents {
            let left = (((a.0 + c.0) / 2), ((a.1 + c.1) / 2));
            let right = (((b.0 + c.0) / 2), ((b.1 + c.1) / 2));
            errors[mi] = errors[mi].max(errors[idx(left)]).max(errors[idx(right)]);
        }
    }

    let mut mesh = Mesh { verts: Vec::new(), tris: Vec::new() };
    let mut index = std::collections::HashMap::new();
    let mut stack: Vec<(P, P, P)> = vec![((0, 0), (tile, tile), (tile, 0)), ((tile, tile), (0, 0), (0, tile))];
    while let Some((a, b, c)) = stack.pop() {
        let m = ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
        if a.0.abs_diff(c.0) + a.1.abs_diff(c.1) > 1 && errors[idx(m)] > max_error {
            stack.push((c, a, m));
            stack.push((b, c, m));
        } else {
            let mut v = |p: P| -> u32 {
                *index.entry(p).or_insert_with(|| {
                    mesh.verts.push(p);
                    (mesh.verts.len() - 1) as u32
                })
            };
            let t = [v(a), v(b), v(c)];
            // Make it counter-clockwise seen from above.
            let (pa, pb, pc) = (a, b, c);
            let cross =
                (pb.0 as i64 - pa.0 as i64) * (pc.1 as i64 - pa.1 as i64) - (pb.1 as i64 - pa.1 as i64) * (pc.0 as i64 - pa.0 as i64);
            mesh.tris.push(if cross >= 0 { t } else { [t[0], t[2], t[1]] });
        }
    }
    mesh
}

/// Corners (a, b, c) of triangle `i` in Martini's implicit numbering: a and b span the hypotenuse, c is the right angle.
fn coords(i: usize, tile: usize) -> (P, P, P) {
    let mut id = i + 2;
    let (mut ax, mut ay, mut bx, mut by, mut cx, mut cy) = (0, 0, 0, 0, 0, 0);
    if id & 1 == 1 {
        bx = tile;
        by = tile;
        cx = tile;
    } else {
        ax = tile;
        ay = tile;
        cy = tile;
    }
    loop {
        id >>= 1;
        if id <= 1 {
            break;
        }
        let (mx, my) = ((ax + bx) / 2, (ay + by) / 2);
        if id & 1 == 1 {
            bx = ax;
            by = ay;
            ax = cx;
            ay = cy;
        } else {
            ax = bx;
            ay = by;
            bx = cx;
            by = cy;
        }
        cx = mx;
        cy = my;
    }
    ((ax, ay), (bx, by), (cx, cy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_border_has_every_border_sample() {
        let size = 9;
        let s = Samples { size, height: vec![0.0; size * size], walkable: vec![true; size * size] };
        let m = build(&s, 0.1, true);
        for k in 0..size {
            for p in [(k, 0), (k, size - 1), (0, k), (size - 1, k)] {
                assert!(m.verts.contains(&p), "border sample {p:?} missing");
            }
        }
    }

    #[test]
    fn flat_tile_is_two_triangles() {
        let size = 17;
        let s = Samples { size, height: vec![0.0; size * size], walkable: vec![true; size * size] };
        assert_eq!(build(&s, 0.1, false).tris.len(), 2);
    }

    #[test]
    fn mesh_is_conforming_and_covers_the_tile() {
        let size = 17;
        let height: Vec<f32> = (0..size * size).map(|i| ((i % size) as f32 * 0.7).sin() * 2.0 + (i / size) as f32 * 0.1).collect();
        let s = Samples { size, height, walkable: vec![true; size * size] };
        let m = build(&s, 0.05, false);
        let area: f64 = m
            .tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| m.verts[i as usize]);
                ((b.0 as f64 - a.0 as f64) * (c.1 as f64 - a.1 as f64) - (b.1 as f64 - a.1 as f64) * (c.0 as f64 - a.0 as f64)) / 2.0
            })
            .sum();
        assert!((area - 256.0).abs() < 1e-6, "area {area}");
        // Conforming: every interior edge is shared by exactly two triangles.
        let mut edges = std::collections::HashMap::new();
        for t in &m.tris {
            for k in 0..3 {
                let (a, b) = (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]));
                *edges.entry((a, b)).or_insert(0) += 1;
            }
        }
        let on_border = |e: &(u32, u32)| {
            let (p, q) = (m.verts[e.0 as usize], m.verts[e.1 as usize]);
            (p.0 == q.0 && (p.0 == 0 || p.0 == 16)) || (p.1 == q.1 && (p.1 == 0 || p.1 == 16))
        };
        for (e, n) in &edges {
            assert!(*n == 2 || (*n == 1 && on_border(e)), "edge {e:?} used {n} times");
        }
    }
}
