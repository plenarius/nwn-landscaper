// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The area level: the planner's state, the straight-line test across tiles,
//! what a walk request comes to, and the smoothing of a found path.
//!
//! A request first tries the straight line from start to goal. If that is blocked by the walkmesh it plans a tile route (`inter`,
//! `intra`); if that fails close to the goal (within 20 m) it tries the grid search (`grid`); whatever fails falls back to the
//! farthest point the straight line reaches (found by bisection), unless the request forbids a partial path.
//!
//! The server plans in time slices, so a long search runs out of time. Time is modelled by counting line tests: each costs `line_us`
//! microseconds, a call has `call_us` and an NPC's request gets `calls_cap` calls.
//!
//! A found path is smoothed in three passes that skip waypoints a straight
//! walk can reach, then its sharp corners are cut where the line allows.

use super::grid::GridEnd;
use super::inter::RouteStep;
use super::*;

/// One walk request's state: what was asked and what the planning found.
#[derive(Clone, Debug, Default)]
pub struct PathState {
    /// No partial-line fallback when nonzero.
    pub no_partial: i32,
    /// Creature radius.
    pub rad: f32,
    /// Creature height (the clearance a line needs above the ground).
    pub hgt: f32,
    /// Keep the raw points instead of smoothing them.
    pub no_smooth: i32,
    /// Goal (world).
    pub goal: (f32, f32),
    /// How close to the goal is close enough (0: reach it).
    pub range: f32,
    /// Start (world).
    pub start: (f32, f32),
    /// Planning calls made.
    pub calls: i32,
    /// Nonzero once the in-area plan is finished.
    pub finished: i32,
    /// Final waypoints.
    pub waypoints: Vec<(f32, f32)>,
    /// Raw points of the in-tile paths (world).
    pub raw: Vec<(f32, f32)>,
    /// Tile crossings found impossible, (tile, (entry exit, leave exit)).
    pub denials: Vec<((i32, i32), (i32, i32))>,
    /// Tiles left on the route.
    pub route_len: i32,
    /// The route's tiles, the current one first.
    pub route_tiles: Vec<(i32, i32)>,
    /// The exit point leaving each route tile, tile-local real.
    pub route_exits: Vec<(f32, f32)>,
}

/// What one request's planning cost, for `sim-check` and the route check's reports: the server gives up past its limits.
#[derive(Default, Debug, Clone)]
pub struct Stats {
    /// In-tile searches run.
    pub intra_calls: usize,
    /// In-tile work units (12 a search step, 300 a successor list), summed over all calls.
    pub intra_used: i64,
    /// Tile route nodes expanded.
    pub inter_nodes: usize,
    /// Times the tile route was planned again after an in-tile search failed.
    pub replans: usize,
    /// Stopped by the simulator's replan cap.
    pub capped: bool,
    /// Grid search: steps tried and straight-line tests made.
    pub grid_steps: usize,
    /// Straight-line tests the grid search made.
    pub grid_lines: usize,
    /// Line tests made by the in-tile searches.
    pub intra_lines: u64,
    /// Planning calls the tile route took (time model).
    pub calls: i32,
}

/// The planner: the area, the request's state and the route search's working data.
pub struct Planner<'a> {
    /// The area's walkmesh, by tile.
    pub area: &'a Area,
    /// The request and what has been found so far.
    pub pf: PathState,
    /// The goal's tile, its region there, and the point the route search heads for.
    pub goal_tile: (i32, i32),
    /// The region of the goal's tile that holds the goal.
    pub goal_region: i32,
    /// Where the route search heads for (world).
    pub goal_pt: (f32, f32),
    /// The route search's best bound seen at each (tile, region): a node is expanded again only with a higher one.
    pub visited: Vec<u8>,
    /// The route search's successors, eight per depth, best first.
    pub succ: Vec<RouteStep>,
    /// NPC and not a possessed familiar: the coarse-mode rule applies.
    pub coarse_mode: bool,
    /// The simulator's own limit on replans, so a route the server would loop on ends (as `capped`).
    pub max_replans: usize,
    /// What the planning has cost.
    pub stats: Stats,
    /// The planning's steps in words, when asked for (`sim-check`'s trace).
    pub trace: Option<Vec<String>>,
    /// The last in-tile search's goal and box (x, y, then x and y bounds), for the trace.
    pub last_goal_box: (f32, f32, f32, f32, f32, f32),
    /// The work units the last in-tile search used, for the trace.
    pub last_used: i64,
    /// Whether the first smoothing pass also breaks long legs into 1 m steps. The server's choice here could not be pinned down from
    /// its results; off by default, `LS_SIM_SUB` turns it on.
    pub subdivide_first_pass: bool,
    /// Time model: one line test costs `line_us` of the server's time; each planning call has `call_us` (an NPC's time slice) and an
    /// NPC gets `calls_cap` calls.
    pub line_us: f64,
    /// A straight-line test in the grid search (on the server it also checks objects and creatures, so it costs more).
    pub grid_line_us: f64,
    /// The length of one planning call (an NPC's time slice), in microseconds.
    pub call_us: f64,
    /// How many planning calls an NPC's request gets before the server gives up.
    pub calls_cap: i32,
    /// The tile route's line-test allowance: past it the request is out of calls.
    pub line_cap: u64,
}

/// A path request.
#[derive(Clone, Copy, Debug)]
pub struct Query {
    /// Where the creature starts (world).
    pub from: (f32, f32),
    /// Where it is asked to go (world).
    pub to: (f32, f32),
    /// The creature's radius (m).
    pub rad: f32,
    /// The creature's height: the clearance a line needs above the ground.
    pub hgt: f32,
    /// How close to `to` is close enough (0: reach it).
    pub range: f32,
    /// Refuse a partial path: a request that can't reach the goal fails instead of walking part way.
    pub no_partial: bool,
    /// The creature is an NPC (coarse mode, 100-call cap).
    pub npc: bool,
    /// The creature is created at `from` first (creating it moves it to a safe location), as in the oracle runs.
    pub spawn: bool,
}

impl Query {
    /// A request from `from` to `to` by the oracle's creature, so simulated routes compare with the recorded ones.
    pub fn new(from: (f32, f32), to: (f32, f32)) -> Query {
        // Radius and height of the oracle's creature (nw_dog).
        Query { from, to, rad: 0.4, hgt: 1.0, range: 0.0, no_partial: false, npc: true, spawn: true }
    }
}

/// How a request ended, as the server would have walked it.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The straight line is clear: walk straight.
    Direct,
    /// The tile route was found; waypoints after smoothing.
    Complete,
    /// Planning failed and the partial straight line was used.
    Partial { why: String },
    /// Nothing usable.
    Failed { why: String },
    /// The tile route failed within 20 m and the grid search produced a path (to the goal, or the closest point it found).
    Grid,
}

/// What a simulated request comes to: the outcome, the waypoints and where the creature stops.
#[derive(Clone, Debug)]
pub struct SimResult {
    /// How the request ended.
    pub outcome: Outcome,
    /// Where the creature started (after being placed on creation).
    pub start: (f32, f32),
    /// The straight-line test from the start: 1 clear, 0 off the area, -1 blocked by the walkmesh.
    pub direct_line: i32,
    /// The path after smoothing (world).
    pub waypoints: Vec<(f32, f32)>,
    /// Where the creature should stop (last waypoint, or the start).
    pub end: (f32, f32),
    /// What the planning has cost.
    pub stats: Stats,
    /// The planning's steps in words (empty unless asked for).
    pub trace: Vec<String>,
}

impl<'a> Planner<'a> {
    /// A planner for one request `q` in `area`, with the server's limits; `trace` keeps its steps in words.
    pub fn new(area: &'a Area, q: &Query, trace: bool) -> Planner<'a> {
        let n = (MAX_X_TILES * MAX_Y_TILES) as usize;
        Planner {
            area,
            pf: PathState {
                no_partial: i32::from(q.no_partial),
                rad: q.rad,
                hgt: q.hgt,
                goal: q.to,
                range: q.range,
                start: q.from,
                route_tiles: vec![(0, 0); n],
                route_exits: vec![(0.0, 0.0); n],
                ..Default::default()
            },
            goal_tile: (0, 0),
            goal_region: 0,
            goal_pt: (0.0, 0.0),
            visited: Vec::new(),
            succ: Vec::new(),
            coarse_mode: q.npc,
            max_replans: 2000,
            stats: Stats::default(),
            trace: trace.then(Vec::new),
            last_goal_box: (0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            last_used: 0,
            subdivide_first_pass: std::env::var_os("LS_SIM_SUB").is_some(),
            line_us: std::env::var("LS_SIM_LINE_US").ok().and_then(|v| v.parse().ok()).unwrap_or(0.7),
            grid_line_us: std::env::var("LS_SIM_GRID_US").ok().and_then(|v| v.parse().ok()).unwrap_or(0.7),
            call_us: 1000.0,
            calls_cap: 100,
            line_cap: u64::MAX,
        }
    }

    /// Adds a step to the trace, when there is one.
    pub fn log(&mut self, s: String) {
        if let Some(t) = &mut self.trace {
            t.push(s);
        }
    }

    // ---- Straight lines across tiles ----

    /// Whether a creature of radius `rad` and height `hgt` can walk straight from (x1, y1) to (x2, y2): tile by tile, the line must
    /// stay on the walkmesh and a band of the creature's width along it must clear the blocker triangles within the heights walked.
    /// Returns 1 clear, 0 off the area, -1 walkmesh blocked (-2 placeables and -3 creatures, which the server also reports, cannot
    /// happen offline).
    pub fn test_direct_line(&self, x1: f32, y1: f32, x2: f32, y2: f32, rad: f32, hgt: f32) -> i32 {
        let area = self.area;
        let dir_x: i32 = if x1 <= x2 { 1 } else { -1 };
        let dir_y: i32 = if y1 <= y2 { 1 } else { -1 };
        let mut tx = (x1 / TILE_SIZE) as i32;
        let mut ty = (y1 / TILE_SIZE) as i32;
        let (mut acc_min, mut acc_max) = (0.0f32, 0.0f32);
        loop {
            let Some(t) = area.tile(tx, ty) else { return 0 };
            let (mut zmin, mut zmax) = (0.0f32, 0.0f32);
            let (ok, c1, c2) = self.tile_walk_line(t, (x1, y1), (x2, y2), &mut zmin, &mut zmax);
            if !ok {
                return -1;
            }
            let zl = zmin - 0.1;
            let zh = (0.1 + hgt) + zmax;
            if acc_min == 0.0 && acc_max == 0.0 {
                acc_max = zh;
                acc_min = zl;
            } else {
                acc_min = if zl < acc_min { zl } else { acc_min };
                acc_max = if zh > acc_max { zh } else { acc_max };
            }
            if !self.tile_clear_of_blockers(t, c1.0, c1.1, c2.0, c2.1, zl, zh, rad, 0) {
                return -1;
            }
            if c2.0 >= x2 - EPSILON && c2.0 <= x2 + EPSILON && c2.1 >= y2 - EPSILON && c2.1 <= y2 + EPSILON {
                break;
            }
            if dir_x == 1 {
                if c2.0 - tx as f32 * TILE_SIZE >= MAX_TILE_EPS {
                    tx += 1;
                }
            } else if MIN_TILE_EPS >= c2.0 - tx as f32 * TILE_SIZE {
                tx -= 1;
            }
            if dir_y == 1 {
                if c2.1 - ty as f32 * TILE_SIZE >= MAX_TILE_EPS {
                    ty += 1;
                }
            } else if MIN_TILE_EPS >= c2.1 - ty as f32 * TILE_SIZE {
                ty -= 1;
            }
        }
        // The creature's radius past the end point, if that is another tile.
        let xm = dir_x as f32 * rad + x2;
        let ym = dir_y as f32 * rad + y2;
        if ((x2 * 0.1) as i32 != (xm * 0.1) as i32 || (y2 * 0.1) as i32 != (ym * 0.1) as i32) && !self.line_walkable(x2, y2, xm, ym) {
            return -1;
        }
        // The server then checks placeables, doors and creatures: there are none offline.
        1
    }

    /// The tile-by-tile walkmesh part of `test_direct_line` alone (no clearance test).
    pub fn line_walkable(&self, x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
        let dir_x: i32 = if x1 <= x2 { 1 } else { -1 };
        let dir_y: i32 = if y1 <= y2 { 1 } else { -1 };
        let mut tx = (x1 / TILE_SIZE) as i32;
        let mut ty = (y1 / TILE_SIZE) as i32;
        loop {
            let Some(t) = self.area.tile(tx, ty) else { return false };
            let (mut zmin, mut zmax) = (0.0f32, 0.0f32);
            let (ok, _c1, c2) = self.tile_walk_line(t, (x1, y1), (x2, y2), &mut zmin, &mut zmax);
            if !ok {
                return false;
            }
            if c2.0 >= x2 - EPSILON && c2.0 <= x2 + EPSILON && c2.1 >= y2 - EPSILON && c2.1 <= y2 + EPSILON {
                return true;
            }
            if dir_x == 1 {
                if c2.0 - tx as f32 * TILE_SIZE >= MAX_TILE_EPS {
                    tx += 1;
                }
            } else if MIN_TILE_EPS >= c2.0 - tx as f32 * TILE_SIZE {
                tx -= 1;
            }
            if dir_y == 1 {
                if c2.1 - ty as f32 * TILE_SIZE >= MAX_TILE_EPS {
                    ty += 1;
                }
            } else if MIN_TILE_EPS >= c2.1 - ty as f32 * TILE_SIZE {
                ty -= 1;
            }
        }
    }

    /// `Mesh::walk_line` for a placed tile: rotate to canonical, run the mesh test, rotate back, add the tile's z.
    pub(super) fn tile_walk_line(
        &self,
        t: &Tile,
        p: (f32, f32),
        q: (f32, f32),
        zmin: &mut f32,
        zmax: &mut f32,
    ) -> (bool, (f32, f32), (f32, f32)) {
        let cp = t.r2c_tile(p.0, p.1);
        let cq = t.r2c_tile(q.0, q.1);
        let (ox, oy) = t.origin();
        let (mut zl, mut zh) = (*zmin - t.z, *zmax - t.z);
        let (ok, c1, c2) = t.mesh.walk_line(ox, oy, cp, cq, &mut zl, &mut zh);
        *zmin = t.z + zl;
        *zmax = t.z + zh;
        (ok, t.c2r_tile(c1.0, c1.1), t.c2r_tile(c2.0, c2.1))
    }

    /// `Mesh::clear_of_blockers` for a placed tile: area coords in.
    pub(super) fn tile_clear_of_blockers(&self, t: &Tile, x1: f32, y1: f32, x2: f32, y2: f32, zl: f32, zh: f32, w: f32, flag: i32) -> bool {
        let (ox, oy) = t.origin();
        let a = rot_r2c(t.o, x1 - ox, y1 - oy);
        let b = rot_r2c(t.o, x2 - ox, y2 - oy);
        t.mesh.clear_of_blockers(a.0, a.1, b.0, b.1, zl - t.z, zh - t.z, w, flag)
    }

    // ---- The request ----

    /// Runs an NPC's whole request, as one uninterrupted call sequence.
    pub fn plan_request(&mut self) -> SimResult {
        let s = self.pf.start;
        let g = self.pf.goal;
        let (rad, hgt) = (self.pf.rad, self.pf.hgt);
        let mut d = g;
        if self.pf.range > 0.0 && (g.0 - s.0) * (g.0 - s.0) + (g.1 - s.1) * (g.1 - s.1) > self.pf.range * self.pf.range {
            let v = mesh::normalize([s.0 - g.0, s.1 - g.1, 0.0]);
            d = (v[0] * self.pf.range + g.0, v[1] * self.pf.range + g.1);
        }
        self.pf.calls = 1;
        let t = if d != s { self.test_direct_line(s.0, s.1, d.0, d.1, rad, hgt) } else { 1 };
        self.log(format!("direct line ({:.2},{:.2}) -> ({:.2},{:.2}) = {t}", s.0, s.1, d.0, d.1));
        let outcome;
        if t == 1 {
            self.pf.waypoints = vec![s, d];
            outcome = Outcome::Direct;
        } else if t == -1 || t == -2 {
            // The tile route runs in calls 1..=101; past that the request falls back.
            let calls_for = |lines: u64, me: &Self| (((lines as f64 * me.line_us) / me.call_us).ceil() as i32).max(1);
            self.line_cap = ((f64::from(self.calls_cap + 1) * self.call_us) / self.line_us) as u64;
            let r = self.plan_tile_route();
            let calls = calls_for(self.stats.intra_lines, self);
            self.stats.calls = calls;
            self.log(format!("tile route: {} line tests, about {calls} planning calls", self.stats.intra_lines));
            if r == ACTION_COMPLETE {
                outcome = Outcome::Complete;
            } else if r == ACTION_FAILED && self.calls_cap > calls && (d.0 - s.0) * (d.0 - s.0) + (d.1 - s.1) * (d.1 - s.1) + 0.0 < 400.0 {
                // The grid search starts from the next call.
                self.log("tile route failed within 20 m: grid search".into());
                let windows = (self.calls_cap + 1 - (calls + 1)).max(0) as usize;
                let per_window = (self.call_us / self.grid_line_us) as usize;
                match self.grid_fallback(windows, per_window) {
                    GridEnd::Path => outcome = Outcome::Grid,
                    GridEnd::NoPath => outcome = self.partial_fallback(s, d, "grid search failed"),
                    GridEnd::OutOfTime => outcome = self.partial_fallback(s, d, "grid search out of calls"),
                }
            } else {
                let why = if r == ACTION_FAILED { "tile route failed" } else { "tile route out of calls" };
                outcome = self.partial_fallback(s, d, why);
            }
        } else {
            outcome = self.partial_fallback(s, d, "direct line off the area");
        }
        let end = match &outcome {
            Outcome::Failed { .. } => s,
            _ => self.pf.waypoints.last().copied().unwrap_or(s),
        };
        SimResult {
            outcome,
            start: s,
            direct_line: t,
            waypoints: self.pf.waypoints.clone(),
            end,
            stats: self.stats.clone(),
            trace: self.trace.take().unwrap_or_default(),
        }
    }

    /// The 6-step bisection for the farthest straight-line point, each test starting from the last point found.
    fn partial_fallback(&mut self, s: (f32, f32), d: (f32, f32), why: &str) -> Outcome {
        if self.pf.no_partial != 0 {
            return Outcome::Failed { why: why.into() };
        }
        let (rad, hgt) = (self.pf.rad, self.pf.hgt);
        let (vx, vy) = (d.0 - s.0, d.1 - s.1);
        let (mut t, mut step) = (0.5f32, 0.25f32);
        let mut from = s;
        let mut found = None;
        for _ in 0..6 {
            let py = s.1 + vy * t;
            let px = vx * t + s.0;
            if self.test_direct_line(from.0, from.1, px, py, rad, hgt) == 1 {
                found = Some((px, py));
                from = (px, py);
                t += step;
            } else {
                t -= step;
            }
            step *= 0.5;
        }
        match found {
            Some(p) => {
                self.log(format!("{why}: partial line to ({:.2},{:.2})", p.0, p.1));
                self.pf.waypoints = vec![s, p];
                Outcome::Partial { why: why.into() }
            }
            None => Outcome::Failed { why: why.into() },
        }
    }

    // ---- Smoothing ----

    /// Smooths `pf.raw` into `pf.waypoints`: a pass that only follows the points (optionally in 1 m steps), two passes that jump to
    /// the farthest point the straight line reaches, then corner cutting.
    pub fn smooth_path(&mut self) {
        let raw = self.pf.raw.clone();
        if raw.is_empty() {
            self.pf.waypoints.clear();
        } else if self.pf.no_smooth == 1 {
            self.pf.waypoints = raw;
        } else {
            let p1 = self.smooth_pass(&raw, self.subdivide_first_pass, false);
            let p2 = self.smooth_pass(&p1, true, true);
            let p3 = self.smooth_pass(&p2, false, true);
            self.pf.waypoints = self.cut_corners(&p3);
        }
    }

    fn line_clear(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        self.test_direct_line(a.0, a.1, b.0, b.1, self.pf.rad, self.pf.hgt) == 1
    }

    /// One smoothing pass. With `shortcut`, from the last point kept it tries to jump ahead to a point the straight line reaches (the
    /// farthest first, halving the jump, then one by one for the last four); with `sub`, a leg of 1.1 m or more is broken into 1 m
    /// steps, walked from the near end.
    fn smooth_pass(&self, input: &[(f32, f32)], sub: bool, shortcut: bool) -> Vec<(f32, f32)> {
        let n = input.len();
        let mut out = vec![input[0]];
        let mut i = 0usize;
        let mut buf: Vec<(f32, f32)> = Vec::new();
        let mut pending: i32 = -1;
        loop {
            let p;
            if i + 1 < n {
                let mut jumped = None;
                if shortcut {
                    let mut k = n - (i + 1);
                    while k > 0 {
                        let j = i + k;
                        if self.line_clear(*out.last().expect("out starts with the path's first point"), input[j]) {
                            jumped = Some(j);
                            break;
                        }
                        if k <= 4 { k -= 1 } else { k >>= 1 }
                    }
                }
                if let Some(j) = jumped {
                    p = input[j];
                    i = j;
                    buf.clear();
                    pending = 0;
                } else if pending > 0 {
                    p = buf[(pending - 1) as usize];
                    pending -= 1;
                } else {
                    i += 1;
                    p = input[i];
                    buf.clear();
                    pending = 0;
                }
            } else {
                if pending < 1 {
                    return out;
                }
                p = buf[(pending - 1) as usize];
                pending -= 1;
            }
            let mut p = p;
            if sub {
                let l = *out.last().expect("out starts with the path's first point");
                let (dx, dy) = (p.0 - l.0, p.1 - l.1);
                let d = (dx * dx + dy * dy).sqrt();
                if f64::from(d) >= 1.1 {
                    buf.clear();
                    let mut s = d;
                    while s > 0.0 {
                        buf.push(((s * (p.0 - l.0)) / d + l.0, (s * (p.1 - l.1)) / d + l.1));
                        s -= 1.0;
                    }
                    pending = buf.len() as i32 - 1;
                    p = buf[pending as usize];
                }
            }
            out.push(p);
        }
    }

    /// For the corner a->b->c (when it turns by more than about 25 degrees), the widest cut: the points on b->a and b->c, at the same
    /// distance from b, joined by a clear straight line (bisection on the distance).
    fn best_corner(&self, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> Option<((f32, f32), (f32, f32))> {
        let (bax, bay) = (b.0 - a.0, b.1 - a.1);
        let (cbx, cby) = (c.0 - b.0, c.1 - b.1);
        let lba = (bax * bax + bay * bay).sqrt();
        let lcb = (cbx * cbx + cby * cby).sqrt();
        if !(lba >= 1e-4 && lcb >= 1e-4 && (bax * cbx + bay * cby) / (lcb * lba) < 0.9) {
            return None;
        }
        let m = if lba <= lcb { lba } else { lcb };
        let (mut t, mut step) = (0.5f32, 0.25f32);
        let mut out = None;
        loop {
            let s = m * t;
            let x = (b.0 + ((a.0 - b.0) * s) / lba, b.1 + ((a.1 - b.1) * s) / lba);
            let y = (b.0 + ((c.0 - b.0) * s) / lcb, b.1 + ((c.1 - b.1) * s) / lcb);
            if self.line_clear(x, y) {
                out = Some((x, y));
                t += step;
            } else {
                t -= step;
            }
            step *= 0.5;
            #[expect(clippy::neg_cmp_op_on_partial_ord, reason = "a NaN must take this branch too, as on the server")]
            if !(m * step > 0.1) {
                break;
            }
        }
        out
    }

    /// Cuts the path's corners with `best_corner`, cutting the two new corners a cut makes once more.
    fn cut_corners(&self, input: &[(f32, f32)]) -> Vec<(f32, f32)> {
        let n = input.len();
        let mut out = vec![input[0]];
        for i in 1..n.saturating_sub(1) {
            let a = *out.last().expect("out starts with the path's first point");
            let (b, c) = (input[i], input[i + 1]);
            match self.best_corner(a, b, c) {
                None => out.push(b),
                Some((p1, p2)) => {
                    if let Some((q1, q2)) = self.best_corner(a, p1, p2) {
                        out.push(q1);
                        out.push(q2);
                        match self.best_corner(q2, p2, c) {
                            Some((r1, r2)) => {
                                out.push(r1);
                                out.push(r2);
                            }
                            None => out.push(p2),
                        }
                    } else if let Some((r1, r2)) = self.best_corner(p1, p2, c) {
                        out.push(p1);
                        out.push(r1);
                        out.push(r2);
                    } else {
                        out.push(p1);
                        out.push(p2);
                    }
                }
            }
        }
        if n >= 2 {
            out.push(input[n - 1]);
        }
        out
    }
}

/// Plans one request on `area`.
pub fn simulate(area: &Area, q: &Query, trace: bool) -> SimResult {
    let mut p = Planner::new(area, q, trace);
    if q.spawn {
        match p.find_safe_spot(q.from, 20.0) {
            Some(s) => {
                if s != q.from {
                    p.log(format!("creating the creature places it at ({:.2},{:.2})", s.0, s.1));
                }
                p.pf.start = s;
            }
            None => p.log("creating the creature finds no safe location; left where asked".into()),
        }
    }
    p.plan_request()
}
