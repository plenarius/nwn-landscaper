// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An export's progress and the time it has left, for the editor's progress bar: the step it is at, its tiles done, and an
//! estimate made before the tiles start from what each will cost on this machine (its ground baked or kept, its shadows drawn or
//! kept, its baked objects' triangles, the threads), refined as tiles finish. `landscaper serve`'s `/api/export/job` reads it;
//! with `LS_PROGRESS` set (the browser page's engine, which answers nothing until its run ends) each change is also printed as a
//! line `LS_PROGRESS {json}`. What one export measured is kept in the data folder's `export-speed.json` for the next estimate.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde_json::{Value, json};

/// What an export's work costs on this machine, in seconds of one thread's time unless said.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Speed {
    /// One tile's ground baked (not kept from a past export). As many textures or few, it reads each pixel's (lsforest's 4.3
    /// textures a tile and lsplains' 2.6 both took 1.45 s).
    pub bake: f64,
    /// One tile's shadow map drawn, and its share per thousand baked objects' triangles on the tile, which cast it (lsplains'
    /// 0.3 thousand a tile took 0.08 s, lsforest's 6 thousand 0.21 s).
    pub shadow: f64,
    pub shadow_ktri: f64,
    /// One tile's ground or shadow map read back from the cache (and encoded).
    pub kept: f64,
    /// The rest of a tile: its walkmesh, model, minimap and compiling.
    pub tile: f64,
    /// The rest's share per thousand baked objects' triangles on the tile.
    pub ktri: f64,
    /// How much of the threads the tiles keep busy (0 to 1).
    pub busy: f64,
    /// Wall time after the tiles (the hak, the edge scenery, the routes' check, installing), per tile and per thousand
    /// triangles, at `threads`: lsplains' 10 s against lsforest's 46 s with its 1.5 million triangles.
    pub after: f64,
    pub after_ktri: f64,
    /// The threads `after` was measured on.
    pub threads: usize,
}

impl Speed {
    /// Used until this machine has exported once: lsforest's 2.2 s of bake and 0.7 s of shadows a tile (LS_TIMING), the shares
    /// of triangles and the rest as lsforest and lsplains measured on 8 threads.
    pub(crate) const DEFAULT: Speed = Speed {
        bake: 2.2,
        shadow: 0.233,
        shadow_ktri: 0.078,
        kept: 0.02,
        tile: 0.008,
        ktri: 0.0095,
        busy: 0.95,
        after: 0.04,
        after_ktri: 0.023,
        threads: 8,
    };

    fn file() -> std::path::PathBuf {
        crate::paint::data_dir().join("export-speed.json")
    }

    /// The speed the last export measured here, else [`Speed::DEFAULT`].
    pub(crate) fn load() -> Speed {
        std::fs::read(Self::file())
            .ok()
            .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
            .map_or(Speed::DEFAULT, |v| Speed::from_json(&v))
    }

    fn from_json(v: &Value) -> Speed {
        let d = Speed::DEFAULT;
        // (A value missing, or out of reason, keeps the default.)
        let f = |k: &str, dflt: f64, hi: f64| v[k].as_f64().filter(|x| x.is_finite() && *x >= 0.0 && *x <= hi).unwrap_or(dflt);
        Speed {
            bake: f("bake", d.bake, 600.0),
            shadow: f("shadow", d.shadow, 600.0),
            shadow_ktri: f("shadow_ktri", d.shadow_ktri, 60.0),
            kept: f("kept", d.kept, 60.0),
            tile: f("tile", d.tile, 600.0),
            ktri: f("ktri", d.ktri, 60.0),
            busy: f("busy", d.busy, 1.0).max(0.2),
            after: f("after", d.after, 600.0),
            after_ktri: f("after_ktri", d.after_ktri, 60.0),
            threads: v["threads"].as_u64().map_or(d.threads, |t| t.clamp(1, 1024) as usize),
        }
    }

    fn to_json(self) -> Value {
        json!({
            "bake": self.bake, "shadow": self.shadow, "shadow_ktri": self.shadow_ktri, "kept": self.kept, "tile": self.tile, "ktri": self.ktri, "busy": self.busy,
            "after": self.after, "after_ktri": self.after_ktri, "threads": self.threads,
        })
    }

    /// Thread time of a tile of this work.
    pub(crate) fn tile_work(&self, t: &TileWork) -> f64 {
        (if t.bake { self.bake } else { self.kept })
            + (if t.shadow { self.shadow + self.shadow_ktri * t.ktri } else { self.kept })
            + self.tile
            + self.ktri * t.ktri
    }

    /// Wall time of the tiles of `work` on `threads` threads.
    pub(crate) fn tiles_wall(&self, work: &[TileWork], threads: usize) -> f64 {
        let each: Vec<f64> = work.iter().map(|t| self.tile_work(t)).collect();
        let threads = threads.clamp(1, work.len().max(1));
        let busy = if threads == 1 { 1.0 } else { self.busy };
        let longest = each.iter().copied().fold(0.0, f64::max);
        (each.iter().sum::<f64>() / (threads as f64 * busy)).max(longest)
    }

    /// Wall time after the tiles, on `threads` threads: what of it runs in parallel (the routes' check) scales with them.
    pub(crate) fn after_wall(&self, tiles: usize, ktri: f64, threads: usize) -> f64 {
        let at = self.after * tiles as f64 + self.after_ktri * ktri;
        at * (self.threads as f64 / threads.max(1) as f64).sqrt()
    }
}

/// What one tile of an export has to do: known before it starts, from the cache's keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TileWork {
    /// Its ground is baked here (not kept, and it has one of its own).
    pub bake: bool,
    /// Its shadow map is drawn here.
    pub shadow: bool,
    /// Thousands of baked objects' triangles on it.
    pub ktri: f64,
}

/// The export under way, or last finished.
struct State {
    /// An export is under way (`begin`, until `end`).
    active: bool,
    step: &'static str,
    started: Instant,
    threads: usize,
    speed: Speed,
    work: Vec<f64>,
    tiles: Vec<TileWork>,
    done: usize,
    work_done: f64,
    /// When the tiles started and ended, and the estimate of their wall time.
    tiles_at: Option<Instant>,
    tiles_end: Option<Instant>,
    tiles_wall: f64,
    /// The estimate of the time after the tiles.
    after: f64,
    /// The whole export's estimate as the tiles started, from the start.
    estimate: Option<f64>,
    /// The time left as last worked out, and when.
    left: Option<f64>,
}

impl State {
    fn ktri(&self) -> f64 {
        self.tiles.iter().map(|t| t.ktri).sum()
    }

    /// Seconds left, as the work so far says.
    fn left_now(&self) -> Option<f64> {
        let tiles_at = self.tiles_at?;
        let total: f64 = self.work.iter().sum();
        match self.tiles_end {
            None => {
                let ran = tiles_at.elapsed().as_secs_f64();
                let planned = (self.tiles_wall * (1.0 - self.work_done / total.max(1e-9))).max(0.0);
                // (The work done so far against its time, trusted fully from a quarter of the work on; the tiles under way are
                // half done, on average.)
                let in_flight = self.threads.min(self.work.len() - self.done) as f64 * 0.5 * total / self.work.len().max(1) as f64;
                let seen = (self.work_done + in_flight).min(total);
                let trust = (seen / (total * 0.25)).min(1.0);
                let measured = if seen > 0.0 && ran > 0.5 { (total - seen) * ran / seen } else { planned };
                let tiles_left = planned * (1.0 - trust) + measured * trust;
                Some(tiles_left + self.after)
            }
            Some(end) => Some((self.after - end.elapsed().as_secs_f64()).max(1.0)),
        }
    }

    fn json(&self) -> Value {
        let cached = self.tiles.iter().filter(|t| !t.bake && !t.shadow).count();
        json!({
            "step": self.step,
            "done": self.done,
            "total": self.tiles.len(),
            "cached": cached,
            "threads": self.threads,
            "elapsed": round1(self.started.elapsed().as_secs_f64()),
            "estimate": self.estimate.map(round1),
            "left": self.left.map(round1),
        })
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(g.get_or_insert_with(fresh))
}

fn fresh() -> State {
    State {
        active: false,
        step: "",
        started: Instant::now(),
        threads: 1,
        speed: Speed::DEFAULT,
        work: Vec::new(),
        tiles: Vec::new(),
        done: 0,
        work_done: 0.0,
        tiles_at: None,
        tiles_end: None,
        tiles_wall: 0.0,
        after: 0.0,
        estimate: None,
        left: None,
    }
}

/// Prints the state for the browser page (`LS_PROGRESS`).
fn tell(s: &State) {
    if std::env::var_os("LS_PROGRESS").is_some() {
        eprintln!("LS_PROGRESS {}", s.json());
    }
}

/// An export starts: its clock, step `preparing`, not cancelled.
pub(crate) fn begin() {
    with(|s| {
        *s = State { active: true, step: "preparing", ..fresh() };
        tell(s);
    });
}

/// The export ended (`ok`: it worked): its tiles' and the rest's times measured become this machine's speed.
pub(crate) fn end(ok: bool) {
    let measured = with(|s| {
        // (Measured at this many threads: the estimate scales it to the next export's.)
        let scaled = (s.speed.threads as f64 / s.threads.max(1) as f64).sqrt();
        if !s.active {
            return None;
        }
        s.active = false;
        CANCEL.store(false, Ordering::Relaxed);
        s.step = if ok { "done" } else { "failed" };
        s.left = Some(0.0);
        tell(s);
        s.tiles_end.map(|e| (s.speed, e.elapsed().as_secs_f64() / scaled, s.tiles.len(), s.ktri())).filter(|_| ok)
    });
    if let Some((speed, after, tiles, ktri)) = measured {
        let mut speed = speed;
        // (Its two parts scaled together, as for the shadows; at the threads it was measured at before.)
        let planned = speed.after * tiles as f64 + speed.after_ktri * ktri;
        if tiles > 0 && planned > 0.0 {
            let scale = (after / planned).clamp(0.05, 20.0);
            speed.after *= scale;
            speed.after_ktri *= scale;
        }
        let _ = crate::atomic::write(&Speed::file(), serde_json::to_string_pretty(&speed.to_json()).unwrap_or_default().as_bytes());
        with(|s| s.speed = speed);
    }
}

/// The export is at `step` (`hak`, `routes`, `installing`); stops it here if it was cancelled.
pub(crate) fn step(step: &'static str) -> anyhow::Result<()> {
    with(|s| {
        if s.active {
            s.step = step;
            s.left = s.left_now();
            tell(s);
        }
    });
    check()
}

/// The tiles start: `work` is each tile's, in order, on `threads` threads. Makes the estimate.
pub(crate) fn tiles(work: Vec<TileWork>, threads: usize) {
    with(|s| {
        if !s.active {
            return;
        }
        s.speed = Speed::load();
        s.threads = threads.max(1);
        s.work = work.iter().map(|t| s.speed.tile_work(t)).collect();
        s.tiles_wall = s.speed.tiles_wall(&work, s.threads);
        s.after = s.speed.after_wall(work.len(), work.iter().map(|t| t.ktri).sum(), s.threads);
        s.tiles = work;
        s.step = "tiles";
        s.tiles_at = Some(Instant::now());
        s.estimate = Some(s.started.elapsed().as_secs_f64() + s.tiles_wall + s.after);
        if std::env::var_os("LS_TIMING").is_some() {
            eprintln!(
                "  estimate: {:.1}s ({:.1}s of tiles, {:.1}s after) for {} tiles: {} to bake, {} shadows to draw, {:.0}k triangles, {} \
                 threads",
                s.estimate.unwrap_or_default(),
                s.tiles_wall,
                s.after,
                s.tiles.len(),
                s.tiles.iter().filter(|t| t.bake).count(),
                s.tiles.iter().filter(|t| t.shadow).count(),
                s.ktri(),
                s.threads
            );
        }
        tell(s);
    });
}

/// Tile `i` (of [`tiles`]' `work`) is done.
pub(crate) fn tile_done(i: usize) {
    with(|s| {
        if !s.active || i >= s.work.len() {
            return;
        }
        s.done += 1;
        s.work_done += s.work[i];
        s.left = s.left_now();
        tell(s);
    });
}

/// The tiles are done; `thread` is the thread time each part of them took: the ground baked, the shadows, and the rest. What
/// they cost here is measured from it for the next estimate.
pub(crate) fn tiles_end(bake: f64, shadow: f64, rest: f64) {
    with(|s| {
        if !s.active || s.tiles_at.is_none() {
            return;
        }
        let end = Instant::now();
        s.tiles_end = Some(end);
        let wall = end.duration_since(s.tiles_at.unwrap_or(end)).as_secs_f64();
        s.speed = measured(s.speed, &s.tiles, s.threads, wall, [bake, shadow, rest]);
        s.left = s.left_now();
        tell(s);
    });
}

/// `speed` with what these tiles measured: `thread` is the bake's, the shadows' and the rest's thread time, `wall` theirs.
pub(crate) fn measured(speed: Speed, tiles: &[TileWork], threads: usize, wall: f64, thread: [f64; 3]) -> Speed {
    let mut s = speed;
    let n = tiles.len();
    if n == 0 {
        return s;
    }
    let baked = tiles.iter().filter(|t| t.bake).count();
    let drawn = tiles.iter().filter(|t| t.shadow).count();
    let shadow_planned: f64 = tiles.iter().filter(|t| t.shadow).map(|t| s.shadow + s.shadow_ktri * t.ktri).sum();
    let ktri: f64 = tiles.iter().map(|t| t.ktri).sum();
    // (Each from enough tiles to mean something; the kept files' cost from a part that had nothing to make.)
    if baked == 0 {
        s.kept = thread[0] / n as f64;
    } else if baked >= 4 {
        s.bake = ((thread[0] - (n - baked) as f64 * s.kept) / baked as f64).max(0.01);
    }
    // (The shadows' two parts scaled together: one export can't tell them apart.)
    if drawn >= 4 && shadow_planned > 0.0 {
        let scale = ((thread[1] - (n - drawn) as f64 * s.kept).max(0.0) / shadow_planned).clamp(0.05, 20.0);
        s.shadow *= scale;
        s.shadow_ktri *= scale;
    }
    if n >= 4 {
        let planned = s.tile * n as f64 + s.ktri * ktri;
        let scale = if planned > 0.0 { (thread[2] / planned).clamp(0.05, 20.0) } else { 1.0 };
        s.tile *= scale;
        s.ktri *= scale;
    }
    let threads = threads.clamp(1, n);
    if threads > 1 && wall > 0.0 {
        s.busy = ((thread[0] + thread[1] + thread[2]) / (wall * threads as f64)).clamp(0.2, 1.0);
    }
    s
}

/// Where the export under way (or last ended) is, as JSON: `step`, `done` and `total` tiles, `cached` tiles (ground and
/// shadows kept), `threads`, `elapsed`, `estimate` (the whole export's, made as the tiles started) and `left` (seconds).
pub(crate) fn json() -> Value {
    with(|s| {
        if s.active {
            s.left = s.left_now().or(s.left);
        }
        s.json()
    })
}

/// Forgets the last export's state (a job whose turn came: what it shows until its export begins).
pub(crate) fn reset() {
    CANCEL.store(false, Ordering::Relaxed);
    with(|s| *s = State { step: "waiting", ..fresh() });
}

/// Asks the export under way (or about to begin, after [`reset`]) to stop at its next tile or step. Nothing is left changed:
/// the export's failure puts back what it replaced.
pub(crate) fn cancel() {
    with(|s| {
        if s.active || s.step == "waiting" {
            CANCEL.store(true, Ordering::Relaxed);
        }
    });
}

/// Why an export stopped when it was cancelled.
#[derive(Debug)]
pub(crate) struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the export was cancelled: the last export's files are as they were")
    }
}

impl std::error::Error for Cancelled {}

/// An error when the export under way was cancelled.
pub(crate) fn check() -> anyhow::Result<()> {
    if CANCEL.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(bake: bool, shadow: bool, ktri: f64) -> TileWork {
        TileWork { bake, shadow, ktri }
    }

    #[test]
    fn a_kept_tile_costs_little_and_the_threads_share_the_rest() {
        let s = Speed::DEFAULT;
        let cold = vec![tile(true, true, 0.0); 64];
        let warm = vec![tile(false, false, 0.0); 64];
        let (c1, c8, w8) = (s.tiles_wall(&cold, 1), s.tiles_wall(&cold, 8), s.tiles_wall(&warm, 8));
        assert!((c1 - 64.0 * (2.2 + 0.233 + 0.008)).abs() < 1e-6, "{c1}");
        assert!(c8 < c1 / 6.0 && c8 > c1 / 8.0, "{c8} against {c1}");
        assert!(w8 < c8 / 5.0, "warm {w8} against cold {c8}");
        // (One tile can't be shared: never less than the longest.)
        assert!((s.tiles_wall(&cold[..1], 8) - s.tile_work(&cold[0])).abs() < 1e-9);
    }

    #[test]
    fn triangles_add_to_a_tile_s_time() {
        let s = Speed::DEFAULT;
        assert!(s.tile_work(&tile(false, false, 50.0)) > s.tile_work(&tile(false, false, 0.0)));
        assert!(s.tile_work(&tile(false, true, 50.0)) - s.tile_work(&tile(false, false, 50.0)) > 5.0 * s.shadow);
    }

    #[test]
    fn an_export_measures_this_machine_s_speed() {
        let s = Speed::DEFAULT;
        let tiles = [vec![tile(true, true, 1.0); 30], vec![tile(false, false, 1.0); 10]].concat();
        // Twice the default bake, twice the shadows, the rest as planned.
        let bake = 30.0 * 4.4 + 10.0 * s.kept;
        let shadow = 30.0 * 2.0 * (s.shadow + s.shadow_ktri) + 10.0 * s.kept;
        let rest = 40.0 * (s.tile + s.ktri);
        let m = measured(s, &tiles, 8, (bake + shadow + rest) / 8.0, [bake, shadow, rest]);
        assert!((m.bake - 4.4).abs() < 1e-6, "{m:?}");
        assert!((m.shadow - 2.0 * s.shadow).abs() < 1e-6 && (m.shadow_ktri - 2.0 * s.shadow_ktri).abs() < 1e-6, "{m:?}");
        assert!((m.tile - s.tile).abs() < 1e-9, "{m:?}");
        assert!((m.busy - 1.0).abs() < 1e-6, "{m:?}");
        // A warm export measures what a kept file costs and leaves the bake's cost alone.
        let warm = vec![tile(false, false, 0.0); 16];
        let m2 = measured(m, &warm, 8, 1.0, [16.0 * 0.05, 16.0 * 0.05, 16.0 * s.tile]);
        assert!((m2.kept - 0.05).abs() < 1e-9 && (m2.bake - 4.4).abs() < 1e-6, "{m2:?}");
    }

    #[test]
    fn a_speed_file_out_of_reason_keeps_the_defaults() {
        let s = Speed::from_json(&json!({ "bake": -3, "shadow": "x", "busy": 7, "threads": 0, "tile": 0.5 }));
        assert_eq!((s.bake, s.shadow, s.busy, s.threads, s.tile), (2.2, 0.233, 0.95, 1, 0.5));
        assert_eq!(Speed::from_json(&Speed::DEFAULT.to_json()), Speed::DEFAULT);
    }

    #[test]
    fn the_time_left_follows_the_tiles_as_they_finish() {
        let mut s = fresh();
        s.active = true;
        s.threads = 1;
        let work = vec![tile(true, true, 0.0); 10];
        s.work = work.iter().map(|t| s.speed.tile_work(t)).collect();
        s.tiles_wall = s.speed.tiles_wall(&work, 1);
        s.tiles = work;
        s.after = 2.0;
        // Started 10 s ago and half done: the tiles go twice as fast as estimated (31.5 s), so about 10 s are left plus the rest.
        s.tiles_at = Some(Instant::now() - std::time::Duration::from_secs(10));
        s.done = 5;
        s.work_done = s.work.iter().take(5).sum();
        let left = s.left_now().expect("tiles under way");
        assert!(left > 6.0 && left < 14.0, "{left}");
    }
}
