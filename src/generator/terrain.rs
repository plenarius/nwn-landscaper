// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The ground the generator cuts: the heights (a painted terrain's, else the built-in test terrain and its LS_FLAT and
//! rugged variants), where nothing can walk, and each tile's height level.

use super::{SHALLOW, TILE, VOID_BELOW};
use crate::{heightmap, objects};

const CLIFF: f32 = 4.0;

/// The built-in test terrain's cliff height as f32 bits (0: as `LS_CLIFF` says): `sim-check` sets it for each oracle file, so
/// it rebuilds the area that file was recorded on.
pub(crate) static CLIFF_OVERRIDE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Whether the built-in test terrain is flat: 0 as `LS_FLAT` says, 1 flat, 2 not (`sim-check` sets it, as the cliff height).
pub(crate) static FLAT_OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The cliff height: the override, else LS_CLIFF if set (to re-create an
/// oracle corpus's area without editing the constant), else CLIFF.
fn cliff_height() -> f32 {
    let bits = CLIFF_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    if bits != 0 {
        return f32::from_bits(bits);
    }
    static H: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *H.get_or_init(|| std::env::var("LS_CLIFF").ok().and_then(|v| v.parse().ok()).unwrap_or(CLIFF))
}
const CLIFF_WIDTH: f32 = 0.6;

/// Ground nothing can walk on besides slopes: LS_FLAT's strip, deep water.
pub(crate) fn blocked(x: f32, y: f32) -> bool {
    flat_blocked(x, y) || heightmap::deep_water(x, y, SHALLOW) || objects::scene().is_some_and(|s| s.blocks(x, y)) || in_void(x, y)
}

/// Whether (x, y) is a cave's void: rock at its roof (nothing walks there).
pub(super) fn in_void(x: f32, y: f32) -> bool {
    heightmap::current().filter(|h| h.underground).is_some_and(|h| h.roof_at(x, y).is_some_and(|r| h.at(x, y) >= r - VOID_BELOW))
}

/// LS_FLAT: the same layout with no height: the cliff becomes a strip of non-walkable ground (x 24..25, y 10..40), to tell height from
/// topology. The rugged variant: terraced fractal noise over 8 x 8 tiles (stress test).
pub(crate) static RUGGED_OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether the built-in test terrain is the rugged one: the override, else `LS_RUGGED`.
pub(super) fn rugged() -> bool {
    match RUGGED_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => std::env::var_os("LS_RUGGED").is_some(),
    }
}

/// Value noise in [0, 1) on a unit lattice, smoothly interpolated.
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let hash = |i: i32, j: i32| -> f32 {
        let mut h = (i as u32).wrapping_mul(374761393) ^ (j as u32).wrapping_mul(668265263) ^ seed.wrapping_mul(2246822519);
        h = (h ^ (h >> 13)).wrapping_mul(1274126177);
        ((h ^ (h >> 16)) & 0xffffff) as f32 / 16777216.0
    };
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * sx;
    let b = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * sx;
    a + (b - a) * sy
}

/// Terraced fractal noise: 3 m steps whose edges are cliffs where the noise is steep and ramps where it is gentle.
fn rugged_height(x: f32, y: f32) -> f32 {
    let mut n = 0.0;
    let (mut amp, mut freq) = (1.0, 1.0 / 22.0);
    for o in 0..4 {
        n += amp * value_noise(x * freq, y * freq, 7 + o);
        amp *= 0.5;
        freq *= 2.0;
    }
    let t = n / 1.875 * 12.0 / 3.0; // ~0..4 terraces of 3 m
    let level = t.floor();
    let f = t - level;
    3.0 * (level + smoothstep(0.82, 1.0, f))
}

/// Whether the built-in test terrain is flat: the override, else `LS_FLAT`.
pub(super) fn flat() -> bool {
    match FLAT_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => std::env::var_os("LS_FLAT").is_some(),
    }
}

fn flat_blocked(x: f32, y: f32) -> bool {
    if heightmap::current().is_some() {
        return false;
    }
    flat() && (24.0..25.0).contains(&x) && y >= 10.0
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The terrain, in area coordinates (0..40 m each way, south-west origin): a hill in the north-west, and a 4 m plateau to the east
/// behind a cliff along x = 24, except for a ramp between y = 3 and y = 9.
pub(crate) fn height(x: f32, y: f32) -> f32 {
    if let Some(z) = heightmap::sample(x, y) {
        return z;
    }
    if rugged() {
        return rugged_height(x, y);
    }
    if flat() {
        return 0.0;
    }
    let hill = 2.5 * (-((x - 10.0).powi(2) + (y - 30.0).powi(2)) / 40.0).exp();
    let ramp = smoothstep(2.0, 4.0, y) * (1.0 - smoothstep(8.0, 10.0, y));
    let cliff = smoothstep(24.0, 24.0 + CLIFF_WIDTH, x);
    let slope = smoothstep(14.0, 26.0, x);
    let s = cliff + (slope - cliff) * ramp;
    hill * (1.0 - s) + cliff_height() * s
}

/// The engine's height step between tile levels (the .set's Transition).
pub(super) const TRANSITION: f32 = 5.0;

/// A tile's height level: its mean ground height in TRANSITION steps. The engine expects a tile's ground near Tile_Height *
/// Transition; pathing across tiles fails once ground strays ~2.5-4 m from it.
pub(super) fn tile_level(tx: usize, ty: usize) -> i32 {
    if std::env::var_os("LS_NOLEVEL").is_some() {
        return 0;
    }
    let n = 10;
    let mut sum = 0.0;
    for j in 0..=n {
        for i in 0..=n {
            sum += height(tx as f32 * TILE + i as f32, ty as f32 * TILE + j as f32);
        }
    }
    (sum / ((n + 1) * (n + 1)) as f32 / TRANSITION).round() as i32
}
