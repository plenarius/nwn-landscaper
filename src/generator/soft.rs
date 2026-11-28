// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Soft paint: where a baked tile's textures meet, drawn from the paint smoothed, not along its 0.625 m grid
//! (docs: the soft paint design, plenarius/nwn-landscaper#88). The editor (nwn-landscaper-web) draws the same edges and must
//! follow this file to the letter: every constant and step its view needs is here.
//!
//! 1. **Weights.** Each palette entry's samples (1 where it is painted, 0 elsewhere, the halo's beyond the area and the nearest
//!    sample's past that) are smoothed by a Gaussian of [`SIGMA`] metres: separable, taps `d` = -R..=R samples with
//!    R = ceil(3 [`SIGMA`] / spacing) + 1, each `exp(-d² / (2 s²))` with s = [`SIGMA`] / spacing, divided by their sum; first
//!    along rows, then columns, in f32.
//! 2. **Two strongest a sample.** Each sample keeps the entry weighing most (ties: the lower palette index), the second, and the
//!    second's share of the two, `round(255 w2 / (w1 + w2))` as a byte; a share of 0 keeps the first alone. This is [`Soft::top`],
//!    and what the editor uploads for its view. A sample with a blend layer (a second texture and its weight `w` > 0, soft
//!    paint's stage 2: weights from the brush) takes its own instead: `[paint, second, w]`, or `[second, paint, 255 - w]` when
//!    `w` > 128, `[t, t, 0]` when the two are one texture ([`layered`]). The smoothing still reads the paint alone.
//! 3. **A point's weights.** The four samples around it, bilinear: each adds its weight times (1 - share) to its first and its
//!    weight times share to its second. Entries weighing [`MIN_WEIGHT`] or less are dropped.
//! 4. **Edge style.** The softest of the entries left in the catalog (`edge`): soft when any is soft, else broken when any is
//!    broken, else crisp. A soft point's colour is its entries' colours weighed by their weights (no scores).
//! 5. **Scores.** Crisp: an entry's weight. Broken: its weight, plus [`NOISE`] × [`fbm`](x / [`NOISE_SCALE`], y / [`NOISE_SCALE`],
//!    seed), plus [`LUM`] × (luminance of its colour there − its picture's mean luminance) × [`toward`] its partner; seed:
//!    [`seed`] of its texture id; luminance: 0.3 R + 0.59 G + 0.11 B ([`luminance`]). The heaviest entry's partner is the
//!    second heaviest, everyone else's the heaviest. [`toward`] is the two mean luminances' difference over [`CONTRAST`], at
//!    most 1, positive for the darker of the two (on a tie, 0): the darker texture's light pixels reach into the lighter one
//!    (gravel's stones into snow) and the lighter one's dark pixels reach back. Two textures alike in brightness (two grasses)
//!    get little of it: picking by brightness there draws a pale or dark outline along the edge.
//! 6. **Colour.** Each entry's colour times `exp((score − best score) / tau)`, divided by their sum; tau [`TAU_CRISP`] or
//!    [`TAU_BROKEN`] (in weight: a few centimetres of ground at a usual edge).
//!
//! The bake leaves out entries whose score can't come within [`CUTOFF`] tau of the best (it doesn't read their colour); their
//! share would be under e^-12.

use crate::heightmap::Heightmap;
use crate::paint::Edge;

/// The smoothing's Gaussian sigma (m).
pub const SIGMA: f32 = 0.6;
/// Score easing of a crisp edge.
pub const TAU_CRISP: f32 = 0.03;
/// Score easing of a broken edge.
pub const TAU_BROKEN: f32 = 0.04;
/// How far noise moves a broken edge's scores.
pub const NOISE: f32 = 0.16;
/// The noise's larger features are about this many metres across.
pub const NOISE_SCALE: f32 = 1.5;
/// How far a texture's own light and dark move a broken edge's scores.
pub const LUM: f32 = 0.6;
/// The difference in mean luminance at which [`LUM`] counts in full.
pub const CONTRAST: f32 = 0.25;
/// Entries of a point weighing this or less are dropped.
pub const MIN_WEIGHT: f32 = 0.02;
/// Entries scoring more than this many tau under the best are left out.
const CUTOFF: f32 = 12.0;

/// A point's entries: up to the four samples' two each.
pub(super) type Mix = ([(u8, f32); 8], usize);

/// The paint's smoothed weights, two a sample, over the area and its halo.
pub(super) struct Soft {
    spacing: f32,
    /// The grid's first sample (below 0 with a halo) and its size.
    i0: i64,
    j0: i64,
    w: usize,
    h: usize,
    /// Per sample, rows from the south: [first, second, the second's share (0 to 255)].
    top: Vec<[u8; 3]>,
}

impl Soft {
    /// The weights of `hm`'s paint.
    pub(super) fn new(hm: &Heightmap) -> Soft {
        let sp = hm.spacing;
        let r = (hm.reach() / sp).round() as i64;
        let (i0, j0) = (-r, -r);
        let (w, h) = ((hm.width as i64 + 2 * r) as usize, (hm.height as i64 + 2 * r) as usize);
        let at = |k: usize| (i0 + (k % w) as i64, j0 + (k / w) as i64);
        let mats: Vec<u8> = (0..w * h).map(|k| hm.material(at(k).0, at(k).1)).collect();
        let mut top = top_two(&mats, w, h, sp);
        if !hm.blend.is_empty() {
            let blend: Vec<[u8; 2]> = (0..w * h).map(|k| hm.blend(at(k).0, at(k).1)).collect();
            own_weights(&mut top, &mats, &blend);
        }
        Soft { spacing: sp, i0, j0, w, h, top }
    }

    /// Sample (i, j)'s [first, second, share], clamped to the grid.
    pub(super) fn sample(&self, i: i64, j: i64) -> [u8; 3] {
        let x = (i - self.i0).clamp(0, self.w as i64 - 1) as usize;
        let y = (j - self.j0).clamp(0, self.h as i64 - 1) as usize;
        self.top[y * self.w + x]
    }

    /// The entries at (x, y) weighing more than [`MIN_WEIGHT`], heaviest first.
    pub(super) fn mix(&self, x: f32, y: f32) -> Mix {
        let fx = (x / self.spacing - self.i0 as f32).clamp(0.0, (self.w - 1) as f32);
        let fy = (y / self.spacing - self.j0 as f32).clamp(0.0, (self.h - 1) as f32);
        let (ix, iy) = ((fx.floor() as usize).min(self.w - 2), (fy.floor() as usize).min(self.h - 2));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let mut out = ([(0u8, 0f32); 8], 0usize);
        let mut add = |m: u8, v: f32| {
            if v <= 0.0 {
                return;
            }
            match out.0[..out.1].iter_mut().find(|e| e.0 == m) {
                Some(e) => e.1 += v,
                None => {
                    out.0[out.1] = (m, v);
                    out.1 += 1;
                }
            }
        };
        for (i, j, b) in
            [(ix, iy, (1.0 - tx) * (1.0 - ty)), (ix + 1, iy, tx * (1.0 - ty)), (ix, iy + 1, (1.0 - tx) * ty), (ix + 1, iy + 1, tx * ty)]
        {
            let [a, s, q] = self.top[j * self.w + i];
            let share = f32::from(q) / 255.0;
            add(a, b * (1.0 - share));
            add(s, b * share);
        }
        // (Dropped, then heaviest first; ties by palette index, so the order never depends on the samples' order.)
        let n = out.1;
        let mut kept = 0;
        for k in 0..n {
            if out.0[k].1 > MIN_WEIGHT {
                out.0[kept] = out.0[k];
                kept += 1;
            }
        }
        out.0[..kept].sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out.1 = kept;
        out
    }
}

/// The samples with a blend layer take their own weights in place of the smoothing's.
fn own_weights(top: &mut [[u8; 3]], mats: &[u8], blend: &[[u8; 2]]) {
    for ((t, &m), &b) in top.iter_mut().zip(mats).zip(blend) {
        if let Some(own) = layered(m, b) {
            *t = own;
        }
    }
}

/// A sample's own [first, second, share] from its paint and blend layer ([second, weight]); None without a second texture.
/// (Heaviest first, as the smoothing's; the editor writes the heavier in the paint, so it's mostly as stored.)
pub(super) fn layered(paint: u8, [second, w]: [u8; 2]) -> Option<[u8; 3]> {
    Some(match w {
        0 => return None,
        _ if second == paint => [paint, paint, 0],
        1..=128 => [paint, second, w],
        _ => [second, paint, 255 - w],
    })
}

/// What [`colour`] needs of each palette entry.
pub(super) struct Look {
    pub edge: Edge,
    /// [`seed`] of its id.
    pub seed: u32,
    /// Its picture's mean luminance.
    pub mean: f32,
    /// How far any of its pixels' luminance is from the mean (bounds the [`LUM`] term).
    pub range: f32,
}

/// The ground's colour where `mix` was read at (x, y): `sample(m)` is entry m's colour there (read only when it can show).
pub(super) fn colour(mix: &Mix, x: f32, y: f32, looks: &[Look], sample: impl Fn(u8) -> [f32; 3]) -> [f32; 3] {
    let entries = &mix.0[..mix.1];
    match entries {
        [] => return [0.0; 3],
        [(m, _)] => return sample(*m),
        _ => {}
    }
    let edge = entries.iter().map(|(m, _)| looks[*m as usize].edge).max().unwrap_or(Edge::Crisp);
    if edge == Edge::Soft {
        let (mut out, mut total) = ([0.0; 3], 0.0);
        for &(m, w) in entries {
            let c = sample(m);
            total += w;
            for k in 0..3 {
                out[k] += c[k] * w;
            }
        }
        return out.map(|v| v / total);
    }
    let broken = edge == Edge::Broken;
    let tau = if broken { TAU_BROKEN } else { TAU_CRISP };
    let noise = |m: u8| if broken { NOISE * fbm(x / NOISE_SCALE, y / NOISE_SCALE, looks[m as usize].seed) } else { 0.0 };
    let (first, second) = (entries[0].0, entries[1].0);
    let partner = |m: u8| if m == first { second } else { first };
    // (Heaviest first, so the best score is usually known before the rest are judged.)
    let mut read: [([f32; 3], f32); 8] = [([0.0; 3], 0.0); 8];
    let (mut n, mut best) = (0, f32::MIN);
    for &(m, w) in entries {
        let l = &looks[m as usize];
        let base = w + noise(m);
        if broken && base + LUM * l.range < best - CUTOFF * tau || !broken && base < best - CUTOFF * tau {
            continue;
        }
        let c = sample(m);
        let s = if broken { base + LUM * (luminance(c) - l.mean) * toward(looks, m, partner(m)) } else { base };
        best = best.max(s);
        read[n] = (c, s);
        n += 1;
    }
    let (mut out, mut total) = ([0.0; 3], 0.0);
    for (c, s) in &read[..n] {
        let e = ((s - best) / tau).exp();
        total += e;
        for k in 0..3 {
            out[k] += c[k] * e;
        }
    }
    out.map(|v| v / total)
}

/// How far entry `m`'s own light (positive) or dark (negative) pixels reach into `other`'s ground: towards the other's look, by
/// how different the two are in brightness (-1 to 1).
pub(super) fn toward(looks: &[Look], m: u8, other: u8) -> f32 {
    ((looks[other as usize].mean - looks[m as usize].mean) / CONTRAST).clamp(-1.0, 1.0)
}

/// Each sample's two heaviest entries of the smoothed paint `mats` (`w` x `h`, rows from the south) and the second's share.
fn top_two(mats: &[u8], w: usize, h: usize, spacing: f32) -> Vec<[u8; 3]> {
    let s = SIGMA / spacing;
    let r = (3.0 * s).ceil() as i64 + 1;
    let kernel: Vec<f32> = {
        let k: Vec<f32> = (-r..=r).map(|d| (-(d * d) as f32 / (2.0 * s * s)).exp()).collect();
        let t: f32 = k.iter().sum();
        k.into_iter().map(|v| v / t).collect()
    };
    let mut used = [false; 256];
    for &m in mats {
        used[m as usize] = true;
    }
    // (First and second, with their weights.)
    let mut best: Vec<(u8, f32, u8, f32)> = vec![(0, -1.0, 0, -1.0); w * h];
    let (mut row, mut field) = (vec![0f32; w * h], vec![0f32; w * h]);
    for p in (0..256).filter(|&p| used[p]) {
        let p = p as u8;
        for y in 0..h {
            for x in 0..w {
                let mut a = 0.0;
                for (k, kv) in kernel.iter().enumerate() {
                    let xx = (x as i64 + k as i64 - r).clamp(0, w as i64 - 1) as usize;
                    if mats[y * w + xx] == p {
                        a += kv;
                    }
                }
                row[y * w + x] = a;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut a = 0.0;
                for (k, kv) in kernel.iter().enumerate() {
                    let yy = (y as i64 + k as i64 - r).clamp(0, h as i64 - 1) as usize;
                    a += row[yy * w + x] * kv;
                }
                field[y * w + x] = a;
            }
        }
        for (b, &v) in best.iter_mut().zip(&field) {
            if v > b.1 {
                *b = (p, v, b.0, b.1);
            } else if v > b.3 {
                (b.2, b.3) = (p, v);
            }
        }
    }
    best.into_iter()
        .map(|(a, wa, b, wb)| {
            let q = if wb > 0.0 { (255.0 * wb / (wa + wb)).round() as u8 } else { 0 };
            if q == 0 { [a, a, 0] } else { [a, b, q] }
        })
        .collect()
}

/// Luminance of a colour (0..1).
pub fn luminance(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// A texture's noise seed: FNV-1a (32 bit) of its id's bytes, so its edge looks the same whatever its place in the palette.
pub fn seed(id: &str) -> u32 {
    id.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// A lattice point's value in -1..1. (32-bit wrapping arithmetic: in JavaScript, `Math.imul` and `>>> 0`.)
fn hash(x: i32, y: i32, s: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ s.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0 * 2.0 - 1.0
}

/// Value noise: the lattice's values about (x, y), eased by smoothstep.
fn value(x: f32, y: f32, s: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash(xi, yi, s) + (hash(xi + 1, yi, s) - hash(xi, yi, s)) * tx;
    let b = hash(xi, yi + 1, s) + (hash(xi + 1, yi + 1, s) - hash(xi, yi + 1, s)) * tx;
    a + (b - a) * ty
}

/// Noise in about -1..1: features about 1 unit across and a third of that.
pub fn fbm(x: f32, y: f32, s: u32) -> f32 {
    0.65 * value(x, y, s) + 0.35 * value(x * 3.1 + 17.0, y * 3.1 - 5.0, s.wrapping_add(101))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soft(mats: Vec<u8>, w: usize, h: usize) -> Soft {
        Soft { spacing: 0.625, i0: 0, j0: 0, w, h, top: top_two(&mats, w, h, 0.625) }
    }

    fn looks(edge: Edge) -> Vec<Look> {
        (0..4).map(|m| Look { edge, seed: m, mean: 0.5, range: 0.0 }).collect()
    }

    #[test]
    fn one_texture_far_from_another_is_alone() {
        // Texture 1 in the east half of 40 x 8 samples.
        let s = soft((0..320).map(|k| u8::from(k % 40 >= 20)).collect(), 40, 8);
        assert_eq!(s.sample(2, 4), [0, 0, 0]);
        assert_eq!(s.sample(37, 4), [1, 1, 0]);
        // Next to the edge: both, the nearer side first, the shares rising towards it.
        let [a, b, q1] = s.sample(18, 4);
        assert_eq!((a, b), (0, 1));
        let [_, _, q2] = s.sample(19, 4);
        assert!(q1 < q2 && q2 < 128, "{q1} {q2}");
        let m = s.mix(5.0, 2.5);
        assert_eq!(m.1, 1);
    }

    #[test]
    fn a_staircase_s_edge_is_smoothed_into_a_line() {
        // A diagonal staircase of single samples (the sawtooth): the 50% line of the bilinear weights stays within a few
        // centimetres of the diagonal, where the sample grid's own edge strays 0.3 m from it.
        let n = 40;
        let s = soft((0..n * n).map(|k| u8::from(k % n > k / n)).collect(), n, n);
        let mut worst = 0f32;
        for k in 0..200 {
            // Points along the diagonal's normal through (t, t), find where texture 1 starts winning.
            let t = 8.0 + k as f32 * 0.06;
            let at = |d: f32| {
                let m = s.mix(t + d, t - d);
                m.0[..m.1].iter().find(|e| e.0 == 1).map_or(0.0, |e| e.1)
            };
            let mut d = -0.6;
            while at(d) < 0.5 && d < 0.6 {
                d += 0.002;
            }
            // (By symmetry the edge lies half a sample east of the diagonal: x - y = 0.3125 m, so d = 0.156 m.)
            worst = worst.max((d - 0.15625).abs());
        }
        assert!(worst < 0.02, "the smoothed edge strays {worst} m");
    }

    #[test]
    fn a_crisp_edge_is_sharp_and_a_broken_one_moves() {
        let s = soft((0..40 * 40).map(|k| u8::from(k % 40 >= 20)).collect(), 40, 40);
        let paint = |m: u8| if m == 0 { [0.0; 3] } else { [1.0; 3] };
        let crisp = looks(Edge::Crisp);
        // Across the crisp edge (half way between samples 19 and 20, 12.19 m): from black to white within 15 cm.
        let row: Vec<f32> =
            (0..400).map(|k| colour(&s.mix(10.0 + k as f32 * 0.01, 12.0), 10.0 + k as f32 * 0.01, 12.0, &crisp, paint)[0]).collect();
        let first = row.iter().position(|v| *v > 0.05).expect("white starts");
        let last = row.iter().position(|v| *v > 0.95).expect("white wins");
        assert!(last - first <= 15, "{first}..{last}");
        // Broken: the edge's place differs along it, by tens of centimetres.
        let broken = looks(Edge::Broken);
        let edge_at =
            |y: f32| (0..400).position(|k| colour(&s.mix(10.0 + k as f32 * 0.01, y), 10.0 + k as f32 * 0.01, y, &broken, paint)[0] > 0.5);
        let places: Vec<usize> = (0..40).filter_map(|k| edge_at(4.0 + k as f32 * 0.4)).collect();
        let (lo, hi) = (places.iter().min().unwrap(), places.iter().max().unwrap());
        assert!(hi - lo >= 15, "{lo}..{hi}");
    }

    #[test]
    fn a_crisp_texture_beside_a_broken_one_is_broken() {
        let s = soft((0..40 * 8).map(|k| u8::from(k % 40 >= 20)).collect(), 40, 8);
        let mut l = looks(Edge::Crisp);
        l[1].edge = Edge::Broken;
        let paint = |m: u8| if m == 0 { [0.0; 3] } else { [1.0; 3] };
        // At the smoothed edge's middle, noise decides: not always the 50:50 a crisp pair gives there.
        let x = 20.0 * 0.625 - 0.3125;
        let c: Vec<f32> = (0..20).map(|k| colour(&s.mix(x, k as f32 * 0.2), x, k as f32 * 0.2, &l, paint)[0]).collect();
        assert!(c.iter().any(|v| (v - 0.5).abs() > 0.3), "{c:?}");
    }

    #[test]
    fn the_darker_texture_s_light_pixels_reach_into_the_lighter_and_alike_ones_hardly() {
        let mut l = looks(Edge::Broken);
        (l[0].mean, l[1].mean, l[2].mean) = (0.25, 0.85, 0.28);
        // Gravel and snow: gravel's light pixels, snow's dark ones, in full.
        assert_eq!((toward(&l, 0, 1), toward(&l, 1, 0)), (1.0, -1.0));
        // Two grasses: a little, so the edge draws no pale or dark outline.
        assert!((toward(&l, 0, 2) - 0.12).abs() < 1e-6 && (toward(&l, 2, 0) + 0.12).abs() < 1e-6);
    }

    /// The blend layer a round soft brush of texture `t` lays along the line through `a` and `b` (m) over `mats` (`w` wide):
    /// texture t's share falls from 1 within `core` of the line to 0 at `core + feather`. The heavier texture goes in the paint,
    /// fully covered samples are pinned ([t, t, 255]), as the editor's brush writes them.
    fn brush(mats: &mut [u8], w: usize, t: u8, a: [f32; 2], b: [f32; 2], core: f32, feather: f32) -> Vec<[u8; 2]> {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let mut blend = vec![[0u8; 2]; mats.len()];
        for (k, m) in mats.iter_mut().enumerate() {
            let (x, y) = ((k % w) as f32 * 0.625, (k / w) as f32 * 0.625);
            let u = (((x - a[0]) * dx + (y - a[1]) * dy) / len2).clamp(0.0, 1.0);
            let d = ((x - a[0] - u * dx).powi(2) + (y - a[1] - u * dy).powi(2)).sqrt();
            let share = ((core + feather - d) / feather).clamp(0.0, 1.0);
            let q = (share * 255.0).round() as u8;
            if q == 0 {
                continue;
            }
            let other = *m;
            blend[k] = if other == t || q == 255 {
                *m = t;
                [t, 255]
            } else if q >= 128 {
                *m = t;
                [other, 255 - q]
            } else {
                [t, q]
            };
        }
        blend
    }

    #[test]
    fn a_sample_s_own_weights_heaviest_first() {
        assert_eq!(layered(3, [5, 0]), None);
        assert_eq!(layered(3, [5, 40]), Some([3, 5, 40]));
        assert_eq!(layered(3, [5, 128]), Some([3, 5, 128]));
        assert_eq!(layered(3, [5, 129]), Some([5, 3, 126]));
        assert_eq!(layered(3, [5, 255]), Some([5, 3, 0]));
        // (Pinned: the paint alone, never smoothed.)
        assert_eq!(layered(3, [3, 255]), Some([3, 3, 0]));
    }

    #[test]
    fn weights_of_zero_change_nothing() {
        let mats: Vec<u8> = (0..40 * 40).map(|k| u8::from(k % 40 > k / 40)).collect();
        let plain = top_two(&mats, 40, 40, 0.625);
        let mut top = plain.clone();
        own_weights(&mut top, &mats, &(0..40 * 40).map(|k| [(k % 3) as u8, 0]).collect::<Vec<_>>());
        assert_eq!(top, plain);
    }

    #[test]
    fn weights_place_a_crisp_edge_where_they_say() {
        // Texture 1 east of 12.4 m: a ramp of weights 1.25 m wide about it, where the paint's own grid edge lies at 12.19 m.
        let (w, h, at) = (40, 8, 12.4f32);
        let mut mats: Vec<u8> = (0..w * h).map(|k| u8::from((k % w) as f32 * 0.625 >= at)).collect();
        let mut blend = vec![[0u8; 2]; w * h];
        for k in 0..w * h {
            let x = (k % w) as f32 * 0.625;
            let q = ((0.5 + (x - at) / 1.25).clamp(0.0, 1.0) * 255.0).round() as u8;
            if q > 0 && q < 255 {
                (mats[k], blend[k]) = if q > 127 { (1, [0, 255 - q]) } else { (0, [1, q]) };
            }
        }
        let mut s = soft(mats.clone(), w, h);
        own_weights(&mut s.top, &mats, &blend);
        let paint = |m: u8| if m == 0 { [0.0; 3] } else { [1.0; 3] };
        let crisp = looks(Edge::Crisp);
        for y in [1.0, 2.2, 3.1] {
            let row: Vec<f32> = (0..600).map(|k| 10.0 + k as f32 * 0.005).collect();
            let c: Vec<f32> = row.iter().map(|&x| colour(&s.mix(x, y), x, y, &crisp, paint)[0]).collect();
            let half = row[c.iter().position(|v| *v > 0.5).expect("an edge")];
            assert!((half - at).abs() < 0.01, "the edge at {half} m, not {at}");
            // Sharp: black to white within 15 cm, as a crisp edge from the smoothing.
            let (first, last) = (c.iter().position(|v| *v > 0.05).unwrap(), c.iter().position(|v| *v > 0.95).unwrap());
            assert!((last - first) as f32 * 0.005 <= 0.15, "{first}..{last}");
        }
        // Without the weights the edge stays on the paint's grid.
        let plain = soft(mats, w, h);
        let x = (0..600).map(|k| 10.0 + k as f32 * 0.005).find(|&x| colour(&plain.mix(x, 2.2), x, 2.2, &crisp, paint)[0] > 0.5).unwrap();
        assert!((x - 12.1875).abs() < 0.03, "{x}");
    }

    #[test]
    fn a_soft_texture_blends_by_its_weights_and_wins() {
        let s = soft((0..40 * 8).map(|k| u8::from(k % 40 >= 20)).collect(), 40, 8);
        let mut l = looks(Edge::Broken);
        l[1].edge = Edge::Soft;
        let paint = |m: u8| if m == 0 { [0.0; 3] } else { [1.0; 3] };
        for k in 0..100 {
            let x = 10.0 + k as f32 * 0.04;
            let m = s.mix(x, 2.5);
            let want: f32 = m.0[..m.1].iter().filter(|e| e.0 == 1).map(|e| e.1).sum::<f32>() / m.0[..m.1].iter().map(|e| e.1).sum::<f32>();
            assert!((colour(&m, x, 2.5, &l, paint)[0] - want).abs() < 1e-5);
        }
    }

    /// Writes the editor's parity fixture (nwn-landscaper-web `tests/fixtures/softpaint-bake.json`) to `$SOFTPAINT_FIXTURE`:
    /// `cargo test --lib write_editor_fixture -- --ignored`.
    #[test]
    #[ignore = "writes the editor's fixture"]
    fn write_editor_fixture() {
        use serde_json::json;
        let path = std::env::var("SOFTPAINT_FIXTURE").expect("SOFTPAINT_FIXTURE: where to write it");
        let (nx, ny, sp) = (28usize, 22usize, 0.625f32);
        let ids = ["snowPacked", "gravelGrey", "cobbleGrey"];
        let base = [[0.86f32, 0.89, 0.93], [0.46, 0.41, 0.36], [0.53, 0.52, 0.5]];
        // A staircase trail of gravel, a block of cobbles and a line of them one sample wide.
        let mut mats: Vec<u8> = (0..nx * ny)
            .map(|k| {
                let (i, j) = (k % nx, k / nx);
                if (i + 3 == j * 2 || i + 2 == j * 2) && j < 14 {
                    1
                } else if (i >= 20 && j <= 6) || (j == 17 && (4..14).contains(&i)) {
                    2
                } else {
                    0
                }
            })
            .collect();
        // A soft brush's trail of gravel across them, as stage 2 stores it.
        let blend = brush(&mut mats, nx, 1, [2.0, 1.0], [17.0, 7.0], 0.6, 0.9);
        let mut top = top_two(&mats, nx, ny, sp);
        own_weights(&mut top, &mats, &blend);
        let s = Soft { spacing: sp, i0: 0, j0: 0, w: nx, h: ny, top: top.clone() };
        let looks = |edges: [Edge; 3]| -> Vec<Look> {
            (0..3).map(|m| Look { edge: edges[m], seed: seed(ids[m]), mean: luminance(base[m]), range: 0.12 }).collect()
        };
        let colour_at = |x: f32, y: f32| {
            move |m: u8| {
                let r = 0.12 * (x * 5.3 + y * 3.7 + f32::from(m)).sin() * (y * 4.1 - x * 2.3).cos();
                base[m as usize].map(|v| v + r)
            }
        };
        let round = |v: f32| (v * 1e5).round() / 1e5;
        let style = |edges: [Edge; 3]| {
            let l = looks(edges);
            let mut points = Vec::new();
            for j in 0..28 {
                for i in 0..56 {
                    let (x, y) = (round(i as f32 * 0.31), round(j as f32 * 0.47));
                    let c = colour(&s.mix(x, y), x, y, &l, colour_at(x, y));
                    points.push(json!([x, y, round(c[0]), round(c[1]), round(c[2])]));
                }
            }
            json!({ "edges": edges.map(Edge::name), "points": points })
        };
        let fbm_points: Vec<_> = (0..200)
            .map(|k| {
                let (x, y, sd) = (-20.0 + k as f32 * 0.37, 30.0 * (k as f32 * 0.7).sin(), seed(ids[k % 3]).wrapping_add(k as u32));
                json!([round(x), y, sd, fbm(round(x), y, sd)])
            })
            .collect();
        let out = json!({
            "source": "nwn-landscaper src/generator/soft.rs write_editor_fixture (soft paint stage 2: the blend layer)",
            "nx": nx, "ny": ny, "spacing": sp, "ids": ids,
            "materials": mats,
            "blend": blend.as_flattened(),
            "top": top.as_flattened(),
            "base": base, "means": base.map(luminance).map(round), "seeds": ids.map(seed),
            "fbm": fbm_points,
            "crisp": style([Edge::Crisp; 3]),
            "broken": style([Edge::Broken; 3]),
            "mixed": style([Edge::Broken, Edge::Broken, Edge::Crisp]),
            "soft": style([Edge::Broken, Edge::Soft, Edge::Crisp]),
        });
        std::fs::write(path, serde_json::to_string(&out).unwrap()).unwrap();
    }

    #[test]
    fn the_noise_matches_the_editor_s() {
        // Values the editor's port must give (nwn-landscaper-web keeps the same numbers in its test).
        assert_eq!(seed("grassMedium"), 0xad9f_7dfe);
        let v = [fbm(0.0, 0.0, 1), fbm(1.37, -2.5, seed("dirt")), fbm(213.3, 97.1, seed("snowFresh"))];
        for (a, b) in v.iter().zip([0.216_716_23, 0.260_154_55, 0.051_045_932]) {
            assert!((a - b).abs() < 1e-6, "{v:?}");
        }
    }
}
