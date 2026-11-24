// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The path across one tile, from where the creature enters to a goal box: the goal point itself on the route's last tile, else a box
//! on the exit edge chosen among five candidate points by how much free room each has.
//!
//! The search is IDA*: depth-first from the start triangle with a cost bound (the estimate plus 10 m, raised by 5 m a round up to 40
//! m), the estimate being the straight distance to the goal box, with a small table of the best remaining bound seen per triangle.
//! From a point, the first successor tried is the straight line to the goal (or a corner of the goal box); otherwise it steps over the
//! edges of the point's triangle at each level of detail to the neighbour's centroid (or to the edge midpoint on the tile's border),
//! best first. Far from where the walk started ("coarse" mode, for NPCs) only level 0 is used and the steps aren't line-tested.

use super::area::Planner;
use super::mesh::Mesh;
use super::*;

/// One successor of the in-tile search.
#[derive(Clone, Copy, Debug)]
struct MeshStep {
    /// Edge point (stored in the path).
    px: f32,
    py: f32,
    /// Next node position.
    nx: f32,
    ny: f32,
    cost: f32,
    /// Next node, level-2 id.
    tri: i32,
}

/// An entry of the search's table of bounds per triangle: two slots.
#[derive(Clone, Copy)]
struct HashEntry {
    key0: i32,
    key1: i32,
    val0: f32,
    val1: f32,
}

impl HashEntry {
    const NEW: HashEntry = HashEntry { key0: -1, key1: -1, val0: -1.0, val1: -1.0 };

    fn fetch(&self, k: i32) -> f32 {
        if self.key0 == k {
            self.val0
        } else if self.key1 == k {
            self.val1
        } else {
            0.0
        }
    }

    fn store(&mut self, k: i32, v: f32) {
        if v >= self.val0 {
            self.key0 = k;
            self.val0 = v;
        } else {
            self.key1 = k;
            self.val1 = v;
        }
    }
}

fn bucket(id: i32) -> usize {
    (((id >> 8) + id) & 127) as usize
}

/// The in-tile search's state for one call.
struct Search<'m> {
    mesh: &'m Mesh,
    /// Tile origin (ox, oy).
    o: (f32, f32),
    gx: f32,
    gy: f32,
    xmin: f32,
    xmax: f32,
    ymin: f32,
    ymax: f32,
    goal_tri: i32,
    bound: f32,
    /// Index of the last valid point.
    last: usize,
    /// `path[2d]` = node d, `path[2d+1]` = edge point after node d.
    path: Vec<(f32, f32)>,
    used: i64,
    hash: Vec<HashEntry>,
    /// Which edges a coarser level has already offered, per level (so a finer triangle on the same edge isn't offered again).
    s: [[i32; 3]; 4],
    coarse: bool,
    rad: f32,
    hgt: f32,
    range: f32,
    debug: bool,
    /// In-tile line tests made (a proxy for real time).
    lines: std::cell::Cell<u64>,
    /// Abandon the search past this many line tests (the call cap).
    line_limit: u64,
    /// Abandon the search past this many work units ([`crate::sim::Query::intra_cap`]).
    work_limit: i64,
}

impl Search<'_> {
    /// The search's estimate: the distance to the goal box.
    fn estimate(&self, x: f32, y: f32) -> f32 {
        let mut cx = if x <= self.xmax { x } else { self.xmax };
        cx = if self.xmin <= x { cx } else { self.xmin };
        let mut cy = if y <= self.ymax { y } else { self.ymax };
        cy = if self.ymin <= y { cy } else { self.ymin };
        ((cx - x) * (cx - x) + (cy - y) * (cy - y)).sqrt()
    }

    /// `line_walkable_in_tile`, counted; tile-local canonical.
    fn test_line_on_tile(&self, x1: f32, y1: f32, x2: f32, y2: f32, check_end: bool) -> bool {
        self.lines.set(self.lines.get() + 1);
        line_walkable_in_tile(self.mesh, self.o, self.rad, self.hgt, x1, y1, x2, y2, check_end)
    }

    /// The test for placeables, doors and creatures on a line. None exist offline, so it always passes.
    fn test_line_on_area(&self) -> bool {
        true
    }

    /// One depth-first step of the in-tile search, uninterrupted (no time
    /// slicing; the line-test limit stands for running out of time).
    fn dfs(&mut self, depth: usize, g: f32, tri: i32, x: f32, y: f32) -> i32 {
        if self.lines.get() > self.line_limit {
            // Out of time: the request's call cap ends it.
            return ACTION_IN_PROGRESS;
        }
        if self.used > self.work_limit {
            return ACTION_IN_PROGRESS;
        }
        self.used += 12;
        self.last = 2 * depth;
        if self.path.len() < 2 * depth + 2 {
            self.path.resize(2 * depth + 2, (0.0, 0.0));
        }
        self.path[2 * depth] = (x, y);
        if self.range > 0.01
            && depth > 0
            && (self.xmax - self.xmin) <= 3.0 * EPSILON
            && (self.ymax - self.ymin) <= 3.0 * EPSILON
            && ((self.gy - y) * (self.gy - y) + (self.gx - x) * (self.gx - x)) + 0.0 < self.range * self.range
        {
            return ACTION_COMPLETE;
        }
        if self.xmin <= x && x <= self.xmax && self.ymin <= y && y <= self.ymax {
            return ACTION_COMPLETE;
        }
        let f = self.estimate(x, y) + g;
        #[expect(clippy::neg_cmp_op_on_partial_ord, reason = "a NaN must take this branch too, as on the server")]
        if !(f < self.bound) {
            return ACTION_FAILED;
        }
        let b = bucket(tri);
        if self.hash[b].fetch(tri) >= self.bound - g {
            return ACTION_FAILED;
        }
        self.hash[b].store(tri, (self.bound - g) - EPSILON);
        let succ = self.generate_successors(x, y, tri);
        if self.debug {
            let list: Vec<String> =
                succ.iter().map(|s| format!("[{:.2},{:.2}]->({:.2},{:.2}) c{:.2} t{}", s.px, s.py, s.nx, s.ny, s.cost, s.tri)).collect();
            eprintln!(
                "{:indent$}d{depth} ({x:.3},{y:.3}) tri {tri} g {g:.2} f {f:.2} bound {:.1}: {}",
                "",
                self.bound,
                list.join(" "),
                indent = depth
            );
        }
        let mut r = ACTION_FAILED;
        for s in succ {
            self.last = 2 * depth + 1;
            self.path[2 * depth + 1] = (s.px, s.py);
            r = self.dfs(depth + 1, g + s.cost, s.tri, s.nx, s.ny);
            if r != ACTION_FAILED {
                break;
            }
        }
        r
    }

    /// The successors of a point in triangle `node` (a level-2 id), best first by cost plus estimate; a straight line to the goal (or
    /// a goal box corner) is the only successor when it is clear.
    fn generate_successors(&mut self, x: f32, y: f32, node: i32) -> Vec<MeshStep> {
        self.used += 300;
        let m = self.mesh;
        let max_l = if self.coarse { 0 } else { 2 };
        let mut out: Vec<MeshStep> = Vec::new();
        let mut keys: Vec<(usize, f32)> = Vec::new();
        for l in 0..=max_l {
            let sh = 4 - 2 * l as i32;
            let t = node >> sh;
            let vs = m.tri_verts(l, t);
            let adj = m.adj[l][t as usize];
            let gt = if self.goal_tri < 0 { -1 } else { self.goal_tri >> sh };
            if l == 0 {
                // (A) direct line to the goal point.
                let mut ok = self.test_line_on_tile(x, y, self.gx, self.gy, false);
                if ok && !self.coarse {
                    ok = self.test_line_on_area();
                }
                if ok {
                    return vec![MeshStep { px: self.gx, py: self.gy, nx: self.gx, ny: self.gy, cost: 0.0, tri: 0 }];
                }
                // (B) goal-box corners, only when the goal is on the walkmesh.
                if gt != -1 {
                    let (xmin, xmax, ymin, ymax) = (self.xmin, self.xmax, self.ymin, self.ymax);
                    for (cx, cy, test) in [
                        (xmin, ymin, x < xmin || y < ymin),
                        (xmin, ymax, x < xmin || y > ymax),
                        (xmax, ymin, x > xmax || y < ymin),
                        (xmax, ymax, x > xmax || y > ymax),
                    ] {
                        if !test {
                            continue;
                        }
                        let mut ok = self.test_line_on_tile(x, y, cx, cy, true);
                        if ok && !self.coarse {
                            ok = self.test_line_on_area();
                        }
                        if ok {
                            return vec![MeshStep { px: cx, py: cy, nx: cx, ny: cy, cost: 0.0, tri: 0 }];
                        }
                    }
                }
                self.s[1] = [0, 0, 0];
            } else {
                let (s, l1) = (self.s, l + 1);
                self.s[l1] = [0, 0, 0];
                if t % 4 != 2 {
                    self.s[l1][0] = s[l][0];
                    self.s[l1][1] = s[l][1];
                } else {
                    if s[l][0] == 1 && s[l][2] == 1 {
                        self.s[l1][2] = 1;
                    }
                    if s[l][0] == 1 && s[l][2] == 1 && s[l][1] == 1 {
                        self.s[l1][1] = 1;
                        self.s[l1][0] = 1;
                    }
                    if s[l][0] == 1 && s[l][2] != 1 && s[l][1] == 1 {
                        self.s[l1][0] = 1;
                    }
                    if s[l][0] != 1 && s[l][1] == 1 && s[l][2] == 1 {
                        self.s[l1][1] = 1;
                    }
                    if l == 2 && (node >> 2) % 4 == 2 {
                        if s[1][0] == 1 {
                            self.s[3][2] = 1;
                        }
                        if s[1][1] == 1 {
                            self.s[3][0] = 1;
                        }
                        if s[1][2] == 1 {
                            self.s[3][1] = 1;
                        }
                    }
                }
            }
            // (C) the edges of the level-l triangle.
            for e in 0..3 {
                if self.s[l + 1][e] != 0 {
                    continue;
                }
                let a = adj[e];
                if a < 0 && a != TRIADJ_TILE_END {
                    continue;
                }
                let (va, vb) = (vs[e], vs[(e + 1) % 3]);
                let mx = (va[0] + vb[0]) * 0.5;
                let my = (va[1] + vb[1]) * 0.5;
                if self.debug {
                    let clear = line_walkable_in_tile(m, self.o, self.rad, self.hgt, x, y, mx, my, true);
                    eprintln!("   L{l} t{t} e{e} adj {a} v {va:?} {vb:?} mid ({mx:.3},{my:.3}) line {clear}");
                }
                if !self.coarse && !(self.test_line_on_tile(x, y, mx, my, true) && self.test_line_on_area()) {
                    continue;
                }
                let (succ, fkey);
                if a == TRIADJ_TILE_END {
                    if !(self.xmin <= mx && mx <= self.xmax && self.ymin <= my && my <= self.ymax) {
                        continue;
                    }
                    let tri = if l == 2 { t } else { m.find_triangle(l as i32 + 1, 1 - l as i32, mx, my, t) };
                    let cost = ((x - mx) * (x - mx) + (y - my) * (y - my)).sqrt();
                    succ = MeshStep { px: mx, py: my, nx: mx, ny: my, cost, tri };
                    fkey = cost + self.estimate(mx, my) * 1.0;
                } else {
                    let nb = match l {
                        2 => a,
                        1 => 4 * a + 2,
                        _ => (4 * a + 2) * 4 + 2,
                    };
                    let c = m.centroid(nb);
                    let (mut cx, mut cy, mut tri) = (c[0], c[1], nb);
                    if gt != -1 && a == gt {
                        cx = self.gx;
                        cy = self.gy;
                        tri = if l == 2 { gt } else { m.find_triangle(l as i32 + 1, 1 - l as i32, self.gx, self.gy, gt) };
                    }
                    if !self.coarse && !(self.test_line_on_tile(mx, my, cx, cy, true) && self.test_line_on_area()) {
                        continue;
                    }
                    let cost = magnitude(x - mx, y - my) + magnitude(cx - mx, cy - my);
                    succ = MeshStep { px: mx, py: my, nx: cx, ny: cy, cost, tri };
                    fkey = cost + self.estimate(cx, cy) * 1.0;
                }
                out.push(succ);
                keys.push((l, fkey));
                self.s[l + 1][e] = 1;
                // One bubble pass from the end.
                let n = out.len();
                for k in (0..n.saturating_sub(1)).rev() {
                    if keys[k + 1].0 <= keys[k].0 && keys[k].1 > keys[k + 1].1 {
                        out.swap(k, k + 1);
                        keys.swap(k, k + 1);
                    }
                }
            }
        }
        out
    }
}

/// The length of (x, y, 0), with a vector's length operations in a fixed order.
fn magnitude(x: f32, y: f32) -> f32 {
    let s = ((0.0f32 * 0.0) + x * x) + y * y;
    if s == 1.0 {
        s
    } else if s > 0.0 {
        s.sqrt()
    } else {
        0.0
    }
}

/// Whether a creature can walk the line within this tile: on the walkmesh, optionally reaching (x2, y2) itself inside the tile
/// (`check_end`), and clear of blocker triangles. Tile-local canonical in.
pub fn line_walkable_in_tile(m: &Mesh, o: (f32, f32), rad: f32, hgt: f32, x1: f32, y1: f32, x2: f32, y2: f32, check_end: bool) -> bool {
    let (ox, oy) = o;
    let (mut zl, mut zh) = (0.0f32, 0.0f32);
    let (ok, c1, c2) = m.walk_line(ox, oy, (x1 + ox, y1 + oy), (x2 + ox, y2 + oy), &mut zl, &mut zh);
    if !ok {
        return false;
    }
    zl -= 0.1;
    zh += 0.1 + hgt;
    if check_end {
        let (dx, dy) = (c2.0 - (x2 + ox), c2.1 - (y2 + oy));
        #[expect(clippy::neg_cmp_op_on_partial_ord, reason = "a NaN must take this branch too, as on the server")]
        if !(dx * dx + dy * dy + 0.0 <= 1e-10) {
            return false;
        }
    }
    m.clear_of_blockers(c1.0 - ox, c1.1 - oy, c2.0 - ox, c2.1 - oy, zl, zh, rad, 0)
}

/// The exit box's spacing for an edge with `n` exits, this being exit `k`
/// from the low corner (the same table `pathnode.rs` relies on).
fn exit_table(n: u8, k: u8) -> (f32, f64, f32, f32, f32) {
    // (step, inner (double), outer, half, fin)
    match (n, k) {
        (2, _) => (0.5, 1.0, 2.0, 2.5, 0.225),
        (3, 1) => (0.6, f64::from(1.2f32), 2.4, 3.0, 0.27),
        (3, _) => (0.2, f64::from(0.4f32), 0.8, 1.0, 0.09),
        _ => (1.0, 2.0, 4.0, 5.0, 0.45),
    }
}

/// The candidate whose bin contains the start coordinate (last match wins).
fn bin(s: f32, c: f32, step: f32, idx: usize) -> usize {
    let mut idx = idx;
    let mut j: i32 = -5;
    for i in 0..5 {
        if s < j as f32 * step + c {
            j += 2;
            continue;
        }
        j += 2;
        if j as f32 * step + c >= s {
            idx = i;
        }
    }
    idx
}

impl<'a> Planner<'a> {
    /// The path across tile (tx, ty): turns the request into the tile's canonical frame, runs the mesh search and turns the points
    /// found back. `cur`/`srel` tile-local real.
    pub fn plan_in_tile(&mut self, tx: i32, ty: i32, cur: (f32, f32), srel: (f32, f32)) -> i32 {
        let area = self.area;
        let t = area.tile(tx, ty).expect("the route stays inside the area");
        let (ox, oy) = t.origin();
        let n0 = self.pf.raw.len();
        let ccur = rot_r2c(t.o, cur.0, cur.1);
        let _csrel = rot_r2c(t.o, srel.0, srel.1);
        let route_len = self.pf.route_len;
        let saved_goal = self.pf.goal;
        let (lgx, lgy) = (self.pf.goal.0 - ox, self.pf.goal.1 - oy);
        if route_len == 1 {
            let (a, b) = rot_r2c(t.o, lgx, lgy);
            self.pf.goal = (a + ox, b + oy);
        } else if route_len >= 2 {
            for e in &mut self.pf.route_exits[..=(route_len - 2) as usize] {
                *e = rot_r2c(t.o, e.0, e.1);
            }
        }
        let dx = (self.pf.start.0 - ox) - 5.0;
        let dy = (self.pf.start.1 - oy) - 5.0;
        // Coarse mode: tile centre > 15 m from the path start, for an NPC that is not a possessed familiar.
        let coarse = self.coarse_mode && dx * dx + dy * dy > 225.0;
        let r = self.search_mesh(t, ccur.0, ccur.1, coarse);
        if route_len == 1 {
            self.pf.goal = (ox + lgx, oy + lgy);
            let _ = saved_goal;
        } else if route_len >= 2 {
            for e in &mut self.pf.route_exits[..=(route_len - 2) as usize] {
                *e = rot_c2r(t.o, e.0, e.1);
            }
        }
        if self.pf.raw.len() != n0 && t.o != 0 {
            for p in &mut self.pf.raw[n0..] {
                *p = t.c2r_tile(p.0, p.1);
            }
        }
        if self.trace.is_some() {
            let what = match r {
                ACTION_COMPLETE => "complete".to_string(),
                ACTION_FAILED => "FAILED".to_string(),
                _ => "in progress".to_string(),
            };
            let goal = self.last_goal_box;
            self.log(format!(
                "  tile ({tx},{ty}) from ({:.2},{:.2}){} goal ({:.2},{:.2}) box x {:.3}..{:.3} y {:.3}..{:.3}: {what} ({} units)",
                cur.0,
                cur.1,
                if coarse { " coarse" } else { "" },
                goal.0,
                goal.1,
                goal.2,
                goal.3,
                goal.4,
                goal.5,
                self.last_used
            ));
        }
        r
    }

    /// The in-tile search on the tile's mesh, uninterrupted: sets the goal box, then raises the bound until a path is found or the
    /// bound passes 40 m.
    fn search_mesh(&mut self, t: &Tile, x: f32, y: f32, coarse: bool) -> i32 {
        let m = &t.mesh;
        let (ox, oy) = t.origin();
        let mut s = Search {
            mesh: m,
            o: (ox, oy),
            gx: 0.0,
            gy: 0.0,
            xmin: 0.0,
            xmax: 0.0,
            ymin: 0.0,
            ymax: 0.0,
            goal_tri: -1,
            bound: 0.0,
            last: 0,
            path: Vec::new(),
            used: 0,
            hash: vec![HashEntry::NEW; 128],
            s: [[0; 3]; 4],
            coarse,
            rad: self.pf.rad,
            hgt: self.pf.hgt,
            range: self.pf.range,
            debug: std::env::var_os("LS_SIM_DEBUG").is_some(),
            lines: std::cell::Cell::new(0),
            line_limit: self.line_cap.saturating_sub(self.stats.intra_lines),
            work_limit: self.intra_cap.saturating_sub(self.stats.intra_used),
        };
        self.stats.intra_calls += 1;
        if self.pf.route_len == 1 {
            s.gx = self.pf.goal.0 - ox;
            s.xmin = s.gx - EPSILON;
            s.xmax = s.gx + EPSILON;
            s.gy = self.pf.goal.1 - oy;
            s.ymin = s.gy - EPSILON;
            s.ymax = EPSILON + s.gy;
            s.goal_tri = m.find_triangle(0, 2, s.gx, s.gy, -1);
        } else if !self.choose_exit_box(t, &mut s, x, y) {
            self.last_goal_box = (s.gx, s.gy, s.xmin, s.xmax, s.ymin, s.ymax);
            self.last_used = 0;
            return ACTION_FAILED;
        }
        self.last_goal_box = (s.gx, s.gy, s.xmin, s.xmax, s.ymin, s.ymax);
        let start_tri = m.find_triangle(0, 2, x, y, -1);
        if start_tri == -1 {
            self.last_used = 0;
            return ACTION_FAILED;
        }
        let est = s.estimate(x, y);
        let mut bound = est + 10.0;
        let r = loop {
            s.bound = bound;
            let r = s.dfs(0, 0.0, start_tri, x, y);
            bound += 5.0;
            if r == ACTION_FAILED {
                if f64::from(bound) > 40.0 {
                    break ACTION_FAILED;
                }
                continue;
            }
            break r;
        };
        if r == ACTION_IN_PROGRESS {
            self.stats.intra_used += s.used;
            self.stats.intra_lines += s.lines.get();
            self.last_used = s.used;
            return ACTION_IN_PROGRESS;
        }
        self.stats.intra_used += s.used;
        self.stats.intra_lines += s.lines.get();
        self.last_used = s.used;
        if r == ACTION_COMPLETE {
            for p in &s.path[..=s.last] {
                self.pf.raw.push((ox + p.0, oy + p.1));
            }
        }
        r
    }

    /// Picks the goal box on the exit edge for a tile that isn't the last: five candidates along the edge around the exit, each scored
    /// by whether the start sees it and how far free room reaches inward from it, weighted toward the side the route turns to two
    /// tiles ahead and toward the candidate in line with the start. (sx, sy) = start, canonical local.
    fn choose_exit_box(&mut self, t: &Tile, s: &mut Search<'_>, sx: f32, sy: f32) -> bool {
        let m = &t.mesh;
        let pn = self.area.table(t);
        let (ex, ey) = self.pf.route_exits[0];
        s.goal_tri = -1;
        s.gx = ex;
        s.gy = ey;
        let r = if 0.1 <= self.pf.rad { self.pf.rad } else { 0.1 };
        let mut xs = [ex; 5];
        let mut ys = [ey; 5];
        let mut horiz = false;
        let mut idx = 0usize;
        let mut pref: i32 = -1;
        let mut fin = 0.0f32;
        let mut dir = 1.0f32;
        let (ddx, ddy);
        if ey > MAX_TILE_EPS || ey < MIN_TILE_EPS {
            let mut n = 0u8;
            let mut k = 0u8;
            for &(px, py) in &pn.exits {
                if (ey - py).abs() <= EPSILON {
                    n = n.wrapping_add(1);
                    k = k.wrapping_add(u8::from(px < ex));
                }
            }
            let (step, inner, outer, half, f) = exit_table(n, k);
            fin = f;
            dir = if ey <= MAX_TILE_EPS { 1.0 } else { -1.0 };
            xs = [ex - outer, (f64::from(ex) - inner) as f32, ex, (f64::from(ex) + inner) as f32, outer + ex];
            idx = bin(sx, ex, step, 0);
            if self.pf.route_len > 2 {
                pref = i32::from(t.o);
            }
            horiz = true;
            s.xmin = ex - half;
            s.xmax = half + ex;
        } else {
            s.xmin = ex - EPSILON;
            s.xmax = EPSILON + ex;
        }
        if ex > MAX_TILE_EPS || ex < MIN_TILE_EPS {
            let mut n = 0u8;
            let mut k = 0u8;
            for &(px, py) in &pn.exits {
                if (ex - px).abs() <= EPSILON {
                    n = n.wrapping_add(1);
                    k = k.wrapping_add(u8::from(py < ey));
                }
            }
            let (step, inner, outer, half, f) = exit_table(n, k);
            fin = f;
            dir = if MAX_TILE_EPS < ex { -1.0 } else { 1.0 };
            xs = [ex; 5];
            ys = [ey - outer, (f64::from(ey) - inner) as f32, ey, (f64::from(ey) + inner) as f32, outer + ey];
            idx = bin(sy, ey, step, idx);
            if self.pf.route_len > 2 {
                pref = match t.o {
                    0 => 1,
                    1 => 0,
                    2 => 3,
                    _ => 2,
                };
            }
            s.ymax = ey + half;
            s.ymin = ey - half;
            (ddx, ddy) = (1.0f32, 0.0f32);
        } else {
            s.ymin = ey - EPSILON;
            s.ymax = ey + EPSILON;
            if !horiz {
                return true;
            }
            (ddx, ddy) = (0.0f32, 1.0f32);
        }
        // Weights: favour the side the route turns to two tiles ahead.
        let (r1, r2) = (self.pf.route_tiles[1], self.pf.route_tiles[2]);
        let (mut w, dw) = match pref {
            0 | 1 => {
                let (a, b) = if pref == 0 { (r1.0, r2.0) } else { (r1.1, r2.1) };
                if a < b {
                    (1, 1)
                } else if a == b {
                    (3, 0)
                } else {
                    (5, -1)
                }
            }
            2 | 3 => {
                let (a, b) = if pref == 2 { (r1.0, r2.0) } else { (r1.1, r2.1) };
                if a < b {
                    (5, -1)
                } else if a == b {
                    (3, 0)
                } else {
                    (1, 1)
                }
            }
            _ => (3, 0),
        };
        let (ox, oy) = t.origin();
        let mut best: i32 = -1;
        let mut best_score = 10.0f32;
        // zl/zh carry over from one candidate to the next (a failed walk may leave the previous values).
        let (mut zl, mut zh) = (0.0f32, 0.0f32);
        let hgt = self.pf.hgt;
        for i in 0..5 {
            let wi = if i == idx { w + 1 } else { w };
            let (cx, cy) = (xs[i] + ox, ys[i] + oy);
            let (ok, c1, c2) = m.walk_line(ox, oy, (sx + ox, sy + oy), (cx, cy), &mut zl, &mut zh);
            let mut score = best_score;
            let mut pick = best;
            let mut direct = false;
            if ok {
                zl -= 0.1;
                zh += 0.1 + hgt;
                if m.clear_of_blockers(c1.0 - ox, c1.1 - oy, c2.0 - ox, c2.1 - oy, zl, zh, r, 0) {
                    score = ((wi as f32 * 6.0) * r) * 7.0;
                    direct = true;
                }
            }
            if !direct {
                // Probe free space inward from the candidate.
                let mut d = 0.5 * r;
                let lim = r * 6.1;
                zl -= 20.01;
                zh += 20.01;
                let mut blocked = false;
                if d <= lim {
                    loop {
                        let px = (ddx * d) * dir + cx;
                        let py = (ddy * d) * dir + cy;
                        if !m.clear_of_blockers(px - ox, py - oy, cx - ox, cy - oy, zl, zh, d + d, 0) {
                            blocked = true;
                            break;
                        }
                        d += r;
                        if lim < d {
                            break;
                        }
                    }
                }
                if blocked {
                    if r <= d && (best == -1 || best_score < (wi as f32 * d) * 0.5) {
                        best_score = (d * 0.5) * wi as f32;
                        best = i as i32;
                    }
                    w += dw;
                    continue;
                }
                score = (wi as f32 * 6.0) * r;
            }
            if best != -1 && score <= best_score {
                score = best_score;
            } else {
                pick = i as i32;
            }
            best = pick;
            best_score = score;
            w += dw;
        }
        if best < 0 {
            return false;
        }
        let b = best as usize;
        s.gx = xs[b];
        s.gy = ys[b];
        if 0.5 <= ddy {
            s.xmax = s.gx + fin;
            s.xmin = s.gx - fin;
        }
        if 0.5 <= ddx {
            s.ymax = s.gy + fin;
            s.ymin = s.gy - fin;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bins_cover_the_candidates() {
        // One exit at 5 on its edge: candidates 1, 3, 5, 7, 9, bins 2 m wide.
        assert_eq!(bin(5.0, 5.0, 1.0, 0), 2);
        assert_eq!(bin(1.2, 5.0, 1.0, 0), 0);
        assert_eq!(bin(8.5, 5.0, 1.0, 0), 4);
        // On a bin boundary the later bin wins.
        assert_eq!(bin(6.0, 5.0, 1.0, 0), 3);
        // Outside every bin keeps the given index.
        assert_eq!(bin(-1.0, 5.0, 1.0, 3), 3);
    }

    #[test]
    fn hash_entry_two_slots() {
        let mut h = HashEntry::NEW;
        h.store(5, 3.0);
        h.store(6, 1.0);
        assert_eq!(h.fetch(5), 3.0);
        assert_eq!(h.fetch(6), 1.0);
        assert_eq!(h.fetch(7), 0.0);
        // A bigger value always replaces slot 0, whatever its key.
        h.store(7, 4.0);
        assert_eq!(h.fetch(5), 0.0);
        assert_eq!(h.fetch(7), 4.0);
    }
}
