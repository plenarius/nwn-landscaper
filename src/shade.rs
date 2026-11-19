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

/// The shadow-map value at (x, y): 1 unshaded, darker in shadow; `strength` 0 (none) to 1; sun shadows and ambient occlusion, or with
/// `sun` false (underground) only ambient occlusion.
pub fn shade_lit(x: f32, y: f32, strength: f32, sun: bool, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    let z = height(x, y);
    let lit = if sun { 1.0 - 0.6 * cast_shadow(x, y, z, height) } else { 1.0 };
    let open = 1.0 - 0.8 * occlusion(x, y, z, height);
    (1.0 - strength * (1.0 - lit * open)).clamp(0.15, 1.0)
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
}
