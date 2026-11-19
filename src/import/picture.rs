// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Drawing a tile's ground picture: rasterising triangles, sampling the tileset's textures, filling what nothing was drawn on
//! and what a thing stood on, and the tile's part of the pick map.

use glam::Vec3;
use mg_image::Rgba;
use mg_rules::GameData;

use super::{PICK, TILE};

/// A tile's part of the pick map (`PICK` rows of `PICK`) from its picture's texture numbers (`s` rows of `s`, 0 for none): each
/// block's most common number, the highest on a tie, 0 where it has none.
pub(super) fn pick_blocks(ids: &[u8], s: usize) -> Vec<u8> {
    let block = s / PICK;
    let mut map = vec![0u8; PICK * PICK];
    // (`seen` is back to all 0 after each block.)
    let mut seen = [0u16; 256];
    for bj in 0..PICK {
        for bi in 0..PICK {
            let at = |dj: usize, di: usize| ids[(bj * block + dj) * s + bi * block + di] as usize;
            for dj in 0..block {
                for di in 0..block {
                    seen[at(dj, di)] += 1;
                }
            }
            let mut top = (0, 0);
            for dj in 0..block {
                for di in 0..block {
                    let id = at(dj, di);
                    if id != 0 {
                        top = top.max((seen[id], id));
                    }
                    seen[id] = 0;
                }
            }
            map[bj * PICK + bi] = top.1 as u8;
        }
    }
    map
}

/// A thing's ground patch: alpha over the tile's picture, 1 within 0.25 m of its base (footprint), fading to 0 at 0.75 m; and the
/// rectangle of pixels outside which it is 0 (x0, y0, x1, y1, the ends excluded).
pub(super) fn patch(base: &[[[f32; 2]; 3]], s: usize, px: impl Fn(f32) -> f32) -> (Vec<f32>, [usize; 4]) {
    let mut inside = vec![false; s * s];
    let (mut i0, mut j0, mut i1, mut j1) = (s, s, 0, 0);
    for t in base {
        raster(t.map(|v| Vec3::new(px(v[0]), px(v[1]), 0.0)), s, |i, j, _| {
            inside[j * s + i] = true;
            (i0, j0, i1, j1) = (i0.min(i), j0.min(j), i1.max(i), j1.max(j));
        });
    }
    // Distance (pixels) from the base, by a two-pass chamfer.
    let far = 0.75 / TILE * s as f32;
    let full = 0.25 / TILE * s as f32;
    let mut d: Vec<f32> = inside.iter().map(|&b| if b { 0.0 } else { f32::MAX }).collect();
    if i0 > i1 {
        return (vec![0.0; s * s], [0; 4]);
    }
    // Only around the base: a pixel more than `m` pixels from all of it (in x or y) is at least that far by every path (each step
    // costs 1 or more), and so is all it could pass on, so what the chamfer gives within `far` is the same as over the whole picture.
    let m = far.ceil() as usize + 1;
    let (x0, y0, x1, y1) = (i0.saturating_sub(m), j0.saturating_sub(m), (i1 + m + 1).min(s), (j1 + m + 1).min(s));
    let (o, g) = (1.0f32, std::f32::consts::SQRT_2);
    for j in y0..y1 {
        for i in x0..x1 {
            let k = j * s + i;
            let mut v = d[k];
            if i > x0 {
                v = v.min(d[k - 1] + o);
            }
            if j > y0 {
                v = v.min(d[k - s] + o);
            }
            if i > x0 && j > y0 {
                v = v.min(d[k - s - 1] + g);
            }
            if i + 1 < x1 && j > y0 {
                v = v.min(d[k - s + 1] + g);
            }
            d[k] = v;
        }
    }
    for j in (y0..y1).rev() {
        for i in (x0..x1).rev() {
            let k = j * s + i;
            let mut v = d[k];
            if i + 1 < x1 {
                v = v.min(d[k + 1] + o);
            }
            if j + 1 < y1 {
                v = v.min(d[k + s] + o);
            }
            if i + 1 < x1 && j + 1 < y1 {
                v = v.min(d[k + s + 1] + g);
            }
            if i > x0 && j + 1 < y1 {
                v = v.min(d[k + s - 1] + g);
            }
            d[k] = v;
        }
    }
    (
        d.iter()
            .map(|&v| {
                if v <= full {
                    1.0
                } else if v >= far {
                    0.0
                } else {
                    1.0 - (v - full) / (far - full)
                }
            })
            .collect(),
        [x0, y0, x1, y1],
    )
}

/// Fills the picture's `hole` pixels from the ground around them (mirrored
/// across the hole's edge), then softens them a little (a 5 x 5 average).
pub(super) fn inpaint(img: &mut Rgba, hole: &[bool], s: usize) {
    if !hole.iter().any(|&h| h) {
        return;
    }
    let mut src: Vec<usize> = (0..s * s).collect();
    let mut queue: std::collections::VecDeque<usize> = (0..s * s).filter(|&k| !hole[k]).collect();
    let mut done: Vec<bool> = hole.iter().map(|&h| !h).collect();
    while let Some(k) = queue.pop_front() {
        let (i, j) = (k % s, k / s);
        for (x, y) in [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)] {
            if x < s && y < s && !done[y * s + x] {
                done[y * s + x] = true;
                src[y * s + x] = src[k];
                queue.push_back(y * s + x);
            }
        }
    }
    // Each hole pixel takes the ground mirrored across its nearest edge (keeping the texture's grain), else that edge's pixel.
    let orig = img.data.clone();
    for k in 0..s * s {
        if hole[k] {
            let (bi, bj) = ((src[k] % s) as i64, (src[k] / s) as i64);
            let (pi, pj) = ((k % s) as i64, (k / s) as i64);
            // The mirror point, else nearer ones on the same line: 4, 3, 2 and 1 quarters of the way back across the edge, rounded
            // half away from zero (as `f32::round` of that fraction, done in integers).
            let quarters = |d: i64, q: i64| (d * q).signum() * (((d * q).abs() + 2) / 4);
            let from = [4, 3, 2, 1]
                .iter()
                .map(|&q| (bi + quarters(bi - pi, q), bj + quarters(bj - pj, q)))
                .find(|&(ri, rj)| ri >= 0 && rj >= 0 && (ri as usize) < s && (rj as usize) < s && !hole[rj as usize * s + ri as usize])
                .map_or(src[k], |(ri, rj)| rj as usize * s + ri as usize);
            img.data[k * 4..k * 4 + 4].copy_from_slice(&orig[from * 4..from * 4 + 4]);
        }
    }
    // The 5 x 5 sums (those of the pixels inside the picture) from a summed-area table of the picture as it is now: `at[(y * w1) +
    // x]` is the sum of the pixels left of x and above y.
    let w1 = s + 1;
    let mut at = vec![[0u32; 3]; w1 * w1];
    for y in 0..s {
        let mut row = [0u32; 3];
        for x in 0..s {
            for c in 0..3 {
                row[c] += u32::from(img.data[(y * s + x) * 4 + c]);
                at[(y + 1) * w1 + x + 1][c] = at[y * w1 + x + 1][c] + row[c];
            }
        }
    }
    for k in (0..s * s).filter(|&k| hole[k]) {
        let (i, j) = (k % s, k / s);
        let (x0, y0, x1, y1) = (i.saturating_sub(2), j.saturating_sub(2), (i + 3).min(s), (j + 3).min(s));
        let n = ((x1 - x0) * (y1 - y0)) as u32;
        for c in 0..3 {
            let sum = at[y1 * w1 + x1][c] + at[y0 * w1 + x0][c] - at[y0 * w1 + x1][c] - at[y1 * w1 + x0][c];
            img.data[k * 4 + c] = (sum / n) as u8;
        }
    }
}

/// A texture (rows from the bottom: v up), or None: the image the game means by the name ([`nwn_bake::texture`]: an EE material of
/// that name gives its texture0).
pub(super) fn load_texture(game: &GameData, name: &str) -> Option<Rgba> {
    nwn_bake::texture::rgba(game, name).ok()
}

/// Bilinear RGBA at (u, v), repeating.
pub(super) fn sample(t: &Rgba, u: f32, v: f32) -> [u8; 4] {
    let (w, h) = (t.width as usize, t.height as usize);
    let fx = u.rem_euclid(1.0) * w as f32 - 0.5;
    let fy = v.rem_euclid(1.0) * h as f32 - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (ax, ay) = (fx - x0, fy - y0);
    let wrap = |a: f32, n: usize| (a as i64).rem_euclid(n as i64) as usize;
    let (x0, x1, y0, y1) = (wrap(x0, w), wrap(x0 + 1.0, w), wrap(y0, h), wrap(y0 + 1.0, h));
    let p = |x: usize, y: usize| &t.data[(y * w + x) * 4..(y * w + x) * 4 + 4];
    let mut out = [0u8; 4];
    #[expect(clippy::needless_range_loop, reason = "`c` picks the same colour channel in the four corners and out")]
    for c in 0..4 {
        let top = f32::from(p(x0, y0)[c]) * (1.0 - ax) + f32::from(p(x1, y0)[c]) * ax;
        let bottom = f32::from(p(x0, y1)[c]) * (1.0 - ax) + f32::from(p(x1, y1)[c]) * ax;
        out[c] = (top * (1.0 - ay) + bottom * ay).round() as u8;
    }
    out
}

/// Calls `f(i, j, barycentric)` for each pixel centre of an `s` square inside the triangle (pixel coordinates in x, y).
pub(super) fn raster(t: [Vec3; 3], s: usize, mut f: impl FnMut(usize, usize, [f32; 3])) {
    let (a, b, c) = (t[0], t[1], t[2]);
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if area.abs() < 1e-9 {
        return;
    }
    let lo = |v: f32| (v.floor().max(0.0) as usize).min(s);
    let hi = |v: f32| (v.ceil().max(0.0) as usize).min(s);
    for j in lo(a.y.min(b.y).min(c.y))..hi(a.y.max(b.y).max(c.y)) {
        for i in lo(a.x.min(b.x).min(c.x))..hi(a.x.max(b.x).max(c.x)) {
            let (x, y) = (i as f32 + 0.5, j as f32 + 0.5);
            let w0 = ((b.x - x) * (c.y - y) - (b.y - y) * (c.x - x)) / area;
            let w1 = ((c.x - x) * (a.y - y) - (c.y - y) * (a.x - x)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                f(i, j, [w0, w1, w2]);
            }
        }
    }
}

/// Fills pixels nothing was drawn on (ground hidden under a rock, say) from the drawn pixels around them: each takes the mean of
/// its drawn neighbours (west, east, south, north), pass after pass, until every pixel is drawn, a pass draws nothing (a picture
/// with nothing drawn at all) or 64 passes are done (what is further stays undrawn).
pub(super) fn fill_holes(img: &mut Rgba) {
    let (w, h) = (img.width as usize, img.height as usize);
    let (pixels, _) = img.data.as_chunks_mut::<4>();
    nwn_bake::area::floor::fill(
        pixels,
        w,
        h,
        Some(64),
        |p: &[u8; 4]| p[3] != 0,
        |near: &[[u8; 4]]| {
            let n = near.len() as u32;
            let mean = |c: usize| (near.iter().map(|p| u32::from(p[c])).sum::<u32>() / n) as u8;
            [mean(0), mean(1), mean(2), 255]
        },
    );
}

/// Fills pixels no walkmesh face covered from their neighbours, pass after pass until every pixel has ground (a big footprint can
/// be wide), a pass fills nothing (a tile with no ground at all) or `s` passes (10 m) are done: what is further keeps NaN, and the
/// sample the tile's height.
pub(super) fn fill_gaps(z: &mut [f32], s: usize) {
    nwn_bake::area::floor::fill_heights(z, s, s, Some(s));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::PICTURE;

    /// The import's gap filling as it was until the probe's fill replaced it: every pass scans the whole raster, and each empty
    /// pixel takes the mean of its filled neighbours as they were before the pass.
    fn fill_gaps_by_passes(z: &mut [f32], s: usize) {
        for _ in 0..s {
            let before = z.to_vec();
            let (mut left, mut filled) = (false, false);
            for j in 0..s {
                for i in 0..s {
                    if !before[j * s + i].is_nan() {
                        continue;
                    }
                    let around = [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)];
                    let near: Vec<f32> =
                        around.iter().filter(|(x, y)| *x < s && *y < s).map(|(x, y)| before[y * s + x]).filter(|v| !v.is_nan()).collect();
                    if near.is_empty() {
                        left = true;
                    } else {
                        z[j * s + i] = near.iter().sum::<f32>() / near.len() as f32;
                        filled = true;
                    }
                }
            }
            if !left || !filled {
                break;
            }
        }
    }

    // The picture's hole filling, ground patches, inpainting and pick map as they were before they were made faster, to check the
    // faster ones give the same bytes.
    /// (As it was: 64 passes over the whole picture.)
    fn patch_before(base: &[[[f32; 2]; 3]], s: usize, px: impl Fn(f32) -> f32) -> Vec<f32> {
        let mut inside = vec![false; s * s];
        for t in base {
            raster(t.map(|v| Vec3::new(px(v[0]), px(v[1]), 0.0)), s, |i, j, _| inside[j * s + i] = true);
        }
        // Distance (pixels) from the base, by a two-pass chamfer.
        let far = 0.75 / TILE * s as f32;
        let full = 0.25 / TILE * s as f32;
        let mut d: Vec<f32> = inside.iter().map(|&b| if b { 0.0 } else { f32::MAX }).collect();
        let (o, g) = (1.0f32, std::f32::consts::SQRT_2);
        for j in 0..s {
            for i in 0..s {
                let k = j * s + i;
                let mut v = d[k];
                if i > 0 {
                    v = v.min(d[k - 1] + o);
                }
                if j > 0 {
                    v = v.min(d[k - s] + o);
                }
                if i > 0 && j > 0 {
                    v = v.min(d[k - s - 1] + g);
                }
                if i + 1 < s && j > 0 {
                    v = v.min(d[k - s + 1] + g);
                }
                d[k] = v;
            }
        }
        for j in (0..s).rev() {
            for i in (0..s).rev() {
                let k = j * s + i;
                let mut v = d[k];
                if i + 1 < s {
                    v = v.min(d[k + 1] + o);
                }
                if j + 1 < s {
                    v = v.min(d[k + s] + o);
                }
                if i + 1 < s && j + 1 < s {
                    v = v.min(d[k + s + 1] + g);
                }
                if i > 0 && j + 1 < s {
                    v = v.min(d[k + s - 1] + g);
                }
                d[k] = v;
            }
        }
        d.iter()
            .map(|&v| {
                if v <= full {
                    1.0
                } else if v >= far {
                    0.0
                } else {
                    1.0 - (v - full) / (far - full)
                }
            })
            .collect()
    }
    fn inpaint_before(img: &mut Rgba, hole: &[bool], s: usize) {
        if !hole.iter().any(|&h| h) {
            return;
        }
        let mut src: Vec<usize> = (0..s * s).collect();
        let mut queue: std::collections::VecDeque<usize> = (0..s * s).filter(|&k| !hole[k]).collect();
        let mut done: Vec<bool> = hole.iter().map(|&h| !h).collect();
        while let Some(k) = queue.pop_front() {
            let (i, j) = (k % s, k / s);
            for (x, y) in [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)] {
                if x < s && y < s && !done[y * s + x] {
                    done[y * s + x] = true;
                    src[y * s + x] = src[k];
                    queue.push_back(y * s + x);
                }
            }
        }
        // Each hole pixel takes the ground mirrored across its nearest edge (keeping the texture's grain), else that edge's pixel.
        let orig = img.data.clone();
        for k in 0..s * s {
            if hole[k] {
                let (bi, bj) = ((src[k] % s) as i64, (src[k] / s) as i64);
                let (pi, pj) = ((k % s) as i64, (k / s) as i64);
                // The mirror point, else nearer ones on the same line.
                let from = [1.0f32, 0.75, 0.5, 0.25]
                    .iter()
                    .map(|t| (bi + ((bi - pi) as f32 * t).round() as i64, bj + ((bj - pj) as f32 * t).round() as i64))
                    .find(|&(ri, rj)| ri >= 0 && rj >= 0 && (ri as usize) < s && (rj as usize) < s && !hole[rj as usize * s + ri as usize])
                    .map_or(src[k], |(ri, rj)| rj as usize * s + ri as usize);
                img.data[k * 4..k * 4 + 4].copy_from_slice(&orig[from * 4..from * 4 + 4]);
            }
        }
        for _ in 0..1 {
            let before = img.data.clone();
            for k in (0..s * s).filter(|&k| hole[k]) {
                let (i, j) = ((k % s) as i64, (k / s) as i64);
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for dy in -2..=2i64 {
                    for dx in -2..=2i64 {
                        let (x, y) = (i + dx, j + dy);
                        if x >= 0 && y >= 0 && (x as usize) < s && (y as usize) < s {
                            let q = (y as usize * s + x as usize) * 4;
                            for c in 0..3 {
                                sum[c] += u32::from(before[q + c]);
                            }
                            n += 1;
                        }
                    }
                }
                for c in 0..3 {
                    img.data[k * 4 + c] = (sum[c] / n) as u8;
                }
            }
        }
    }
    fn fill_holes_before(img: &mut Rgba) {
        let (w, h) = (img.width as usize, img.height as usize);
        for _ in 0..64 {
            let before = img.data.clone();
            let mut left = false;
            for j in 0..h {
                for i in 0..w {
                    if before[(j * w + i) * 4 + 3] != 0 {
                        continue;
                    }
                    let (mut sum, mut n) = ([0u32; 3], 0);
                    for (x, y) in [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)] {
                        if x < w && y < h && before[(y * w + x) * 4 + 3] != 0 {
                            for c in 0..3 {
                                sum[c] += u32::from(before[(y * w + x) * 4 + c]);
                            }
                            n += 1;
                        }
                    }
                    if n == 0 {
                        left = true;
                        continue;
                    }
                    let k = (j * w + i) * 4;
                    img.data[k..k + 4].copy_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
                }
            }
            if !left {
                break;
            }
        }
    }

    /// (As it was: every block counted into a fresh table of all 256 numbers.)
    fn pick_blocks_before(ids: &[u8], s: usize) -> Vec<u8> {
        let block = s / PICK;
        let mut map = vec![0u8; PICK * PICK];
        for bj in 0..PICK {
            for bi in 0..PICK {
                let mut seen = [0u16; 256];
                for dj in 0..block {
                    for di in 0..block {
                        seen[ids[(bj * block + dj) * s + bi * block + di] as usize] += 1;
                    }
                }
                let top = (1..256).max_by_key(|&i| seen[i]).filter(|&i| seen[i] > 0).unwrap_or(0);
                map[bj * PICK + bi] = top as u8;
            }
        }
        map
    }

    /// A pseudo-random number below `n` (the same every run).
    fn next(seed: &mut u32, n: u32) -> u32 {
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (*seed >> 8) % n
    }

    /// A tile's picture as it is drawn: `blobs` round bits of ground, with nothing drawn between them, and `lone` single pixels.
    fn drawn(seed: &mut u32, blobs: usize, lone: usize) -> Rgba {
        let s = PICTURE as usize;
        let mut img = Rgba::new(PICTURE, PICTURE);
        for _ in 0..blobs {
            let (ci, cj, r) = (i64::from(next(seed, 512)), i64::from(next(seed, 512)), 3 + i64::from(next(seed, 60)));
            let c = [next(seed, 256) as u8, next(seed, 256) as u8, next(seed, 256) as u8];
            for j in (cj - r).max(0)..(cj + r).min(s as i64) {
                for i in (ci - r).max(0)..(ci + r).min(s as i64) {
                    if (i - ci).pow(2) + (j - cj).pow(2) <= r * r {
                        let k = (j as usize * s + i as usize) * 4;
                        let grain = next(seed, 40) as u8;
                        img.data[k..k + 4].copy_from_slice(&[c[0].saturating_add(grain), c[1].saturating_sub(grain), c[2], 255]);
                    }
                }
            }
        }
        for _ in 0..lone {
            let k = next(seed, (s * s) as u32) as usize * 4;
            img.data[k..k + 4].copy_from_slice(&[next(seed, 256) as u8, next(seed, 256) as u8, next(seed, 256) as u8, 255]);
        }
        img
    }

    #[test]
    fn the_hole_fill_gives_the_pass_by_pass_fill_s_bytes() {
        let s = PICTURE as usize;
        let check = |img: Rgba, what: &str| {
            let (mut expected, mut got) = (img.clone(), img);
            fill_holes_before(&mut expected);
            fill_holes(&mut got);
            assert!(got.data == expected.data, "{what}");
            expected
        };
        let mut seed = 11;
        // Ground with gaps between the blobs (all filled), and sparse blobs (some too far: 64 passes, then they stay undrawn).
        assert!(check(drawn(&mut seed, 300, 30), "blobs").data.chunks(4).all(|p| p[3] == 255));
        check(drawn(&mut seed, 3, 5), "sparse");
        // One pixel in a corner: what is more than 64 passes (pixels, across and down) away stays undrawn.
        let mut corner = Rgba::new(PICTURE, PICTURE);
        corner.data[..4].copy_from_slice(&[10, 200, 30, 255]);
        let filled = check(corner, "a corner");
        assert!(filled.data[(64 * s) * 4 + 3] == 255 && filled.data[(65 * s) * 4 + 3] == 0 && filled.data[(s * s - 1) * 4 + 3] == 0);
        // All drawn, and nothing drawn.
        let mut all = Rgba::new(PICTURE, PICTURE);
        all.data.chunks_mut(4).for_each(|p| p.copy_from_slice(&[1, 2, 3, 255]));
        check(all, "all drawn");
        check(Rgba::new(PICTURE, PICTURE), "nothing");
    }

    #[test]
    fn a_patch_is_the_same_as_over_the_whole_picture_and_nothing_outside_its_rectangle() {
        let s = PICTURE as usize;
        let px = |v: f32| (v + TILE / 2.0) / TILE * PICTURE as f32;
        let check = |base: &[[[f32; 2]; 3]], what: &str| {
            let expected = patch_before(base, s, px);
            let (got, [x0, y0, x1, y1]) = patch(base, s, px);
            assert_eq!(
                got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                "{what}"
            );
            for (q, a) in got.iter().enumerate() {
                let (i, j) = (q % s, q / s);
                assert!(*a == 0.0 || (i >= x0 && i < x1 && j >= y0 && j < y1), "{what}: ({i}, {j}) outside its rectangle");
            }
        };
        let tri = |x: f32, y: f32, r: f32| [[x - r, y - r], [x + r, y - r], [x, y + r]];
        check(&[tri(0.0, 0.0, 0.4)], "a trunk in the middle");
        check(&[tri(-4.9, -4.9, 0.3)], "in a corner");
        check(&[tri(4.8, 0.3, 0.6), tri(4.5, 1.5, 0.2)], "at an edge, two triangles");
        check(&[tri(0.0, 0.0, 6.0)], "over the whole tile");
        check(&[tri(-3.0, 2.0, 0.5), tri(3.0, -2.0, 0.5)], "two far apart");
        check(&[[[1.0, 1.0], [1.0, 1.0], [1.0, 1.0]]], "nothing in it");
        check(&[], "no base");
    }

    #[test]
    fn inpainting_gives_the_same_bytes_as_before() {
        let s = PICTURE as usize;
        let mut seed = 5;
        let check = |img: &Rgba, hole: &[bool], what: &str| {
            let (mut expected, mut got) = (img.clone(), img.clone());
            inpaint_before(&mut expected, hole, s);
            inpaint(&mut got, hole, s);
            assert!(got.data == expected.data, "{what}");
        };
        let mut img = drawn(&mut seed, 60, 0);
        fill_holes(&mut img);
        // Round holes (things' patches), some at the edges.
        let mut holes = vec![false; s * s];
        for (ci, cj, r) in [(100i64, 100i64, 30i64), (0, 300, 45), (511, 511, 20), (260, 20, 70), (400, 250, 3)] {
            for j in 0..s as i64 {
                for i in 0..s as i64 {
                    if (i - ci).pow(2) + (j - cj).pow(2) <= r * r {
                        holes[j as usize * s + i as usize] = true;
                    }
                }
            }
        }
        check(&img, &holes, "round holes");
        // Nearly all of it a hole (an interior's room), a few pixels of ground left.
        let mut most: Vec<bool> = vec![true; s * s];
        for _ in 0..20 {
            most[next(&mut seed, (s * s) as u32) as usize] = false;
        }
        check(&img, &most, "nearly all");
        // Scattered single pixels, and none.
        let scattered: Vec<bool> = (0..s * s).map(|_| next(&mut seed, 7) == 0).collect();
        check(&img, &scattered, "scattered");
        check(&img, &vec![false; s * s], "none");
    }

    #[test]
    fn the_pick_map_gives_the_same_blocks_as_before() {
        let s = PICTURE as usize;
        let mut seed = 3;
        // Few numbers (ties in many blocks), many numbers, zeros between, and none at all.
        for (numbers, zeros) in [(3u32, 0u32), (3, 2), (255, 1), (2, 50)] {
            let ids: Vec<u8> =
                (0..s * s).map(|_| if next(&mut seed, 100) < zeros { 0 } else { 1 + next(&mut seed, numbers) as u8 }).collect();
            assert_eq!(pick_blocks(&ids, s), pick_blocks_before(&ids, s), "{numbers} numbers");
        }
        let ids = vec![0u8; s * s];
        assert_eq!(pick_blocks(&ids, s), pick_blocks_before(&ids, s), "none");
    }

    #[test]
    fn the_gap_fill_gives_the_pass_by_pass_fill_s_values_bit_for_bit() {
        let s = 96;
        let bits = |v: &[f32]| v.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        let check = |grid: Vec<f32>, what: &str| {
            let (mut expected, mut got) = (grid.clone(), grid);
            fill_gaps_by_passes(&mut expected, s);
            fill_gaps(&mut got, s);
            assert_eq!(bits(&got), bits(&expected), "{what}");
            expected
        };
        // A tile's walkmesh as the import rasterises it: two walkable levels with a cliff (no walkmesh) between them, a raised
        // patch a few pixels wide in the gap, and a hole in the middle of the lower level.
        let px = |v: f32| (v + TILE / 2.0) / TILE * s as f32;
        let q = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| [a, b, c].map(|v| Vec3::new(px(v[0]), px(v[1]), v[2]));
        let walk = [
            q([-5.0, -5.0, 1.0], [-1.0, -5.0, 1.3], [-1.0, 5.0, 0.7]),
            q([-5.0, -5.0, 1.0], [-1.0, 5.0, 0.7], [-5.0, 5.0, 1.1]),
            q([2.0, -5.0, 4.0], [5.0, -5.0, 4.2], [5.0, 5.0, 3.9]),
            q([2.0, -5.0, 4.0], [5.0, 5.0, 3.9], [2.0, 5.0, 4.1]),
            q([0.3, 0.0, 6.0], [0.6, 0.0, 6.0], [0.6, 0.3, 6.0]),
        ];
        let mut z = vec![f32::NAN; s * s];
        for f in walk {
            raster(f, s, |i, j, b| {
                let h = b[0] * f[0].z + b[1] * f[1].z + b[2] * f[2].z;
                if z[j * s + i].is_nan() || h > z[j * s + i] {
                    z[j * s + i] = h;
                }
            });
        }
        for j in 40..50 {
            for i in 10..20 {
                z[j * s + i] = f32::NAN;
            }
        }
        assert!(check(z, "a tile").iter().all(|v| !v.is_nan()));
        // Scattered walkable pixels.
        let mut seed = 7u32;
        let scattered: Vec<f32> = (0..s * s)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                if (seed >> 16).is_multiple_of(50) { ((seed >> 8) % 1000) as f32 / 37.0 } else { f32::NAN }
            })
            .collect();
        check(scattered, "scattered");
        // One walkable pixel in a corner: the fill stops after `s` passes (10 m), and the far corner keeps nothing (the tile's height).
        let mut corner = vec![f32::NAN; s * s];
        corner[0] = 2.5;
        let filled = check(corner, "a corner");
        assert!(filled[s * s - 1].is_nan() && !filled[s - 1].is_nan());
        // No walkmesh at all: nothing to fill from.
        assert!(check(vec![f32::NAN; s * s], "nothing").iter().all(|v| v.is_nan()));
    }
}
