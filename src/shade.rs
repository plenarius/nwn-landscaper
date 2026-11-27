// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Baked shadows: each tile's shadow map, multiplied over its lit ground by
//! our shader pair (`ls_lm_vs` and `ls_lm_fs`, made from the game's lit pair in shared.rs, through a per-mesh MTR).
//!
//! The game lights every area from one fixed direction, towards (4000,
//! 4500, 7000), by day and by night; only the colours change. So shadows
//! cast along it always agree with the game's own shading. Two terms:
//! - cast shadow: marching from the ground towards the sun, is terrain in
//!   the way (softened by how far above the ray it rises);
//! - ambient occlusion: how much of the sky the surrounding terrain hides
//!   (hollows, cliff feet), from the horizon in eight directions.
//!
//! Baked objects shade the ground by their own triangles ([`sun_through`], [`sky_through`]): rays to a sun of some width from points spread over
//! the texel, so edges come out soft and unstepped, and short rays over the sky for the shade under and around them.
//!
//! Heights beyond the area continue its edge (as the edge scenery does).

/// Towards the game's sun and moon (NWN axes: x east, y north, z up).
fn sun() -> [f32; 3] {
    let (x, y, z) = (4000.0f32, 4500.0f32, 7000.0f32);
    let l = (x * x + y * y + z * z).sqrt();
    [x / l, y / l, z / l]
}

/// How much of the sun terrain hides from (x, y) at height z: 0 lit, 1 dark.
fn cast_shadow(x: f32, y: f32, z: f32, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    let s = sun();
    let horizontal = (s[0] * s[0] + s[1] * s[1]).sqrt();
    let (dx, dy, rise) = (s[0] / horizontal, s[1] / horizontal, s[2] / horizontal);
    let mut dark = 0.0f32;
    let mut t = 0.3;
    while t < 80.0 {
        let above = height(x + dx * t, y + dy * t) - (z + 0.05 + rise * t);
        if above > 0.0 {
            // Softer the farther the occluder: a penumbra growing with distance.
            dark = dark.max((above / (0.25 + 0.04 * t)).min(1.0));
            if dark >= 1.0 {
                break;
            }
        }
        t += if t < 4.0 { 0.25 } else { 0.5 };
    }
    dark
}

/// How far the ground at (x, y) turns towards the sun, against flat ground: 1 flat or turned to it, 0 turned away. The game lights
/// ground turned away with the area's ambient light alone, so a cast shadow has nothing more to take from it: shading it again
/// left a cliff on the side away from the sun near black.
fn facing(x: f32, y: f32, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    const D: f32 = 0.5;
    let gx = (height(x + D, y) - height(x - D, y)) / (2.0 * D);
    let gy = (height(x, y + D) - height(x, y - D)) / (2.0 * D);
    let s = sun();
    let n_dot_l = (s[2] - gx * s[0] - gy * s[1]) / (gx * gx + gy * gy + 1.0).sqrt();
    (n_dot_l / s[2]).clamp(0.0, 1.0)
}

/// How much of the sky the terrain around (x, y) hides: 0 open, 1 closed.
fn occlusion(x: f32, y: f32, z: f32, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    const STEPS: [f32; 8] = [0.5, 1.0, 1.75, 2.75, 4.0, 6.0, 9.0, 13.0];
    let mut sum = 0.0;
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (a.cos(), a.sin());
        let mut slope = 0.0f32;
        for t in STEPS {
            slope = slope.max((height(x + dx * t, y + dy * t) - z) / t);
        }
        // The sine of the horizon's elevation.
        sum += slope / (1.0 + slope * slope).sqrt();
    }
    sum / 8.0
}

/// What the baked objects let through to a point of ground: of the sun and of the sky, 1 all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Through {
    pub sun: f32,
    pub sky: f32,
}

/// Nothing in the way.
pub const OPEN: Through = Through { sun: 1.0, sky: 1.0 };

/// The sun's angular radius (radians): about twice the real one, so object shadows sharpen at the trunk and soften with height.
const SUN_RADIUS: f32 = 0.009;
/// How far the sky rays look for objects (m).
const SKY_REACH: f32 = 6.0;
/// How far beside an object its shade reaches on level ground (m): the sky rays, and a texel's spread and the sun's width.
pub const OBJECT_REACH: f32 = SKY_REACH + 0.5;
/// How dark the sky the objects hide makes the ground, at most.
const SKY_DARK: f32 = 0.7;

/// The `k`th of `n` points of a 2D low-discrepancy sequence (R2), turned by `seed` so neighbouring texels don't share it.
fn r2(k: u32, seed: u32) -> [f32; 2] {
    const A: [f64; 2] = [0.754_877_666_246_692_7, 0.569_840_290_998_053_3];
    let s = f64::from(seed.wrapping_mul(0x9E37_79B9) >> 8) / f64::from(1u32 << 24);
    let t = f64::from(seed.wrapping_mul(0x85EB_CA6B) >> 8) / f64::from(1u32 << 24);
    [((s + A[0] * f64::from(k)).fract()) as f32, ((t + A[1] * f64::from(k)).fract()) as f32]
}

/// Two unit vectors square to `n` and to each other.
fn frame(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let a = if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let t = [a[1] * n[2] - a[2] * n[1], a[2] * n[0] - a[0] * n[2], a[0] * n[1] - a[1] * n[0]];
    let l = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
    let t = [t[0] / l, t[1] / l, t[2] / l];
    (t, [n[1] * t[2] - n[2] * t[1], n[2] * t[0] - n[0] * t[2], n[0] * t[1] - n[1] * t[0]])
}

/// A point of the texel `texel` metres square about (x, y), on the ground: the `k`th of a pattern `seed` turns.
fn in_texel(x: f32, y: f32, texel: f32, k: u32, seed: u32, height: &dyn Fn(f32, f32) -> f32) -> [f32; 3] {
    let p = r2(k, seed);
    let (px, py) = (x + (p[0] - 0.5) * texel, y + (p[1] - 0.5) * texel);
    [px, py, height(px, py) + 0.03]
}

/// How much of the sun the baked objects let through to the texel `texel` metres square about (x, y): rays from points spread over
/// it towards points spread over the sun's disc. `seed` turns the pattern texel by texel.
pub fn sun_through(occ: &crate::occluders::Occluders, x: f32, y: f32, texel: f32, seed: u32, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    let s = sun();
    let (t1, t2) = frame(s);
    let ray = |k: u32| {
        let q = r2(k, seed ^ 0x5bd1_e995);
        let (r, a) = (SUN_RADIUS * q[0].sqrt(), std::f32::consts::TAU * q[1]);
        let d = [0, 1, 2].map(|i| s[i] + r * (a.cos() * t1[i] + a.sin() * t2[i]));
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        occ.through(in_texel(x, y, texel, k, seed, height), d.map(|v| v / l), 200.0)
    };
    // Four first: all lit or all dark is the answer for most texels.
    let first = [0, 1, 2, 3].map(ray);
    if first.iter().all(|&v| v == first[0]) && (first[0] == 0.0 || first[0] == 1.0) {
        return first[0];
    }
    (first.iter().sum::<f32>() + (4..SUN_RAYS).map(ray).sum::<f32>()) / SUN_RAYS as f32
}

/// Rays a texel to the sun, where its first four disagree.
const SUN_RAYS: u32 = 16;
/// Rays a texel to the sky.
const SKY_RAYS: u32 = 32;

/// How much of the sky the baked objects let through to the texel about (x, y): rays cosine-weighted about the ground's normal, as
/// far as [`SKY_REACH`]. Noisy point to point: blur it.
pub fn sky_through(occ: &crate::occluders::Occluders, x: f32, y: f32, texel: f32, seed: u32, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    let e = 0.3;
    let (dx, dy) = ((height(x + e, y) - height(x - e, y)) / (2.0 * e), (height(x, y + e) - height(x, y - e)) / (2.0 * e));
    let l = (dx * dx + dy * dy + 1.0).sqrt();
    let n = [-dx / l, -dy / l, 1.0 / l];
    let (b1, b2) = frame(n);
    (0..SKY_RAYS)
        .map(|k| {
            let q = r2(k, seed ^ 0x2c1b_3c6d);
            let (r, a) = (q[0].sqrt(), std::f32::consts::TAU * q[1]);
            let up = (1.0 - q[0]).sqrt();
            let d = [0, 1, 2].map(|i| r * a.cos() * b1[i] + r * a.sin() * b2[i] + up * n[i]);
            occ.through(in_texel(x, y, texel, k, seed, height), d, SKY_REACH)
        })
        .sum::<f32>()
        / SKY_RAYS as f32
}

/// What the terrain does to (x, y): how much of the sun it hides ([`cast_shadow`], as far as the ground faces the sun:
/// [`facing`]; 0 with `sun` false: underground) and of the sky ([`occlusion`]). Smooth: fine to interpolate.
pub fn terrain(x: f32, y: f32, sun: bool, height: &dyn Fn(f32, f32) -> f32) -> [f32; 2] {
    let z = height(x, y);
    [if sun { cast_shadow(x, y, z, height) * facing(x, y, height) } else { 0.0 }, occlusion(x, y, z, height)]
}

/// The shadow-map value from [`terrain`]'s two and what objects let through: 1 unshaded, darker in shadow; `strength` 0 (none) to 1.
pub fn combine(strength: f32, [cast, hidden]: [f32; 2], through: Through) -> f32 {
    let lit = 1.0 - 0.6 * (1.0 - (1.0 - cast) * through.sun);
    let open = (1.0 - 0.8 * hidden) * (1.0 - SKY_DARK * (1.0 - through.sky));
    (1.0 - strength * (1.0 - lit * open)).clamp(0.15, 1.0)
}

/// The shadow-map value at (x, y) from the terrain alone: 1 unshaded, darker in shadow; `strength` 0 (none) to 1; sun shadows and
/// ambient occlusion, or with `sun` false (underground) only ambient occlusion. (The editor's js/shade.js is the same.)
pub fn shade_lit(x: f32, y: f32, strength: f32, sun: bool, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    combine(strength, terrain(x, y, sun, height), OPEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_shades_the_side_away_from_the_sun() {
        // A 10 m wall along y at x 20..22: the sun is towards +x +y, so
        // ground west of it (x < 20) is in shadow, ground east of it isn't.
        let h = |x: f32, _y: f32| if (20.0..22.0).contains(&x) { 10.0 } else { 0.0 };
        let (shadowed, sunny, far) =
            (shade_lit(18.0, 10.0, 1.0, true, &h), shade_lit(30.0, 10.0, 1.0, true, &h), shade_lit(60.0, 10.0, 1.0, true, &h));
        assert!(shadowed < 0.6, "{shadowed}");
        // East of it only the wall's hiding of the sky darkens (ambient occlusion).
        assert!(sunny > shadowed + 0.15 && sunny < 0.95, "{sunny}");
        assert_eq!(far, 1.0);
        // The editor's js/shade.js gives the same values: keep them in step.
        assert_eq!(format!("{shadowed:.4} {sunny:.4}"), "0.2872 0.8037");
        // Flat open ground is untouched; strength 0 leaves everything.
        assert_eq!(shade_lit(60.0, 10.0, 1.0, true, &|_, _| 0.0), 1.0);
        assert_eq!(shade_lit(18.0, 10.0, 0.0, true, &h), 1.0);
    }

    #[test]
    fn a_face_turned_from_the_sun_is_shaded_only_by_the_sky_it_loses() {
        // A 70 degree face rising to the north (facing south, away from the sun): the game gives it ambient light only, so its
        // shadow map darkens that by its occlusion, not by the shadow it casts on itself as well.
        let h = |_x: f32, y: f32| ((y - 10.0) * 2.75).clamp(0.0, 7.0);
        let on_face = 10.0 + 3.5 / 2.75;
        let [cast, hidden] = terrain(20.0, on_face, true, &h);
        assert_eq!(cast, 0.0, "{cast}");
        assert!(shade_lit(20.0, on_face, 1.0, true, &h) > 0.6, "{hidden}");
        // A face rising to the south (facing north, towards the sun) still takes nearly all of a cast shadow.
        let f = facing(20.0, 10.0 + 3.5 / 2.75, &|_x, y| ((10.0 + 7.0 / 2.75 - y) * 2.75).clamp(0.0, 7.0));
        assert!(f > 0.9, "{f}");
    }
}
