// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! BioWare DXT1 DDS with its whole mip chain, from a picture kept as [`Rgba`] (rows bottom first, as the game stores them). Its
//! own file so `scripts/real-skies.rs` can take it in as well.

use mg_image::Rgba;

/// Half the size (box filter), at least 1 x 1.
pub fn half(img: &Rgba) -> Rgba {
    let (w, h) = ((img.width / 2).max(1), (img.height / 2).max(1));
    let mut out = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = img.pixel((x * 2 + dx).min(img.width - 1), (y * 2 + dy).min(img.height - 1));
                for k in 0..4 {
                    sum[k] += u32::from(p[k]);
                }
            }
            let i = ((y * w + x) * 4) as usize;
            for k in 0..4 {
                out.data[i + k] = ((sum[k] + 2) / 4) as u8;
            }
        }
    }
    out
}

fn pack565(c: [f32; 3]) -> u16 {
    let q = |v: f32, max: f32| (v.clamp(0.0, 255.0) * max / 255.0 + 0.5) as u16;
    (q(c[0], 31.0) << 11) | (q(c[1], 63.0) << 5) | q(c[2], 31.0)
}

fn unpack565(c: u16) -> [f32; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [(f32::from(r) * 255.0 / 31.0), (f32::from(g) * 255.0 / 63.0), (f32::from(b) * 255.0 / 31.0)]
}

/// One DXT1 block (4-colour mode): the colour box's corners, inset a little.
pub(crate) fn dxt1_block(px: &[[f32; 3]; 16]) -> [u8; 8] {
    let (mut lo, mut hi) = ([255f32; 3], [0f32; 3]);
    for p in px {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    // The box diagonal that follows the block's colours: flip channels that run against the brightest one.
    let mean: [f32; 3] = std::array::from_fn(|k| px.iter().map(|p| p[k]).sum::<f32>() / 16.0);
    let mut cov = [0f32; 3];
    let main = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).expect("0..3 is not empty");
    for p in px {
        for k in 0..3 {
            cov[k] += (p[k] - mean[k]) * (p[main] - mean[main]);
        }
    }
    let (mut a, mut b) = (hi, lo);
    for k in 0..3 {
        if cov[k] < 0.0 {
            std::mem::swap(&mut a[k], &mut b[k]);
        }
    }
    for k in 0..3 {
        let inset = (a[k] - b[k]) / 16.0;
        a[k] -= inset;
        b[k] += inset;
    }
    let (mut c0, mut c1) = (pack565(a), pack565(b));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    if c0 == c1 {
        return out; // every index 0
    }
    let (p0, p1) = (unpack565(c0), unpack565(c1));
    let pal: [[f32; 3]; 4] =
        [p0, p1, std::array::from_fn(|k| (2.0 * p0[k] + p1[k]) / 3.0), std::array::from_fn(|k| (p0[k] + 2.0 * p1[k]) / 3.0)];
    let mut bits = 0u32;
    for (i, p) in px.iter().enumerate() {
        let d = |c: &[f32; 3]| (0..3).map(|k| (c[k] - p[k]).powi(2)).sum::<f32>();
        let best = (0..4).min_by(|&x, &y| d(&pal[x]).total_cmp(&d(&pal[y]))).expect("0..4 is not empty") as u32;
        bits |= best << (i * 2);
    }
    out[4..8].copy_from_slice(&bits.to_le_bytes());
    out
}

fn dxt1(img: &Rgba) -> Vec<u8> {
    let (w, h) = (img.width, img.height);
    let mut out = Vec::with_capacity(((w as usize).div_ceil(4)) * ((h as usize).div_ceil(4)) * 8);
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            let px: [[f32; 3]; 16] = std::array::from_fn(|i| {
                let p = img.pixel((bx + i as u32 % 4).min(w - 1), (by + i as u32 / 4).min(h - 1));
                [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])]
            });
            out.extend(dxt1_block(&px));
        }
    }
    out
}

/// A BioWare DDS (3 channels, DXT1) with its whole mip chain.
pub fn dds(img: &Rgba) -> Vec<u8> {
    let top = dxt1(img);
    let mut out = Vec::new();
    out.extend(img.width.to_le_bytes());
    out.extend(img.height.to_le_bytes());
    out.extend(3u32.to_le_bytes());
    out.extend((top.len() as u32).to_le_bytes());
    out.extend(1f32.to_le_bytes());
    out.extend(&top);
    let mut level = img.clone();
    while level.width > 1 || level.height > 1 {
        level = half(&level);
        out.extend(dxt1(&level));
    }
    out
}
