// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! One tile's walkmesh, as the planner sees it.
//!
//! Loading welds the .wok's vertices (after snapping them to whole metres when they are within about a millimetre), splits the faces
//! into walkable triangles and non-walkable ("blocker") triangles, finds which walkable triangles share an edge, and subdivides every
//! walkable triangle twice into four, giving three levels of detail (a level-2 triangle id carries its level-1 and level-0 parents in
//! its upper bits).
//!
//! The queries: which triangle holds a point (at any level), the ground height under a point, walking a straight line from triangle to
//! triangle until it reaches its end or leaves the walkable ground, and whether a creature-wide band along a line touches a blocker
//! triangle.
//!
//! Everything is in the tile's canonical local frame (x, y in 0..10), in f32, with the operations in a fixed order: the route depends
//! on the exact rounding, so expressions are not to be "simplified".

use super::{MAX_TILE_EPS, MIN_TILE_EPS, TRIADJ_NO_EDGE, TRIADJ_TILE_END};

/// A point or vector (m).
pub type V3 = [f32; 3];

/// One tile's walkmesh.
pub struct Mesh {
    /// Path node letter and orientation (which table the tile uses).
    pub letter: u8,
    /// The path node table's orientation, in quarter turns.
    pub pn_orient: u8,
    /// Vertex positions: the welded walkable vertices, then the midpoints
    /// each subdivision adds, so one array serves every level's indices.
    pub verts: Vec<V3>,
    /// Triangle vertex indices and adjacency, levels 0, 1, 2.
    pub tris: [Vec<[i32; 3]>; 3],
    /// Each triangle's neighbour across each edge, by level: an index, [`TRIADJ_NO_EDGE`] or [`TRIADJ_TILE_END`].
    pub adj: [Vec<[i32; 3]>; 3],
    /// Level-0 plane per walkable triangle.
    pub normal: Vec<V3>,
    /// The plane's offset per walkable triangle: `normal · p + plane_d` is 0 on it.
    pub plane_d: Vec<f32>,
    /// Non-walkable ("denied") vertices and triangles.
    pub deny_verts: Vec<V3>,
    /// The non-walkable triangles, as indices into `deny_verts`.
    pub deny_tris: Vec<[i32; 3]>,
}

/// Normalises a vector, leaving one already of length about 1 as it is and turning the zero vector into (1, 0, 0).
pub fn normalize(v: V3) -> V3 {
    let s = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if s == 1.0 || (0.999_99..=1.000_01).contains(&s) {
        return v;
    }
    if s == 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let k = (1.0 / s).sqrt();
    [v[0] * k, v[1] * k, v[2] * k]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Vertex snap on load: within ~1/1024 of an integer -> that integer.
fn snap(c: f32) -> f32 {
    let i = (512.0f32 * c + 0.5).trunc() as i32;
    if i & 0x1ff == 0 { (i >> 9) as f32 } else { c }
}

impl Mesh {
    /// Loads a tile-centred walkmesh (the .wok text's `verts`, which our writer prints round-trip exactly) and faces with their
    /// surfacemat Walk flag. Position 0 0 0, identity orientation.
    pub fn load(letter: u8, pn_orient: u8, file_verts: &[V3], faces: &[([u32; 3], bool)]) -> Mesh {
        // Offset (px + 5, py + 5, pz), snap, weld by exact equality.
        let mut welded: Vec<V3> = Vec::new();
        let mut remap = Vec::with_capacity(file_verts.len());
        for v in file_verts {
            let p = [snap(v[0] + 5.0), snap(v[1] + 5.0), snap(v[2] + 0.0)];
            match welded.iter().position(|w| w[0] == p[0] && w[1] == p[1] && w[2] == p[2]) {
                Some(i) => remap.push(i),
                None => {
                    remap.push(welded.len());
                    welded.push(p);
                }
            }
        }
        // Walk split: each class renumbers the vertices it uses, in order.
        let mut walk_used = vec![false; welded.len()];
        let mut deny_used = vec![false; welded.len()];
        for (f, w) in faces {
            for &i in f {
                if *w { walk_used[remap[i as usize]] = true } else { deny_used[remap[i as usize]] = true }
            }
        }
        let number = |used: &[bool]| {
            let mut map = vec![-1i32; used.len()];
            let mut out = Vec::new();
            for (i, &u) in used.iter().enumerate() {
                if u {
                    map[i] = out.len() as i32;
                    out.push(welded[i]);
                }
            }
            (map, out)
        };
        let (walk_map, walk_verts) = number(&walk_used);
        let (deny_map, deny_verts) = number(&deny_used);
        let mut tris = Vec::new();
        let mut deny_tris = Vec::new();
        for (f, w) in faces {
            if *w {
                tris.push(f.map(|i| walk_map[remap[i as usize]]));
            } else {
                deny_tris.push(f.map(|i| deny_map[remap[i as usize]]));
            }
        }
        let mut normal = Vec::with_capacity(tris.len());
        let mut plane_d = Vec::with_capacity(tris.len());
        for t in &tris {
            let [a, b, c] = t.map(|i| walk_verts[i as usize]);
            let n = normalize(cross(sub(b, a), sub(c, b)));
            plane_d.push(-((n[0] * a[0] + n[1] * a[1]) + n[2] * a[2]));
            normal.push(n);
        }
        let adj0 = triangle_adjacency(&walk_verts, &tris);
        let mut m = Mesh {
            letter,
            pn_orient,
            verts: walk_verts,
            tris: [tris, Vec::new(), Vec::new()],
            adj: [adj0, Vec::new(), Vec::new()],
            normal,
            plane_d,
            deny_verts,
            deny_tris,
        };
        m.subdivide();
        m
    }

    /// Builds levels 1 and 2 from level 0.
    fn subdivide(&mut self) {
        for l in 0..2 {
            let (t, a) = subdivide_level(&mut self.verts, &self.tris[l], &self.adj[l]);
            self.tris[l + 1] = t;
            self.adj[l + 1] = a;
        }
    }

    /// How many triangles level `level` has (each level splits every triangle in four).
    pub fn tri_count(&self, level: usize) -> usize {
        self.tris[level].len()
    }

    /// Vertex `i`.
    pub fn v(&self, i: i32) -> V3 {
        self.verts[i as usize]
    }

    /// Triangle `t`'s corners at level `level`.
    pub fn tri_verts(&self, level: usize, t: i32) -> [V3; 3] {
        self.tris[level][t as usize].map(|i| self.v(i))
    }

    /// The triangle holding (x, y): with `hint` -1, the first level-`level` triangle that contains it (edges inclusive,
    /// counter-clockwise faces only), else the child of `hint` one level down; then `depth` more steps down through the children. -1
    /// when no triangle holds it.
    pub fn find_triangle(&self, level: i32, depth: i32, x: f32, y: f32, hint: i32) -> i32 {
        let (mut level, mut hint) = (level, hint);
        let mut it = 0;
        loop {
            let lv = level.clamp(0, 2) as usize;

            let res = if hint == -1 {
                let mut found = -1;
                for t in 0..self.tris[lv].len() {
                    let [a, b, c] = self.tri_verts(lv, t as i32);
                    if (a[1] - b[1]) * (x - a[0]) + (y - a[1]) * (b[0] - a[0]) >= 0.0
                        && (b[1] - c[1]) * (x - b[0]) + (c[0] - b[0]) * (y - b[1]) >= 0.0
                        && (a[0] - c[0]) * (y - c[1]) + (c[1] - a[1]) * (x - c[0]) >= 0.0
                    {
                        found = t as i32;
                        break;
                    }
                }
                if found == -1 {
                    return -1;
                }
                found
            } else {
                let c = 4 * hint;
                let [a, b, cc] = self.tri_verts(lv, c + 2);
                let mut r = c;
                if (a[1] - b[1]) * (x - a[0]) + (y - a[1]) * (b[0] - a[0]) >= 0.0 {
                    r = c + 1;
                    if (b[1] - cc[1]) * (x - b[0]) + (cc[0] - b[0]) * (y - b[1]) >= 0.0 {
                        r = c + 2;
                        if (a[0] - cc[0]) * (y - cc[1]) + (cc[1] - a[1]) * (x - cc[0]) < 0.0 {
                            r = c + 3;
                        }
                    }
                }
                r
            };
            if it == depth {
                return res;
            }
            it += 1;
            level += 1;
            hint = res;
        }
    }

    /// The centroid of a level-2 triangle.
    pub fn centroid(&self, id: i32) -> V3 {
        let [a, b, c] = self.tri_verts(2, id);
        [((a[0] + b[0]) + c[0]) / 3.0, ((a[1] + b[1]) + c[1]) / 3.0, ((a[2] + b[2]) + c[2]) / 3.0]
    }

    /// The height of level-0 triangle t's plane at (x, y).
    pub fn plane_height(&self, t: i32, x: f32, y: f32) -> f32 {
        let n = self.normal[t as usize];
        let d = self.plane_d[t as usize];
        if n[2] != 0.0 { -(((n[0] * x) + d) + n[1] * y) / n[2] } else { 0.0 }
    }

    /// Walks the part of P->Q inside this tile from triangle to neighbour over the level-0 walkable triangles: ok when it reaches Q or
    /// the tile edge, not when it starts off the walkmesh or leaves it. P, Q in area coords (canonical), (ox, oy) the tile origin.
    /// Returns (ok, c1, c2), the clipped segment; zmin/zmax collect the heights met and are only written once the walk has started (a
    /// caller may see earlier values).
    pub fn walk_line(
        &self,
        ox: f32,
        oy: f32,
        p: (f32, f32),
        q: (f32, f32),
        zmin: &mut f32,
        zmax: &mut f32,
    ) -> (bool, (f32, f32), (f32, f32)) {
        let Some((c1, c2)) = clip_to_tile(ox, oy, p, q) else {
            return (false, (0.0, 0.0), (0.0, 0.0));
        };
        let mut pp = (c1.0 - ox, c1.1 - oy);
        let e = (q.0 - ox, q.1 - oy);
        let mut t = self.find_triangle(0, 0, pp.0, pp.1, -1);
        let mut zp = if t >= 0 { self.plane_height(t, pp.0, pp.1) } else { 0.0 };
        let mut it = 0;
        while t >= 0 && it < 512 {
            it += 1;
            let vs = self.tri_verts(0, t);
            let a = self.adj[0][t as usize];
            if it == 1 {
                *zmin = zp;
                *zmax = zp;
            } else if zp >= *zmax {
                *zmax = zp;
            } else if zp <= *zmin {
                *zmin = zp;
            }
            let mut inside = [false; 3];
            for k in 0..3 {
                let (vk, vk1) = (vs[k], vs[(k + 1) % 3]);
                inside[k] = (vk[1] - vk1[1]) * (e.0 - vk[0]) + (vk1[0] - vk[0]) * (e.1 - vk[1]) >= 0.0;
            }
            let cnt = inside.iter().filter(|&&b| b).count();
            if cnt == 0 {
                return (false, c1, c2);
            }
            if cnt == 3 {
                let ze = self.plane_height(t, e.0, e.1);
                if *zmax <= ze {
                    *zmax = ze;
                } else if *zmin < ze {
                } else {
                    *zmin = ze;
                }
                return (true, c1, c2);
            }
            let from = [pp.0, pp.1, zp];
            let to = [e.0, e.1, 0.0];
            let mut next = None;
            if cnt == 2 {
                let k = (0..3).find(|&k| !inside[k]).expect("two corners of three are inside, so one is not");
                match intersect_line_segments_3d(from, to, vs[k], vs[(k + 1) % 3]) {
                    Some(h) => next = Some((k, h)),
                    None => return (false, c1, c2),
                }
            } else {
                for k in [2usize, 1, 0] {
                    if inside[k] {
                        continue;
                    }
                    if let Some(h) = intersect_line_segments_3d(from, to, vs[k], vs[(k + 1) % 3]) {
                        next = Some((k, h));
                        break;
                    }
                }
                if next.is_none() {
                    return (false, c1, c2);
                }
            }
            let (k, h) = next.expect("found above, or returned");
            t = a[k];
            pp = (h[0], h[1]);
            zp = h[2];
        }
        if it < 512 && t == TRIADJ_TILE_END {
            if zp >= *zmax {
                *zmax = zp;
            } else if zp <= *zmin {
                *zmin = zp;
            }
            return (true, c1, c2);
        }
        (false, c1, c2)
    }

    /// True when no blocker triangle within the z range [zl, zh] touches the rectangle swept by a band of half-width `w` along (x1,
    /// y1)->(x2, y2). With `flag` 0 a half-width over 0.1 and up to 0.65 counts as 0.1 (so ordinary creatures pass close to walls);
    /// `flag` 1 keeps it. Tile-local canonical coordinates.
    pub fn clear_of_blockers(&self, x1: f32, y1: f32, x2: f32, y2: f32, zl: f32, zh: f32, w: f32, flag: i32) -> bool {
        let mut w = w;
        if flag == 0 && 0.1 < w && w <= 0.65 {
            w = 0.1;
        }
        if (x1 == x2 && y1 == y2) || zl > zh {
            return true;
        }
        let dx = x2 - x1;
        let dy = y2 - y1;
        let l = (dy * dy + dx * dx).sqrt();
        let uy = (dy / l) * w;
        let ux = w * (dx / l);
        let xs = [x1 - uy, (x2 - uy) + ux, (x2 + uy) + ux, uy + x1];
        let ys = [y1 + ux, (y2 + ux) + uy, (y2 - ux) + uy, y1 - ux];
        let (max_x, max_y, min_x, min_y);
        if x2 < x1 {
            let m = y1 <= y2;
            min_x = if m { xs[1] } else { xs[2] };
            min_y = if m { ys[0] } else { ys[1] };
            max_x = if m { xs[3] } else { xs[0] };
            max_y = if m { ys[2] } else { ys[3] };
        } else {
            let b = if y1 <= y2 { 1 } else { 0 };
            max_y = ys[b];
            max_x = xs[b + 1];
            min_x = xs[(b + 3) & 3];
            min_y = ys[b + 2];
        }
        // There is deliberately no test for a triangle lying wholly inside
        // the rectangle: as on the server, such a triangle does not block.
        for tri in &self.deny_tris {
            let tv = tri.map(|i| self.deny_verts[i as usize]);
            let any = |f: &dyn Fn(&V3) -> bool| tv.iter().any(f);
            if !(any(&|v| v[0] <= max_x)
                && any(&|v| v[1] <= max_y)
                && any(&|v| v[0] >= min_x)
                && any(&|v| v[1] >= min_y)
                && any(&|v| v[2] <= zh)
                && any(&|v| v[2] >= zl))
            {
                continue;
            }
            let mut ef = [true; 3];
            for r in 0..4 {
                let r1 = (r + 1) & 3;
                let mut vin = [true; 3];
                for v in 0..3 {
                    let side = (tv[v][0] - xs[r]) * (ys[r1] - ys[r]) + (tv[v][1] - ys[r]) * (xs[r] - xs[r1]);
                    #[expect(clippy::neg_cmp_op_on_partial_ord, reason = "a NaN side counts as inside, as on the server")]
                    let inside = !(side < 0.0);
                    vin[v] = inside;
                }
                for e in 0..3 {
                    if !vin[e] && !vin[(e + 1) % 3] {
                        ef[e] = false;
                    }
                }
            }
            for e in 0..3 {
                if ef[e] {
                    for r in 0..4 {
                        let r1 = (r + 1) & 3;
                        if intersect_line_segments_3d([xs[r], ys[r], 0.0], [xs[r1], ys[r1], 0.0], tv[e], tv[(e + 1) % 3]).is_some() {
                            return false;
                        }
                    }
                }
            }
            let (t0, t1, t2) = (tv[0], tv[1], tv[2]);
            let (cx, cy) = (xs[0], ys[0]);
            if (cx - t0[0]) * (t0[1] - t1[1]) + (t1[0] - t0[0]) * (cy - t0[1]) >= 0.0
                && (t1[1] - t2[1]) * (cx - t1[0]) + (t2[0] - t1[0]) * (cy - t1[1]) >= 0.0
                && (t2[1] - t0[1]) * (cx - t2[0]) + (cy - t2[1]) * (t0[0] - t2[0]) >= 0.0
            {
                return false;
            }
        }
        true
    }
}

/// Which triangle is across each edge: matched by welded vertex index; an edge on the tile's border is TILE_END, an unmatched one
/// NO_EDGE; on an edge shared by more than two triangles the last partner wins.
pub fn triangle_adjacency(verts: &[V3], tris: &[[i32; 3]]) -> Vec<[i32; 3]> {
    let border = |v: V3| v[0] <= MIN_TILE_EPS || v[1] <= MIN_TILE_EPS || v[0] >= MAX_TILE_EPS || v[1] >= MAX_TILE_EPS;
    let mut adj = vec![[TRIADJ_NO_EDGE; 3]; tris.len()];
    for i in 0..tris.len() {
        for e in 0..3 {
            if adj[i][e] != TRIADJ_NO_EDGE {
                continue;
            }
            let (a, b) = (tris[i][e], tris[i][(e + 1) % 3]);
            let (va, vb) = (verts[a as usize], verts[b as usize]);
            if border(va)
                && border(vb)
                && ((va[0] <= MIN_TILE_EPS && vb[0] <= MIN_TILE_EPS)
                    || (va[0] >= MAX_TILE_EPS && vb[0] >= MAX_TILE_EPS)
                    || (va[1] <= MIN_TILE_EPS && vb[1] <= MIN_TILE_EPS)
                    || (va[1] >= MAX_TILE_EPS && vb[1] >= MAX_TILE_EPS))
            {
                adj[i][e] = TRIADJ_TILE_END;
                continue;
            }
            for j in i + 1..tris.len() {
                for k in 0..3 {
                    let (c, d) = (tris[j][k], tris[j][(k + 1) % 3]);
                    if (c == a && d == b) || (c == b && d == a) {
                        adj[j][k] = i as i32;
                        adj[i][e] = j as i32;
                    }
                }
            }
        }
    }
    adj
}

/// One subdivision step: each parent splits into 4 (child 2 is the middle one), with new midpoints per parent (not shared with the
/// neighbour); the children's adjacency follows from the parents'.
fn subdivide_level(verts: &mut Vec<V3>, ptris: &[[i32; 3]], padj: &[[i32; 3]]) -> (Vec<[i32; 3]>, Vec<[i32; 3]>) {
    let m = verts.len() as i32;
    let mid = |a: V3, b: V3| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5];
    let mut tris = Vec::with_capacity(ptris.len() * 4);
    let mut adj = vec![[0i32; 3]; ptris.len() * 4];
    for (i, t) in ptris.iter().enumerate() {
        let (v0, v1, v2) = (verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
        verts.push(mid(v0, v1));
        verts.push(mid(v0, v2));
        verts.push(mid(v2, v1));
        let i32i = i as i32;
        let (m01, m02, m12) = (m + 3 * i32i, m + 3 * i32i + 1, m + 3 * i32i + 2);
        tris.push([t[0], m01, m02]);
        tris.push([m01, t[1], m12]);
        tris.push([m02, m01, m12]);
        tris.push([m02, m12, t[2]]);
        let b = 4 * i;
        adj[b][1] = b as i32 + 2;
        adj[b + 1][2] = b as i32 + 2;
        adj[b + 3][0] = b as i32 + 2;
        adj[b + 2] = [b as i32, b as i32 + 1, b as i32 + 3];
        for k in 0..3 {
            let a = padj[i][k];
            let (first, second) = if a < 0 {
                (a, a)
            } else if padj[a as usize][0] == i32i {
                (4 * a + 1, 4 * a)
            } else if padj[a as usize][1] == i32i {
                (4 * a + 3, 4 * a + 1)
            } else {
                (4 * a, 4 * a + 3)
            };
            match k {
                0 => {
                    adj[b][0] = first;
                    adj[b + 1][0] = second;
                }
                1 => {
                    adj[b + 1][1] = first;
                    adj[b + 3][1] = second;
                }
                _ => {
                    adj[b + 3][2] = first;
                    adj[b][2] = second;
                }
            }
        }
    }
    (tris, adj)
}

/// The part of P->Q inside the tile square at (ox, oy), or None.
pub fn clip_to_tile(ox: f32, oy: f32, p: (f32, f32), q: (f32, f32)) -> Option<((f32, f32), (f32, f32))> {
    let (x0, x1s, y0, y1s) = (ox, ox + 10.0, oy, oy + 10.0);
    let (x1, y1) = p;
    let (x2, y2) = q;
    let inside = |x: f32, y: f32| x0 <= x && x <= x1s && y0 <= y && y <= y1s;
    let c1 = if inside(x1, y1) {
        Some(p)
    } else {
        let mut r = None;
        if x1 != x2 {
            let xb = if x2 < x1 { x1s } else { x0 };
            let yb = ((y2 - y1) / (x2 - x1)) * (xb - x1) + y1;
            if y0 <= yb && yb <= y1s {
                r = Some((xb, yb));
            }
        }
        if r.is_none() && y1 != y2 {
            let yb = if y2 < y1 { y1s } else { y0 };
            let xb = ((x2 - x1) / (y2 - y1)) * (yb - y1) + x1;
            if x0 <= xb && xb <= x1s {
                r = Some((xb, yb));
            }
        }
        r
    };
    let c2 = if inside(x2, y2) {
        Some(q)
    } else {
        let mut r = None;
        if x1 != x2 {
            let xe = if x1 < x2 { x1s } else { x0 };
            let ye = ((y2 - y1) / (x2 - x1)) * (xe - x1) + y1;
            if y0 <= ye && ye <= y1s {
                r = Some((xe, ye));
            }
        }
        if r.is_none() && y1 != y2 {
            let ye = if y1 < y2 { y1s } else { y0 };
            let xe = ((x2 - x1) / (y2 - y1)) * (ye - y1) + x1;
            if x0 <= xe && xe <= x1s {
                r = Some((xe, ye));
            }
        }
        r
    };
    Some((c1?, c2?))
}

fn in_range(v: f32, a: f32, b: f32) -> bool {
    (a <= b && a <= v && v <= b) || (b <= a && b <= v && v <= a)
}

/// "Within [a, b], unless b < a": the range test the segment intersection applies to one segment's span (deliberately not symmetric).
fn in_x(v: f32, a: f32, b: f32) -> bool {
    (b < a || (a <= v && v <= b)) && (a < b || (b <= v && v <= a))
}

/// Where segment P (the walked line) crosses segment Q (a triangle edge), with z interpolated along Q.
pub fn intersect_line_segments_3d(p1: V3, p2: V3, q1: V3, q2: V3) -> Option<V3> {
    let (hx, hy);
    if p1[0] == p2[0] {
        if q2[0] == q1[0] {
            return None;
        }
        hx = p1[0];
        let m = (q2[1] - q1[1]) / (q2[0] - q1[0]);
        hy = m * p1[0] + (q1[1] - m * q1[0]);
        if !(in_range(hy, q1[1], q2[1]) && in_x(p1[0], q1[0], q2[0]) && in_x(hy, p1[1], p2[1])) {
            return None;
        }
    } else {
        let mp = (p2[1] - p1[1]) / (p2[0] - p1[0]);
        if q2[0] == q1[0] {
            hx = q1[0];
            hy = mp * q1[0] + (p1[1] - p1[0] * mp);
            if !(in_range(hy, p1[1], p2[1]) && in_x(q1[0], p1[0], p2[0]) && in_x(hy, q1[1], q2[1])) {
                return None;
            }
        } else {
            let mq = (q2[1] - q1[1]) / (q2[0] - q1[0]);
            if mq == mp {
                return None;
            }
            let bq = q1[1] - mq * q1[0];
            hx = (1.0 / (mp - mq)) * (bq - (p1[1] - p1[0] * mp));
            hy = mq * hx + bq;
            if !(in_range(hx, q1[0], q2[0]) && in_x(hx, p1[0], p2[0])) {
                return None;
            }
        }
    }
    let s = if (q1[0] - q2[0]).abs() < (q1[1] - q2[1]).abs() { (hy - q1[1]) / (q2[1] - q1[1]) } else { (hx - q1[0]) / (q2[0] - q1[0]) };
    Some([hx, hy, q1[2] + (q2[2] - q1[2]) * s])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles over the unit square at (2..4, 2..4), CCW.
    fn square() -> Mesh {
        let v = [[-3.0, -3.0, 0.0], [-1.0, -3.0, 0.0], [-1.0, -1.0, 0.0], [-3.0, -1.0, 0.0]];
        Mesh::load(b'A', 0, &v, &[([0, 1, 2], true), ([0, 2, 3], true)])
    }

    #[test]
    fn find_triangle_ccw_inclusive() {
        let m = square();
        assert_eq!(m.find_triangle(0, 0, 3.5, 2.5, -1), 0);
        assert_eq!(m.find_triangle(0, 0, 2.5, 3.5, -1), 1);
        // On the shared diagonal: the lower index wins.
        assert_eq!(m.find_triangle(0, 0, 3.0, 3.0, -1), 0);
        assert_eq!(m.find_triangle(0, 0, 5.0, 5.0, -1), -1);
        // Clockwise triangles are never found (all edge tests flip sign).
        let v = [[-3.0, -3.0, 0.0], [-1.0, -1.0, 0.0], [-1.0, -3.0, 0.0]];
        let cw = Mesh::load(b'A', 0, &v, &[([0, 1, 2], true)]);
        assert_eq!(cw.find_triangle(0, 0, 3.5, 2.5, -1), -1);
    }

    #[test]
    fn find_triangle_descends_to_level_2() {
        let m = square();
        let id = m.find_triangle(0, 2, 3.9, 2.05, -1);
        assert_eq!(id >> 4, 0);
        let [a, b, c] = m.tri_verts(2, id);
        let inside = |p: (f32, f32)| {
            let s = |u: V3, v: V3| (v[0] - u[0]) * (p.1 - u[1]) - (v[1] - u[1]) * (p.0 - u[0]);
            s(a, b) >= 0.0 && s(b, c) >= 0.0 && s(c, a) >= 0.0
        };
        assert!(inside((3.9, 2.05)));
    }

    #[test]
    fn a_shared_edge_is_adjacent_at_every_subdivision_level() {
        let m = square();
        // Diagonal 0-2 is shared: tri 0 edge 2 (2->0), tri 1 edge 0 (0->2).
        assert_eq!(m.adj[0][0], [TRIADJ_NO_EDGE, TRIADJ_NO_EDGE, 1]);
        assert_eq!(m.adj[0][1], [0, TRIADJ_NO_EDGE, TRIADJ_NO_EDGE]);
        assert_eq!(m.tri_count(1), 8);
        assert_eq!(m.tri_count(2), 32);
        // Children across the diagonal point at each other.
        for (l, t) in m.adj[1].iter().enumerate() {
            for &a in t {
                if a >= 0 {
                    assert!(m.adj[1][a as usize].contains(&(l as i32)), "level 1 {l} -> {a}");
                }
            }
        }
    }

    #[test]
    fn an_edge_on_the_tile_border_is_a_tile_end() {
        let v = [[-5.0, -5.0, 0.0], [5.0, -5.0, 0.0], [5.0, 5.0, 0.0]];
        let m = Mesh::load(b'A', 0, &v, &[([0, 1, 2], true)]);
        assert_eq!(m.adj[0][0], [TRIADJ_TILE_END, TRIADJ_TILE_END, TRIADJ_NO_EDGE]);
    }

    /// A walkable 4x4 m square at (2..6, 2..6) and a non-walkable triangle north of it, 0.2 m above the square's top edge.
    fn square_and_wall() -> Mesh {
        let v =
            [[-3.0, -3.0, 0.0], [1.0, -3.0, 0.0], [1.0, 1.0, 0.0], [-3.0, 1.0, 0.0], [-3.0, 1.2, 0.0], [1.0, 1.2, 0.0], [-1.0, 3.0, 2.0]];
        Mesh::load(b'A', 0, &v, &[([0, 1, 2], true), ([0, 2, 3], true), ([4, 5, 6], false)])
    }

    #[test]
    fn a_line_walks_only_while_it_stays_on_the_walkmesh() {
        let m = square_and_wall();
        let (mut zl, mut zh) = (0.0, 0.0);
        // Inside the square, across the diagonal.
        let (ok, c1, c2) = m.walk_line(0.0, 0.0, (2.5, 2.5), (5.5, 5.0), &mut zl, &mut zh);
        assert!(ok);
        assert_eq!((c1, c2), ((2.5, 2.5), (5.5, 5.0)));
        // Leaving the square through an interior edge with no neighbour.
        assert!(!m.walk_line(0.0, 0.0, (3.0, 3.0), (3.0, 7.0), &mut zl, &mut zh).0);
        // Starting off the walkmesh.
        assert!(!m.walk_line(0.0, 0.0, (1.0, 1.0), (3.0, 3.0), &mut zl, &mut zh).0);
    }

    #[test]
    fn a_small_radius_is_clamped_past_blockers_unless_asked_not_to() {
        let m = square_and_wall();
        // A line 0.3 m below the wall's bottom edge (y = 6.2).
        let (y, zl, zh) = (5.9, -1.0, 3.0);
        // Radius 0.4 is clamped to 0.1 (flag 0): clear.
        assert!(m.clear_of_blockers(2.5, y, 5.5, y, zl, zh, 0.4, 0));
        // Unclamped (flag 1, as the spawn test asks): blocked.
        assert!(!m.clear_of_blockers(2.5, y, 5.5, y, zl, zh, 0.4, 1));
        // Above 0.65 there is no clamp.
        assert!(!m.clear_of_blockers(2.5, y, 5.5, y, zl, zh, 0.7, 0));
        // The z range must overlap the triangle's.
        assert!(m.clear_of_blockers(2.5, y, 5.5, y, 2.5, 3.0, 0.4, 1));
    }

    #[test]
    fn clip_to_tile_square() {
        // From the west neighbour into the tile at (10..20, 0..10).
        let (c1, c2) = clip_to_tile(10.0, 0.0, (5.0, 5.0), (15.0, 5.0)).unwrap();
        assert_eq!((c1, c2), ((10.0, 5.0), (15.0, 5.0)));
        // Through the tile, clipped at both ends.
        let (c1, c2) = clip_to_tile(10.0, 0.0, (5.0, 2.0), (25.0, 2.0)).unwrap();
        assert_eq!((c1, c2), ((10.0, 2.0), (20.0, 2.0)));
        // Missing the tile.
        assert!(clip_to_tile(10.0, 0.0, (5.0, 12.0), (25.0, 14.0)).is_none());
    }

    #[test]
    fn intersect_3d_interpolates_z() {
        let h = intersect_line_segments_3d([0.0, 0.0, 0.0], [2.0, 2.0, 0.0], [0.0, 2.0, 1.0], [2.0, 0.0, 3.0]).unwrap();
        assert_eq!((h[0], h[1], h[2]), (1.0, 1.0, 2.0));
        assert!(intersect_line_segments_3d([0.0, 0.0, 0.0], [0.5, 0.5, 0.0], [0.0, 2.0, 1.0], [2.0, 0.0, 3.0]).is_none());
    }
}
