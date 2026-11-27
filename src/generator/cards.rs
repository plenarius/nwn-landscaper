// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Trees as cards in the edge scenery: a tree standing [`CARD_FROM`] or more past the area's edge is drawn as two crossed upright
//! planes and a flat one on top, each showing a picture of the tree's own model, instead of the model's few hundred triangles.
//! The pictures of one model are one atlas, the same for every tree of it (turned and scaled with the tree), so it is made once a
//! model.
//!
//! The atlas ([`atlas`]) is drawn here, on the CPU, from the model's triangles and textures: each side seen straight on and the
//! top straight down, orthographic, lit from above so it suits every turn of the tree. Its alpha is on or off at every mip level
//! and each level keeps the leaves' coverage: the game blends half alpha instead of cutting it, and a card with a soft edge read as a
//! ghost next to the models in the trial (`docs/DESIGN-canvas-cut.md`, variant T).

use anyhow::{Context, Result};
use mg_image::Rgba;

/// How far past the area's edge (m) a tree becomes a card: where the edge scenery's fade to the fog colour starts.
pub(super) const CARD_FROM: f32 = 25.0;

/// The atlas's side (pixels).
const SIZE: u32 = 512;
/// Each picture is drawn this many times larger, then shrunk.
const SUPER: u32 = 2;

/// The pictures' places in the atlas (u0, v0, u1, v1, from its bottom-left): the four sides, then the top.
const CELLS: [[f32; 4]; 5] =
    [[0.0, 0.0, 0.5, 0.375], [0.5, 0.0, 1.0, 0.375], [0.0, 0.375, 0.5, 0.75], [0.5, 0.375, 1.0, 0.75], [0.0, 0.75, 0.25, 1.0]];
/// The sides, each by the direction it faces (towards its viewer) and its picture's right, in the model's frame: south, north,
/// east, west.
const SIDES: [([f32; 2], [f32; 2]); 4] =
    [([0.0, -1.0], [1.0, 0.0]), ([0.0, 1.0], [-1.0, 0.0]), ([1.0, 0.0], [0.0, 1.0]), ([-1.0, 0.0], [0.0, -1.0])];
/// A side's normals: how much they face its viewer, lean out from the trunk, and point up at its top and its foot (the trial's,
/// calibrated against the model under the game's sun).
const FACING: f32 = 0.25;
const OUTWARD: f32 = 0.2;
const UP_TOP: f32 = 0.8;
const UP_FOOT: f32 = 0.5;

/// A tree model's atlas and the box it was drawn over.
pub(super) struct Atlas {
    /// The pictures, rows from the bottom.
    pub(super) image: Rgba,
    /// Half the width of the side pictures (m): the model's reach from its trunk.
    pub(super) reach: f32,
    /// The side pictures' foot and top (m, model frame).
    pub(super) z: [f32; 2],
    /// The height of the top card (m): where the crown is widest.
    pub(super) top: f32,
}

/// The atlas of model `model`'s pictures (see the module comment).
pub(super) fn atlas(model: &str) -> Result<Atlas> {
    let game = crate::models::game()?;
    let flat = crate::models::flat(game, model)?;
    let positions = &flat.positions;
    anyhow::ensure!(!positions.is_empty(), "{model} has no triangles");
    let reach = positions.iter().map(|p| p[0].abs().max(p[1].abs())).fold(0.0f32, f32::max) + 0.15;
    let z0 = positions.iter().map(|p| p[2]).fold(0.0f32, f32::min) - 0.05;
    let z1 = positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max) + 0.15;
    // The top card sits where the crown is widest: the height whose triangles reach furthest out, in 0.25 m bands.
    let mut widest = (0.0f32, (z0 + z1) / 2.0);
    for t in positions.as_chunks::<3>().0 {
        let mid_z = (t[0][2] + t[1][2] + t[2][2]) / 3.0;
        let out = t.iter().map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt()).fold(0.0f32, f32::max);
        if out > widest.0 + 0.25 {
            widest = (out, mid_z);
        }
    }
    // Every texture the model names, decoded once (a missing one draws grey).
    let mut textures: Vec<Option<Rgba>> = Vec::new();
    for (name, _, _) in &flat.groups {
        textures.push(nwn_bake::texture::rgba(game, name).ok());
    }
    let mut image = Rgba::new(SIZE, SIZE);
    for (k, cell) in CELLS.iter().enumerate() {
        let (w, h) = (((cell[2] - cell[0]) * SIZE as f32) as u32, ((cell[3] - cell[1]) * SIZE as f32) as u32);
        // Model point -> picture (u, v in 0..1 from the bottom-left, and depth towards the viewer), and the light on a face.
        let project: Box<dyn Fn([f32; 3]) -> [f32; 3]> = match SIDES.get(k) {
            Some(&(face, right)) => Box::new(move |p: [f32; 3]| {
                [(p[0] * right[0] + p[1] * right[1] + reach) / (2.0 * reach), (p[2] - z0) / (z1 - z0), p[0] * face[0] + p[1] * face[1]]
            }),
            None => Box::new(move |p: [f32; 3]| [(p[0] + reach) / (2.0 * reach), (p[1] + reach) / (2.0 * reach), p[2]]),
        };
        // (Matched to the trial's atlases, which were calibrated against the model under the game's sun: a card is about as
        // bright as the tree beside it.)
        let (ambient, diffuse) = if k < 4 { (0.48, 0.40) } else { (0.85, 0.30) };
        let picture = draw(&flat, &textures, w * SUPER, h * SUPER, &*project, ambient, diffuse);
        let picture = shrink(&picture, SUPER);
        let (x0, y0) = ((cell[0] * SIZE as f32) as u32, (cell[1] * SIZE as f32) as u32);
        for y in 0..h {
            for x in 0..w {
                let from = ((y * w + x) * 4) as usize;
                let to = (((y0 + y) * SIZE + x0 + x) * 4) as usize;
                image.data[to..to + 4].copy_from_slice(&picture.data[from..from + 4]);
            }
        }
    }
    Ok(Atlas { image, reach, z: [z0, z1], top: widest.1 })
}

/// The model's triangles drawn into a `w` by `h` picture through `project`, nearest first, each texel lit by `ambient` plus
/// `diffuse` times how much its face looks up (either side). Texels whose texture is less than half opaque are holes.
fn draw(
    flat: &crate::models::Flat,
    textures: &[Option<Rgba>],
    w: u32,
    h: u32,
    project: &dyn Fn([f32; 3]) -> [f32; 3],
    ambient: f32,
    diffuse: f32,
) -> Rgba {
    let mut out = Rgba::new(w, h);
    let mut depth = vec![f32::MIN; (w * h) as usize];
    for (g, (_, first, count)) in flat.groups.iter().enumerate() {
        let tex = textures.get(g).and_then(Option::as_ref);
        for t in *first..first + count {
            let p = [flat.positions[3 * t], flat.positions[3 * t + 1], flat.positions[3 * t + 2]];
            let uv = [flat.uvs[3 * t], flat.uvs[3 * t + 1], flat.uvs[3 * t + 2]];
            let n = {
                let (a, b, c) = (p[0], p[1], p[2]);
                let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                n.map(|c| c / len)
            };
            let light = ambient + diffuse * n[2].abs();
            let q = p.map(|v| {
                let s = project(v);
                [s[0] * w as f32, s[1] * h as f32, s[2]]
            });
            let area = (q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[2][0] - q[0][0]) * (q[1][1] - q[0][1]);
            if area.abs() < 1e-9 {
                continue;
            }
            let (lo_x, hi_x) = (q.iter().map(|v| v[0]).fold(f32::MAX, f32::min), q.iter().map(|v| v[0]).fold(f32::MIN, f32::max));
            let (lo_y, hi_y) = (q.iter().map(|v| v[1]).fold(f32::MAX, f32::min), q.iter().map(|v| v[1]).fold(f32::MIN, f32::max));
            let (x0, x1) = ((lo_x.floor().max(0.0)) as u32, (hi_x.ceil().min(w as f32)) as u32);
            let (y0, y1) = ((lo_y.floor().max(0.0)) as u32, (hi_y.ceil().min(h as f32)) as u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let edge = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0]);
                    let (w0, w1, w2) = (edge(q[1], q[2]) / area, edge(q[2], q[0]) / area, edge(q[0], q[1]) / area);
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * q[0][2] + w1 * q[1][2] + w2 * q[2][2];
                    let k = (y * w + x) as usize;
                    if z <= depth[k] {
                        continue;
                    }
                    let (u, v) = (w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0], w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1]);
                    let c = tex.map_or([150, 150, 150, 255], |t| {
                        let (tx, ty) = ((u.rem_euclid(1.0) * t.width as f32) as u32, (v.rem_euclid(1.0) * t.height as f32) as u32);
                        let p = t.pixel(tx.min(t.width - 1), ty.min(t.height - 1));
                        [p[0], p[1], p[2], p[3]]
                    });
                    if c[3] < 128 {
                        continue;
                    }
                    depth[k] = z;
                    let lit = |c: u8| (f32::from(c) * light).round().clamp(0.0, 255.0) as u8;
                    out.data[k * 4..k * 4 + 4].copy_from_slice(&[lit(c[0]), lit(c[1]), lit(c[2]), 255]);
                }
            }
        }
    }
    out
}

/// `img` shrunk `by` times each way: each pixel the mean of the opaque ones under it, opaque when half or more of them are.
fn shrink(img: &Rgba, by: u32) -> Rgba {
    let (w, h) = (img.width / by, img.height / by);
    let mut out = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut n) = ([0u32; 3], 0u32);
            for dy in 0..by {
                for dx in 0..by {
                    let p = img.pixel(x * by + dx, y * by + dy);
                    if p[3] >= 128 {
                        for k in 0..3 {
                            sum[k] += u32::from(p[k]);
                        }
                        n += 1;
                    }
                }
            }
            let k = ((y * w + x) * 4) as usize;
            if n > 0 {
                for c in 0..3 {
                    out.data[k + c] = sum[c].checked_div(n).unwrap_or(0) as u8;
                }
                out.data[k + 3] = if 2 * n >= by * by { 255 } else { 0 };
            }
        }
    }
    out
}

/// Gives each clear texel the colour of the opaque ones nearest it (rings outward), so filtering never blends in black.
fn bleed(img: &mut Rgba) {
    let (w, h) = (i64::from(img.width), i64::from(img.height));
    let mut known: Vec<bool> = img.data.as_chunks::<4>().0.iter().map(|p| p[3] > 0).collect();
    for _ in 0..64 {
        let mut next = known.clone();
        let mut grew = false;
        for y in 0..h {
            for x in 0..w {
                let k = (y * w + x) as usize;
                if known[k] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0);
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (a, b) = (x + dx, y + dy);
                    if (0..w).contains(&a) && (0..h).contains(&b) && known[(b * w + a) as usize] {
                        let j = ((b * w + a) * 4) as usize;
                        for c in 0..3 {
                            sum[c] += u32::from(img.data[j + c]);
                        }
                        n += 1;
                    }
                }
                if n > 0 {
                    for c in 0..3 {
                        img.data[k * 4 + c] = sum[c].checked_div(n).unwrap_or(0) as u8;
                    }
                    next[k] = true;
                    grew = true;
                }
            }
        }
        known = next;
        if !grew {
            break;
        }
    }
}

/// The atlas as a BioWare DDS with alpha (DXT5) and its whole mip chain: each level's alpha on or off, as many texels opaque (in
/// proportion) as at the top, and its clear texels coloured like the opaque ones beside them.
pub(super) fn dds(atlas: &Rgba) -> Vec<u8> {
    let coverage = atlas.data.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128).count() as f32 / (atlas.width * atlas.height) as f32;
    let mut level = atlas.clone();
    bleed(&mut level);
    let mut levels = vec![dxt5(&level)];
    let mut soft = atlas.clone();
    while soft.width > 1 || soft.height > 1 {
        soft = crate::dds::half(&soft);
        let mut hard = soft.clone();
        // The alpha that keeps the coverage: the cut at the share of texels the top has opaque.
        let mut alphas: Vec<u8> = soft.data.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        alphas.sort_unstable_by(|a, b| b.cmp(a));
        let keep = ((coverage * alphas.len() as f32).round() as usize).min(alphas.len());
        let cut = if keep == 0 { 255 } else { alphas[keep - 1].max(1) };
        for p in hard.data.as_chunks_mut::<4>().0 {
            p[3] = if p[3] >= cut { 255 } else { 0 };
        }
        bleed(&mut hard);
        levels.push(dxt5(&hard));
    }
    let mut out = Vec::new();
    out.extend(atlas.width.to_le_bytes());
    out.extend(atlas.height.to_le_bytes());
    out.extend(4u32.to_le_bytes());
    out.extend((levels[0].len() as u32).to_le_bytes());
    // (The header's mean alpha: the game reads a texture whose mean is 1 as opaque, and draws the cards' clear texels.)
    out.extend(coverage.to_le_bytes());
    for l in levels {
        out.extend(l);
    }
    out
}

/// One level as DXT5: each 4 by 4 block's alpha on (255) or off (0), and its colour as DXT1's.
fn dxt5(img: &Rgba) -> Vec<u8> {
    let (w, h) = (img.width, img.height);
    let mut out = Vec::new();
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            let px: [[u8; 4]; 16] = std::array::from_fn(|i| img.pixel((bx + i as u32 % 4).min(w - 1), (by + i as u32 / 4).min(h - 1)));
            // Alpha: endpoints 255 and 0 (eight-value mode), index 0 for on and 1 for off.
            let mut bits = 0u64;
            for (i, p) in px.iter().enumerate() {
                if p[3] < 128 {
                    bits |= 1 << (3 * i);
                }
            }
            out.extend([255u8, 0]);
            out.extend(&bits.to_le_bytes()[..6]);
            out.extend(crate::dds::dxt1_block(&px.map(|p| [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])])));
        }
    }
    out
}

/// A tree's cards: (position, normal, uv) of each vertex, three a triangle, in the model's frame. The four sides of the two crossed
/// planes (each a picture of its own, the game drawing only front faces), then the top card when `top`.
pub(super) fn cards(a: &Atlas, top: bool) -> Vec<([f32; 3], [f32; 3], [f32; 2])> {
    let inset = 1.0 / SIZE as f32;
    let uv = |cell: [f32; 4], f: f32, g: f32| {
        [cell[0] + inset + f * (cell[2] - cell[0] - 2.0 * inset), cell[1] + inset + g * (cell[3] - cell[1] - 2.0 * inset)]
    };
    let mut out = Vec::new();
    let (r, [z0, z1]) = (a.reach, a.z);
    for (k, &(face, right)) in SIDES.iter().enumerate() {
        let corner = |f: f32, g: f32| {
            let p = [(2.0 * f - 1.0) * r * right[0], (2.0 * f - 1.0) * r * right[1], z0 + g * (z1 - z0)];
            let n = [FACING * face[0] + OUTWARD * p[0] / r, FACING * face[1] + OUTWARD * p[1] / r, if g > 0.5 { UP_TOP } else { UP_FOOT }];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            (p, n.map(|c| c / len), uv(CELLS[k], f, g))
        };
        let q = [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)];
        out.extend([q[0], q[1], q[2], q[0], q[2], q[3]]);
    }
    if top {
        let corner = |f: f32, g: f32| ([(2.0 * f - 1.0) * r, (2.0 * g - 1.0) * r, a.top], [0.0, 0.0, 1.0], uv(CELLS[4], f, g));
        let q = [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)];
        out.extend([q[0], q[1], q[2], q[0], q[2], q[3]]);
    }
    out
}

/// The atlas's resref for model `model`: the same in every area, so areas of one module share it.
pub(super) fn atlas_name(model: &str) -> String {
    let m: String = model.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(12).collect();
    format!("lsb_{m}")
}

/// Whether a baked object of model `model` is a tree that may become a card.
pub(super) fn is_tree(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.contains("tree") && !m.starts_with(crate::paint::IMPORT)
}

/// The atlas of `model`, its pixels written beside an export to look at (`LS_DUMP`).
pub(super) fn dump(dir: &std::path::Path, model: &str, a: &Atlas) -> Result<()> {
    crate::paint::write_png(&dir.join(format!("{}.png", atlas_name(model))), &a.image).context("writing a card atlas")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_keep_their_coverage_and_alpha_is_on_or_off() {
        // A thin cross: an eighth of the texels opaque.
        let mut img = Rgba::new(64, 64);
        for y in 0..64u32 {
            for x in 0..64u32 {
                if (28..32).contains(&x) || (28..32).contains(&y) {
                    let k = ((y * 64 + x) * 4) as usize;
                    img.data[k..k + 4].copy_from_slice(&[40, 120, 30, 255]);
                }
            }
        }
        let file = dds(&img);
        let read = mg_image::read(mg_core::ResType::DDS, &file).unwrap().to_rgba();
        assert_eq!((read.width, read.height), (64, 64));
        assert!(read.data.as_chunks::<4>().0.iter().all(|p| p[3] == 0 || p[3] == 255), "alpha on or off");
        // Clear texels take the leaves' colour, not black.
        assert!(read.pixel(0, 0)[1] > 60, "{:?}", read.pixel(0, 0));
    }

    #[test]
    fn a_tree_has_ten_card_triangles_and_each_side_faces_its_viewer() {
        let a = Atlas { image: Rgba::new(4, 4), reach: 3.0, z: [0.0, 8.0], top: 5.0 };
        let v = cards(&a, true);
        assert_eq!(v.len(), 30);
        for (k, &(face, _)) in SIDES.iter().enumerate() {
            let t = &v[6 * k..6 * k + 3];
            let (a, b, c) = (t[0].0, t[1].0, t[2].0);
            let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2]];
            assert!(n[0] * face[0] + n[1] * face[1] > 0.0, "side {k} winds towards its viewer");
        }
        assert!(is_tree("tnp_tree_t21") && !is_tree("plc_a01"));
        assert!(atlas_name("tnp_tree_t21").len() <= 16);
    }
}
