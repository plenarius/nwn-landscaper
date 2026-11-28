// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Cutting a canvas into pieces (docs/DESIGN-canvas-cut.md, build step 4): each piece becomes the area files this engine reads,
//! with its halo from the canvas around it, the water seeded inside it, the things that stand in it, and the crossings along its
//! cuts. The routes are serve/canvases.rs; the canvas's storage is `crate::canvas`.
//!
//! The canvas header carries, besides what `crate::canvas` checks, the landscape's lists in canvas terms (metres from the
//! canvas's south-west corner; samples on the canvas's grid, 16 a tile plus one):
//!
//! - `palette` (the blocks' paint and blend layer index it), `water` (`[{ i, j, level, bounds? }]`), `objects`, `things`, `entrances`, `ambience`, `lights` (two a tile over
//!   the whole canvas, rows from the south), `shadow`, `settings` and `atmosphere` (each piece's own, `pieces[k].settings` and
//!   `pieces[k].atmosphere`, over the canvas's);
//! - `crossingsOff`: stretches the builder turned off, by their key `x<i>_<j>` (the stretch's middle sample);
//! - `startPiece`: the piece a player comes in by, for the canvas route check (else the first piece).
//! - `tagPrefix`: what the engine's tags start with in every piece (else the canvas's name): a canvas made from an area keeps
//!   that area's name here, so the doors and objects it had keep their tags.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::canvas::{BLOCK_TILES, Canvas, Header, TILE_SAMPLES};

/// Metres between samples.
const SPACING: f32 = 0.625;
/// Metres a tile.
const TILE_M: f32 = 10.0;
/// The halo each piece carries: the most the area format takes (5 tiles), the depth the client's edge scenery covers.
pub const HALO: usize = crate::heightmap::MAX_HALO_TILES * TILE_SAMPLES;
/// A crossing's trigger: how deep it reaches into its piece (m), and how far its arrival waypoint stands from the cut (m), past
/// the other side's trigger so nobody is sent straight back.
const TRIGGER_DEPTH: f32 = 1.5;
const ARRIVE: f32 = 4.0;
/// Samples each side of a cut that must be walkable for a crossing: out to the arrival waypoint.
const REACH: isize = 7;
/// The longest crossing, in samples (2 tiles), and the shortest stretch worth one (about 2 m).
const MOST: usize = 32;
const LEAST: usize = 3;
/// Water deeper than this (m) can't be walked (the generator's own).
const SHALLOW: f32 = 0.5;
/// The game's script that sends whoever enters a transition to its target; walking into a trigger only runs its OnEnter.
const CROSS_SCRIPT: &str = "nw_g0_transition";
/// A solid object this close to a cut (m) is walked through from the other side.
const NEAR_CUT: f32 = 2.0;

/// A piece of the canvas, in tiles from its south-west corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub id: u64,
    pub name: String,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub json: Value,
}

impl Piece {
    /// Its rectangle in canvas metres: west, south, east, north.
    fn metres(&self) -> [f32; 4] {
        let t = |v: usize| v as f32 * TILE_M;
        [t(self.x), t(self.y), t(self.x + self.w), t(self.y + self.h)]
    }

    /// Whether canvas point (x, y) is in it: its west and south edges in, its east and north out, except on the canvas's own
    /// east and north border, so a point on a cut is in exactly one piece.
    fn holds(&self, x: f32, y: f32, canvas: [f32; 2]) -> bool {
        let [w, s, e, n] = self.metres();
        x >= w && y >= s && (x < e || (x == e && e == canvas[0])) && (y < n || (y == n && n == canvas[1]))
    }

    /// Whether canvas point (x, y) is in it or on its edges.
    fn covers(&self, x: f32, y: f32) -> bool {
        let [w, s, e, n] = self.metres();
        (w..=e).contains(&x) && (s..=n).contains(&y)
    }
}

/// What the tags of canvas `canvas`'s pieces start with: its header's `tagPrefix` (a canvas made from an area keeps that area's,
/// so its doors keep their tags), else its name.
fn tag_prefix<'a>(canvas: &'a str, header: &'a Header) -> &'a str {
    header.json["tagPrefix"].as_str().unwrap_or(canvas)
}

/// The pieces in `header`, as `crate::canvas::check_pieces` checked them.
pub fn pieces(header: &Header) -> Vec<Piece> {
    header.json["pieces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let u = |k: &str| p[k].as_u64().map(|v| v as usize);
            Some(Piece {
                id: p["id"].as_u64()?,
                name: p["name"].as_str()?.to_string(),
                x: u("x")?,
                y: u("y")?,
                w: u("w")?,
                h: u("h")?,
                json: p.clone(),
            })
        })
        .collect()
}

/// The whole canvas's ground, its blocks put together, and its water flooded.
pub struct Ground {
    /// Samples across and up.
    pub w: usize,
    pub h: usize,
    pub heights: Vec<f32>,
    pub paint: Vec<u8>,
    pub chasm: Vec<bool>,
    /// The soft paint's blend layer, [second, weight] a sample; empty: no block has one.
    pub blend: Vec<[u8; 2]>,
    /// Each sample's water level (NaN: dry), as `crate::heightmap::flood` gives it.
    pub water: Vec<f32>,
    /// The water bodies, each sample's level (as f32 bits) to the body's own.
    levels: BTreeMap<u32, f64>,
}

impl Ground {
    /// Canvas `c`'s ground: every block (flat ground for one never saved), the water flooded from the header's seeds on the
    /// whole canvas, chasms walls to it as in an area.
    pub fn load(c: &Canvas, header: &Header) -> Result<Ground> {
        let (w, h) = (header.size.tiles_x * TILE_SAMPLES + 1, header.size.tiles_y * TILE_SAMPLES + 1);
        let mut heights = vec![header.ground; w * h];
        let mut paint = vec![0u8; w * h];
        let mut chasm = vec![false; w * h];
        let mut blend: Vec<[u8; 2]> = Vec::new();
        for id in header.size.all() {
            let (b, _) = c.block(header, id)?;
            let (x0, y0) = (id.x * BLOCK_TILES * TILE_SAMPLES, id.y * BLOCK_TILES * TILE_SAMPLES);
            for j in 0..b.height {
                let (from, to) = (j * b.width, (y0 + j) * w + x0);
                heights[to..to + b.width].copy_from_slice(&b.heights[from..from + b.width]);
                paint[to..to + b.width].copy_from_slice(&b.paint[from..from + b.width]);
                if let Some(m) = &b.chasm {
                    for i in 0..b.width {
                        chasm[to + i] = m[from + i] == 1;
                    }
                }
                if let Some(l) = &b.blend {
                    if blend.is_empty() {
                        blend = vec![[0, 0]; w * h];
                    }
                    blend[to..to + b.width].copy_from_slice(&l[from..from + b.width]);
                }
            }
        }
        let mut g = Ground::new(w, h, heights, paint, chasm, &header.json["water"])?;
        g.blend = blend;
        Ok(g)
    }

    /// Ground from its grids, the water flooded from `water` (`[{ i, j, level, bounds? }]`).
    pub fn new(w: usize, h: usize, heights: Vec<f32>, paint: Vec<u8>, chasm: Vec<bool>, water: &Value) -> Result<Ground> {
        let mut bodies = Vec::new();
        for b in water.as_array().into_iter().flatten() {
            let bounds = b["bounds"].as_array().filter(|a| a.len() == 4).and_then(|a| {
                let v: Vec<usize> = a.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect();
                (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
            });
            let (Some(i), Some(j), Some(level)) = (b["i"].as_u64(), b["j"].as_u64(), b["level"].as_f64()) else {
                bail!("water: each body needs i, j and level");
            };
            bodies.push(crate::heightmap::Body { i: i as usize, j: j as usize, level, bounds });
        }
        let levels = bodies.iter().map(|b| ((b.level as f32).to_bits(), b.level)).collect();
        let walled: Vec<f32> = heights.iter().zip(&chasm).map(|(&v, &c)| if c { f32::INFINITY } else { v }).collect();
        let water = if bodies.is_empty() { Vec::new() } else { crate::heightmap::flood(&walled, w, &bodies) };
        Ok(Ground { w, h, heights, paint, chasm, blend: Vec::new(), water, levels })
    }

    /// The sample nearest (i, j), which may be off the canvas: its own edge carries on outward there.
    fn at(&self, i: isize, j: isize) -> usize {
        let i = i.clamp(0, self.w as isize - 1) as usize;
        let j = j.clamp(0, self.h as isize - 1) as usize;
        j * self.w + i
    }

    fn water_at(&self, k: usize) -> f32 {
        self.water.get(k).copied().unwrap_or(f32::NAN)
    }

    /// Whether a player can stand at sample (i, j): no chasm, no water deeper than they wade, no slope steeper than the walkmesh
    /// takes.
    fn walkable(&self, i: isize, j: isize) -> bool {
        let k = self.at(i, j);
        if self.chasm[k] {
            return false;
        }
        let l = self.water_at(k);
        if !l.is_nan() && l - self.heights[k] > SHALLOW {
            return false;
        }
        let h = |di: isize, dj: isize| self.heights[self.at(i + di, j + dj)];
        let (dx, dy) = ((h(1, 0) - h(-1, 0)) / (2.0 * SPACING), (h(0, 1) - h(0, -1)) / (2.0 * SPACING));
        let nz = 1.0 / (1.0 + dx * dx + dy * dy).sqrt();
        nz >= crate::generator::WALKABLE_NZ
    }
}

/// One way across a cut: a transition trigger in the piece a player leaves, leading to a waypoint in the one they arrive in.
#[derive(Clone, Debug, PartialEq)]
pub struct Crossing {
    /// The trigger's tag, `<canvas>_x<i>_<j><d>`, and its arrival waypoint's, `<canvas>_w<i>_<j><d>`: (i, j) the stretch's
    /// middle sample on the canvas, d the way the player goes (`n`, `s`, `e`, `w`).
    pub tag: String,
    pub waypoint: String,
    /// The stretch's key, `x<i>_<j>`, the same both ways: what `crossingsOff` names.
    pub key: String,
    /// The pieces it leads from and to, by name.
    pub from: String,
    pub to: String,
    pub dir: char,
    /// The trigger's outline and the waypoint's place and facing, canvas metres and radians from east.
    pub outline: [[f32; 2]; 4],
    pub arrive: [f32; 2],
    pub bearing: f32,
    /// The things' ids in the piece, from its place, so a re-export finds its own (above any id the editor gives).
    pub id: u32,
}

impl Crossing {
    fn json(&self) -> Value {
        json!({
            "tag": self.tag, "waypoint": self.waypoint, "key": self.key, "from": self.from, "to": self.to,
            "dir": self.dir.to_string(), "outline": self.outline, "arrive": self.arrive, "bearing": self.bearing,
        })
    }
}

/// The ids crossings' things take: from here up, the editor's never reach them, and still a game int (the `ls_thing` mark).
pub const CROSSING_IDS: u32 = 1_000_000_000;

/// A cut between two pieces: `a` west of (or south of) `b`, on tile line `at`, shared from tile `lo` to `hi`.
struct Cut<'a> {
    a: &'a Piece,
    b: &'a Piece,
    /// True: a west-east cut (a south of b); false: a north-south one (a west of b).
    across: bool,
    at: usize,
    lo: usize,
    hi: usize,
}

fn cuts(pieces: &[Piece]) -> Vec<Cut<'_>> {
    let mut out = Vec::new();
    for a in pieces {
        for b in pieces {
            if a.x + a.w == b.x {
                let (lo, hi) = (a.y.max(b.y), (a.y + a.h).min(b.y + b.h));
                if lo < hi {
                    out.push(Cut { a, b, across: false, at: b.x, lo, hi });
                }
            }
            if a.y + a.h == b.y {
                let (lo, hi) = (a.x.max(b.x), (a.x + a.w).min(b.x + b.w));
                if lo < hi {
                    out.push(Cut { a, b, across: true, at: b.y, lo, hi });
                }
            }
        }
    }
    out
}

/// Every crossing of the canvas: along each cut, each stretch where the ground is walkable on the cut and out to the arrival
/// waypoints on both sides, and no solid object stands, becomes crossings of at most 2 tiles, one each way. Stretches in
/// `crossingsOff` are left out. Also the warnings: solid objects players walk through from the other side of a cut.
pub fn crossings(canvas: &str, header: &Header, g: &Ground, pieces: &[Piece]) -> (Vec<Crossing>, Vec<Value>) {
    let prefix = tag_prefix(canvas, header);
    let off: BTreeSet<&str> = header.json["crossingsOff"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let solid: Vec<(f32, f32, f32, String)> = header.json["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["solid"].as_bool().unwrap_or(true))
        .filter_map(|o| {
            let r = o["scale"].as_f64().unwrap_or(1.0) as f32;
            Some((o["x"].as_f64()? as f32, o["y"].as_f64()? as f32, r, o["model"].as_str().unwrap_or("an object").to_string()))
        })
        .collect();
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for cut in cuts(pieces) {
        let c = (cut.at * TILE_SAMPLES) as isize;
        // (t along the cut, k across it: negative into a, positive into b)
        let sample = |t: isize, k: isize| if cut.across { (t, c + k) } else { (c + k, t) };
        let cut_m = cut.at as f32 * TILE_M;
        for (x, y, _, model) in &solid {
            let (along, off_cut) = if cut.across { (*x, *y - cut_m) } else { (*y, *x - cut_m) };
            if off_cut.abs() < NEAR_CUT && (cut.lo as f32 * TILE_M..=cut.hi as f32 * TILE_M).contains(&along) {
                let (mine, other) = if off_cut < 0.0 { (cut.a, cut.b) } else { (cut.b, cut.a) };
                warnings.push(json!({
                    "kind": "object_at_cut", "at": [x, y], "piece": mine.name,
                    "why": format!("{model} stands within {NEAR_CUT} m of the cut: players in {} walk through it", other.name),
                }));
            }
        }
        // (Off the pieces' corners by two samples: a trigger stays inside its piece.)
        let (t0, t1) = ((cut.lo * TILE_SAMPLES + 2) as isize, (cut.hi * TILE_SAMPLES - 2) as isize);
        let open = |t: isize| -> bool {
            let along = t as f32 * SPACING;
            (-REACH..=REACH).all(|k| {
                let (i, j) = sample(t, k);
                g.walkable(i, j)
            }) && !solid.iter().any(|(x, y, r, _)| {
                let (a, o) = if cut.across { (*x, *y - cut_m) } else { (*y, *x - cut_m) };
                (a - along).abs() < *r && o.abs() < REACH as f32 * SPACING + r
            })
        };
        let mut t = t0;
        while t <= t1 {
            if !open(t) {
                t += 1;
                continue;
            }
            let first = t;
            while t <= t1 && open(t) {
                t += 1;
            }
            let last = t - 1;
            let len = (last - first) as usize;
            if len < LEAST {
                continue;
            }
            let n = len.div_ceil(MOST);
            for p in 0..n {
                let (s0, s1) = (first + (len * p / n) as isize, first + (len * (p + 1) / n) as isize);
                let mid = (s0 + s1) / 2;
                let (i, j) = sample(mid, 0);
                let key = format!("x{i}_{j}");
                if off.contains(key.as_str()) {
                    continue;
                }
                let (lo, hi) = (s0 as f32 * SPACING, s1 as f32 * SPACING);
                let mid_m = mid as f32 * SPACING;
                // Each way: (from, to, direction, its index, which side the trigger is on: -1 a, +1 b)
                let ways = if cut.across {
                    [(cut.a, cut.b, 'n', 0u32, -1.0f32), (cut.b, cut.a, 's', 1, 1.0)]
                } else {
                    [(cut.a, cut.b, 'e', 2, -1.0), (cut.b, cut.a, 'w', 3, 1.0)]
                };
                for (from, to, dir, d, side) in ways {
                    // The trigger 1.5 m deep in the piece left, its outer side 5 cm in from the cut.
                    let (near, far) = (cut_m + side * 0.05, cut_m + side * TRIGGER_DEPTH);
                    let rect = |a0: f32, a1: f32, o0: f32, o1: f32| -> [[f32; 2]; 4] {
                        if cut.across { [[a0, o0], [a1, o0], [a1, o1], [a0, o1]] } else { [[o0, a0], [o0, a1], [o1, a1], [o1, a0]] }
                    };
                    let (o0, o1) = (near.min(far), near.max(far));
                    let outline = rect(lo, hi, o0, o1);
                    let arrive_off = cut_m - side * ARRIVE;
                    let arrive = if cut.across { [mid_m, arrive_off] } else { [arrive_off, mid_m] };
                    let bearing = match dir {
                        'n' => std::f32::consts::FRAC_PI_2,
                        's' => -std::f32::consts::FRAC_PI_2,
                        'e' => 0.0,
                        _ => std::f32::consts::PI,
                    };
                    out.push(Crossing {
                        tag: format!("{prefix}_x{i}_{j}{dir}"),
                        waypoint: format!("{prefix}_w{i}_{j}{dir}"),
                        key: key.clone(),
                        from: from.name.clone(),
                        to: to.name.clone(),
                        dir,
                        outline,
                        arrive,
                        bearing,
                        id: CROSSING_IDS + ((d * 2049 + j as u32) * 2049 + i as u32) * 2,
                    });
                }
            }
        }
    }
    (out, warnings)
}

/// One piece cut out: the area header as the engine reads it (grids counted with its halo, no file names), its heights and
/// paint (empty when the canvas is unpainted), and what the cut says about it.
pub struct Area {
    pub name: String,
    pub header: Value,
    pub heights: Vec<u8>,
    pub materials: Vec<u8>,
    /// Its blend layer, two bytes a sample of the same grid; empty: none.
    pub blend: Vec<u8>,
}

/// Piece `p` of canvas `canvas` as an area: the canvas's ground in it and `halo` samples round it (past the canvas's own edge,
/// its edge carried outward), one water seed for each part of the canvas's water in that grid (flooding inside it reaches the
/// same samples, since each part is joined inside it), what stands in it moved to its own metres (objects round it on its halo
/// too), and the crossings that leave it and arrive in it. Refused: a trigger or encounter whose outline crosses a cut, and a
/// cave entrance whose cave does.
pub fn area(canvas: &str, header: &Header, g: &Ground, p: &Piece, crossings: &[Crossing], halo: usize) -> Result<Area> {
    let (nx, ny) = (p.w * TILE_SAMPLES + 1, p.h * TILE_SAMPLES + 1);
    let (fw, fh) = (nx + 2 * halo, ny + 2 * halo);
    let (i0, j0) = ((p.x * TILE_SAMPLES) as isize - halo as isize, (p.y * TILE_SAMPLES) as isize - halo as isize);
    let src: Vec<usize> = (0..fh).flat_map(|b| (0..fw).map(move |a| (a, b))).map(|(a, b)| g.at(i0 + a as isize, j0 + b as isize)).collect();
    let heights: Vec<u8> = src.iter().flat_map(|&k| g.heights[k].to_le_bytes()).collect();
    let palette = header.json["palette"].as_array().cloned().unwrap_or_default();
    let materials: Vec<u8> = if palette.is_empty() { Vec::new() } else { src.iter().map(|&k| g.paint[k]).collect() };
    let blend: Vec<u8> =
        if materials.is_empty() || g.blend.is_empty() { Vec::new() } else { src.iter().flat_map(|&k| g.blend[k]).collect() };
    let mut h = json!({
        "name": p.name, "tiles": p.w, "tilesY": p.h, "spacing": SPACING, "width": nx, "height": ny,
        "tagPrefix": tag_prefix(canvas, header), "module": canvas,
    });
    if halo > 0 {
        h["halo"] = halo.into();
    }
    if !palette.is_empty() {
        h["palette"] = Value::Array(palette);
    }
    // Chasm runs over the piece's grid.
    let mut runs: Vec<[usize; 2]> = Vec::new();
    for (k, &s) in src.iter().enumerate() {
        if g.chasm[s] {
            match runs.last_mut() {
                Some(r) if r[0] + r[1] == k && k % fw != 0 => r[1] += 1,
                _ => runs.push([k, 1]),
            }
        }
    }
    if !runs.is_empty() {
        h["chasm"] = json!(runs);
    }
    if !g.water.is_empty() {
        h["water"] = Value::Array(water_seeds(g, &src, fw));
    }
    let [west, south, east, north] = p.metres();
    let canvas_m = [header.size.tiles_x as f32 * TILE_M, header.size.tiles_y as f32 * TILE_M];
    let shift = |x: f32, y: f32| [x - west, y - south];
    // What the builder placed moves in f64, to the nanometre: in f32 a point 90 m out moves 2 µm, enough to tip an object's
    // footprint over a walkmesh cell, so a piece that is a whole area would not give that area back.
    let moved = |v: &Value| {
        let at = |q: &Value, o: f32| q.as_f64().map_or(0.0, |x| ((x - f64::from(o)) * 1e9).round() / 1e9);
        [at(&v[0], west), at(&v[1], south)]
    };
    let place = |v: &mut Value| {
        let [x, y] = moved(&json!([v["x"], v["y"]]));
        v["x"] = x.into();
        v["y"] = y.into();
    };
    let f = |v: &Value, k: &str| v[k].as_f64().map(|x| x as f32);
    // Objects: those standing in it, and those on its halo (drawn only in its edge scenery).
    let reach = halo as f32 * SPACING;
    let mut objects = Vec::new();
    for o in header.json["objects"].as_array().into_iter().flatten() {
        let (Some(x), Some(y)) = (f(o, "x"), f(o, "y")) else { continue };
        let on_halo = x >= west - reach && x <= east + reach && y >= south - reach && y <= north + reach;
        if p.holds(x, y, canvas_m) || (on_halo && !p.covers(x, y)) {
            let mut o = o.clone();
            place(&mut o);
            objects.push(o);
        }
    }
    if !objects.is_empty() {
        h["objects"] = Value::Array(objects);
    }
    // Things: by their place, a trigger's or an encounter's by its whole outline.
    let mut things = Vec::new();
    for t in header.json["things"].as_array().into_iter().flatten() {
        let (Some(x), Some(y)) = (f(t, "x"), f(t, "y")) else { continue };
        let outline: Vec<[f32; 2]> =
            t["outline"].as_array().into_iter().flatten().filter_map(|q| Some([q[0].as_f64()? as f32, q[1].as_f64()? as f32])).collect();
        let named = || t["name"].as_str().or(t["tag"].as_str()).map_or_else(|| format!("thing {}", t["id"]), str::to_string);
        let mine = if outline.is_empty() {
            p.holds(x, y, canvas_m)
        } else {
            let inside = outline.iter().filter(|q| p.covers(q[0], q[1])).count();
            if inside > 0 && inside < outline.len() {
                bail!("{}'s outline crosses the edge of piece {}: move it inside one piece", named(), p.name);
            }
            let n = outline.len() as f32;
            let (cx, cy) = (outline.iter().map(|q| q[0]).sum::<f32>() / n, outline.iter().map(|q| q[1]).sum::<f32>() / n);
            inside == outline.len() && p.holds(cx, cy, canvas_m)
        };
        if mine {
            let mut t = t.clone();
            place(&mut t);
            if !outline.is_empty() {
                t["outline"] = json!(t["outline"].as_array().into_iter().flatten().map(moved).collect::<Vec<_>>());
            }
            things.push(t);
        }
    }
    for c in crossings {
        if c.from == p.name {
            let mut fields = serde_json::Map::new();
            fields.insert("Type".into(), 1.into());
            fields.insert("LinkedTo".into(), c.waypoint.clone().into());
            fields.insert("LinkedToFlags".into(), 2.into());
            fields.insert("ScriptOnEnter".into(), CROSS_SCRIPT.into());
            let mid = [(c.outline[0][0] + c.outline[2][0]) / 2.0, (c.outline[0][1] + c.outline[2][1]) / 2.0];
            let [lx, ly] = shift(mid[0], mid[1]);
            things.push(json!({
                "id": c.id, "kind": "trigger", "blueprint": "", "x": lx, "y": ly, "bearing": 0,
                "outline": c.outline.iter().map(|q| shift(q[0], q[1])).collect::<Vec<_>>(),
                "tag": c.tag, "name": format!("To {}", c.to), "fields": Value::Object(fields),
            }));
        }
        if c.to == p.name {
            let [lx, ly] = shift(c.arrive[0], c.arrive[1]);
            things.push(json!({
                "id": c.id + 1, "kind": "waypoint", "blueprint": "", "x": lx, "y": ly, "bearing": c.bearing,
                "tag": c.waypoint, "name": format!("From {}", c.from),
            }));
        }
    }
    if !things.is_empty() {
        h["things"] = Value::Array(things);
    }
    // Entrances: by their mouth; the cave they dig must stay in the piece.
    let mut entrances = Vec::new();
    for e in header.json["entrances"].as_array().into_iter().flatten() {
        let (Some(x), Some(y)) = (f(e, "x"), f(e, "y")) else { continue };
        if !p.holds(x, y, canvas_m) {
            continue;
        }
        let (dir, width, depth) = (f(e, "dir").unwrap_or(0.0), f(e, "width").unwrap_or(0.0), f(e, "depth").unwrap_or(0.0));
        let (c, s) = (dir.cos(), dir.sin());
        let corners = [(-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (1.0, 1.0)]
            .map(|(side, along)| (x - s * side * width / 2.0 + c * along * depth, y + c * side * width / 2.0 + s * along * depth));
        if corners.iter().any(|(cx, cy)| !p.covers(*cx, *cy)) {
            bail!("entrance {}'s cave crosses the edge of piece {}: move it inside one piece", e["id"], p.name);
        }
        let mut e = e.clone();
        place(&mut e);
        entrances.push(e);
    }
    if !entrances.is_empty() {
        h["entrances"] = Value::Array(entrances);
    }
    let ambience: Vec<Value> = header.json["ambience"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| f(a, "x").zip(f(a, "y")).is_some_and(|(x, y)| p.holds(x, y, canvas_m)))
        .map(|a| {
            let mut a = a.clone();
            place(&mut a);
            a
        })
        .collect();
    if !ambience.is_empty() {
        h["ambience"] = Value::Array(ambience);
    }
    if let Some(lights) = header.json["lights"].as_array().filter(|l| !l.is_empty()) {
        let tx = header.size.tiles_x;
        ensure!(
            lights.len() == tx * header.size.tiles_y,
            "lights: two colours a tile of the canvas, {} expected",
            tx * header.size.tiles_y
        );
        h["lights"] = json!(
            (p.y..p.y + p.h)
                .flat_map(|ty| (p.x..p.x + p.w).map(move |x| (x, ty)))
                .map(|(x, ty)| lights[ty * tx + x].clone())
                .collect::<Vec<_>>()
        );
    }
    // An imported area's own objects as the editor changed them, in its own metres: the piece made from it carries them.
    if let Some(o) = p.json.get("originals").filter(|o| o.is_object()) {
        h["originals"] = o.clone();
    }
    for key in ["settings", "atmosphere"] {
        if let Some(v) = p.json.get(key).or(header.json.get(key)).filter(|v| v.is_object()) {
            h[key] = v.clone();
        }
    }
    if let Some(s) = header.json["shadow"].as_f64() {
        h["shadow"] = s.into();
    }
    Ok(Area { name: p.name.clone(), header: h, heights, materials, blend })
}

/// One water seed for each part of the canvas's water inside a piece's grid (`src`: each grid sample's canvas sample, `fw`
/// across): the samples of one level joined inside the grid, seeded at the first, bounded by the part's extent, at the level of
/// the body it came from.
fn water_seeds(g: &Ground, src: &[usize], fw: usize) -> Vec<Value> {
    let n = src.len();
    let level = |k: usize| g.water_at(src[k]);
    let mut seen = vec![false; n];
    let mut out = Vec::new();
    for start in 0..n {
        let l = level(start);
        if seen[start] || l.is_nan() {
            continue;
        }
        let (mut lo, mut hi) = ([start % fw, start / fw], [start % fw, start / fw]);
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        while let Some(k) = queue.pop_front() {
            let (x, y) = (k % fw, k / fw);
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
            let mut next = |m: usize| {
                if !seen[m] && level(m).to_bits() == l.to_bits() {
                    seen[m] = true;
                    queue.push_back(m);
                }
            };
            if x > 0 {
                next(k - 1);
            }
            if x + 1 < fw {
                next(k + 1);
            }
            if y > 0 {
                next(k - fw);
            }
            if k + fw < n {
                next(k + fw);
            }
        }
        let body = g.levels.get(&l.to_bits()).copied().unwrap_or(f64::from(l));
        out.push(json!({ "i": start % fw, "j": start / fw, "level": body, "bounds": [lo[0], lo[1], hi[0], hi[1]] }));
    }
    out
}

/// Things on the canvas that no piece holds (in a gap): never exported, listed so the builder knows.
pub fn in_gaps(header: &Header, pieces: &[Piece]) -> Vec<Value> {
    let canvas_m = [header.size.tiles_x as f32 * TILE_M, header.size.tiles_y as f32 * TILE_M];
    let mut out = Vec::new();
    for (list, what) in [("things", "thing"), ("entrances", "entrance")] {
        for t in header.json[list].as_array().into_iter().flatten() {
            let (Some(x), Some(y)) = (t["x"].as_f64(), t["y"].as_f64()) else { continue };
            if !pieces.iter().any(|p| p.holds(x as f32, y as f32, canvas_m)) {
                out.push(json!({ "kind": "in_gap", "what": what, "id": t["id"], "at": [x, y],
                    "why": format!("this {what} stands where no piece is: it isn't exported") }));
            }
        }
    }
    out
}

/// The route check across the canvas, from each piece's check (`reports`: piece name to its report, whose `fixed` lists its
/// crossings' and entrances' places and the routes that join them): a graph whose nodes are those places and whose edges are
/// working routes inside a piece and crossings between pieces. Reports pieces no crossing leads into, pieces a player can't
/// reach from the start piece, and crossings whose trigger or arrival stands where no route reaches.
pub fn canvas_routes(pieces: &[Piece], crossings: &[Crossing], reports: &BTreeMap<String, Value>, start: Option<&str>) -> Value {
    // (Node: piece and tag.)
    let mut walk: BTreeSet<(String, String)> = BTreeSet::new();
    let mut joined: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
    for (piece, r) in reports {
        for pt in r["fixed"]["points"].as_array().into_iter().flatten() {
            if pt["walkmesh"].as_bool() == Some(true) {
                walk.insert((piece.clone(), pt["tag"].as_str().unwrap_or_default().to_string()));
            }
        }
        for pair in r["fixed"]["joined"].as_array().into_iter().flatten() {
            let (Some(a), Some(b)) = (pair[0].as_str(), pair[1].as_str()) else { continue };
            let (a, b) = ((piece.clone(), a.to_string()), (piece.clone(), b.to_string()));
            joined.entry(a.clone()).or_default().push(b.clone());
            joined.entry(b).or_default().push(a);
        }
    }
    for c in crossings {
        joined.entry((c.from.clone(), c.tag.clone())).or_default().push((c.to.clone(), c.waypoint.clone()));
    }
    let start = start.map(str::to_string).or_else(|| pieces.first().map(|p| p.name.clone()));
    let mut reached: BTreeSet<(String, String)> = BTreeSet::new();
    // (From the module's start point when the start piece's check has it on the walkmesh, else from all its places.)
    let from_start = start.clone().map(|p| (p, crate::generator::START.to_string())).filter(|n| walk.contains(n));
    let mut queue: VecDeque<(String, String)> = match from_start {
        Some(n) => VecDeque::from([n]),
        None => walk.iter().filter(|(p, _)| Some(p) == start.as_ref()).cloned().collect(),
    };
    reached.extend(queue.iter().cloned());
    while let Some(n) = queue.pop_front() {
        for m in joined.get(&n).into_iter().flatten() {
            if walk.contains(m) && reached.insert(m.clone()) {
                queue.push_back(m.clone());
            }
        }
    }
    let checked = |p: &str| reports.contains_key(p);
    // (Some pieces only: crossings from the others are in their last exports, which the export checks match the cut; a walk
    // from a start piece left out proves nothing.)
    let start_checked = start.as_deref().is_some_and(checked);
    let mut problems = Vec::new();
    for p in pieces.iter().filter(|p| checked(&p.name)) {
        let ins: Vec<&Crossing> = crossings.iter().filter(|c| c.to == p.name).collect();
        if Some(&p.name) != start.as_ref() && !ins.iter().any(|c| walk.contains(&(p.name.clone(), c.waypoint.clone()))) {
            problems.push(json!({ "kind": "no_way_in", "piece": p.name, "why": format!("no crossing leads into {}", p.name) }));
        } else if start_checked && Some(&p.name) != start.as_ref() && !reached.iter().any(|(q, _)| *q == p.name) {
            problems.push(json!({ "kind": "unreached", "piece": p.name,
                "why": format!("a player can't walk to {} from {}", p.name, start.clone().unwrap_or_default()) }));
        }
    }
    for c in crossings.iter().filter(|c| checked(&c.from) && checked(&c.to)) {
        let trig = (c.from.clone(), c.tag.clone());
        let way = (c.to.clone(), c.waypoint.clone());
        if !walk.contains(&trig) {
            problems.push(json!({ "kind": "crossing_off_walkmesh", "tag": c.tag, "piece": c.from,
                "why": format!("{}'s trigger stands where the walkmesh doesn't reach", c.tag) }));
        }
        if !walk.contains(&way) {
            problems.push(json!({ "kind": "arrival_off_walkmesh", "tag": c.waypoint, "piece": c.to,
                "why": format!("{} arrives where the walkmesh doesn't reach", c.tag) }));
        } else if joined.get(&way).is_none_or(Vec::is_empty) {
            problems.push(json!({ "kind": "arrival_stranded", "tag": c.waypoint, "piece": c.to,
                "why": format!("no route leads on from where {} arrives", c.tag) }));
        }
    }
    json!({
        "start": start,
        "reached": pieces.iter().filter(|p| reached.iter().any(|(q, _)| *q == p.name)).map(|p| p.name.clone()).collect::<Vec<_>>(),
        "problems": problems,
    })
}

/// The canvas cut, without building anything: its pieces, crossings and what the cut says (for the editor's piece view).
pub fn summary(canvas: &Canvas) -> Result<Value> {
    let header = canvas.header()?;
    let ground = Ground::load(canvas, &header)?;
    let list = pieces(&header);
    let (crossings, mut warnings) = crossings(&canvas.name, &header, &ground, &list);
    warnings.extend(in_gaps(&header, &list));
    let mut rows = Vec::new();
    for p in &list {
        match area(&canvas.name, &header, &ground, p, &crossings, HALO) {
            Ok(a) => rows.push(json!({
                "name": p.name, "id": p.id,
                "things": a.header["things"].as_array().map_or(0, Vec::len),
                "objects": a.header["objects"].as_array().map_or(0, Vec::len),
                "entrances": a.header["entrances"].as_array().map_or(0, Vec::len),
                "water": a.header["water"].as_array().map_or(0, Vec::len),
            })),
            Err(e) => rows.push(json!({ "name": p.name, "id": p.id, "refused": format!("{e:#}") })),
        }
    }
    Ok(json!({
        "canvas": canvas.name, "pieces": rows,
        "crossings": crossings.iter().map(Crossing::json).collect::<Vec<_>>(),
        "warnings": warnings,
    }))
}

/// The pieces of `list` named in `only` (all of them when it is empty), each name checked.
pub fn chosen<'a>(list: &'a [Piece], only: &[String]) -> Result<Vec<&'a Piece>> {
    if only.is_empty() {
        return Ok(list.iter().collect());
    }
    only.iter().map(|n| list.iter().find(|p| &p.name == n).with_context(|| format!("the canvas has no piece {n}"))).collect()
}

/// The crossings' JSON, for a report.
pub fn crossings_json(list: &[Crossing]) -> Value {
    Value::Array(list.iter().map(Crossing::json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Size;

    fn header(json: Value) -> Header {
        Header::parse("rivl", json, String::new()).unwrap()
    }

    fn base(tiles: u64, tiles_y: u64, pieces: Value) -> Value {
        json!({ "name": "rivl", "tiles": tiles, "tilesY": tiles_y, "ground": 0.0, "palette": ["grass", "dirt"], "pieces": pieces })
    }

    fn piece(id: u64, x: u64, y: u64, w: u64, h: u64) -> Value {
        json!({ "id": id, "name": format!("rivl{id:02}"), "x": x, "y": y, "w": w, "h": h })
    }

    /// Ground of `size` with heights `f(i, j)` and the paint `p(i, j)`.
    fn ground(size: Size, f: impl Fn(usize, usize) -> f32, water: &Value) -> Ground {
        let (w, h) = (size.tiles_x * 16 + 1, size.tiles_y * 16 + 1);
        let heights = (0..h).flat_map(|j| (0..w).map(move |i| (i, j))).map(|(i, j)| f(i, j)).collect();
        let paint = (0..w * h).map(|k| u8::from(k % w > 100)).collect();
        Ground::new(w, h, heights, paint, vec![false; w * h], water).unwrap()
    }

    #[test]
    fn a_piece_is_cut_out_with_the_canvas_round_it_and_its_edge_carried_on_past_the_canvas() {
        let h = header(base(20, 10, json!([piece(1, 0, 0, 10, 10), piece(2, 10, 0, 10, 10)])));
        let g = ground(h.size, |i, j| i as f32 * 0.01 + j as f32 * 0.001, &Value::Null);
        let list = pieces(&h);
        let a = area("rivl", &h, &g, &list[1], &[], 16).unwrap();
        let fw = 161 + 32;
        assert_eq!(a.heights.len(), fw * fw * 4);
        let read = |i: usize, j: usize| {
            let k = (j * fw + i) * 4;
            f32::from_le_bytes([a.heights[k], a.heights[k + 1], a.heights[k + 2], a.heights[k + 3]])
        };
        // The piece's own south-west sample is the canvas's (160, 0); 16 west of it, the neighbour's (144, 0).
        assert_eq!(read(16, 16), 160.0 * 0.01);
        assert_eq!(read(0, 16), 144.0 * 0.01);
        // Past the canvas's south and east edges: its edge samples carried outward.
        assert_eq!(read(16, 0), read(16, 16));
        assert_eq!(read(fw - 1, 16), 320.0 * 0.01);
        assert_eq!(a.header["tagPrefix"], "rivl");
        assert_eq!(a.header["module"], "rivl");
        assert_eq!(a.materials.len(), fw * fw);
        assert!(a.blend.is_empty(), "no layer on the canvas: none in the piece");
        // A blend layer on the canvas: cut out with the paint, two bytes a sample.
        let mut g = g;
        g.blend = (0..g.w * g.h).map(|k| [1, (k % 7) as u8]).collect();
        let b = area("rivl", &h, &g, &list[1], &[], 16).unwrap();
        assert_eq!(b.blend.len(), fw * fw * 2);
        let k = 16 * fw + 16;
        assert_eq!(&b.blend[2 * k..2 * k + 2], &[1, ((160) % 7) as u8]);
    }

    fn flat(size: Size, water: &Value) -> Ground {
        ground(size, |_, _| 5.0, water)
    }

    #[test]
    fn water_seeded_in_one_piece_floods_its_neighbour_the_same() {
        // A channel west to east across the cut at x = 100 m, seeded in the east piece only.
        let h = header(base(20, 10, json!([piece(1, 0, 0, 10, 10), piece(2, 10, 0, 10, 10)])));
        let channel = |_: usize, j: usize| if (60..70).contains(&j) { 2.0 } else { 5.0 };
        let g = ground(h.size, channel, &json!([{ "i": 250, "j": 64, "level": 3.0 }]));
        let list = pieces(&h);
        for p in &list {
            let a = area("rivl", &h, &g, p, &[], 32).unwrap();
            let fw = 161 + 64;
            let heights: Vec<f32> = a.heights.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
            let bodies: Vec<crate::heightmap::Body> = a.header["water"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| {
                    let v: Vec<usize> = b["bounds"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as usize).collect();
                    crate::heightmap::Body {
                        i: b["i"].as_u64().unwrap() as usize,
                        j: b["j"].as_u64().unwrap() as usize,
                        level: b["level"].as_f64().unwrap(),
                        bounds: Some([v[0], v[1], v[2], v[3]]),
                    }
                })
                .collect();
            assert_eq!(bodies.len(), 1, "{}: one part of the channel", p.name);
            let mine = crate::heightmap::flood(&heights, fw, &bodies);
            let (i0, j0) = (p.x as isize * 16 - 32, p.y as isize * 16 - 32);
            for (k, l) in mine.iter().enumerate() {
                let want = g.water_at(g.at(i0 + (k % fw) as isize, j0 + (k / fw) as isize));
                assert_eq!(l.to_bits(), want.to_bits(), "{} sample {k}", p.name);
            }
        }
    }

    #[test]
    fn things_go_by_their_place_and_an_outline_across_a_cut_is_refused() {
        let mut j = base(30, 10, json!([piece(1, 0, 0, 10, 10), piece(2, 10, 0, 10, 10)]));
        j["things"] = json!([
            { "id": 1, "kind": "creature", "blueprint": "nw_goblina", "x": 50, "y": 50 },
            // On the cut: the east piece's.
            { "id": 2, "kind": "waypoint", "blueprint": "", "x": 100, "y": 20 },
            { "id": 3, "kind": "trigger", "blueprint": "", "x": 150, "y": 50, "outline": [[140, 40], [160, 40], [160, 60]] },
            // In the gap east of both pieces.
            { "id": 4, "kind": "waypoint", "blueprint": "", "x": 250, "y": 20 },
        ]);
        j["objects"] = json!([
            { "model": "tree", "x": 99, "y": 30, "z": 5 },
            { "model": "rock", "x": 120, "y": 30, "z": 5 },
        ]);
        let h = header(j.clone());
        let g = flat(h.size, &Value::Null);
        let list = pieces(&h);
        let west = area("rivl", &h, &g, &list[0], &[], 80).unwrap();
        let east = area("rivl", &h, &g, &list[1], &[], 80).unwrap();
        let ids = |a: &Area| a.header["things"].as_array().map_or_else(Vec::new, |l| l.iter().map(|t| t["id"].as_u64().unwrap()).collect());
        assert_eq!(ids(&west), [1]);
        assert_eq!(ids(&east), [2, 3]);
        let t3 = &east.header["things"][1];
        assert_eq!((t3["x"].as_f64(), t3["outline"][0][0].as_f64()), (Some(50.0), Some(40.0)), "in the piece's own metres");
        // Objects: its own, and its neighbour's within the halo (edge scenery), in its metres.
        let xs = |a: &Area| a.header["objects"].as_array().unwrap().iter().map(|o| o["x"].as_f64().unwrap()).collect::<Vec<_>>();
        assert_eq!(xs(&west), [99.0, 120.0]);
        assert_eq!(xs(&east), [-1.0, 20.0]);
        let gaps = in_gaps(&h, &list);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0]["id"], 4);
        // A trigger across the cut: refused by both pieces, by name.
        j["things"][2]["outline"] = json!([[90, 40], [110, 40], [110, 60]]);
        j["things"][2]["name"] = json!("Ambush");
        let h = header(j.clone());
        let e = format!("{:#}", area("rivl", &h, &g, &list[0], &[], 80).err().unwrap());
        assert!(e.contains("Ambush's outline crosses the edge of piece rivl01"), "{e}");
        // A cave dug across the cut.
        j["things"] = json!([]);
        j["entrances"] = json!([{ "id": 1, "x": 95, "y": 50, "dir": 0, "width": 4, "depth": 8, "floor": 5 }]);
        let h = header(j);
        let e = format!("{:#}", area("rivl", &h, &g, &list[0], &[], 80).err().unwrap());
        assert!(e.contains("entrance 1's cave crosses the edge of piece rivl01"), "{e}");
    }

    #[test]
    fn crossings_cover_the_open_ground_of_each_cut_both_ways() {
        let mut j = base(20, 20, json!([piece(1, 0, 0, 10, 10), piece(2, 10, 0, 10, 10), piece(3, 0, 10, 10, 10)]));
        let h = header(j.clone());
        let size = h.size;
        let g = flat(size, &Value::Null);
        let list = pieces(&h);
        let (all, warnings) = crossings("rivl", &h, &g, &list);
        assert!(warnings.is_empty());
        // Each cut: 156 open samples (2 off each corner), 5 crossings of at most 32, each way.
        assert_eq!(all.len(), 2 * 2 * 5);
        let east: Vec<&Crossing> = all.iter().filter(|c| c.dir == 'e').collect();
        assert_eq!(east.len(), 5);
        let c = east[0];
        assert_eq!((c.from.as_str(), c.to.as_str()), ("rivl01", "rivl02"));
        assert!(c.tag.starts_with("rivl_x160_") && c.tag.ends_with('e') && c.tag.len() <= 32);
        assert_eq!(c.waypoint, c.tag.replace("_x", "_w"));
        assert_eq!(c.arrive[0], 104.0);
        assert!(c.outline.iter().all(|q| (98.5..100.0).contains(&q[0])), "{:?}", c.outline);
        let back = all.iter().find(|b| b.key == c.key && b.dir == 'w').unwrap();
        assert!(back.outline.iter().all(|q| (100.0..=101.5).contains(&q[0])) && back.arrive[0] == 96.0);
        assert!(all.iter().all(|c| (c.outline[0][1] - c.outline[2][1]).abs() <= 20.0 || (c.outline[0][0] - c.outline[2][0]).abs() <= 20.0));
        // Ids: each crossing's own pair, never twice.
        let ids: BTreeSet<u32> = all.iter().flat_map(|c| [c.id, c.id + 1]).collect();
        assert_eq!(ids.len(), all.len() * 2);
        // (The largest canvas's last sample, the last direction: still a game int.)
        assert!(CROSSING_IDS + ((3 * 2049 + 2048) * 2049 + 2048) * 2 + 1 < i32::MAX as u32);
        // In the pieces: the trigger where the player leaves, linked to the waypoint where they arrive.
        let a = area("rivl", &h, &g, &list[0], &all, 80).unwrap();
        let things = a.header["things"].as_array().unwrap();
        let trig = things.iter().find(|t| t["tag"] == c.tag.as_str()).unwrap();
        assert_eq!(trig["fields"]["LinkedTo"], c.waypoint.as_str());
        assert_eq!((trig["fields"]["Type"].as_u64(), trig["fields"]["LinkedToFlags"].as_u64()), (Some(1), Some(2)));
        assert_eq!(trig["fields"]["ScriptOnEnter"], CROSS_SCRIPT);
        let b = area("rivl", &h, &g, &list[1], &all, 80).unwrap();
        let way = b.header["things"].as_array().unwrap().iter().find(|t| t["tag"] == c.waypoint.as_str()).unwrap();
        assert_eq!((way["kind"].as_str(), way["x"].as_f64(), way["bearing"].as_f64()), (Some("waypoint"), Some(4.0), Some(0.0)));
        // Deep water across the cut splits its stretch; a stretch turned off is gone both ways.
        let wet =
            ground(size, |i, j| if i > 140 && (70..80).contains(&j) { 2.0 } else { 5.0 }, &json!([{ "i": 150, "j": 75, "level": 4.0 }]));
        let (split, _) = crossings("rivl", &h, &wet, &list);
        let e: Vec<&Crossing> = split.iter().filter(|c| c.dir == 'e').collect();
        assert!(e.iter().all(|c| c.outline.iter().all(|q| q[1] <= 70.0 * 0.625 || q[1] >= 80.0 * 0.625)), "none over the water");
        j["crossingsOff"] = json!([c.key]);
        let (fewer, _) = crossings("rivl", &header(j), &g, &list);
        assert_eq!(fewer.len(), all.len() - 2);
    }

    #[test]
    fn the_canvas_route_check_finds_pieces_no_one_reaches() {
        let h = header(base(30, 10, json!([piece(1, 0, 0, 10, 10), piece(2, 10, 0, 10, 10), piece(3, 20, 0, 10, 10)])));
        let g = flat(h.size, &Value::Null);
        let list = pieces(&h);
        let (all, _) = crossings("rivl", &h, &g, &list);
        // Every place on the walkmesh, and each piece's places all joined: everything is reached.
        let report = |piece: &str, off: &[&str], joined: bool| {
            let tags: Vec<String> = all
                .iter()
                .flat_map(|c| [(c.from.clone(), c.tag.clone()), (c.to.clone(), c.waypoint.clone())])
                .filter(|(p, _)| p == piece)
                .map(|(_, t)| t)
                .collect();
            let pairs: Vec<Value> = if joined {
                tags.iter().flat_map(|a| tags.iter().filter(move |b| a < *b).map(move |b| json!([a, b]))).collect()
            } else {
                vec![]
            };
            json!({ "fixed": {
                "points": tags.iter().map(|t| json!({ "tag": t, "walkmesh": !off.contains(&t.as_str()) })).collect::<Vec<_>>(),
                "joined": pairs } })
        };
        let mut reports: BTreeMap<String, Value> =
            ["rivl01", "rivl02", "rivl03"].iter().map(|p| (p.to_string(), report(p, &[], true))).collect();
        let r = canvas_routes(&list, &all, &reports, None);
        assert_eq!(r["reached"], json!(["rivl01", "rivl02", "rivl03"]));
        assert_eq!(r["problems"], json!([]));
        // The middle piece's places are walkable but nothing joins them: the far piece can't be reached, and arrivals there
        // lead nowhere.
        reports.insert("rivl02".into(), report("rivl02", &[], false));
        let r = canvas_routes(&list, &all, &reports, None);
        assert_eq!(r["reached"], json!(["rivl01", "rivl02"]));
        let kinds: BTreeSet<&str> = r["problems"].as_array().unwrap().iter().filter_map(|p| p["kind"].as_str()).collect();
        assert!(kinds.contains("unreached") && kinds.contains("arrival_stranded"), "{kinds:?}");
        // Every arrival in the far piece off the walkmesh: no way in.
        let arrivals: Vec<String> = all.iter().filter(|c| c.to == "rivl03").map(|c| c.waypoint.clone()).collect();
        let off: Vec<&str> = arrivals.iter().map(String::as_str).collect();
        reports.insert("rivl02".into(), report("rivl02", &[], true));
        reports.insert("rivl03".into(), report("rivl03", &off, true));
        let r = canvas_routes(&list, &all, &reports, Some("rivl01"));
        assert!(r["problems"].as_array().unwrap().iter().any(|p| p["kind"] == "no_way_in" && p["piece"] == "rivl03"), "{r}");
        // The middle piece exported alone: its neighbours' crossings into it count, and no walk from the start is claimed.
        let alone: BTreeMap<String, Value> = [("rivl02".to_string(), report("rivl02", &[], true))].into_iter().collect();
        let r = canvas_routes(&list, &all, &alone, Some("rivl01"));
        assert_eq!(r["problems"], json!([]), "{r}");
    }

    #[test]
    fn a_canvas_made_from_an_area_tags_its_pieces_and_crossings_with_that_area_s_prefix() {
        let mut j = base(20, 10, json!([piece(1, 0, 0, 10, 10), piece(2, 10, 0, 10, 10)]));
        j["tagPrefix"] = "lsplains".into();
        let h = header(j.clone());
        let g = flat(h.size, &Value::Null);
        let list = pieces(&h);
        let (all, _) = crossings("rivl", &h, &g, &list);
        assert!(all.iter().all(|c| c.tag.starts_with("lsplains_x") && c.waypoint.starts_with("lsplains_w")));
        let a = area("rivl", &h, &g, &list[0], &all, 0).unwrap();
        assert_eq!(a.header["tagPrefix"], "lsplains");
        assert_eq!(a.header["module"], "rivl");
        j["tagPrefix"] = "Not-one".into();
        assert!(Header::parse("rivl", j, String::new()).is_err());
    }
}
