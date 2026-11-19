// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The grid search, the fallback a request uses when the tile route fails within 20 m of the goal: a best-first walk from the start
//! toward the goal over a grid whose step is the creature's (0.1 m for most creatures), each step checked with the straight-line test.
//!
//! Grid frame: origin at the goal, j axis toward the start, i axis to its side; the start is (0, N).
//!
//! Up to 32 paths grow side by side: the one whose end is nearest the goal is extended next, trying the step straight at the goal
//! first and then the steps beside it, alternating sides; a path that finds a second way on forks. A successful first step is followed
//! by a jump toward the goal as far as the line allows (bisection).
//!
//! The server plans in time slices (1000 us a call): the search gives up and takes the best partial path when a whole call makes no
//! progress, and the request falls back to a straight line when it runs out of calls. Calls are modelled as windows of straight-line
//! tests (see `Planner::line_us`).

/// How the grid search ended.
pub enum GridEnd {
    /// A path, to the goal or to the closest point found.
    Path,
    /// No point beyond the start.
    NoPath,
    /// Still improving when the calls ran out.
    OutOfTime,
}

use std::collections::HashMap;

use super::area::Planner;

#[derive(Clone, Copy, Debug)]
struct PathInfo {
    /// Squared grid distance of the last point to the goal.
    dist2: i32,
    count: i32,
    cost: i32,
    i: i32,
    j: i32,
    dead: bool,
}

struct Grid {
    /// Goal, i axis, j axis (world, one grid step long).
    gx: f32,
    gy: f32,
    ix: f32,
    iy: f32,
    jx: f32,
    jy: f32,
    /// The start's distance from the goal in grid steps.
    n: i32,
    /// Path capacity.
    cap: i32,
    /// Searched-flag grid half size.
    half: i32,
    searched: HashMap<i64, bool>,
    paths: Vec<(PathInfo, Vec<(i32, i32)>)>,
    /// Best partial path and its end's squared distance to the goal.
    best: Vec<(i32, i32)>,
    best_dist2: i32,
    start: (f32, f32),
    range_steps: i32,
}

impl Grid {
    fn flat(&self, i: i32, j: i32) -> i64 {
        i64::from(i + self.half) + i64::from(j + self.half) * i64::from(self.half * 2 + 1)
    }

    fn searched(&self, i: i32, j: i32) -> bool {
        self.searched.get(&self.flat(i, j)).copied().unwrap_or(false)
    }

    /// The world point of grid point (i, j).
    fn src(&self, i: i32, j: i32) -> (f32, f32) {
        let (fi, fj) = (i as f32, j as f32);
        ((fi * self.ix + self.gx) + fj * self.jx, (self.iy * fi + self.gy) + self.jy * fj)
    }

    /// Appends (i, j) to path k (cut back to its count first) and marks it searched; keeps the best partial path.
    fn add(&mut self, k: usize, i: i32, j: i32, cost: i32) {
        let f = self.flat(i, j);
        let p = &mut self.paths[k];
        p.1.truncate(p.0.count as usize);
        p.1.push((i, j));
        p.0.count += 1;
        p.0.cost += cost;
        p.0.i = i;
        p.0.j = j;
        p.0.dist2 = j * j + i * i;
        self.searched.insert(f, true);
        if p.0.dist2 < self.best_dist2 {
            self.best_dist2 = p.0.dist2;
            self.best = p.1.clone();
        }
    }

    /// Starts a new, empty path: at most 32.
    fn create(&mut self) -> Option<usize> {
        if self.paths.len() >= 32 {
            return None;
        }
        self.paths.push((PathInfo { dist2: 1_000_000, count: 0, cost: 0, i: 0, j: 0, dead: false }, Vec::new()));
        Some(self.paths.len() - 1)
    }
}

/// A direction index (mod 8, 0 toward the goal) -> grid step.
fn step_of(d: i32) -> (i32, i32) {
    let mut d = d;
    if d > 4 {
        d -= 8;
    } else if d < -3 {
        d += 8;
    }
    match d {
        -2 => (-1, 0),
        -1 => (-1, -1),
        0 => (0, -1),
        1 => (1, -1),
        2 => (1, 0),
        3 => (1, 1),
        4 => (0, 1),
        _ => (-1, 1),
    }
}

impl<'a> Planner<'a> {
    fn grid_line(&mut self, a: (f32, f32), b: (f32, f32)) -> bool {
        self.stats.grid_lines += 1;
        self.test_direct_line(a.0, a.1, b.0, b.1, self.pf.rad, self.pf.hgt) == 1
    }

    /// Tries one step from (i, j) by (di, dj); with `jump`, instead the farthest clear point toward the goal (or toward the point
    /// `range_steps` short of it). Returns whether it moved and where to.
    fn grid_try_step(&mut self, g: &Grid, count: i32, i: i32, j: i32, di: i32, dj: i32, jump: bool) -> (bool, (i32, i32)) {
        let mut out = (i, j);
        let mut src = if count == 0 { g.start } else { g.src(i, j) };
        if !jump {
            let (fi, fj) = ((i + di) as f32, (j + dj) as f32);
            let t = (fj * g.jx + fi * g.ix + g.gx, g.iy * fi + g.gy + g.jy * fj);
            if self.grid_line(src, t) {
                return (true, (i + di, j + dj));
            }
            return (false, out);
        }
        // Jump toward the goal (or the point range_steps short of it).
        let (ti, tj, target) = if g.range_steps < 1 {
            (0, 0, (g.gx, g.gy))
        } else {
            let r = g.range_steps as f32;
            let (a, b);
            if i.abs() < j.abs() {
                let q = i as f32 / j as f32;
                let k = (1.0 / (q * q + 1.0)).sqrt() * r;
                a = q * k;
                b = k;
            } else {
                let q = j as f32 / i as f32;
                let k = (1.0 / (q * q + 1.0)).sqrt() * r;
                a = k;
                b = k * q;
            }
            let (ti, tj) = (a as i32, b as i32);
            (ti, tj, g.src(ti, tj))
        };
        if self.grid_line(src, target) {
            return (true, (ti, tj));
        }
        let mut found = false;
        let (mut t, mut step) = (0.5f32, 0.25f32);
        let mut k = 6;
        loop {
            let ci = ((ti - i) as f32 * t) as i32 + i;
            let cj = ((tj - j) as f32 * t) as i32 + j;
            if ci != i || cj != j {
                let w = g.src(ci, cj);
                if self.grid_line(src, w) {
                    t += step;
                    found = true;
                    out = (ci, cj);
                    src = w;
                } else {
                    t -= step;
                }
            }
            k -= 1;
            step *= 0.5;
            if k == 0 {
                return (found, out);
            }
        }
    }

    /// Extends path k by one step (forking on a second success), or marks it dead. The direction to the goal is taken from the end
    /// point's slope, in eight sectors.
    fn grid_extend(&mut self, g: &mut Grid, k: usize) {
        let info = g.paths[k].0;
        let (i, j, count) = (info.i, info.j, info.count);
        let (base, sense): (i32, i32);
        if i == 0 {
            base = if j < 0 { 4 } else { 0 };
            sense = if j < 0 { -1 } else { 1 };
        } else {
            let ratio = j as f32 / f64::from(i.unsigned_abs()) as f32;
            if ratio < -2.0 {
                base = 4;
                sense = if i < 0 { 1 } else { -1 };
            } else if ratio < -0.5 {
                if i >= 0 {
                    base = -3;
                    sense = if -1.0 <= ratio { 1 } else { -1 };
                } else {
                    base = 3;
                    sense = if -1.0 > ratio { 1 } else { -1 };
                }
            } else if ratio < 0.5 {
                if i >= 0 {
                    base = -2;
                    sense = if 0.0 <= ratio { 1 } else { -1 };
                } else {
                    base = 2;
                    sense = if 0.0 > ratio { 1 } else { -1 };
                }
            } else if ratio < 2.0 {
                let b = if i < 0 { 1 } else { -1 };
                base = b;
                sense = if 1.0 > ratio { b } else { -b };
            } else {
                base = 0;
                sense = if i < 0 { -1 } else { 1 };
            }
        }
        let r = g.n - count + 36;
        let r2 = r * r;
        let mut sides = 0i32;
        let mut extended = false;
        for u in 0..8i32 {
            let side = u & 1;
            if u != 0 && u != 7 && sides & (side + 1) != 0 {
                continue;
            }
            let d = (side * 2 - 1) * sense * ((u + 1) >> 1) + base;
            let (di, dj) = step_of(d);
            let (ni, nj) = (i + di, j + dj);
            if ni * ni + nj * nj > r2 {
                continue;
            }
            if g.searched(ni, nj) {
                if u != 0 {
                    sides |= side + 1;
                }
                continue;
            }
            let (ok, mut out) = self.grid_try_step(g, count, i, j, di, dj, false);
            let first = u == 0 && ok;
            if first && !(di == i && dj == j) {
                let (_, o2) = self.grid_try_step(g, count, out.0, out.1, di, dj, true);
                out = o2;
            }
            self.stats.grid_steps += 1;
            if !ok {
                continue;
            }
            if u == 0 {
                if !extended {
                    g.add(k, out.0, out.1, 1);
                    return;
                }
            } else if !extended {
                sides |= side + 1;
                g.add(k, out.0, out.1, 1);
                extended = true;
                continue;
            }
            // A second success: fork the path as it was at entry.
            if u == 7 {
                return;
            }
            let Some(nk) = g.create() else { return };
            let pts: Vec<(i32, i32)> = g.paths[k].1[..count as usize].to_vec();
            g.paths[nk] = (info, pts);
            g.add(nk, out.0, out.1, 1);
            return;
        }
        if !extended {
            g.paths[k].0.dead = true;
        }
    }

    /// The grid search over `windows` calls of `per_window` line tests each:
    /// to the goal, until no path is left, or until a call brings no new best.
    pub fn grid_fallback(&mut self, windows: usize, per_window: usize) -> GridEnd {
        let s = self.pf.start;
        let gl = self.pf.goal;
        let rad = self.pf.rad;
        let step = if 0.1 < rad && rad <= 0.65 {
            0.1
        } else if 0.1 <= rad {
            rad
        } else {
            0.1
        };
        let v = super::mesh::normalize([s.0 - gl.0, s.1 - gl.1, 0.0]);
        let (sx, sy) = (s.0 - gl.0, s.1 - gl.1);
        let len = (sx * sx + sy * sy).sqrt();
        let n = (f64::from(len / step) + 0.5) as i32;
        let cap = n / 4 + 36;
        let mut g = Grid {
            gx: gl.0,
            gy: gl.1,
            ix: v[1] * step,
            iy: -(v[0] * step),
            jx: v[0] * step,
            jy: v[1] * step,
            n,
            cap,
            half: n / 4 + 35,
            searched: HashMap::new(),
            paths: Vec::new(),
            best: Vec::new(),
            best_dist2: n * n,
            start: s,
            range_steps: (self.pf.range / step) as i32,
        };
        let p0 = g.create().expect("a new search has no paths yet");
        g.searched.insert(g.flat(0, n), true);
        g.add(p0, 0, n, 1);
        let start_lines = self.stats.grid_lines;
        let mut window = 0usize;
        let mut window_best = g.best_dist2;
        let mut last_pick = g.best_dist2;
        let reached;
        loop {
            if self.stats.grid_lines - start_lines >= (window + 1) * per_window {
                // A call's time is up: go on only if it found a new best.
                window += 1;
                if self.trace.is_some() {
                    let live = g.paths.iter().filter(|(i, _)| !i.dead && i.count < g.cap).count();
                    let msg = format!(
                        "  grid call {window}: best {:.2} m, {} paths ({live} live), last pick {:.2} m",
                        (g.best_dist2 as f32).sqrt() * step,
                        g.paths.len(),
                        (last_pick as f32).sqrt() * step
                    );
                    self.log(msg);
                }
                if g.best_dist2 >= window_best {
                    reached = None;
                    break;
                }
                if window >= windows {
                    self.log(format!("grid search: still improving after {windows} calls"));
                    return GridEnd::OutOfTime;
                }
                // The best distance is then reset to the last pick's, so a worse path may become the best.
                g.best_dist2 = last_pick;
                window_best = g.best_dist2;
            }
            let mut pick = None;
            let mut bestd = 1_000_000;
            for (k, (info, _)) in g.paths.iter().enumerate() {
                if !info.dead && info.count < g.cap && info.dist2 < bestd {
                    bestd = info.dist2;
                    pick = Some(k);
                }
            }
            let Some(k) = pick else {
                reached = None;
                break;
            };
            last_pick = bestd;
            self.grid_extend(&mut g, k);
            let info = g.paths[k].0;
            if !(info.dist2 > 0 && g.range_steps * g.range_steps < info.dist2) {
                reached = Some(k);
                break;
            }
        }
        let pts: Vec<(i32, i32)> = match reached {
            Some(k) => g.paths[k].1.clone(),
            None => {
                if g.best.len() < 2 {
                    self.log(format!("grid search: no progress ({} paths)", g.paths.len()));
                    return GridEnd::NoPath;
                }
                g.best.clone()
            }
        };
        self.log(format!(
            "grid search: {} ({} points, {} paths, best {:.2} m from the goal)",
            if reached.is_some() { "reached the goal" } else { "stopped; best partial path" },
            pts.len(),
            g.paths.len(),
            (g.best_dist2 as f32).sqrt() * step
        ));
        // Line-of-sight pruning of the grid points before smoothing.
        let emit = |p: (i32, i32)| {
            let (fi, fj) = (p.0 as f32, p.1 as f32);
            ((fi * g.ix + g.gx) + fj * g.jx, (g.iy * fi + g.gy) + g.jy * fj)
        };
        let probe = |p: (i32, i32)| {
            let (fi, fj) = (p.0 as f32, p.1 as f32);
            ((fi * g.ix + g.gx) + fj * g.jx, (g.jy * fj + g.iy * fi) + g.gy)
        };
        let mut raw = vec![s];
        let mut last = s;
        let m = pts.len();
        for k in 1..m {
            if m - 1 <= k || !self.grid_line(last, probe(pts[k + 1])) {
                last = emit(pts[k]);
                raw.push(last);
            }
        }
        self.pf.raw = raw;
        self.smooth_path();
        GridEnd::Path
    }
}

#[cfg(test)]
mod tests {
    use super::step_of;

    #[test]
    fn grid_directions_go_round() {
        // 0 heads for the goal (j decreasing); +/-1 the diagonals beside it; indices wrap mod 8.
        let ring: Vec<(i32, i32)> = (-3..=4).map(step_of).collect();
        assert_eq!(ring, vec![(-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1)]);
        assert_eq!(step_of(5), step_of(-3));
        assert_eq!(step_of(-4), step_of(4));
        assert_eq!(step_of(8), step_of(0));
    }
}
