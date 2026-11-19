// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Ground truth for pathing: which points of the terrain are connected, random start/goal pairs, and an NWScript that walks a ghosted
//! creature between each pair in one server run. The server's results are the oracle the planner simulator is checked against.

/// Walkable connectivity of the whole area on a fine grid.
pub struct Reach {
    /// The grid's spacing (m).
    pub step: f32,
    /// Samples across.
    pub w: usize,
    /// Samples up.
    pub h: usize,
    /// Component of each sample (-1: not walkable).
    pub comp: Vec<i32>,
}

impl Reach {
    /// `height` and `blocked` describe the terrain in area coordinates; ground between neighbouring samples is walkable if its rise is
    /// under `max_slope` (rise over run).
    pub fn new(
        width_m: f32,
        height_m: f32,
        step: f32,
        max_slope: f32,
        height: impl Fn(f32, f32) -> f32,
        blocked: impl Fn(f32, f32) -> bool,
    ) -> Reach {
        let (w, h) = ((width_m / step) as usize + 1, (height_m / step) as usize + 1);
        let z: Vec<f32> = (0..w * h).map(|i| height((i % w) as f32 * step, (i / w) as f32 * step)).collect();
        let steep = |a: usize, b: usize| (z[a] - z[b]).abs() > max_slope * step;
        // A sample is walkable if no neighbour is too steep and it is not blocked.
        let walk: Vec<bool> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let mut ok = !blocked(x as f32 * step, y as f32 * step);
                if x > 0 {
                    ok &= !steep(i, i - 1);
                }
                if x + 1 < w {
                    ok &= !steep(i, i + 1);
                }
                if y > 0 {
                    ok &= !steep(i, i - w);
                }
                if y + 1 < h {
                    ok &= !steep(i, i + w);
                }
                ok
            })
            .collect();
        let mut comp = vec![-1i32; w * h];
        let mut next = 0;
        for start in 0..w * h {
            if !walk[start] || comp[start] >= 0 {
                continue;
            }
            let mut stack = vec![start];
            comp[start] = next;
            while let Some(i) = stack.pop() {
                let (x, y) = (i % w, i / w);
                let mut push = |j: usize| {
                    if walk[j] && comp[j] < 0 {
                        comp[j] = next;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    push(i - 1);
                }
                if x + 1 < w {
                    push(i + 1);
                }
                if y > 0 {
                    push(i - w);
                }
                if y + 1 < h {
                    push(i + w);
                }
            }
            next += 1;
        }
        Reach { step, w, h, comp }
    }

    /// Components joined only through passages a creature fits: walkable samples with walkable ground `k` samples all round (a strip
    /// under about 2k+1 samples wide joins nothing). -1 elsewhere.
    pub fn wide(&self, k: usize) -> Reach {
        let (w, h) = (self.w, self.h);
        let core: Vec<bool> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let k = k as i64;
                self.comp[i] >= 0
                    && (-k..=k).all(|dy| {
                        (-k..=k).all(|dx| {
                            let (a, b) = (x + dx, y + dy);
                            // The area's edge is no wall.
                            a < 0 || b < 0 || a >= w as i64 || b >= h as i64 || self.comp[b as usize * w + a as usize] >= 0
                        })
                    })
            })
            .collect();
        let mut comp = vec![-1i32; w * h];
        let mut next = 0;
        for start in 0..w * h {
            if !core[start] || comp[start] >= 0 {
                continue;
            }
            let mut stack = vec![start];
            comp[start] = next;
            while let Some(i) = stack.pop() {
                let (x, y) = (i % w, i / w);
                for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
                    .into_iter()
                    .flatten()
                {
                    if core[j] && comp[j] < 0 {
                        comp[j] = next;
                        stack.push(j);
                    }
                }
            }
            next += 1;
        }
        Reach { step: self.step, w, h, comp }
    }

    /// The component at area point (`x`, `y`): the nearest sample's, -1 when not walkable or outside the area.
    pub fn at(&self, x: f32, y: f32) -> i32 {
        let (i, j) = ((x / self.step).round() as usize, (y / self.step).round() as usize);
        if i >= self.w || j >= self.h { -1 } else { self.comp[j * self.w + i] }
    }

    /// The component under a point, if every sample within `margin` agrees.
    pub fn solid(&self, x: f32, y: f32, margin: f32) -> Option<i32> {
        let c = self.at(x, y);
        if c < 0 {
            return None;
        }
        let k = (margin / self.step).ceil() as i32;
        for dj in -k..=k {
            for di in -k..=k {
                if self.at(x + di as f32 * self.step, y + dj as f32 * self.step) != c {
                    return None;
                }
            }
        }
        Some(c)
    }
}

/// A walk the oracle sends a creature on: from, to, and whether the ground joins them.
#[derive(Debug, Clone, Copy)]
pub struct Pair {
    /// Where the walk starts (area metres).
    pub from: (f32, f32),
    /// Where it should end.
    pub to: (f32, f32),
    /// Whether the walkmesh joins the two: a route the server can't find there is no fault of the export.
    pub reachable: bool,
}

/// `n` random pairs at least 5 m apart, each point on solid walkable ground
/// and away from tile borders (where the tile a point is in is ambiguous).
pub fn pairs(reach: &Reach, area_w: f32, area_h: f32, tile: f32, n: usize, seed: u64) -> Vec<Pair> {
    let mut rng = fastrand::Rng::with_seed(seed);
    // A random point with room around it (None if a long search finds none: an area with no open ground has no pairs).
    let point = |rng: &mut fastrand::Rng| {
        for _ in 0..20_000 {
            let (x, y) = (rng.f32() * area_w, rng.f32() * area_h);
            let off_border = |c: f32| {
                let r = c % tile;
                (0.75..tile - 0.75).contains(&r)
            };
            if off_border(x)
                && off_border(y)
                && let Some(c) = reach.solid(x, y, 1.0)
            {
                return Some(((x * 10.0).round() / 10.0, (y * 10.0).round() / 10.0, c));
            }
        }
        None
    };
    let mut out = Vec::new();
    let mut tries = 0;
    while out.len() < n && tries < n * 50 {
        tries += 1;
        let (Some((ax, ay, ac)), Some((bx, by, bc))) = (point(&mut rng), point(&mut rng)) else { break };
        if ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt() < 5.0 {
            continue;
        }
        out.push(Pair { from: (ax, ay), to: (bx, by), reachable: ac == bc });
    }
    out
}

/// A test script that walks a ghosted, high-AI creature for each pair and logs where each one is after `seconds`.
pub fn script(pairs: &[Pair], seconds: f32) -> String {
    let mut s = String::from(
        "#include \"mg_test\"\n\n\
         void Done(object o, int nPair)\n{\n    vector p = GetPosition(o);\n    \
         WriteTimestampedLogEntry(\"LSRES \" + IntToString(nPair) + \" \" + FloatToString(p.x, 0, 2) + \" \" + \
         FloatToString(p.y, 0, 2));\n}\n\n\
         void Walk(int nPair, float x0, float y0, float x1, float y1)\n{\n    object oArea = GetFirstArea();\n    \
         object o = CreateObject(OBJECT_TYPE_CREATURE, \"nw_dog\", Location(oArea, Vector(x0, y0, 0.0), 0.0));\n    \
         if (!GetIsObjectValid(o)) { WriteTimestampedLogEntry(\"LSRES \" + IntToString(nPair) + \" nospawn\"); return; }\n    \
         ApplyEffectToObject(DURATION_TYPE_PERMANENT, SupernaturalEffect(EffectCutsceneGhost()), o);\n    \
         SetAILevel(o, AI_LEVEL_HIGH);\n    \
         AssignCommand(o, ActionMoveToLocation(Location(oArea, Vector(x1, y1, 0.0), 0.0), TRUE));\n",
    );
    s += &format!("    DelayCommand({seconds:.1}, Done(o, nPair));\n}}\n\nvoid main()\n{{\n");
    for (i, p) in pairs.iter().enumerate() {
        s += &format!("    Walk({i}, {:.1}, {:.1}, {:.1}, {:.1});\n", p.from.0, p.from.1, p.to.0, p.to.1);
    }
    s += "    Pass(\"pairs started\");\n}\n";
    s
}

/// Parses `LSRES <i> <x> <y>` lines: each pair's final position.
pub fn results(log: &str, n: usize) -> Vec<Option<(f32, f32)>> {
    let mut out = vec![None; n];
    for line in log.lines() {
        let Some(rest) = line.split("LSRES ").nth(1) else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        if let (Some(i), Some(x), Some(y)) =
            (f.first().and_then(|v| v.parse::<usize>().ok()), f.get(1).and_then(|v| v.parse().ok()), f.get(2).and_then(|v| v.parse().ok()))
            && i < n
        {
            out[i] = Some((x, y));
        }
    }
    out
}
