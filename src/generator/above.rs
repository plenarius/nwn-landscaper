// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The minimaps from above: the exported area drawn straight down by nwn-bake from its own files (the tiles in the hak it
//! wrote, the placeables and doors of its `.git`, their real textures), cut into each tile's [`MINIMAP`] pixels a side. Where
//! nwn-bake draws nothing (a cave's void) the terrain's own minimap ([`super::minimap`]) shows through.

use anyhow::{Context, Result, ensure};
use nwn_bake::area::SceneOptions;
use nwn_bake::iso::region::Region;
use nwn_bake::iso::{Background, GAME_SUN, IsoOptions, Look, Picture, View};
use nwn_bake::module::AreaFiles;

use super::{MINIMAP, TILE};

/// Pixels of the top view a minimap pixel takes along each side (each drawn as nwn-bake's 2 by 2 supersamples).
const OVERSAMPLE: u32 = 2;

/// Shadows half as long as the game's sun casts them: a wall's shadow still marks it on the minimap, a 10 m tower's (4.3 m) stays
/// off the next tile.
const SHADOW_LENGTH: f32 = 0.5;

/// The area `area` (`are`, `git`) drawn from above, north up, at [`OVERSAMPLE`] pixels a minimap pixel, with the game opened over
/// `haks` (the module's hak list: the area's own hak, then the shared one).
pub(super) fn top_view(haks: &[String], area: &str, are: mg_gff::Gff, git: Option<mg_gff::Gff>) -> Result<Picture> {
    let t0 = std::time::Instant::now();
    let game = crate::models::open_game(nwn_bake::game::Haks::List(haks.to_vec()), nwn_bake::game::Tlk::None)?;
    let t_open = t0.elapsed().as_secs_f32();
    // (An import's own haks may be missing here: what they hold is then left out, as in the toolset.)
    let ours = &haks[..haks.len().min(2)];
    ensure!(!game.missing_haks().iter().any(|h| ours.contains(h)), "haks not found: {}", game.missing_haks().join(", "));
    if !game.missing_haks().is_empty() {
        eprintln!("  the minimaps leave out what is in the haks not found: {}", game.missing_haks().join(", "));
    }
    let files = AreaFiles { resref: area.to_ascii_lowercase(), are, git };
    let scene = nwn_bake::area::scene(&game, &files, &SceneOptions { objects: true, threads: crate::config::export_threads() })?;
    let size = scene.size();
    if std::env::var_os("LS_TIMING").is_some() {
        eprintln!("  minimap: game opened {t_open:.1}s, scene {:.1}s", t0.elapsed().as_secs_f32() - t_open);
    }
    let o = IsoOptions {
        views: vec![View::Top],
        scale: (MINIMAP * OVERSAMPLE) as f32 / TILE,
        // (No padding: the picture is the area, cut exactly at its edges. The game's sun, so shadows fall south-west as in the game
        // and on the terrain's own baked shadows.)
        look: Look { padding: 0.0, sun: Some(GAME_SUN), shadow_length: SHADOW_LENGTH, ..Look::default() },
        region: Some(Region::new([0.0, 0.0], size)?),
        background: Background::Transparent,
        ..IsoOptions::default()
    };
    let rendered = nwn_bake::iso::render(&scene, &o)?;
    rendered.pictures.into_iter().next().context("no top view")
}

/// `top` cut into each tile's minimap (row by row from the south, as `under`), drawn over the terrain's minimaps `under`: a minimap
/// pixel is the mean of the top view's pixels whose centres are in its square of the ground, and shows `under` where they are
/// transparent.
pub(super) fn slice(top: &Picture, tiles: [usize; 2], under: &[mg_image::Rgba]) -> Vec<mg_image::Rgba> {
    let m = MINIMAP as usize;
    let step = TILE / MINIMAP as f32;
    let t = &top.transform;
    // A game coordinate's pixel edge along x or y, rounded to the nearest pixel boundary.
    let column = |x: f32| (t.project([x, 0.0, 0.0])[0].round().max(0.0) as usize).min(top.width as usize);
    let row = |y: f32| (t.project([0.0, y, 0.0])[1].round().max(0.0) as usize).min(top.height as usize);
    (0..tiles[1])
        .flat_map(|ty| (0..tiles[0]).map(move |tx| (tx, ty)))
        .zip(under)
        .map(|((tx, ty), under)| {
            let mut out = under.clone();
            for py in 0..m {
                // (Minimap rows from the south; the top view's from the north.)
                let (y0, y1) = (ty as f32 * TILE + py as f32 * step, ty as f32 * TILE + (py + 1) as f32 * step);
                let (r0, r1) = (row(y1).min(row(y0)), row(y1).max(row(y0)));
                for px in 0..m {
                    let (x0, x1) = (tx as f32 * TILE + px as f32 * step, tx as f32 * TILE + (px + 1) as f32 * step);
                    let (c0, c1) = (column(x0), column(x1));
                    let (mut sum, mut n) = ([0f32; 4], 0f32);
                    for j in r0..r1 {
                        for i in c0..c1 {
                            let p = &top.rgba[(j * top.width as usize + i) * 4..][..4];
                            let a = f32::from(p[3]) / 255.0;
                            for k in 0..3 {
                                sum[k] += f32::from(p[k]) * a;
                            }
                            sum[3] += a;
                            n += 1.0;
                        }
                    }
                    if n == 0.0 {
                        continue;
                    }
                    let cover = sum[3] / n;
                    let at = (py * m + px) * 4;
                    for k in 0..3 {
                        let drawn = if sum[3] > 0.0 { sum[k] / sum[3] } else { 0.0 };
                        out.data[at + k] = (drawn * cover + f32::from(under.data[at + k]) * (1.0 - cover)).round() as u8;
                    }
                    out.data[at + 3] = 255;
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nwn_bake::iso::Transform;

    /// A top view of a `tiles` area at [`OVERSAMPLE`], each pixel coloured by where it is: red the column, green the row (from
    /// the top), blue 255 in the south-west tile only; transparent in the north-east tile.
    fn picture(tiles: [usize; 2]) -> Picture {
        let s = (MINIMAP * OVERSAMPLE) as f32 / TILE;
        let (w, h) = (tiles[0] as u32 * MINIMAP * OVERSAMPLE, tiles[1] as u32 * MINIMAP * OVERSAMPLE);
        let tile = (MINIMAP * OVERSAMPLE) as usize;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for j in 0..h as usize {
            for i in 0..w as usize {
                let (tx, ty) = (i / tile, tiles[1] - 1 - j / tile);
                let p = &mut rgba[(j * w as usize + i) * 4..][..4];
                p.copy_from_slice(&[(i % 256) as u8, (j % 256) as u8, if (tx, ty) == (0, 0) { 255 } else { 0 }, 255]);
                if (tx, ty) == (tiles[0] - 1, tiles[1] - 1) {
                    p[3] = 0;
                }
            }
        }
        let transform = Transform {
            view: View::Top,
            elevation_deg: 90.0,
            scale: s,
            width: w,
            height: h,
            rotation: [[1, 0], [0, 1]],
            offset: [0.0, h as f32],
            matrix: [[s, 0.0, 0.0, 0.0], [0.0, -s, 0.0, h as f32]],
        };
        Picture { view: View::Top, width: w, height: h, rgba, transform, exposure: 1.0, depth: None, time: Default::default() }
    }

    fn grey() -> mg_image::Rgba {
        let mut g = mg_image::Rgba::new(MINIMAP, MINIMAP);
        g.data.chunks_mut(4).for_each(|p| p.copy_from_slice(&[9, 9, 9, 255]));
        g
    }

    /// Each tile takes its own square, north up, rows from the south; a transparent top view shows the terrain's minimap.
    #[test]
    fn each_tile_gets_its_own_square_north_up() {
        let tiles = [3, 2];
        let top = picture(tiles);
        let under = vec![grey(); 6];
        let maps = slice(&top, tiles, &under);
        assert_eq!(maps.len(), 6);
        let px = |k: usize, x: usize, y: usize| maps[k].data[(y * MINIMAP as usize + x) * 4..][..4].to_vec();
        let (o, h) = (OVERSAMPLE as usize, tiles[1] * (MINIMAP * OVERSAMPLE) as usize);
        // The mean of the 2 by 2 top-view pixels a minimap pixel covers (two neighbours' mean, a + 0.5, rounds up).
        let want = |tx: usize, ty: usize, x: usize, y: usize| {
            let (i, j) = (tx * MINIMAP as usize * o + x * o, h - (ty * MINIMAP as usize * o + y * o + o));
            vec![(i % 256 + 1) as u8, (j % 256 + 1) as u8, if (tx, ty) == (0, 0) { 255 } else { 0 }, 255]
        };
        for (k, (tx, ty)) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1)].into_iter().enumerate() {
            for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31), (5, 17)] {
                assert_eq!(px(k, x, y), want(tx, ty, x, y), "tile ({tx}, {ty}) pixel ({x}, {y})");
            }
        }
        // The south-west tile's south-west pixel is the picture's bottom-left; the north-west tile's north-west, its top-left.
        assert_eq!(px(0, 0, 0), [1, (h - 1) as u8, 255, 255]);
        assert_eq!(px(3, 0, 31), [1, 1, 0, 255]);
        // Tile (2, 1), the north-east one, is transparent: the terrain's minimap.
        assert!(maps[5].data.chunks(4).all(|p| p == [9, 9, 9, 255]));
    }
}
