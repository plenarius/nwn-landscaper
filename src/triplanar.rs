// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Triplanar texturing: a repeating library texture drawn on the ground from three sides at once (from above, along x and along
//! y), blended by which way the ground faces, so it keeps its size on every slope from every direction.
//!
//! Drawn from one side only, a texture stretches where the ground turns away from that side: from above, the steepest walkable
//! slope (50 degrees) stretched it 1.6 times; from the side along x + y, as steep ground had it before, ground facing north-east or
//! south-west smeared it into streaks, because x + y hardly changes across it.
//!
//! Three places draw it, and they agree: the game's shader for tiles in a library texture (`ls_tp_fs`, made in shared.rs), the
//! baked texture of a tile painted with several (painter.rs, `bake`) and the editor's view (js/triplanar.js in the web page, the
//! same [`SHARPNESS`] and projections). The tiles' own texture coordinates, for viewers without the game's shaders (Moonglow,
//! nwn-bake), give each face one side alone ([`box_projection`]): no blend, so a seam where two sides meet, but no streaks.

/// How sharply the three projections blend: each weighs as the normal's part along its axis to this power. With 4, ground at 45
/// degrees takes the top and the side projection evenly, and ground 30 degrees from flat takes 90% from the top: the blend is
/// narrow enough not to blur the texture and wide enough to hide where one projection gives way to another.
pub const SHARPNESS: i32 = 4;

/// The weights of the three projections for ground facing `n` (unit or not): along x, along y, and from above, summing to 1.
pub fn weights(n: [f32; 3]) -> [f32; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len == 0.0 {
        return [0.0, 0.0, 1.0];
    }
    let w = n.map(|c| (c / len).abs().powi(SHARPNESS));
    let sum = w[0] + w[1] + w[2];
    w.map(|c| c / sum)
}

/// The normal the projections blend by at area point (x, y) of ground whose height `h` gives, from its slopes `step` metres each
/// way: on each axis the gentler of the two one-sided slopes (none where they disagree in sign) on walkable ground, the steeper
/// on a steep face. The light's normal is smoothed over several samples on steep ground ([`crate::heightmap::Heightmap::normal`]);
/// blended by that, flat floor at a wall's foot took the projection from the side and smeared into stripes along it, and the
/// wall's foot took the one from above and streaked up. With this, the floor ends flat at the wall and the wall stands upright.
pub fn blend_normal(h: impl Fn(f32, f32) -> f32, x: f32, y: f32, step: f32, steep: bool) -> [f32; 3] {
    let c = h(x, y);
    let pick = |a: f32, b: f32| {
        if steep {
            if a.abs() >= b.abs() { a } else { b }
        } else if a * b <= 0.0 {
            0.0
        } else if a.abs() <= b.abs() {
            a
        } else {
            b
        }
    };
    let gx = pick((h(x + step, y) - c) / step, (c - h(x - step, y)) / step);
    let gy = pick((h(x, y + step) - c) / step, (c - h(x, y - step)) / step);
    let len = (gx * gx + gy * gy + 1.0).sqrt();
    [-gx / len, -gy / len, 1.0 / len]
}

/// The projection a face facing `n` takes in its own texture coordinates, for viewers without the game's shader (box mapping):
/// from above (2) up to 45 degrees from flat; steeper, the side it faces most of eight, 45 degrees apart: along x (0), along y
/// (1), or along a diagonal, x = y (3) or x = -y (4). The diagonals keep a flank facing north-east from flipping face by face
/// between x and y, as four sides alone would; a face is stretched 1.6 times at most.
pub fn box_projection(n: [f32; 3]) -> usize {
    let [x, y, z] = n.map(f32::abs);
    if z >= (x * x + y * y).sqrt() {
        2
    } else if x.min(y) < x.max(y) * (std::f32::consts::FRAC_PI_8).tan() {
        if x >= y { 0 } else { 1 }
    } else if (n[0] >= 0.0) == (n[1] >= 0.0) {
        3
    } else {
        4
    }
}

/// The texture coordinates of area point `p` (metres: x east, y north, z up) in projection `axis` ([`weights`]' order, then the
/// diagonals of [`box_projection`]), in repeats of `metres` (the texture's [`crate::paint::Texture::metres`]): from above (x, y), as
/// flat ground always had them; along x (y, z); along y (x, z); along a diagonal, across it and up. Up is the texture's v, so a
/// cliff's texture stands upright.
pub fn uv(axis: usize, p: [f32; 3], metres: f32) -> [f32; 2] {
    let [x, y, z] = p.map(|c| c / metres);
    match axis {
        0 => [y, z],
        1 => [x, z],
        3 => [(y - x) * std::f32::consts::FRAC_1_SQRT_2, z],
        4 => [(x + y) * std::f32::consts::FRAC_1_SQRT_2, z],
        _ => [x, y],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::TEXTURE_METRES;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn flat_ground_is_drawn_from_above_alone() {
        assert_eq!(weights([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]);
        assert_eq!(box_projection([0.0, 0.0, 1.0]), 2);
    }

    #[test]
    fn the_weights_sum_to_one_whatever_the_normal_s_length_or_sign() {
        for n in [[0.3, -0.4, 0.8], [-2.0, 1.0, 0.5], [1.0, 1.0, 0.0], [0.0, -5.0, 0.0]] {
            let w = weights(n);
            assert!(close(w[0] + w[1] + w[2], 1.0), "{n:?}: {w:?}");
            assert!(w.iter().all(|&c| c >= 0.0));
        }
        let (a, b) = (weights([0.3, -0.4, 0.8]), weights([-0.6, 0.8, 1.6]));
        assert!(a.iter().zip(b).all(|(x, y)| close(*x, y)), "the same direction, twice as long: {a:?} {b:?}");
    }

    #[test]
    fn ground_facing_a_diagonal_takes_both_side_projections_evenly() {
        // The case the projection along x + y smeared: ground facing north-east (and south-west).
        for n in [[1.0, 1.0, 0.0], [-1.0, -1.0, 0.0], [1.0, -1.0, 0.0]] {
            let w = weights(n);
            assert!(close(w[0], 0.5) && close(w[1], 0.5) && close(w[2], 0.0), "{n:?}: {w:?}");
        }
    }

    #[test]
    fn the_blend_is_narrow() {
        // 30 degrees from flat (gentle walkable ground): 90% from above. 45 degrees: even. 60 degrees: 90% from the side.
        let at = |deg: f32| weights([deg.to_radians().sin(), 0.0, deg.to_radians().cos()]);
        assert!(close(at(30.0)[2], 0.9), "{:?}", at(30.0));
        assert!(close(at(45.0)[2], 0.5));
        assert!(close(at(60.0)[0], 0.9));
    }

    #[test]
    fn each_projection_keeps_the_texture_s_size_on_the_ground_it_faces() {
        // One texture repeat across 4 m of ground, whichever way it faces: a wall facing east runs on along y and up.
        assert_eq!(uv(2, [4.0, 8.0, 3.0], TEXTURE_METRES), [1.0, 2.0]);
        assert_eq!(uv(0, [4.0, 8.0, 2.0], TEXTURE_METRES), [2.0, 0.5]);
        assert_eq!(uv(1, [4.0, 8.0, 2.0], TEXTURE_METRES), [1.0, 0.5]);
        // A texture that covers 8 m (cliff rock): half the repeats.
        assert_eq!(uv(0, [4.0, 8.0, 2.0], 8.0), [1.0, 0.25]);
    }

    #[test]
    fn floor_at_a_wall_s_foot_blends_flat_and_the_wall_s_foot_upright() {
        // A floor at 0 and an 11 m wall rising over 0.625 m from x = 10, as a cave's.
        let h = |x: f32, _y: f32| ((x - 10.0) / 0.625).clamp(0.0, 1.0) * 11.0;
        let floor = blend_normal(h, 10.0, 5.0, 0.625, false);
        assert_eq!(weights(floor), [0.0, 0.0, 1.0], "the floor's last point takes the projection from above alone");
        let wall = blend_normal(h, 10.0, 5.0, 0.625, true);
        assert!(weights(wall)[0] > 0.99, "the wall's foot takes the one along x: {wall:?}");
        // Even ground: either way, its own slope.
        let slope = |x: f32, y: f32| 0.5 * x + 0.2 * y;
        for steep in [false, true] {
            let n = blend_normal(slope, 3.0, 4.0, 0.625, steep);
            let len = (0.25f32 + 0.04 + 1.0).sqrt();
            assert!(close(n[0], -0.5 / len) && close(n[1], -0.2 / len) && close(n[2], 1.0 / len), "{n:?}");
        }
    }

    #[test]
    fn a_steep_face_takes_the_side_it_faces_of_eight_and_keeps_the_texture_s_size() {
        assert_eq!(box_projection([0.9, 0.2, 0.3]), 0);
        assert_eq!(box_projection([-0.2, -0.9, 0.3]), 1);
        assert_eq!(box_projection([0.7, 0.0, 0.7]), 2, "45 degrees: still from above");
        // Flanks facing north-east or south-west, and north-west or south-east, a little off the diagonal.
        for (n, axis) in [([0.7, 0.6, 0.3], 3), ([-0.6, -0.7, 0.3], 3), ([0.7, -0.6, 0.3], 4), ([-0.6, 0.7, 0.3], 4)] {
            assert_eq!(box_projection(n), axis, "{n:?}");
        }
        // One metre across a wall facing north-east (along (-1, 1)) is a quarter repeat, as on flat ground; so is one up.
        let across = |axis, a: [f32; 3], b: [f32; 3]| {
            let (p, q) = (uv(axis, a, TEXTURE_METRES), uv(axis, b, TEXTURE_METRES));
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
        };
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(across(3, [10.0, 10.0, 2.0], [10.0 - s, 10.0 + s, 2.0]), 0.25));
        assert!(close(across(4, [10.0, 10.0, 2.0], [10.0 + s, 10.0 + s, 2.0]), 0.25));
        assert!(close(across(3, [10.0, 10.0, 2.0], [10.0, 10.0, 3.0]), 0.25));
    }
}
