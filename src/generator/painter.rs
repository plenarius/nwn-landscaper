// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The painted textures: the palette and decals a tile model takes, its baked texture, its minimap picture and its shadow
//! map.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use super::{TILE, WALKABLE_NZ, entrance, height, in_void, tile_name, tiles_x, tiles_y};
use crate::{heightmap, models, objects, paint, shade};

/// The texture an entrance's hood takes: the editor paints its walls with
/// rock (the palette's rockCliff), else the palette's first texture.
pub(super) fn entrance_rock(p: &Painter) -> u8 {
    p.hm.palette.iter().position(|id| id == "rockCliff").unwrap_or(0) as u8
}

/// The loaded terrain's painted textures, for tile models, baked textures and minimaps.
pub(super) struct Painter {
    /// The terrain it paints, held so a reload during the export can't change it halfway.
    pub(super) hm: std::sync::Arc<heightmap::Heightmap>,
    /// By palette index.
    pub(super) textures: Vec<paint::Texture>,
    /// Imported things' ground patches where they stand now, by tile.
    decals: HashMap<(usize, usize), Vec<std::sync::Arc<Decal>>>,
}

/// A ground patch laid where its object stands: its picture over `rect` (metres about the object's origin), turned and scaled with it.
struct Decal {
    x: f32,
    y: f32,
    cos: f32,
    sin: f32,
    scale: f32,
    rect: [f32; 4],
    image: std::sync::Arc<mg_image::Rgba>,
}

impl Decal {
    /// Its colour and alpha (0..1) at area point (x, y), if it covers it.
    fn at(&self, x: f32, y: f32) -> Option<([f32; 3], f32)> {
        let (dx, dy) = ((x - self.x) / self.scale, (y - self.y) / self.scale);
        let (u, v) = (self.cos * dx + self.sin * dy, -self.sin * dx + self.cos * dy);
        let [x0, y0, x1, y1] = self.rect;
        if u < x0 || u >= x1 || v < y0 || v >= y1 {
            return None;
        }
        let img = &self.image;
        let (i, j) = ((((u - x0) / (x1 - x0)) * img.width as f32) as usize, (((v - y0) / (y1 - y0)) * img.height as f32) as usize);
        let k = (j.min(img.height as usize - 1) * img.width as usize + i.min(img.width as usize - 1)) * 4;
        let a = f32::from(img.data[k + 3]) / 255.0;
        (a > 0.0).then(|| ([f32::from(img.data[k]) / 255.0, f32::from(img.data[k + 1]) / 255.0, f32::from(img.data[k + 2]) / 255.0], a))
    }
}

impl Painter {
    /// None when the terrain isn't painted.
    pub(super) fn load() -> Result<Option<Painter>> {
        let Some(hm) = heightmap::current().filter(|h| h.painted()) else { return Ok(None) };
        let lib = paint::load_library(&hm.palette)?;
        let textures = hm.palette.iter().map(|id| lib[id].clone()).collect();
        // Imported things' ground patches, bucketed by the tiles they cover.
        let mut decals: HashMap<(usize, usize), Vec<std::sync::Arc<Decal>>> = HashMap::new();
        let mut images: HashMap<PathBuf, std::sync::Arc<mg_image::Rgba>> = HashMap::new();
        for o in hm.objects.iter().filter(|o| o.model.starts_with(paint::IMPORT)) {
            let Some(name) = o.model.strip_prefix(paint::IMPORT).and_then(|r| r.split(':').next()) else { continue };
            let Ok(flat) = models::flat(models::game()?, &o.model) else { continue };
            let Some(d) = &flat.decal else { continue };
            let path = paint::import_dir(name).join(&d.file);
            let image = match images.get(&path) {
                Some(i) => i.clone(),
                None => {
                    let Ok(i) = paint::read_png(&path) else { continue };
                    let i = std::sync::Arc::new(i);
                    images.insert(path, i.clone());
                    i
                }
            };
            let decal = std::sync::Arc::new(Decal {
                x: o.x,
                y: o.y,
                cos: o.bearing.cos(),
                sin: o.bearing.sin(),
                scale: o.scale,
                rect: d.rect,
                image,
            });
            // The tiles its turned rectangle touches.
            let [x0, y0, x1, y1] = d.rect;
            let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
                .map(|(u, v)| (o.x + (decal.cos * u - decal.sin * v) * o.scale, o.y + (decal.sin * u + decal.cos * v) * o.scale));
            let (lo_x, hi_x) = corners.iter().fold((f32::MAX, f32::MIN), |a, c| (a.0.min(c.0), a.1.max(c.0)));
            let (lo_y, hi_y) = corners.iter().fold((f32::MAX, f32::MIN), |a, c| (a.0.min(c.1), a.1.max(c.1)));
            let tile = |v: f32, n: usize| ((v / TILE).floor().max(0.0) as usize).min(n - 1);
            for ty in tile(lo_y, tiles_y())..=tile(hi_y, tiles_y()) {
                for tx in tile(lo_x, tiles_x())..=tile(hi_x, tiles_x()) {
                    decals.entry((tx, ty)).or_default().push(decal.clone());
                }
            }
        }
        Ok(Some(Painter { hm, textures, decals }))
    }

    fn samples_per_tile(&self) -> i64 {
        (TILE / self.hm.spacing).round() as i64
    }

    /// Whether palette entry `m` is an import's pictures.
    pub(super) fn imported(&self, m: u8) -> bool {
        self.textures[m as usize].tiles.is_some()
    }

    /// Whether tile (tx, ty) gets its own baked texture: several textures in it, or an import's picture.
    pub(super) fn baked(&self, tx: usize, ty: usize) -> bool {
        let used = self.tile_materials(tx, ty);
        used.len() > 1 || used.iter().any(|&m| self.imported(m)) || self.decals.contains_key(&(tx, ty))
    }

    /// Palette indices painted in tile (tx, ty), sorted.
    pub(super) fn tile_materials(&self, tx: usize, ty: usize) -> Vec<u8> {
        let k = self.samples_per_tile();
        let mut used: Vec<u8> = Vec::new();
        for j in ty as i64 * k..=(ty as i64 + 1) * k {
            for i in tx as i64 * k..=(tx as i64 + 1) * k {
                used.push(self.hm.material(i, j));
            }
        }
        used.sort_unstable();
        used.dedup();
        used
    }

    /// The four samples around (x, y) and their bilinear weights.
    fn corners(&self, x: f32, y: f32) -> [(u8, f32); 4] {
        let sp = self.hm.spacing;
        let (last_x, last_y) = ((self.hm.width - 2) as f32, (self.hm.height - 2) as f32);
        let (fx, fy) = ((x / sp).clamp(0.0, last_x + 1.0), (y / sp).clamp(0.0, last_y + 1.0));
        let (i, j) = (fx.floor().min(last_x), fy.floor().min(last_y));
        let (tx, ty) = (fx - i, fy - j);
        let (i, j) = (i as i64, j as i64);
        [
            (self.hm.material(i, j), (1.0 - tx) * (1.0 - ty)),
            (self.hm.material(i + 1, j), tx * (1.0 - ty)),
            (self.hm.material(i, j + 1), (1.0 - tx) * ty),
            (self.hm.material(i + 1, j + 1), tx * ty),
        ]
    }

    /// The ground's colour at (x, y): the four samples' textures blended, as the editor shows it.
    fn ground(&self, x: f32, y: f32) -> [f32; 3] {
        let mut c = [0.0; 3];
        for (m, w) in self.corners(x, y) {
            if w > 0.0 {
                let t = self.textures[m as usize].ground(x, y);
                for k in 0..3 {
                    c[k] += t[k] * w;
                }
            }
        }
        // Imported things' patches of ground over it, where they stand.
        // (Its own tile counts: asking the global terrain here, once a pixel, made the threads wait for each other.)
        let key = (((x / TILE).max(0.0) as usize).min(self.hm.tiles - 1), ((y / TILE).max(0.0) as usize).min(self.hm.tiles_y - 1));
        for d in self.decals.get(&key).into_iter().flatten() {
            if let Some((dc, a)) = d.at(x, y) {
                for k in 0..3 {
                    c[k] += (dc[k] - c[k]) * a;
                }
            }
        }
        c
    }

    /// The blended average colours at (x, y), for minimaps.
    fn mean(&self, x: f32, y: f32) -> [f32; 3] {
        let mut c = [0.0; 3];
        for (m, w) in self.corners(x, y) {
            for k in 0..3 {
                c[k] += self.textures[m as usize].mean[k] * w;
            }
        }
        c
    }

    /// The texture of the sample nearest (x, y).
    pub(super) fn nearest(&self, x: f32, y: f32) -> u8 {
        let sp = self.hm.spacing;
        self.hm.material((x / sp).round() as i64, (y / sp).round() as i64)
    }

    /// Tile (tx, ty)'s own texture, `size` pixels a side: its ground as painted, each pixel the mean of 2 x 2 samples.
    pub(super) fn bake(&self, tx: usize, ty: usize, size: u32) -> mg_image::Rgba {
        let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
        paint::picture(size, size, |px, py| {
            let mut c = [0.0; 3];
            for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let g = self.ground(ox + (px as f32 + dx) / size as f32 * TILE, oy + (py as f32 + dy) / size as f32 * TILE);
                for k in 0..3 {
                    c[k] += g[k] / 4.0;
                }
            }
            c
        })
    }
}

/// Pixels a side of a tile's baked texture (LS_BAKE overrides).
pub(super) fn bake_size() -> u32 {
    std::env::var("LS_BAKE").ok().and_then(|v| v.parse().ok()).filter(|v: &u32| v.is_power_of_two()).unwrap_or(512)
}

/// Pixels a side of a tile's minimap picture (as Moonglow renders them).
pub(super) const MINIMAP: u32 = 32;

/// Tile (tx, ty) from above for the minimap: its textures' average colours (green and grey unpainted), shaded by the slope under light
/// from the north-west.
pub(super) fn minimap(tx: usize, ty: usize, painter: Option<&Painter>) -> mg_image::Rgba {
    let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
    let light = {
        let l = [-1.0f32, 1.0, 1.4];
        let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / len, l[1] / len, l[2] / len]
    };
    paint::picture(MINIMAP, MINIMAP, |px, py| {
        let (x, y) = (ox + (px as f32 + 0.5) / MINIMAP as f32 * TILE, oy + (py as f32 + 0.5) / MINIMAP as f32 * TILE);
        let e = 0.3;
        let dx = (height(x + e, y) - height(x - e, y)) / (2.0 * e);
        let dy = (height(x, y + e) - height(x, y - e)) / (2.0 * e);
        let len = (dx * dx + dy * dy + 1.0).sqrt();
        let n = [-dx / len, -dy / len, 1.0 / len];
        let base = match painter {
            Some(p) => p.mean(x, y),
            None if n[2] >= WALKABLE_NZ => [0.33, 0.45, 0.2],
            None => [0.5, 0.48, 0.45],
        };
        if in_void(x, y) {
            return [0.0, 0.0, 0.0];
        }
        let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
        let shade = 0.45 + 0.75 * lit;
        let ground = [base[0] * shade, base[1] * shade, base[2] * shade];
        // Water: blue over the ground, deeper the bluer.
        match heightmap::current().and_then(|h| h.water_at(x, y).map(|l| l - h.at(x, y))) {
            Some(depth) => {
                let t = (0.45 + depth / 4.0).min(0.85);
                let blue = [0.16, 0.32, 0.45];
                [ground[0] + (blue[0] - ground[0]) * t, ground[1] + (blue[1] - ground[1]) * t, ground[2] + (blue[2] - ground[2]) * t]
            }
            None => ground,
        }
    })
}

/// How dark baked shadows are, 0 (none) to 1: the editor's setting (0.6
/// unset), LS_SHADOW overriding; none for the built-in test terrains.
pub(super) fn shadow_strength() -> f32 {
    if let Some(v) = std::env::var("LS_SHADOW").ok().and_then(|v| v.parse().ok()) {
        return v;
    }
    heightmap::current().map_or(0.0, |h| h.shadow.unwrap_or(0.6)).clamp(0.0, 1.0)
}

/// A tile's baked shadow map.
pub(super) fn shadow_name(tx: usize, ty: usize) -> String {
    format!("{}l", tile_name(tx, ty))
}

/// Pixels a side of a tile's shadow map.
pub(super) const SHADOW_MAP: u32 = 64;

/// Tile (tx, ty)'s shadow map (see shade.rs): rows bottom first, over the tile's square as its second texture coordinates run.
pub(super) fn shadow_map(tx: usize, ty: usize, strength: f32) -> mg_image::Rgba {
    let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
    // Read once a map, not once a pixel: each pixel samples the ground a few hundred times. LS_SHADOW_DEBUG: a 1 m checkerboard
    // instead, to see whether the client runs the shadow shader at all. LS_SHADOW_DEBUG=corner: an L in each tile's south-west corner
    // (dark along the south edge, lighter along the west), to see how the client orients the map.
    let debug = std::env::var("LS_SHADOW_DEBUG").ok();
    // Baked objects cast shadows too: the ground with their tops on it.
    let scene = objects::scene();
    let terrain = heightmap::current();
    let ground = |x: f32, y: f32| {
        let h = terrain.as_ref().map_or_else(|| height(x, y), |t| t.at(x, y));
        scene.as_ref().and_then(|s| s.top(x, y)).map_or(h, |t| t.max(h))
    };
    let sun = !terrain.as_ref().is_some_and(|h| h.underground);
    paint::picture(SHADOW_MAP, SHADOW_MAP, |px, py| {
        let (x, y) = (ox + (px as f32 + 0.5) / SHADOW_MAP as f32 * TILE, oy + (py as f32 + 0.5) / SHADOW_MAP as f32 * TILE);
        match debug.as_deref() {
            // =half: the south half of each tile dark (unmistakable on screen).
            Some("half") => {
                let v = if (py as f32) < SHADOW_MAP as f32 / 2.0 { 0.35 } else { 1.0 };
                return [v, v, v];
            }
            Some("corner") => {
                let (lx, ly) = (px as f32 / SHADOW_MAP as f32, py as f32 / SHADOW_MAP as f32);
                let v = if ly < 0.2 {
                    0.3
                } else if lx < 0.2 {
                    0.65
                } else {
                    1.0
                };
                return [v, v, v];
            }
            Some(_) => {
                let v = if (x.floor() as i64 + y.floor() as i64) % 2 == 0 { 0.45 } else { 1.0 };
                return [v, v, v];
            }
            None => {}
        }
        let v = shade::shade_lit(x, y, strength, sun, &ground) * entrance::darkness(x, y);
        [v, v, v]
    })
}

/// The material of a shadowed mesh: our shaders, its texture, the shadow map.
pub(super) fn shadow_mtr(bitmap: &str, shadow: &str) -> String {
    format!(
        "// NWN Landscaper: lit as usual, then the baked shadow map, then fog
customshadervs ls_lm_vs
customshaderfs ls_lm_fs
texture0 {bitmap}
texture7 {shadow}
"
    )
}
