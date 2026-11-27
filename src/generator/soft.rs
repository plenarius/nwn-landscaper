// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! SPIKE (not for merge): soft paint at bake time, from today's one-texture-a-sample paint. Each texture's samples (1 where it is
//! painted, 0 elsewhere) are smoothed with a Gaussian of `sigma` metres into a weight field, read bilinearly at each pixel, so an
//! edge no longer steps along the 0.625 m grid. LS_SOFT=<mode>:<sigma>:
//!
//! - `blend`: the smoothed weights as they are: a soft blend about 2.5 sigma wide.
//! - `crisp`: the texture weighing most wins, its edge eased over a few centimetres (LS_SOFT_TAU): a sharp edge on a smooth line,
//!   what a finer mask (or weights) would give.
//! - `broken`: `crisp` with each texture's weight moved by noise (LS_SOFT_NOISE, about 1.5 m features) and by the texture's own
//!   brightness there (LS_SOFT_LUM, a "height blend": gravel's stones poke into the snow, the snow fills between them).

use crate::heightmap::Heightmap;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Blend,
    Crisp,
    Broken,
}

pub(super) struct Soft {
    mode: Mode,
    sigma: f32,
    tau: f32,
    noise: f32,
    lum: f32,
    spacing: f32,
    /// The fields' first sample (may be below 0: the halo and a margin), and their size.
    i0: i64,
    j0: i64,
    w: usize,
    h: usize,
    /// Per palette index: its smoothed weight at each sample (empty when unused).
    fields: Vec<Vec<f32>>,
    /// Per palette index: its mean brightness (for `broken`).
    pub(super) means: Vec<f32>,
}

fn env_f(k: &str, d: f32) -> f32 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

impl Soft {
    pub(super) fn from_env(hm: &Heightmap) -> Option<Soft> {
        let v = std::env::var("LS_SOFT").ok()?;
        let mut it = v.split(':');
        let mode = match it.next()? {
            "blend" => Mode::Blend,
            "crisp" => Mode::Crisp,
            "broken" => Mode::Broken,
            _ => return None,
        };
        let sigma: f32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.6);
        let sp = hm.spacing;
        let r = (hm.reach() / sp).round() as i64;
        let m = (3.0 * sigma / sp).ceil() as i64 + 1;
        let (i0, j0) = (-r - m, -r - m);
        let (w, h) = ((hm.width as i64 + 2 * (r + m)) as usize, (hm.height as i64 + 2 * (r + m)) as usize);
        let n = hm.palette.len();
        let mut fields = vec![Vec::new(); n];
        let mut mats = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                mats[y * w + x] = hm.material(i0 + x as i64, j0 + y as i64);
            }
        }
        let kernel: Vec<f32> = {
            let s = sigma / sp;
            let k: Vec<f32> = (-m..=m).map(|d| (-(d as f32).powi(2) / (2.0 * s * s)).exp()).collect();
            let t: f32 = k.iter().sum();
            k.into_iter().map(|v| v / t).collect()
        };
        for (p, f) in fields.iter_mut().enumerate() {
            if !mats.contains(&(p as u8)) {
                continue;
            }
            let one: Vec<f32> = mats.iter().map(|&q| f32::from(q == p as u8)).collect();
            let mut tmp = vec![0.0; w * h];
            for y in 0..h {
                for x in 0..w {
                    let mut a = 0.0;
                    for (k, kv) in kernel.iter().enumerate() {
                        let xx = (x as i64 + k as i64 - m).clamp(0, w as i64 - 1) as usize;
                        a += one[y * w + xx] * kv;
                    }
                    tmp[y * w + x] = a;
                }
            }
            let mut out = vec![0.0; w * h];
            for y in 0..h {
                for x in 0..w {
                    let mut a = 0.0;
                    for (k, kv) in kernel.iter().enumerate() {
                        let yy = (y as i64 + k as i64 - m).clamp(0, h as i64 - 1) as usize;
                        a += tmp[yy * w + x] * kv;
                    }
                    out[y * w + x] = a;
                }
            }
            *f = out;
        }
        // Spike: weights as a brush would store them (LS_SOFT_W: f32 per palette entry per area sample, rows from the south),
        // in place of the smoothed paint.
        if let Ok(path) = std::env::var("LS_SOFT_W") {
            let raw = std::fs::read(&path).ok()?;
            let vals: Vec<f32> = raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            let (aw, ah) = (hm.width, hm.height);
            assert_eq!(vals.len(), n * aw * ah, "LS_SOFT_W size");
            for (p, f) in fields.iter_mut().enumerate() {
                let src = &vals[p * aw * ah..][..aw * ah];
                if src.iter().all(|v| *v == 0.0) {
                    f.clear();
                    continue;
                }
                *f = vec![0.0; w * h];
                for y in 0..h {
                    for x in 0..w {
                        let ax = (i0 + x as i64).clamp(0, aw as i64 - 1) as usize;
                        let ay = (j0 + y as i64).clamp(0, ah as i64 - 1) as usize;
                        f[y * w + x] = src[ay * aw + ax];
                    }
                }
            }
        }
        let (tau, noise, lum) = match mode {
            Mode::Blend => (0.0, 0.0, 0.0),
            Mode::Crisp => (env_f("LS_SOFT_TAU", 0.03), 0.0, 0.0),
            Mode::Broken => (env_f("LS_SOFT_TAU", 0.04), env_f("LS_SOFT_NOISE", 0.16), env_f("LS_SOFT_LUM", 0.6)),
        };
        eprintln!("soft paint spike: {v} (tau {tau}, noise {noise}, lum {lum}), fields {w}x{h}");
        Some(Soft { mode, sigma, tau, noise, lum, spacing: sp, i0, j0, w, h, fields, means: Vec::new() })
    }

    /// Samples beyond a tile whose paint can reach into it.
    pub(super) fn margin(&self) -> i64 {
        (2.5 * self.sigma / self.spacing).ceil() as i64
    }

    fn weights(&self, x: f32, y: f32) -> Vec<(u8, f32)> {
        let fx = (x / self.spacing - self.i0 as f32).clamp(0.0, (self.w - 2) as f32);
        let fy = (y / self.spacing - self.j0 as f32).clamp(0.0, (self.h - 2) as f32);
        let (ix, iy) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let k = iy * self.w + ix;
        let mut out = Vec::with_capacity(4);
        for (m, f) in self.fields.iter().enumerate() {
            if f.is_empty() {
                continue;
            }
            let v = (f[k] * (1.0 - tx) + f[k + 1] * tx) * (1.0 - ty) + (f[k + self.w] * (1.0 - tx) + f[k + self.w + 1] * tx) * ty;
            if v > 0.003 {
                out.push((m as u8, v));
            }
        }
        out
    }

    /// The ground's colour at (x, y), `sample(m)` the colour of texture m there.
    pub(super) fn colour(&self, x: f32, y: f32, sample: impl Fn(u8) -> [f32; 3]) -> [f32; 3] {
        let ws = self.weights(x, y);
        if ws.len() == 1 {
            return sample(ws[0].0);
        }
        let cols: Vec<[f32; 3]> = ws.iter().map(|(m, _)| sample(*m)).collect();
        let score: Vec<f32> = match self.mode {
            Mode::Blend => ws.iter().map(|w| w.1).collect(),
            Mode::Crisp | Mode::Broken => {
                let s: Vec<f32> = ws
                    .iter()
                    .zip(&cols)
                    .map(|((m, w), c)| {
                        let mut s = *w;
                        if self.mode == Mode::Broken && *w > 0.02 {
                            s += self.noise * fbm(x / 1.5, y / 1.5, u32::from(*m));
                            let l = 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
                            s += self.lum * (l - self.means.get(*m as usize).copied().unwrap_or(l));
                        }
                        s
                    })
                    .collect();
                let top = s.iter().copied().fold(f32::MIN, f32::max);
                s.iter().map(|v| ((v - top) / self.tau).exp()).collect()
            }
        };
        let total: f32 = score.iter().sum();
        let mut c = [0.0; 3];
        for (col, s) in cols.iter().zip(&score) {
            for k in 0..3 {
                c[k] += col[k] * s / total;
            }
        }
        c
    }
}

fn hash(x: i32, y: i32, s: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ s.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0 * 2.0 - 1.0
}

fn value(x: f32, y: f32, s: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash(xi, yi, s) + (hash(xi + 1, yi, s) - hash(xi, yi, s)) * tx;
    let b = hash(xi, yi + 1, s) + (hash(xi + 1, yi + 1, s) - hash(xi, yi + 1, s)) * tx;
    a + (b - a) * ty
}

/// Noise in about -1..1, features about 1 unit across and a third of that.
fn fbm(x: f32, y: f32, s: u32) -> f32 {
    0.65 * value(x, y, s) + 0.35 * value(x * 3.1 + 17.0, y * 3.1 - 5.0, s + 101)
}
