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
//!
//! A natural texture's repeat is varied ([`varied`]), so its features don't line up in a grid every 4 m: each projection is split
//! into small hexagonal cells, each cell draws the picture shifted by its own offset, and the three cells round a point blend
//! (hex tiling, after Mikkelsen's "Practical Real-Time Hex-Tiling", 2022), with a slow change of brightness over the ground
//! ([`shade`]). The same three places draw it alike: a cell's offset is a sine of its whole-number coordinates (no hash), so
//! the game's GPU and the bake agree to a small part of a pixel, and a slot changes cell only where its weight is 0, so the GPU's
//! mipmaps never jump.

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

/// Hex cells to a repeat of the game's picture, each way ([`varied`]): a cell is about two thirds of the picture, small enough that
/// no feature recurs in a row, large enough that each cell shows whole stones and tufts.
pub const VARY_CELLS: f32 = 1.5;

/// How sharply the three cells round a point blend: each weighs as its share of the point (barycentric) to this power. At 7, close
/// up in the game, two copies of a stone ghosted into one another where cells met; much higher shows the cells' edges.
pub const VARY_SHARPNESS: f32 = 14.0;

/// The brighter of two cells' colours wins where they blend (its weight times this plus its luminance): a pale stone over dark
/// grit, not a grey average of the two.
pub const VARY_LIGHT: f32 = 0.25;

/// How far the ground's brightness strays over tens of metres ([`shade`]): up to 7% lighter or darker.
pub const VARY_SHADE: f32 = 0.07;

/// The skewed grid's matrix (hex tiling's): x - y / sqrt(3), 2 y / sqrt(3).
pub const SKEW: [f32; 2] = [0.577_350_27, 1.154_700_5];

/// A cell's offset, each way: sin(a x + b y + c) of its whole-number coordinates (x, y), as [a, b, c].
pub const OFFSET: [[f32; 3]; 2] = [[1.7, 3.1, 0.0], [2.3, -1.3, 0.7]];

/// The slow change of brightness ([`shade`]): the mean of two waves sin(a x + b y + c z + d) over area metres, as [a, b, c, d].
pub const SHADE_WAVES: [[f32; 4]; 2] = [[0.13, 0.07, 0.05, 0.0], [-0.06, 0.11, 0.09, 2.1]];

/// A cell's offset (texture repeats) from its whole-number coordinates ([`OFFSET`]): two sines, the same in the game's shader and
/// the editor's; no hash, so the GPU's sine and the bake's differ by a small part of a pixel, not by another cell.
pub fn cell_offset(v: [f32; 2]) -> [f32; 2] {
    OFFSET.map(|[a, b, c]| (v[0] * a + v[1] * b + c).sin())
}

/// The three hex cells round texture point `uv` (repeats) with `cells` cells a repeat: for each slot, the cell (whole-number
/// coordinates) and its share of the point (barycentric, summing to 1). A cell keeps its slot (by its coordinates' class, x + 2y
/// modulo 3), so a slot changes cell only where its share is 0.
pub fn hex_cells(uv: [f32; 2], cells: f32) -> [([f32; 2], f32); 3] {
    let (su, sv) = (uv[0] * cells, uv[1] * cells);
    let k = [su - SKEW[0] * sv, SKEW[1] * sv];
    let base = [k[0].floor(), k[1].floor()];
    let f = [k[0] - base[0], k[1] - base[1]];
    let fz = 1.0 - f[0] - f[1];
    let s = if fz < 0.0 { 1.0 } else { 0.0 };
    let s2 = 2.0 * s - 1.0;
    let w = [-fz * s2, s - f[1] * s2, s - f[0] * s2];
    let v = [[base[0] + s, base[1] + s], [base[0] + s, base[1] + 1.0 - s], [base[0] + 1.0 - s, base[1] + s]];
    let mut out = [([0.0; 2], 0.0); 3];
    for (vk, wk) in v.into_iter().zip(w) {
        let class = (vk[0] as i64 + 2 * vk[1] as i64).rem_euclid(3) as usize;
        out[class] = (vk, wk.max(0.0));
    }
    out
}

/// A varied repeat: the colours `tap` gives at the three cells' offsets round `uv` ([`hex_cells`]), blended by their shares to
/// [`VARY_SHARPNESS`], the brighter winning ([`VARY_LIGHT`]).
pub fn varied(uv: [f32; 2], cells: f32, mut tap: impl FnMut(f32, f32) -> [f32; 3]) -> [f32; 3] {
    let mut sum = [0.0; 3];
    let mut total = 0.0;
    for (v, w) in hex_cells(uv, cells) {
        let o = cell_offset(v);
        let c = tap(uv[0] + o[0], uv[1] + o[1]);
        let weight = w.powf(VARY_SHARPNESS) * (VARY_LIGHT + 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]);
        for k in 0..3 {
            sum[k] += c[k] * weight;
        }
        total += weight;
    }
    sum.map(|c| c / total.max(f32::MIN_POSITIVE))
}

/// The slow change of a varied texture's brightness at area point `p` (metres), a factor about 1 ([`VARY_SHADE`]).
pub fn shade(p: [f32; 3]) -> f32 {
    let [x, y, z] = p;
    1.0 + VARY_SHADE * SHADE_WAVES.iter().map(|[a, b, c, d]| 0.5 * (a * x + b * y + c * z + d).sin()).sum::<f32>()
}

/// GLSL for the game's tile shader (shared.rs) and, as its text, a check for the editor's: `vec4 lsVaried(sampler2D t, highp vec2
/// uv, float cells)`, [`varied`] (`cells` 0: the plain texture), and `float lsVaryShade(highp vec3 p)`, [`shade`].
pub fn varied_glsl() -> String {
    let f = |v: f32| format!("{v:.8}");
    let off = |k: usize| format!("sin(v.x * {} + v.y * {} + {})", f(OFFSET[k][0]), f(OFFSET[k][1]), f(OFFSET[k][2]));
    let wave = |w: [f32; 4]| format!("0.5 * sin(p.x * {} + p.y * {} + p.z * {} + {})", f(w[0]), f(w[1]), f(w[2]), f(w[3]));
    format!(
        "// The Landscaper's varied repeat (its triplanar.rs, `varied`): three hex cells round a point, each drawing the texture
// shifted by its own offset, blended by their shares, the brighter winning; a slot changes cell only where its share is 0.
float lsClass(vec2 v) {{ float c = v.x + 2.0 * v.y; return c - 3.0 * floor((c + 0.5) / 3.0); }}
vec4 lsTap(sampler2D t, highp vec2 uv, highp vec2 v, float w, inout float total)
{{
    vec4 c = texture2D(t, uv + vec2({}, {}));
    float weight = pow(max(w, 0.0), {}) * ({} + dot(c.rgb, vec3(0.3, 0.59, 0.11)));
    total += weight;
    return c * weight;
}}
vec4 lsVaried(sampler2D t, highp vec2 uv, float cells)
{{
    if (cells <= 0.0) return texture2D(t, uv);
    highp vec2 su = uv * cells;
    highp vec2 k = vec2(su.x - {} * su.y, {} * su.y);
    highp vec2 b = floor(k);
    highp vec2 f = k - b;
    float fz = 1.0 - f.x - f.y;
    float s = 1.0 - step(0.0, fz);
    float s2 = 2.0 * s - 1.0;
    vec3 w = vec3(-fz * s2, s - f.y * s2, s - f.x * s2);
    // (largest share 1: a third to the 14th, mid-triangle, fell under the floor below and drew a dark dot every cell)
    w /= max(w.x, max(w.y, w.z));
    highp vec2 v1 = b + vec2(s, s);
    highp vec2 v2 = b + vec2(s, 1.0 - s);
    highp vec2 v3 = b + vec2(1.0 - s, s);
    vec3 cls = vec3(lsClass(v1), lsClass(v2), lsClass(v3));
    float total = 0.0;
    vec4 sum = vec4(0.0);
    for (int i = 0; i < 3; i++)
    {{
        vec3 e = vec3(1.0) - step(vec3(0.5), abs(cls - vec3(float(i))));
        sum += lsTap(t, uv, v1 * e.x + v2 * e.y + v3 * e.z, dot(w, e), total);
    }}
    return sum / max(total, 1e-6);
}}
float lsVaryShade(highp vec3 p) {{ return 1.0 + {} * ({} + {}); }}",
        off(0),
        off(1),
        f(VARY_SHARPNESS),
        f(VARY_LIGHT),
        f(SKEW[0]),
        f(SKEW[1]),
        f(VARY_SHADE),
        wave(SHADE_WAVES[0]),
        wave(SHADE_WAVES[1]),
    )
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
    fn a_point_s_three_cells_share_it_and_each_keeps_its_slot() {
        for uv in [[0.1, 0.2], [3.7, -2.25], [10.5, 7.33], [-0.4, 0.9]] {
            let cells = hex_cells(uv, VARY_CELLS);
            assert!(close(cells.iter().map(|c| c.1).sum(), 1.0), "{uv:?}: {cells:?}");
            for (slot, (v, _)) in cells.iter().enumerate() {
                assert_eq!((v[0] as i64 + 2 * v[1] as i64).rem_euclid(3) as usize, slot, "{uv:?}");
            }
        }
        // Across a triangle's edge a slot changes cell only where its share is 0: the GPU's mipmaps see no jump.
        for x in 0..400 {
            let (a, b) = (hex_cells([x as f32 * 0.01, 0.37], 2.0), hex_cells([x as f32 * 0.01 + 0.0001, 0.37], 2.0));
            for slot in 0..3 {
                if a[slot].0 != b[slot].0 {
                    assert!(a[slot].1 < 1e-3 && b[slot].1 < 1e-3, "slot {slot} at {x}: {a:?} {b:?}");
                }
            }
        }
    }

    #[test]
    fn a_varied_texture_has_no_grid_and_keeps_a_flat_colour() {
        // A flat colour stays itself; a picture that is one bright dot in a repeat no longer has its dots every repeat.
        let flat = varied([1.3, 2.7], VARY_CELLS, |_, _| [0.4, 0.5, 0.6]);
        assert!(flat.iter().zip([0.4, 0.5, 0.6]).all(|(a, b)| close(*a, b)), "{flat:?}");
        let dot = |u: f32, v: f32| if u.rem_euclid(1.0) < 0.1 && v.rem_euclid(1.0) < 0.1 { [1.0; 3] } else { [0.0; 3] };
        let lit: Vec<bool> = (0..8).map(|k| varied([k as f32 + 0.05, 0.05], VARY_CELLS, dot)[0] > 0.5).collect();
        assert!(lit.iter().any(|l| !l), "the dot recurs at every repeat: {lit:?}");
        assert!((0.9..1.1).contains(&shade([123.0, 45.0, 6.0])));
    }

    #[test]
    fn the_shader_keeps_a_dark_colour_mid_triangle() {
        // Mid-triangle each share is a third: weighed as they were, the sum (2e-7) fell under the shader's floor of 1e-6.
        let glsl = varied_glsl();
        assert!(glsl.contains("w /= max(w.x, max(w.y, w.z));") && glsl.contains("sum / max(total, 1e-6)"), "{glsl}");
        let c = 0.05;
        let blend = |w: [f32; 3]| {
            let weights = w.map(|x| x.powf(VARY_SHARPNESS) * (VARY_LIGHT + c));
            weights.iter().map(|x| c * x).sum::<f32>() / weights.iter().sum::<f32>().max(1e-6)
        };
        assert!(blend([1.0 / 3.0; 3]) < 0.5 * c, "unscaled shares no longer darken: the test no longer shows the fault");
        assert!(close(blend([1.0; 3]), c));
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
