// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The tile route: which tiles to cross, and through which exits.
//!
//! Each tile's path node table splits it into regions, each owning some exit points on the tile's edges. The route search is an
//! iterative-deepening depth-first search over (tile, region) states: from a region it may leave through any exit that region owns
//! into the neighbouring tile's region that owns the matching exit point. Successors are tried best first, by how well the exit's edge
//! span lines up with the goal (1 when the line to the goal crosses it, else a cosine). A state is not searched again with a smaller
//! remaining budget.
//!
//! The route is then walked tile by tile with the in-tile search (`intra`). When a tile can't be crossed from its entry to its chosen
//! exit, that crossing is recorded as denied and the route is searched again, until a route works or none is left.

use super::area::Planner;
use super::*;

/// One candidate next step of the route search.
#[derive(Clone, Copy, Debug, Default)]
pub struct RouteStep {
    /// The next tile's column.
    pub nx: i32,
    /// The next tile's row.
    pub ny: i32,
    /// The region of the next tile it enters.
    pub region: i32,
    /// The exit point in the current tile's local real frame: x.
    pub ex: f32,
    /// The exit point: y.
    pub ey: f32,
    /// How well it heads for the goal: 1 when the straight line to the goal crosses its exit, else how near the direction of an end of
    /// the exit is to the goal's; 0 for an exit off the edges. Higher is tried first.
    pub score: f32,
}

/// The number of the exit at a point, canonical: the first exit whose edge
/// span (out to the nearer corner) contains the point, else UNKNOWN_EXIT.
pub fn exit_number_at(pn: &PathNode, cx: f32, cy: f32) -> i32 {
    for (j, &(ex, ey)) in pn.exits.iter().enumerate() {
        let tol_x = if ey > MAX_TILE_EPS || ey < MIN_TILE_EPS { if ex <= 10.0 - ex { ex } else { 10.0 - ex } } else { EPSILON };
        if ex - tol_x <= cx && cx <= ex + tol_x {
            let tol_y = if ex > MAX_TILE_EPS || ex < MIN_TILE_EPS { if ey <= 10.0 - ey { ey } else { 10.0 - ey } } else { EPSILON };
            if ey - tol_y <= cy && cy <= ey + tol_y {
                return j as i32;
            }
        }
    }
    UNKNOWN_EXIT
}

/// The region owning the exit at exactly this point (within EPSILON), or -1.
pub fn region_entered_at(pn: &PathNode, cx: f32, cy: f32) -> i32 {
    for (j, &(ex, ey)) in pn.exits.iter().enumerate() {
        let (dx, dy) = (cx - ex, cy - ey);
        if (-EPSILON..=EPSILON).contains(&dx) && (-EPSILON..=EPSILON).contains(&dy) {
            return pn.exit_node[j];
        }
    }
    -1
}

/// Whether two 2D segments cross, as the successor score tests it (the range checks are deliberately not symmetric).
pub fn intersect_line_segments_2d(x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32, x4: f32, y4: f32) -> bool {
    if x1 == x2 {
        if x3 == x4 {
            return false;
        }
        let m2 = (y4 - y3) / (x4 - x3);
        let ix = x1;
        let iy = m2 * x1 + (y3 - x3 * m2);
        return ((y3 <= y4 && y3 <= iy && iy <= y4) || (y4 <= y3 && y4 <= iy && iy <= y3))
            && (x4 < x3 || (x3 <= ix && ix <= x4))
            && (x3 < x4 || (x4 <= ix && ix <= x3))
            && (y2 < y1 || (y1 <= iy && iy <= y2))
            && (y1 < y2 || (y2 <= iy && iy <= y1));
    }
    let m1 = (y2 - y1) / (x2 - x1);
    if x3 == x4 {
        let ix = x3;
        let iy = m1 * x3 + (y1 - x1 * m1);
        return ((y1 <= y2 && y1 <= iy && iy <= y2) || (y2 <= y1 && y2 <= iy && iy <= y1))
            && (x2 < x1 || (x1 <= ix && ix <= x2))
            && (x1 < x2 || (x2 <= ix && ix <= x1))
            && (y4 < y3 || (y3 <= iy && iy <= y4))
            && (y3 < y4 || (y4 <= iy && iy <= y3));
    }
    let m2 = (y4 - y3) / (x4 - x3);
    if m1 - m2 == 0.0 {
        return false;
    }
    let b2 = y3 - x3 * m2;
    let ix = (b2 - (y1 - m1 * x1)) * (1.0 / (m1 - m2));
    ((x3 <= x4 && x3 <= ix && ix <= x4) || (x4 <= x3 && x4 <= ix && ix <= x3))
        && (x2 < x1 || (x1 <= ix && ix <= x2))
        && (x1 < x2 || (x2 <= ix && ix <= x1))
}

/// The cosine of the angle between v and g, each divided by its length only
/// when it isn't (nearly) zero; the operations in a fixed order.
fn cosish(v: (f32, f32), g: (f32, f32)) -> f32 {
    let mut s = v.0 * g.0 + v.1 * g.1;
    if v.0.abs() >= EPSILON || v.1.abs() >= EPSILON {
        s /= (v.0 * v.0 + v.1 * v.1).sqrt();
    }
    if g.0.abs() >= EPSILON || g.1.abs() >= EPSILON {
        s /= (g.0 * g.0 + g.1 * g.1).sqrt();
    }
    s
}

impl<'a> Planner<'a> {
    // ---- A placed tile's path node table ----

    /// Exit i's point (tile-local real) and owning region; None when there is no exit i.
    pub fn exit_point(&self, t: &Tile, i: i32) -> Option<(f32, f32, i32)> {
        let pn = self.area.table(t);
        if i < 0 || i as usize >= pn.exits.len() {
            return None;
        }
        let (cx, cy) = pn.exits[i as usize];
        let (rx, ry) = rot_c2r(t.o, cx, cy);
        Some((rx, ry, pn.exit_node[i as usize]))
    }

    /// Region `r`'s node point on tile `t`, tile-local in the placed tile's frame.
    pub fn region_point(&self, t: &Tile, r: i32) -> (f32, f32) {
        let (cx, cy) = self.area.table(t).nodes[r as usize];
        rot_c2r(t.o, cx, cy)
    }

    /// The region of tile `t` a walk entering at (`rx`, `ry`) (tile-local, placed frame) is in; below 0 for none.
    pub fn tile_region_entered_at(&self, t: &Tile, rx: f32, ry: f32) -> i32 {
        let (cx, cy) = rot_r2c(t.o, rx, ry);
        region_entered_at(self.area.table(t), cx, cy)
    }

    /// The number of tile `t`'s exit at (`rx`, `ry`) (tile-local, placed frame); [`UNKNOWN_EXIT`] for none.
    pub fn tile_exit_number_at(&self, t: &Tile, rx: f32, ry: f32) -> i32 {
        let (cx, cy) = rot_r2c(t.o, rx, ry);
        exit_number_at(self.area.table(t), cx, cy)
    }

    /// The region a world point belongs to: the first node reached by spreading over the walkable triangles from the point's triangle,
    /// else the nearest node (region 0 when the point is off the walkmesh).
    pub fn region_at(&self, t: &Tile, wx: f32, wy: f32) -> i32 {
        let (cwx, cwy) = t.r2c_tile(wx, wy);
        let pn = self.area.table(t);
        if pn.nodes.len() <= 1 {
            return 0;
        }
        let m = &t.mesh;
        let (lx, ly) = (cwx - t.x as f32 * TILE_SIZE, cwy - t.y as f32 * TILE_SIZE);
        let node_tri: Vec<i32> = pn.nodes.iter().map(|&(nx, ny)| m.find_triangle(0, 0, nx, ny, -1)).collect();
        let mut seen = vec![false; m.tri_count(0)];
        let tri = m.find_triangle(0, 0, lx, ly, -1);
        if tri == -1 {
            return 0;
        }
        let r = region_by_adjacency(m, &node_tri, &mut seen, tri);
        if r != -1 {
            return r;
        }
        let mut best = 0;
        let mut best_d = f32::INFINITY;
        for (i, &(nx, ny)) in pn.nodes.iter().enumerate() {
            let d = (ly - ny) * (ly - ny) + (lx - nx) * (lx - nx);
            if i == 0 || d < best_d {
                best = i as i32;
                best_d = d;
            }
        }
        best
    }

    // ---- Planning the route ----

    /// Finds a tile route, then walks it tile by tile, replanning around each crossing that fails. ACTION_COMPLETE with the raw points
    /// smoothed into waypoints, ACTION_FAILED, or ACTION_IN_PROGRESS when out of time.
    pub fn plan_tile_route(&mut self) -> i32 {
        let pf_start = self.pf.start;
        let pf_goal = self.pf.goal;
        let sx = (pf_start.0 / TILE_SIZE) as i32;
        let sy = (pf_start.1 / TILE_SIZE) as i32;
        let gx = (pf_goal.0 / TILE_SIZE) as i32;
        let gy = (pf_goal.1 / TILE_SIZE) as i32;
        self.goal_tile = (gx, gy);
        let (Some(st), Some(gt)) = (self.area.tile(sx, sy), self.area.tile(gx, gy)) else {
            self.clear_tile_route();
            return ACTION_FAILED;
        };
        let start_region = self.region_at(st, pf_start.0, pf_start.1);
        self.goal_region = self.region_at(gt, pf_goal.0, pf_goal.1);
        self.log(format!("start tile ({sx},{sy}) region {start_region}; goal tile ({gx},{gy}) region {}", self.goal_region));
        let mut bound = (gx - sx).abs() + (gy - sy).abs() - 2;
        if self.pf.route_len == 0 {
            if self.pf.finished != 0 {
                return ACTION_IN_PROGRESS;
            }
            self.goal_pt = pf_goal;
            if !self.find_tile_route(&mut bound, sx, sy, start_region) {
                self.log("no tile route".into());
                self.clear_tile_route();
                return ACTION_FAILED;
            }
            self.log_route();
        }
        let mut advanced = false;
        let (mut tx, mut ty) = self.pf.route_tiles[0];
        let p = self.pf.raw.last().copied().unwrap_or(pf_start);
        let mut cur = (p.0 - tx as f32 * TILE_SIZE, p.1 - ty as f32 * TILE_SIZE);
        let mut srel = (pf_start.0 - tx as f32 * TILE_SIZE, pf_start.1 - ty as f32 * TILE_SIZE);
        loop {
            if self.stats.replans >= self.max_replans {
                self.log(format!("gave up after {} replans (simulator cap)", self.stats.replans));
                self.stats.capped = true;
                return ACTION_IN_PROGRESS;
            }
            let r = self.plan_in_tile(tx, ty, cur, srel);
            if r == ACTION_IN_PROGRESS {
                self.log("  out of time".into());
                return ACTION_IN_PROGRESS;
            }
            if r == ACTION_COMPLETE {
                let last = *self.pf.raw.last().expect("a completed tile has added its points to the path");
                cur = (last.0 - tx as f32 * TILE_SIZE, last.1 - ty as f32 * TILE_SIZE);
                srel = (pf_start.0 - tx as f32 * TILE_SIZE, pf_start.1 - ty as f32 * TILE_SIZE);
                advanced = true;
                if self.pf.route_len < 2 {
                    self.pf.route_len = 0;
                } else {
                    let (nx, ny) = self.pf.route_tiles[1];
                    if nx > tx {
                        cur.0 = 0.0;
                    } else if nx < tx {
                        cur.0 = 10.0;
                    }
                    if ny > ty {
                        cur.1 = 0.0;
                    } else if ny < ty {
                        cur.1 = 10.0;
                    }
                    self.pf.route_len -= 1;
                    self.pf.route_tiles.remove(0);
                    self.pf.route_exits.remove(0);
                    self.pf.route_tiles.push((0, 0));
                    self.pf.route_exits.push((0.0, 0.0));
                    (tx, ty) = self.pf.route_tiles[0];
                }
            } else if r == ACTION_FAILED {
                let t = self.area.tile(tx, ty).expect("the route stays inside the area");
                let a = self.tile_exit_number_at(t, cur.0, cur.1);
                let b = if self.pf.route_len > 1 {
                    self.tile_exit_number_at(t, self.pf.route_exits[0].0, self.pf.route_exits[0].1)
                } else {
                    UNKNOWN_EXIT
                };
                self.log(format!("  deny tile ({tx},{ty}) exit {a} -> {b}"));
                self.pf.denials.push(((tx, ty), (a, b)));
                self.goal_pt = pf_goal;
                self.pf.raw.clear();
                self.pf.waypoints.clear();
                self.stats.replans += 1;
                bound -= 2;
                if !self.find_tile_route(&mut bound, sx, sy, start_region) {
                    self.log("no tile route left".into());
                    self.clear_tile_route();
                    return ACTION_FAILED;
                }
                self.log_route();
                (tx, ty) = self.pf.route_tiles[0];
                cur = (pf_start.0 - tx as f32 * TILE_SIZE, pf_start.1 - ty as f32 * TILE_SIZE);
                srel = cur;
            }
            if self.pf.route_len < 1 {
                break;
            }
        }
        if advanced {
            self.smooth_path();
        }
        ACTION_COMPLETE
    }

    fn clear_tile_route(&mut self) {
        self.pf.denials.clear();
        self.pf.route_len = 0;
    }

    fn log_route(&mut self) {
        if self.trace.is_none() {
            return;
        }
        let n = self.pf.route_len as usize;
        let mut s = String::from("  route:");
        for i in 0..n {
            let (x, y) = self.pf.route_tiles[i];
            s += &format!(" ({x},{y})");
            if i + 1 < n {
                let (fx, fy) = self.pf.route_exits[i];
                s += &format!(" -[{fx:.2},{fy:.2}]->");
            }
        }
        self.log(s);
    }

    /// Iterative deepening over the route search, budgets b+2 .. L-2 (L is the Manhattan distance plus a margin).
    pub fn find_tile_route(&mut self, bound: &mut i32, sx: i32, sy: i32, start_region: i32) -> bool {
        self.visited = vec![0u8; (MAX_X_TILES * MAX_Y_TILES * 6) as usize];
        let l = (sx - self.goal_tile.0).abs() + (sy - self.goal_tile.1).abs() + 23;
        self.succ = vec![RouteStep::default(); (l * 8) as usize];
        let mut b = *bound;
        loop {
            *bound = b + 2;
            if self.route_dfs(0, b + 2, sx, sy, start_region) {
                return true;
            }
            b = *bound;
            if b >= l - 2 {
                return false;
            }
        }
    }

    /// The depth-first route search from (x, y, region r) with budget b.
    fn route_dfs(&mut self, d: i32, b: i32, x: i32, y: i32, r: i32) -> bool {
        self.stats.inter_nodes += 1;
        self.pf.route_len = d + 1;
        self.pf.route_tiles[d as usize] = (x, y);
        if d != 0 || b < 3 {
            if (x, y, r) == (self.goal_tile.0, self.goal_tile.1, self.goal_region) {
                return !self.goal_entry_denied(d, x, y);
            }
            if b < 1 {
                return false;
            }
        }
        if (self.goal_tile.0 - x).abs() + (self.goal_tile.1 - y).abs() > b {
            return false;
        }
        let vi = (r + (MAX_Y_TILES * x + y) * 6) as usize;
        if i32::from(self.visited[vi]) >= b {
            return false;
        }
        self.visited[vi] = b as u8;
        let n = self.route_successors(d, x, y, r);
        for i in 0..n {
            let s = self.succ[(d * 8 + i) as usize];
            self.pf.route_exits[d as usize] = (s.ex, s.ey);
            if self.route_dfs(d + 1, b - 1, s.nx, s.ny, s.region) {
                return true;
            }
        }
        false
    }

    /// The route search's successors of (x, y, region r), at most 8, best score first, skipping exits denied for this entry.
    fn route_successors(&mut self, d: i32, x: i32, y: i32, r: i32) -> i32 {
        let area = self.area;
        let Some(t) = area.tile(x, y) else { return 0 };
        let (crx, cry) = self.region_point(t, r);
        let p = (x as f32 * TILE_SIZE + crx, y as f32 * TILE_SIZE + cry);
        let gp = self.goal_pt;
        let n_exits = area.table(t).exits.len() as i32;
        let mut blocked = [false; 13];
        for &((dx, dy), (a, bx)) in &self.pf.denials {
            if (dx, dy) != (x, y) {
                continue;
            }
            if d == 0 {
                if bx != UNKNOWN_EXIT {
                    blocked[bx as usize] = true;
                }
            } else {
                let (px, py) = self.pf.route_tiles[(d - 1) as usize];
                let (fx, fy) = self.pf.route_exits[(d - 1) as usize];
                let ex = (px - x) as f32 * 10.0 + fx;
                let ey = (py - y) as f32 * 10.0 + fy;
                // A denial recorded with "no exit" as its entry (a == 12) is treated as matching no entry.
                if let Some((lx, ly, _)) = self.exit_point(t, a)
                    && ex - EPSILON <= lx
                    && lx <= ex + EPSILON
                    && ey - EPSILON <= ly
                    && ly <= ey + EPSILON
                {
                    blocked[bx as usize] = true;
                }
            }
        }
        if blocked[12] {
            return 0;
        }
        let base = (d * 8) as usize;
        let mut count = 0usize;
        for k in 0..n_exits {
            let (lx, ly, owner) = self.exit_point(t, k).expect("k is below the tile's exit count");
            if owner != r || blocked[k as usize] {
                continue;
            }
            if count >= 8 {
                break;
            }
            let (mut nx, mut ex2) = (x, lx);
            if lx > MAX_TILE_EPS {
                nx = x + 1;
                ex2 = 0.0;
            }
            if lx < MIN_TILE_EPS {
                nx -= 1;
                ex2 = 10.0;
            }
            let (mut ny, mut ey2) = (y, ly);
            if ly > MAX_TILE_EPS {
                ny = y + 1;
                ey2 = 0.0;
            }
            if ly < MIN_TILE_EPS {
                ny -= 1;
                ey2 = 10.0;
            }
            if !(0 <= nx && nx < area.width && 0 <= ny && ny < area.height) {
                continue;
            }
            let nt = area.tile(nx, ny).expect("checked inside the area above");
            let nr = self.tile_region_entered_at(nt, ex2, ey2);
            if nr < 0 {
                continue;
            }
            let w = (x as f32 * TILE_SIZE + lx, y as f32 * TILE_SIZE + ly);
            let score;
            let span = if ly > MAX_TILE_EPS || ly < MIN_TILE_EPS {
                let h = (if 10.0 - lx < lx { 10.0 - lx } else { lx }) - EPSILON;
                Some(((w.0 - h, w.1), (w.0 + h, w.1)))
            } else if lx > MAX_TILE_EPS || lx < MIN_TILE_EPS {
                let h = (if 10.0 - ly < ly { 10.0 - ly } else { ly }) - EPSILON;
                Some(((w.0, w.1 - h), (w.0, w.1 + h)))
            } else {
                None
            };
            match span {
                None => score = 0.0,
                Some((a, b)) => {
                    if intersect_line_segments_2d(p.0, p.1, gp.0, gp.1, a.0, a.1, b.0, b.1) {
                        score = 1.0;
                    } else {
                        let g = (gp.0 - p.0, gp.1 - p.1);
                        let s1 = cosish((b.0 - p.0, b.1 - p.1), g);
                        let s2 = cosish((a.0 - p.0, a.1 - p.1), g);
                        score = if s1 > s2 { s1 } else { s2 };
                    }
                }
            }
            // Stable insertion, descending by score.
            let mut j = count as i32 - 1;
            while j >= 0 && self.succ[base + j as usize].score < score {
                self.succ[base + j as usize + 1] = self.succ[base + j as usize];
                j -= 1;
            }
            self.succ[base + (j + 1) as usize] = RouteStep { nx, ny, region: nr, ex: lx, ey: ly, score };
            count += 1;
        }
        count as i32
    }

    /// Whether ending the route in the goal tile through this entry was denied.
    fn goal_entry_denied(&self, d: i32, x: i32, y: i32) -> bool {
        let mut a = UNKNOWN_EXIT;
        if d != 0 {
            let (px, py) = self.pf.route_tiles[(d - 1) as usize];
            let (fx, fy) = self.pf.route_exits[(d - 1) as usize];
            let t = self.area.tile(x, y).expect("the route stays inside the area");
            a = self.tile_exit_number_at(t, (px - x) as f32 * 10.0 + fx, (py - y) as f32 * 10.0 + fy);
        }
        self.pf.denials.iter().any(|&(tile, (da, db))| tile == (x, y) && da == a && db == UNKNOWN_EXIT)
    }
}

/// Depth-first over level-0 adjacency from triangle t: the first region whose node's triangle is reached, or -1.
fn region_by_adjacency(m: &Mesh, node_tri: &[i32], seen: &mut [bool], t: i32) -> i32 {
    if seen[t as usize] {
        return -1;
    }
    for (i, &nt) in node_tri.iter().enumerate() {
        if nt == t {
            return i as i32;
        }
    }
    seen[t as usize] = true;
    for a in m.adj[0][t as usize] {
        if a >= 0 {
            let r = region_by_adjacency(m, node_tri, seen, a);
            if r != -1 {
                return r;
            }
        }
    }
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(exits: &[(f32, f32)]) -> PathNode {
        PathNode { nodes: vec![(5.0, 5.0)], exits: exits.to_vec(), exit_node: vec![0; exits.len()] }
    }

    #[test]
    fn exit_number_spans_to_nearer_corner() {
        let pn = table(&[(2.5, 0.0), (10.0, 7.5), (5.0, 10.0)]);
        // South exit at x=2.5: matches x in 0..5 on y=0 only.
        assert_eq!(exit_number_at(&pn, 0.0, 0.0), 0);
        assert_eq!(exit_number_at(&pn, 4.9, 0.0), 0);
        assert_eq!(exit_number_at(&pn, 5.1, 0.0), UNKNOWN_EXIT);
        assert_eq!(exit_number_at(&pn, 2.5, 0.0002), UNKNOWN_EXIT);
        assert_eq!(exit_number_at(&pn, 2.5, 0.00005), 0);
        // East exit at y=7.5: span 5..10 (10 - 7.5 = 2.5 each way).
        assert_eq!(exit_number_at(&pn, 10.0, 5.0), 1);
        assert_eq!(exit_number_at(&pn, 10.0, 4.99), UNKNOWN_EXIT);
        // North exit at x=5: the whole edge.
        assert_eq!(exit_number_at(&pn, 0.0, 10.0), 2);
        // An interior point matches nothing.
        assert_eq!(exit_number_at(&pn, 5.0, 5.0), UNKNOWN_EXIT);
    }

    #[test]
    fn region_entrance_is_exact() {
        let pn = PathNode { nodes: vec![(2.0, 2.0), (8.0, 8.0)], exits: vec![(0.0, 2.5), (10.0, 7.5)], exit_node: vec![0, 1] };
        assert_eq!(region_entered_at(&pn, 0.0, 2.5), 0);
        assert_eq!(region_entered_at(&pn, 10.0, 7.50005), 1);
        assert_eq!(region_entered_at(&pn, 10.0, 7.6), -1);
    }

    #[test]
    fn segments_cross_where_they_meet_and_never_when_parallel() {
        // Diagonal through a horizontal span.
        assert!(intersect_line_segments_2d(0.0, 0.0, 4.0, 4.0, 1.0, 2.0, 3.0, 2.0));
        assert!(!intersect_line_segments_2d(0.0, 0.0, 4.0, 4.0, 2.5, 2.0, 3.0, 2.0));
        // Vertical first segment.
        assert!(intersect_line_segments_2d(2.0, 0.0, 2.0, 4.0, 0.0, 1.0, 4.0, 3.0));
        assert!(!intersect_line_segments_2d(2.0, 0.0, 2.0, 1.0, 0.0, 1.0, 4.0, 3.0));
        // Vertical second segment (the span on an E/W edge).
        assert!(intersect_line_segments_2d(0.0, 0.0, 4.0, 4.0, 3.0, 0.0, 3.0, 5.0));
        assert!(!intersect_line_segments_2d(0.0, 0.0, 4.0, 4.0, 3.0, 0.0, 3.0, 2.0));
        // Both vertical / parallel: never.
        assert!(!intersect_line_segments_2d(1.0, 0.0, 1.0, 4.0, 1.0, 0.0, 1.0, 4.0));
        assert!(!intersect_line_segments_2d(0.0, 0.0, 4.0, 4.0, 0.0, 1.0, 4.0, 5.0));
    }
}
