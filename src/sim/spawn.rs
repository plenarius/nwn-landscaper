// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where a newly created creature is put. The asked point is kept when a creature-wide horizontal segment through it is walkable and
//! clear of blocker triangles; otherwise the first such point on square rings around it, out to 20 m, that also has a walkable
//! straight line back to the asked point (when the asked point has walkable ground itself). Placeables, doors and other creatures are
//! absent offline (the oracle's creatures are ghosts).

use super::area::Planner;
use super::*;

impl<'a> Planner<'a> {
    /// The ground height at a point, approximately: the walkable (else
    /// non-walkable) triangle's plane under the point, plus the tile's z.
    pub fn ground_height(&self, x: f32, y: f32) -> f32 {
        let Some(t) = self.area.tile((x / TILE_SIZE) as i32, (y / TILE_SIZE) as i32) else { return 0.0 };
        let (cx, cy) = t.r2c_tile(x, y);
        let (ox, oy) = t.origin();
        let (lx, ly) = (cx - ox, cy - oy);
        let tri = t.mesh.find_triangle(0, 0, lx, ly, -1);
        if tri >= 0 {
            return t.z + t.mesh.plane_height(tri, lx, ly);
        }
        for d in &t.mesh.deny_tris {
            let [a, b, c] = d.map(|i| t.mesh.deny_verts[i as usize]);
            let s = |u: [f32; 3], v: [f32; 3]| (v[0] - u[0]) * (ly - u[1]) - (v[1] - u[1]) * (lx - u[0]);
            if s(a, b) >= 0.0 && s(b, c) >= 0.0 && s(c, a) >= 0.0 {
                let n = mesh::normalize([
                    (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
                    (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
                    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
                ]);
                if n[2] != 0.0 {
                    return t.z + a[2] - (n[0] * (lx - a[0]) + n[1] * (ly - a[1])) / n[2];
                }
            }
        }
        t.z
    }

    /// Whether a point is safe: a horizontal segment of the creature's diameter through the point must be walkable and clear of
    /// non-walkable faces by the radius (no 0.1 m clamp: flag 1).
    pub fn safe_spot(&self, x: f32, y: f32, z: f32) -> bool {
        let area = self.area;
        if !(0.0 <= x && x <= area.width as f32 * TILE_SIZE && 0.0 <= y && y <= TILE_SIZE * area.height as f32) {
            return false;
        }
        let rad = self.pf.rad;
        let y = y + 0.0;
        let (x1, x2) = (x - rad, x + rad);
        let mut cur = (x1, y);
        loop {
            let Some(t) = area.tile((cur.0 / TILE_SIZE) as i32, (cur.1 / TILE_SIZE) as i32) else { return false };
            let (ox, oy) = t.origin();
            let clipped = mesh::clip_to_tile(ox, oy, t.r2c_tile(x1, y), t.r2c_tile(x2, y));
            let (mut zmin, mut zmax) = (0.0f32, 0.0f32);
            let (ok, c1, c2) = self.tile_walk_line(t, (x1, y), (x2, y), &mut zmin, &mut zmax);
            if !ok {
                return false;
            }
            zmin -= 0.1;
            zmax += 0.1 + self.pf.hgt;
            if !self.tile_clear_of_blockers(t, c1.0, c1.1, c2.0, c2.1, zmin, zmax, rad, 1) {
                return false;
            }
            if let Some((_, e)) = clipped {
                let e = t.c2r_tile(e.0, e.1);
                if x2 - EPSILON <= e.0 && e.0 <= x2 + EPSILON && y - EPSILON <= e.1 && e.1 <= EPSILON + y {
                    break;
                }
            }
            cur = c2;
            while area.tile((cur.0 / TILE_SIZE) as i32, (cur.1 / TILE_SIZE) as i32).map(|n| (n.x, n.y)) == Some((t.x, t.y)) {
                cur = (cur.0 + 0.001, cur.1 + 0.001);
            }
            #[expect(clippy::neg_cmp_op_on_partial_ord, reason = "a NaN must take this branch too, as on the server")]
            if !(cur.0 <= x2) {
                break;
            }
        }
        let _ = z;
        true
    }

    /// The point itself if safe, else the first safe point on square rings of `step` spacing out to `range`.
    pub fn find_safe_spot(&self, p: (f32, f32), range: f32) -> Option<(f32, f32)> {
        let (x, y) = p;
        let z = self.ground_height(x, y);
        if self.safe_spot(x, y, z) {
            return Some(p);
        }
        let rad = self.pf.rad;
        if rad <= 0.0 {
            return None;
        }
        let require_line = self.line_walkable((f64::from(x) - 0.001) as f32, y, x, y);
        let mut step = 0.1f32;
        if 0.1 <= rad {
            step = range / 10.0;
            if rad <= range / 10.0 {
                step = rad;
            }
        }
        let rings = ((range + 0.01) / step) as i32;
        for k in 1..=rings {
            let f10 = k as f32 * step;
            let mut u = 0i32;
            loop {
                u += 1;
                let mut f11 = (u >> 1) as f32;
                if u & 1 != 0 {
                    f11 = -f11;
                }
                f11 *= step;
                for side in 0..4 {
                    let q = match side {
                        0 => (x + f11, y + f10),
                        1 => (x + f10, y - f11),
                        2 => (x - f11, y - f10),
                        _ => (x - f10, y + f11),
                    };
                    let zq = self.ground_height(q.0, q.1);
                    if self.safe_spot(q.0, q.1, zq) && (!require_line || self.line_walkable(q.0, q.1, x, y)) {
                        return Some(q);
                    }
                }
                if k * 2 <= u {
                    break;
                }
            }
        }
        None
    }
}
