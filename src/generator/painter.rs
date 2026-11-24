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
    /// Each library texture's digest (by palette index; worked out when first needed), for the bake cache.
    digests: Vec<std::sync::OnceLock<[u8; 20]>>,
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
    /// The picture's digest ([`digest`]), for the bake cache.
    image_digest: [u8; 20],
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
        let mut images: HashMap<PathBuf, (std::sync::Arc<mg_image::Rgba>, [u8; 20])> = HashMap::new();
        for o in hm.objects.iter().filter(|o| o.model.starts_with(paint::IMPORT)) {
            let Some(name) = o.model.strip_prefix(paint::IMPORT).and_then(|r| r.split(':').next()) else { continue };
            let Ok(flat) = models::flat(models::game()?, &o.model) else { continue };
            let Some(d) = &flat.decal else { continue };
            let path = paint::import_dir(name).join(&d.file);
            let (image, image_digest) = match images.get(&path) {
                Some(i) => i.clone(),
                None => {
                    let Ok(i) = paint::read_png(&path) else { continue };
                    let d = digest(&i);
                    let i = (std::sync::Arc::new(i), d);
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
                image_digest,
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
        let digests = hm.palette.iter().map(|_| std::sync::OnceLock::new()).collect();
        Ok(Some(Painter { hm, textures, decals, digests }))
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
    /// `decals`: the ones on its tile.
    fn ground(&self, x: f32, y: f32, decals: &[std::sync::Arc<Decal>]) -> [f32; 3] {
        let mut c = [0.0; 3];
        // (Corners of the same texture share its colour here: sampled once.)
        let mut last: Option<(u8, [f32; 3])> = None;
        for (m, w) in self.corners(x, y) {
            if w > 0.0 {
                let t = match last {
                    Some((lm, t)) if lm == m => t,
                    _ => self.textures[m as usize].ground(x, y),
                };
                last = Some((m, t));
                for k in 0..3 {
                    c[k] += t[k] * w;
                }
            }
        }
        // Imported things' patches of ground over it, where they stand.
        for d in decals {
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

    /// Tile (tx, ty)'s baked texture as a DDS: the last export's when nothing it is made from has changed (an area exported again
    /// mostly has the same ground), else [`Painter::bake`]'s.
    pub(super) fn baked_dds(&self, tx: usize, ty: usize, size: u32) -> Vec<u8> {
        let key = self.bake_key(tx, ty, size);
        if let Some(dds) = BAKES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return dds;
        }
        let dds = paint::dds(&self.bake(tx, ty, size));
        BAKES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).put(key, &dds);
        dds
    }

    /// A digest of everything [`Painter::bake`] reads for tile (tx, ty): where it is, the paint under it and around it, the textures
    /// that paint names, and the decals on it.
    fn bake_key(&self, tx: usize, ty: usize, size: u32) -> [u8; 20] {
        use sha1::Digest;
        let mut h = sha1::Sha1::new();
        let hm = &self.hm;
        h.update(BAKE_VERSION.to_le_bytes());
        for v in [u64::from(size), tx as u64, ty as u64, hm.width as u64, hm.height as u64, hm.tiles as u64, hm.tiles_y as u64] {
            h.update(v.to_le_bytes());
        }
        h.update(hm.spacing.to_le_bytes());
        // (The samples a pixel's corners reach, and one more each way.)
        let k = self.samples_per_tile();
        let mut used = Vec::new();
        for j in ty as i64 * k - 1..=(ty as i64 + 1) * k + 1 {
            for i in tx as i64 * k - 1..=(tx as i64 + 1) * k + 1 {
                let m = hm.material(i, j);
                h.update([m]);
                used.push(m);
            }
        }
        used.sort_unstable();
        used.dedup();
        let (cx, cy) = ((tx as f32 + 0.5) * TILE, (ty as f32 + 0.5) * TILE);
        for m in used {
            let t = &self.textures[m as usize];
            let d = match t.picture_at(cx, cy) {
                // (An import's ground: only this tile's picture is read here.)
                Some((p, _, _)) => digest(p),
                None => *self.digests[m as usize].get_or_init(|| digest(&t.image)),
            };
            h.update([m]);
            h.update(d);
        }
        for d in self.decals.get(&(tx, ty)).into_iter().flatten() {
            for v in [d.x, d.y, d.cos, d.sin, d.scale, d.rect[0], d.rect[1], d.rect[2], d.rect[3]] {
                h.update(v.to_le_bytes());
            }
            h.update(d.image_digest);
        }
        h.finalize().into()
    }

    /// Tile (tx, ty)'s own texture, `size` pixels a side: its ground as painted, each pixel the mean of 2 x 2 samples.
    pub(super) fn bake(&self, tx: usize, ty: usize, size: u32) -> mg_image::Rgba {
        let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
        // (Every point drawn is on this tile: its decals, looked up once.)
        let decals = self.decals.get(&(tx, ty)).map_or(&[][..], Vec::as_slice);
        paint::picture(size, size, |px, py| {
            let mut c = [0.0; 3];
            for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let g = self.ground(ox + (px as f32 + dx) / size as f32 * TILE, oy + (py as f32 + dy) / size as f32 * TILE, decals);
                for k in 0..3 {
                    c[k] += g[k] / 4.0;
                }
            }
            c
        })
    }
}

/// Changed whenever a baked texture comes out differently from the same paint (the bake, its blending or the DDS encoder): the
/// bake cache's entries from before no longer count.
const BAKE_VERSION: u32 = 1;

/// A picture's digest: its size and pixels.
fn digest(img: &mg_image::Rgba) -> [u8; 20] {
    use sha1::Digest;
    let mut h = sha1::Sha1::new();
    h.update(img.width.to_le_bytes());
    h.update(img.height.to_le_bytes());
    h.update(&img.data);
    h.finalize().into()
}

/// The baked textures of the last exports, by [`Painter::bake_key`].
/// (Enough for a 32 by 32 area's at 512 px and some of another's.)
static BAKES: std::sync::Mutex<Kept> = std::sync::Mutex::new(Kept::new(256 << 20));

/// Files made in an export, kept for the next by a digest of what they were made from, up to `max` bytes; the least recently used
/// go first.
struct Kept {
    /// (key, file, when last used).
    entries: Vec<([u8; 20], Vec<u8>, u64)>,
    bytes: usize,
    max: usize,
    clock: u64,
}

impl Kept {
    const fn new(max: usize) -> Kept {
        Kept { entries: Vec::new(), bytes: 0, max, clock: 0 }
    }

    fn get(&mut self, key: &[u8; 20]) -> Option<Vec<u8>> {
        self.clock += 1;
        let e = self.entries.iter_mut().find(|e| e.0 == *key)?;
        e.2 = self.clock;
        Some(e.1.clone())
    }

    fn put(&mut self, key: [u8; 20], file: &[u8]) {
        if file.len() > self.max || self.entries.iter().any(|e| e.0 == key) {
            return;
        }
        self.clock += 1;
        self.bytes += file.len();
        self.entries.push((key, file.to_vec(), self.clock));
        while self.bytes > self.max {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].2).expect("over the limit, so not empty");
            self.bytes -= self.entries.swap_remove(oldest).1.len();
        }
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

/// Pixels a side of a tile's shadow map (16 cm each).
pub(super) const SHADOW_MAP: u32 = 64;
/// Pixels a side of the shadow map of a tile baked objects shade (8 cm each): fronds and trunks need them.
pub(super) const SHADOW_MAP_FINE: u32 = 128;

/// The baked objects that may shade tile (tx, ty): their shadows towards the ground from as high as they stand over the area's
/// lowest ground, the sky they hide around them. Their placements (`Placed::what`), in the scene's order.
fn shading(scene: &objects::Scene, tx: usize, ty: usize) -> Vec<&str> {
    let lowest = scene.lowest;
    let (x0, y0, x1, y1) = (tx as f32 * TILE, ty as f32 * TILE, (tx + 1) as f32 * TILE, (ty + 1) as f32 * TILE);
    // (The sun is towards (4000, 4500, 7000): a point h over the ground shades it 4/7 h west and 4.5/7 h south of itself.)
    let reach = shade::OBJECT_REACH;
    scene
        .objects
        .iter()
        .filter(|o| o.baked)
        .filter(|o| {
            let h = (o.max[2] - lowest).max(0.0) + 1.0;
            o.min[0] - h * 4.0 / 7.0 - reach < x1 && o.max[0] + reach > x0 && o.min[1] - h * 4.5 / 7.0 - reach < y1 && o.max[1] + reach > y0
        })
        .map(|o| o.what.as_str())
        .collect()
}

/// Pixels a side of tile (tx, ty)'s shadow map.
pub(super) fn shadow_size(tx: usize, ty: usize) -> u32 {
    match objects::scene() {
        Some(s) if s.occluders().is_some() && !shading(&s, tx, ty).is_empty() => SHADOW_MAP_FINE,
        _ => SHADOW_MAP,
    }
}

/// Tile (tx, ty)'s shadow map (see shade.rs): rows bottom first, over the tile's square as its second texture coordinates run. The
/// last export's when the ground it is cast from hasn't changed.
pub(super) fn shadow_map(tx: usize, ty: usize, strength: f32) -> mg_image::Rgba {
    let Some(key) = shadow_key(tx, ty, strength) else { return draw_shadow_map(tx, ty, strength) };
    if let Some(data) = SHADOWS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
        let size = shadow_size(tx, ty);
        return mg_image::Rgba { width: size, height: size, data };
    }
    let map = draw_shadow_map(tx, ty, strength);
    SHADOWS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).put(key, &map.data);
    map
}

/// The shadow maps of the last exports, by [`shadow_key`].
static SHADOWS: std::sync::Mutex<Kept> = std::sync::Mutex::new(Kept::new(32 << 20));

/// A digest of everything tile (tx, ty)'s shadow map is cast from: the ground's heights as far as [`shade::shade_lit`] looks (13 m
/// around, 80 m towards the sun), the baked objects that shade it, the entrances and the strength. None when it can't be kept: no
/// painted terrain, or a debugging picture.
fn shadow_key(tx: usize, ty: usize, strength: f32) -> Option<[u8; 20]> {
    use sha1::Digest;
    if std::env::var_os("LS_SHADOW_DEBUG").is_some() {
        return None;
    }
    let hm = heightmap::current()?;
    let mut h = sha1::Sha1::new();
    h.update(SHADOW_VERSION.to_le_bytes());
    for v in [tx as u64, ty as u64, hm.width as u64, hm.height as u64, u64::from(hm.underground)] {
        h.update(v.to_le_bytes());
    }
    h.update(strength.to_le_bytes());
    h.update(hm.spacing.to_le_bytes());
    // (The sun is towards +x +y: at most 80 m that way, cos and sin of its bearing under 0.75.)
    let (x0, y0) = (tx as f32 * TILE - 14.0, ty as f32 * TILE - 14.0);
    let (x1, y1) = ((tx + 1) as f32 * TILE + 14.0 + 61.0, (ty + 1) as f32 * TILE + 14.0 + 61.0);
    let sp = hm.spacing;
    let (i0, i1) = (((x0 / sp).floor() as i64 - 1).max(0) as usize, ((x1 / sp).ceil() as usize + 1).min(hm.width - 1));
    let (j0, j1) = (((y0 / sp).floor() as i64 - 1).max(0) as usize, ((y1 / sp).ceil() as usize + 1).min(hm.height - 1));
    for j in j0..=j1 {
        for v in &hm.data[j * hm.width + i0..=j * hm.width + i1] {
            h.update(v.to_le_bytes());
        }
    }
    if let Some(scene) = objects::scene() {
        for what in shading(&scene, tx, ty) {
            h.update(what.as_bytes());
            h.update([0]);
        }
    }
    h.update(format!("{:?}", hm.entrances).as_bytes());
    Some(h.finalize().into())
}

/// Changed whenever a shadow map comes out differently from the same ground (shade.rs, [`draw_shadow_map`]): the cache's entries
/// from before no longer count.
const SHADOW_VERSION: u32 = 3;

/// Values on a grid of `cell` m squares over a tile and `apron` squares beyond it, at their centres; rows from the south.
struct Grid {
    corner: (f32, f32),
    cell: f32,
    n: usize,
    apron: usize,
    v: Vec<[f32; 2]>,
}

impl Grid {
    /// `cells` squares a side over the tile from `corner`, `f(i, j, x, y)` at each (i, j from -apron).
    fn new(corner: (f32, f32), cells: usize, apron: usize, f: impl Fn(i64, i64, f32, f32) -> [f32; 2]) -> Grid {
        let (n, cell) = (cells + 2 * apron, TILE / cells as f32);
        let v = (0..n * n)
            .map(|k| {
                let (i, j) = ((k % n) as i64 - apron as i64, (k / n) as i64 - apron as i64);
                f(i, j, corner.0 + (i as f32 + 0.5) * cell, corner.1 + (j as f32 + 0.5) * cell)
            })
            .collect();
        Grid { corner, cell, n, apron, v }
    }

    /// Each value the mean of the 3 x 3 about it: one square less of apron.
    fn blurred(&self) -> Grid {
        let n = self.n - 2;
        let mut v = vec![[0.0; 2]; n * n];
        for j in 0..n {
            for i in 0..n {
                for (dj, di) in (0..3).flat_map(|dj| (0..3).map(move |di| (dj, di))) {
                    let w = self.v[(j + dj) * self.n + i + di];
                    v[j * n + i] = [v[j * n + i][0] + w[0] / 9.0, v[j * n + i][1] + w[1] / 9.0];
                }
            }
        }
        Grid { n, apron: self.apron - 1, v, ..*self }
    }

    /// The values at (x, y), bilinear.
    fn at(&self, x: f32, y: f32) -> [f32; 2] {
        let last = (self.n - 1) as f32;
        let fx = ((x - self.corner.0) / self.cell - 0.5 + self.apron as f32).clamp(0.0, last);
        let fy = ((y - self.corner.1) / self.cell - 0.5 + self.apron as f32).clamp(0.0, last);
        let (i, j) = ((fx.floor() as usize).min(self.n - 2), (fy.floor() as usize).min(self.n - 2));
        let (tx, ty) = (fx - i as f32, fy - j as f32);
        let p = |i: usize, j: usize| self.v[j * self.n + i];
        let (a, b, c, d) = (p(i, j), p(i + 1, j), p(i, j + 1), p(i + 1, j + 1));
        [0, 1].map(|k| (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty)
    }
}

/// Which points of a 64-a-tile grid over a tile (and `APRON` beyond it) have their ground shut in by solid objects
/// ([`Occluders::shut_in`](crate::occluders::Occluders::shut_in)): no one sees it, so it takes no rays.
struct Shut {
    corner: (f32, f32),
    n: usize,
    v: Vec<bool>,
}

impl Shut {
    /// Points beyond the tile (a point off the grid counts as seen).
    const APRON: usize = 6;

    fn new(o: &crate::occluders::Occluders, corner: (f32, f32), ground: &dyn Fn(f32, f32) -> f32) -> Shut {
        let (n, cell) = (SHADOW_MAP as usize + 2 * Self::APRON, TILE / SHADOW_MAP as f32);
        let v = (0..n * n)
            .map(|k| {
                let (i, j) = ((k % n) as f32 - Self::APRON as f32, (k / n) as f32 - Self::APRON as f32);
                let (x, y) = (corner.0 + (i + 0.5) * cell, corner.1 + (j + 0.5) * cell);
                o.shut_in([x, y, ground(x, y)])
            })
            .collect();
        Shut { corner, n, v }
    }

    /// Whether every point within `r` of the one nearest (x, y) is shut in (a point off the grid is not).
    fn all(&self, x: f32, y: f32, r: i64) -> bool {
        let cell = TILE / SHADOW_MAP as f32;
        let at = |v: f32, c: f32| ((v - c) / cell).floor() as i64 + Self::APRON as i64;
        let (i, j) = (at(x, self.corner.0), at(y, self.corner.1));
        let n = self.n as i64;
        (j - r..=j + r).all(|b| (i - r..=i + r).all(|a| (0..n).contains(&a) && (0..n).contains(&b) && self.v[(b * n + a) as usize]))
    }
}

/// `values` (a `size` x `size` map, rows from the south) with each `None` given the mean of its neighbours that have one, again
/// until all have, or 1 when none had any: the ground no one sees, so its texels blend into what is seen around them.
fn fill_unseen(values: Vec<Option<f32>>, size: usize) -> Vec<f32> {
    let mut v = values;
    loop {
        let mut next = v.clone();
        let mut changed = false;
        for j in 0..size {
            for i in 0..size {
                if v[j * size + i].is_some() {
                    continue;
                }
                let around: Vec<f32> = [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)]
                    .iter()
                    .filter_map(|&(di, dj)| {
                        let (a, b) = (i as i64 + di, j as i64 + dj);
                        ((0..size as i64).contains(&a) && (0..size as i64).contains(&b))
                            .then(|| v[b as usize * size + a as usize])
                            .flatten()
                    })
                    .collect();
                if !around.is_empty() {
                    next[j * size + i] = Some(around.iter().sum::<f32>() / around.len() as f32);
                    changed = true;
                }
            }
        }
        v = next;
        if !changed {
            return v.into_iter().map(|x| x.unwrap_or(1.0)).collect();
        }
    }
}

/// Points a tile side the sky the objects hide is sampled at (32 cm apart): it changes slowly.
const SKY_GRID: usize = 32;

/// A seed for point (i, j) of a grid of `n` a tile, from tile (tx, ty): the same from whichever tile asks.
fn seed(tx: usize, ty: usize, n: usize, i: i64, j: i64) -> u32 {
    let (gi, gj) = ((tx as i64 * n as i64 + i) as u32, (ty as i64 * n as i64 + j) as u32);
    gi.wrapping_mul(0x9E37_79B1) ^ gj.wrapping_mul(0x85EB_CA77).rotate_left(13) ^ (n as u32).wrapping_mul(0xC2B2_AE35)
}

/// Tile (tx, ty)'s shadow map, drawn.
fn draw_shadow_map(tx: usize, ty: usize, strength: f32) -> mg_image::Rgba {
    let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
    // Read once a map, not once a pixel: each pixel samples the ground a few hundred times. LS_SHADOW_DEBUG: a 1 m checkerboard
    // instead, to see whether the client runs the shadow shader at all. LS_SHADOW_DEBUG=corner: an L in each tile's south-west corner
    // (dark along the south edge, lighter along the west), to see how the client orients the map.
    let debug = std::env::var("LS_SHADOW_DEBUG").ok();
    let terrain = heightmap::current();
    let ground = |x: f32, y: f32| terrain.as_ref().map_or_else(|| height(x, y), |t| t.at(x, y));
    let sun = !terrain.as_ref().is_some_and(|h| h.underground);
    // Baked objects shade it by their own triangles, when any are near enough: on a finer map, the terrain's shading (smooth) from
    // the coarse map's points, the sky the objects hide from a blurred coarser grid, their shadows texel by texel. Ground they shut
    // in (under an imported cliff's top) takes no rays: what is read near ground that is seen is worked out (a margin of points
    // for each grid's reach), and its texels take their seen neighbours' values.
    let scene = objects::scene();
    let occluders =
        if debug.is_none() { scene.as_ref().filter(|s| !shading(s, tx, ty).is_empty()).and_then(|s| s.occluders()) } else { None };
    let size = shadow_size(tx, ty);
    let texel = TILE / size as f32;
    let fine = occluders.map(|o| {
        let shut = Shut::new(o, (ox, oy), &ground);
        let coarse =
            Grid::new(
                (ox, oy),
                SHADOW_MAP as usize,
                1,
                |_, _, x, y| {
                    if shut.all(x, y, 2) { [0.0; 2] } else { shade::terrain(x, y, sun, &ground) }
                },
            );
        let sky = Grid::new((ox, oy), SKY_GRID, 2, |i, j, x, y| {
            if shut.all(x, y, 5) {
                return [1.0, 0.0];
            }
            [shade::sky_through(o, x, y, TILE / SKY_GRID as f32, seed(tx, ty, SKY_GRID, i, j), &ground), 0.0]
        })
        .blurred();
        // Each texel's value, None where no one sees it.
        let values: Vec<Option<f32>> = (0..size * size)
            .map(|k| {
                let (px, py) = (k % size, k / size);
                let (x, y) = (ox + (px as f32 + 0.5) * texel, oy + (py as f32 + 0.5) * texel);
                if shut.all(x, y, 1) {
                    return None;
                }
                // (Underground there is no sun: objects only hide the sky, as the terrain does.)
                let lit =
                    if sun { shade::sun_through(o, x, y, texel, seed(tx, ty, size as usize, px.into(), py.into()), &ground) } else { 1.0 };
                Some(shade::combine(strength, coarse.at(x, y), shade::Through { sun: lit, sky: sky.at(x, y)[0] }))
            })
            .collect();
        fill_unseen(values, size as usize)
    });
    paint::picture(size, size, |px, py| {
        let (x, y) = (ox + (px as f32 + 0.5) * texel, oy + (py as f32 + 0.5) * texel);
        match debug.as_deref() {
            // =half: the south half of each tile dark (unmistakable on screen).
            Some("half") => {
                let v = if (py as f32) < size as f32 / 2.0 { 0.35 } else { 1.0 };
                return [v, v, v];
            }
            Some("corner") => {
                let (lx, ly) = (px as f32 / size as f32, py as f32 / size as f32);
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
        let v = match &fine {
            Some(values) => values[(py * size + px) as usize],
            None => shade::shade_lit(x, y, strength, sun, &ground),
        };
        let v = v * entrance::darkness(x, y);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_files_go_least_recently_used_first_once_over_the_limit() {
        let mut kept = Kept::new(10);
        kept.put([1; 20], &[1; 4]);
        kept.put([2; 20], &[2; 4]);
        // (Using the first makes the second the oldest.)
        assert_eq!(kept.get(&[1; 20]), Some(vec![1; 4]));
        kept.put([3; 20], &[3; 4]);
        assert_eq!((kept.get(&[2; 20]), kept.get(&[1; 20]), kept.get(&[3; 20])), (None, Some(vec![1; 4]), Some(vec![3; 4])));
        assert_eq!(kept.bytes, 8);
        // Too big to keep at all: nothing else goes.
        kept.put([4; 20], &[4; 11]);
        assert_eq!((kept.get(&[4; 20]), kept.bytes), (None, 8));
    }
}
