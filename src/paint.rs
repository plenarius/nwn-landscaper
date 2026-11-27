// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Painted textures: the editor's texture library (filled by textures.rs) and what the hak needs from it: textures as BioWare DXT1
//! DDS, sampled the way the editor shows them.
//!
//! Pictures are kept as [`mg_image::Rgba`], rows bottom first as the game stores them, so texture coordinate v = 0 is the first stored
//! row; the library PNGs (top row first) are flipped on load. A texture repeats every [`TEXTURE_METRES`] on the ground (or its
//! catalog entry's `metres`: [`Texture::metres`]), north being the picture's up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mg_image::Rgba;

pub use crate::dds::{dds, half};
use crate::triplanar;

/// Metres one texture repeat covers on the ground (the editor's too), unless its catalog entry says otherwise ([`Texture::metres`]).
pub const TEXTURE_METRES: f32 = 4.0;

/// The metres a catalog entry's picture covers: its `metres` (1 to 64), else [`TEXTURE_METRES`].
pub fn metres_of(entry: &serde_json::Value) -> f32 {
    entry["metres"].as_f64().filter(|m| (1.0..=64.0).contains(m)).map_or(TEXTURE_METRES, |m| m as f32)
}

/// Whether a catalog entry's repeat is varied ([`triplanar::varied`]): its `vary`, else for natural ground and rock (a broken
/// edge, [`Edge`]) and not for laid surfaces (cobbles, planks, flagstones), whose regular pattern is right.
pub fn vary_of(entry: &serde_json::Value) -> bool {
    entry["vary"].as_bool().unwrap_or_else(|| Edge::from_entry(entry) == Edge::Broken)
}

/// How a library texture repeats on the ground: the metres a repeat covers, and the hex cells to a repeat its features are
/// varied by ([`triplanar::varied`]; 0 for a plain repeat).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Repeat {
    /// Metres one repeat covers ([`metres_of`]).
    pub metres: f32,
    /// Hex cells to a repeat: [`triplanar::VARY_CELLS`] times the picture's own repeats inside it (the starter list's `repeats`),
    /// or 0 for a plain repeat.
    pub vary: f32,
}

impl Repeat {
    /// A plain repeat every [`TEXTURE_METRES`]: a tileset's own texture, the stock rock.
    pub const PLAIN: Repeat = Repeat { metres: TEXTURE_METRES, vary: 0.0 };

    /// A catalog entry's ([`metres_of`], [`vary_of`]).
    pub fn of(entry: &serde_json::Value) -> Repeat {
        let repeats = entry["repeats"].as_u64().filter(|r| (1..=4).contains(r)).unwrap_or(1) as f32;
        Repeat { metres: metres_of(entry), vary: if vary_of(entry) { triplanar::VARY_CELLS * repeats } else { 0.0 } }
    }
}

/// The Landscaper's data folder: `landscaper` under `XDG_DATA_HOME` (default ~/.local/share; on Windows, which has no `HOME`
/// unless a shell such as Git Bash sets one, `%LOCALAPPDATA%`, whichever way it is started).
pub fn data_dir() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let windows = if cfg!(windows) { var("LOCALAPPDATA") } else { None };
    let base = var("XDG_DATA_HOME")
        .or(windows)
        .or_else(|| var("HOME").map(|h| h.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"));
    base.join("landscaper")
}

/// The texture library: `textures` in the data folder (~/.local/share/landscaper).
pub fn texture_dir() -> PathBuf {
    data_dir().join("textures")
}

/// The resref a library texture gets in the hak (16 characters at most).
pub fn resref(id: &str) -> String {
    let mut s = format!("ls_{}", id.to_ascii_lowercase());
    s.truncate(16);
    s
}

/// A library texture, loaded for painting tile models, baked textures and minimaps.
#[derive(Clone)]
pub struct Texture {
    /// The picture, repeated over the ground.
    pub image: Rgba,
    /// The picture halved again and again (each half the last's size, down to 1 pixel), for drawing it small without
    /// glitter; none for an import.
    pub smaller: std::sync::Arc<Vec<Rgba>>,
    /// Average colour (0..1), for minimaps.
    pub mean: [f32; 3],
    /// surfacemat.2da row of walkable ground painted with it (footsteps).
    pub surface: u32,
    /// How its edge meets another texture's in a baked tile (the catalog's `edge`).
    pub edge: Edge,
    /// An imported area's ground (`@import:<name>`): a picture per tile, drawn over its square instead of repeating.
    pub tiles: Option<std::sync::Arc<TilePictures>>,
    /// Metres one repeat of the picture covers ([`metres_of`]): [`TEXTURE_METRES`], or more for a cliff's rock, whose fine layers
    /// at 4 m blurred into streaks on a tall wall seen from afar.
    pub metres: f32,
    /// Hex cells to a repeat its features are varied by ([`Repeat::vary`]; 0 for a plain repeat).
    pub vary: f32,
}

/// How a texture's edge meets another's in a baked tile ([`crate::generator`]'s soft paint): where two meet, broken wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// A sharp line: laid surfaces (cobbles, flagstones, floors, planks).
    Crisp,
    /// A sharp line ragged by noise and by the textures' own light and dark (gravel poking into snow): natural ground.
    Broken,
}

impl Edge {
    /// The catalog's `edge` ("crisp" or "broken"); else what the category's name suggests ([`edge_for`]).
    pub fn from_entry(entry: &serde_json::Value) -> Edge {
        match entry["edge"].as_str() {
            Some("crisp") => Edge::Crisp,
            Some("broken") => Edge::Broken,
            _ => edge_for(entry["category"].as_str().unwrap_or("")),
        }
    }

    /// Its name in the catalog.
    pub fn name(self) -> &'static str {
        match self {
            Edge::Crisp => "crisp",
            Edge::Broken => "broken",
        }
    }
}

/// The edge a category's name suggests: crisp for laid and built surfaces, broken for the rest.
pub fn edge_for(category: &str) -> Edge {
    let c = category.to_ascii_lowercase();
    let built = ["floor", "wall", "cobble", "flag", "paving", "plank", "wood", "brick", "tile", "slab", "carpet", "rug", "metal"];
    if built.iter().any(|w| c.contains(w)) { Edge::Crisp } else { Edge::Broken }
}

/// An import's tile pictures (rows from the south), `width` tiles a row.
pub struct TilePictures {
    /// Tiles a row.
    pub width: usize,
    /// Each tile's ground picture, row by row from the south.
    pub pictures: Vec<Rgba>,
    /// Each tile's cliff texture (a tileset texture the import copied).
    pub cliffs: Vec<Option<String>>,
}

/// The palette id of an imported area's ground.
pub const IMPORT: &str = "@import:";

/// The folder of the area being exported or checked (its header's), set by the export as it loads it ([`set_area_folder`]).
static AREA_FOLDER: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Says which folder the area being exported or checked is in, for [`import_dir`] (None: none).
pub fn set_area_folder(folder: Option<PathBuf>) {
    *AREA_FOLDER.write().unwrap_or_else(std::sync::PoisonError::into_inner) = folder;
}

/// Where an import keeps its original, pictures and objects: `exports/<name>.import`. On your own machine, an area exported
/// from a folder of its own has its import beside it, when it is there: `landscaper import <source> <name> <dir>` writes
/// `<dir>/<name>.json` and `<dir>/<name>.import/`, and `landscaper export <dir>/<name>.json` reads its models from there. (Not
/// on a team server, whose areas are only ever in the builder's own exports folder: the folder is the last export's, which may
/// be another builder's.)
pub fn import_dir(name: &str) -> PathBuf {
    // An import's name can come from a request (`@import:<name>` in an area's paint or objects). One that isn't a plain name
    // (`../`, `/`...) gets a folder that never exists, so it can't reach another folder or another person's workspace.
    let name = if crate::workspace::plain_name(name) { name } else { ".not-a-name" };
    let file = format!("{name}.import");
    if !crate::workspace::team()
        && let Some(beside) = AREA_FOLDER.read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map(|d| d.join(&file))
        && beside.is_dir()
    {
        return beside;
    }
    crate::workspace::exports_dir().join(file)
}

impl Texture {
    /// Bilinear colour (0..1) at texture coordinates, repeating.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        self.sample_level(u, v, 0)
    }

    /// Bilinear colour (0..1) at texture coordinates, repeating, from the picture halved `level` times (as far as it goes).
    fn sample_level(&self, u: f32, v: f32, level: usize) -> [f32; 3] {
        let image = match level.min(self.smaller.len()) {
            0 => &self.image,
            l => &self.smaller[l - 1],
        };
        let (w, h) = (image.width as usize, image.height as usize);
        let fx = u.rem_euclid(1.0) * w as f32 - 0.5;
        let fy = v.rem_euclid(1.0) * h as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let wrap = |a: f32, n: usize| (a as i64).rem_euclid(n as i64) as usize;
        let (x0, x1, y0, y1) = (wrap(x0, w), wrap(x0 + 1.0, w), wrap(y0, h), wrap(y0 + 1.0, h));
        let px = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            let d = &image.data;
            [f32::from(d[i]), f32::from(d[i + 1]), f32::from(d[i + 2])]
        };
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        let mut out = [0.0; 3];
        for k in 0..3 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            out[k] = (top + (bottom - top) * ty) / 255.0;
        }
        out
    }

    /// An import's picture drawn at area point (x, y), and its tile; None for a library texture.
    pub fn picture_at(&self, x: f32, y: f32) -> Option<(&Rgba, usize, usize)> {
        let t = self.tiles.as_ref()?;
        let rows = t.pictures.len() / t.width.max(1);
        let (tx, ty) = (((x / 10.0) as usize).min(t.width - 1), ((y / 10.0) as usize).min(rows.max(1) - 1));
        Some((&t.pictures[ty * t.width + tx], tx, ty))
    }

    /// Colour at area point `p` (x, y and height) of ground facing `n`: a library texture drawn from three sides ([`triplanar`]), as
    /// the game's shader and the editor draw it; an import's picture lies over its tile as [`Texture::ground`] has it. For points
    /// drawn `spacing` metres apart (0: full detail), a library texture is read from the halving whose pixels are about that far
    /// apart, so detail finer than the points (gravel, grass blades) blends instead of glittering.
    pub fn at(&self, p: [f32; 3], n: [f32; 3], spacing: f32) -> [f32; 3] {
        if self.tiles.is_some() {
            return self.ground(p[0], p[1]);
        }
        // (Whole halvings only, rounded down: points at least as close as the picture's own pixels read the picture itself.)
        let texels = spacing * self.image.width as f32 / self.metres;
        let level = if texels >= 2.0 { texels.log2().floor() as usize } else { 0 };
        let w = triplanar::weights(n);
        let mut c = [0.0; 3];
        for (axis, w) in w.into_iter().enumerate().filter(|(_, w)| *w > 0.0) {
            let [u, v] = triplanar::uv(axis, p, self.metres);
            let s = if self.vary > 0.0 {
                triplanar::varied([u, v], self.vary, |u, v| self.sample_level(u, v, level))
            } else {
                self.sample_level(u, v, level)
            };
            for k in 0..3 {
                c[k] += s[k] * w;
            }
        }
        if self.vary > 0.0 {
            let shade = triplanar::shade(p);
            c = c.map(|v| v * shade);
        }
        c
    }

    /// How it repeats ([`Repeat`]).
    pub fn repeat(&self) -> Repeat {
        Repeat { metres: self.metres, vary: self.vary }
    }

    /// Colour at area point (x, y) on level ground.
    pub fn ground(&self, x: f32, y: f32) -> [f32; 3] {
        let Some((p, tx, ty)) = self.picture_at(x, y) else { return self.sample(x / self.metres, y / self.metres) };
        let (w, h) = (p.width as usize, p.height as usize);
        let fx = ((x - tx as f32 * 10.0) / 10.0 * w as f32 - 0.5).clamp(0.0, w as f32 - 1.0);
        let fy = ((y - ty as f32 * 10.0) / 10.0 * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let px = |x: usize, y: usize, c: usize| f32::from(p.data[(y * w + x) * 4 + c]);
        [0, 1, 2].map(|c| {
            let top = px(x0, y0, c) * (1.0 - ax) + px(x1, y0, c) * ax;
            let bottom = px(x0, y1, c) * (1.0 - ax) + px(x1, y1, c) * ax;
            (top * (1.0 - ay) + bottom * ay) / 255.0
        })
    }
}

/// Why an area painted with `missing` (texture ids the library hasn't got) can't be exported. The editor refuses first, in its
/// own words with each one's share of the ground (`js/missing.js`); this is for an export from the command line or an older editor.
pub fn missing_message(missing: &[&str]) -> String {
    let (n, list) = (missing.len(), missing.join(", "));
    let textures = if n == 1 { "1 texture".to_string() } else { format!("{n} textures") };
    format!(
        "This area is painted with {textures} the texture library doesn't have: {list}. Open it in the editor to add {} or swap {} for others.",
        if n == 1 { "it" } else { "them" },
        if n == 1 { "it" } else { "them" }
    )
}

/// Loads the library textures `ids` (their 512 px render copies).
pub fn load_library(ids: &[String]) -> Result<HashMap<String, Texture>> {
    let dir = texture_dir();
    let catalog: serde_json::Value = serde_json::from_str(
        &crate::atomic::read_to_string(&dir.join("catalog.json"))
            .with_context(|| format!("no texture library in {} (run `landscaper textures init`)", dir.display()))?,
    )?;
    let entries = catalog["textures"].as_array().context("catalog.json: textures")?;
    // All of them at once, as the editor says when the area opens: one at a time took an export per missing texture to find.
    let mut missing: Vec<&str> = Vec::new();
    for id in ids.iter().filter(|id| !id.starts_with(IMPORT) && !entries.iter().any(|t| t["id"] == id.as_str())) {
        if !missing.contains(&id.as_str()) {
            missing.push(id);
        }
    }
    anyhow::ensure!(missing.is_empty(), "{}", missing_message(&missing));
    let mut out = HashMap::new();
    for id in ids {
        if out.contains_key(id) {
            continue;
        }
        if let Some(name) = id.strip_prefix(IMPORT) {
            out.insert(id.clone(), load_import(name)?);
            continue;
        }
        let entry = entries.iter().find(|t| t["id"] == id.as_str()).with_context(|| format!("texture '{id}' is not in the library"))?;
        let file = entry["render"].as_str().or(entry["file"].as_str()).context("catalog entry without a file")?;
        let image = read_png(&dir.join(file))?;
        let mut sum = [0f64; 3];
        for px in image.data.as_chunks::<4>().0 {
            for k in 0..3 {
                sum[k] += f64::from(px[k]);
            }
        }
        let count = f64::from(image.width * image.height) * 255.0;
        let mean = [(sum[0] / count) as f32, (sum[1] / count) as f32, (sum[2] / count) as f32];
        let surface = entry["surface"].as_u64().unwrap_or(3) as u32;
        let smaller = std::sync::Arc::new(halvings(&image));
        out.insert(
            id.clone(),
            Texture {
                image,
                smaller,
                mean,
                surface,
                edge: Edge::from_entry(entry),
                tiles: None,
                metres: metres_of(entry),
                vary: Repeat::of(entry).vary,
            },
        );
    }
    Ok(out)
}

/// `img` halved again and again, down to 1 pixel.
fn halvings(img: &Rgba) -> Vec<Rgba> {
    let mut out: Vec<Rgba> = Vec::new();
    loop {
        let last = out.last().unwrap_or(img);
        if last.width <= 1 && last.height <= 1 {
            return out;
        }
        let next = half(last);
        out.push(next);
    }
}

/// An import's pictures as a texture (its mean colour over them all).
fn load_import(name: &str) -> Result<Texture> {
    let dir = import_dir(name);
    let mut paths = Vec::new();
    let (mut width, mut sum, mut count) = (0, [0f64; 3], 0f64);
    for ty in 0.. {
        if !dir.join(format!("00{ty:02}.png")).exists() {
            break;
        }
        for tx in 0.. {
            let p = dir.join(format!("{tx:02}{ty:02}.png"));
            if !p.exists() {
                if ty == 0 {
                    width = tx;
                }
                break;
            }
            paths.push(p);
        }
    }
    // Read side by side, each with its channels' sums (whole numbers, so adding them up after is exact).
    let read = crate::generator::in_parallel(&paths.iter().collect::<Vec<_>>(), |p| -> Result<(Rgba, [u64; 3])> {
        let img = read_png(p)?;
        let mut sums = [0u64; 3];
        for px in img.data.as_chunks::<4>().0 {
            for k in 0..3 {
                sums[k] += u64::from(px[k]);
            }
        }
        Ok((img, sums))
    });
    let mut pictures = Vec::with_capacity(read.len());
    for r in read {
        let (img, sums) = r?;
        for k in 0..3 {
            sum[k] += sums[k] as f64;
        }
        count += f64::from(img.width * img.height) * 255.0;
        pictures.push(img);
    }
    anyhow::ensure!(width > 0 && !pictures.is_empty(), "no imported pictures in {}", dir.display());
    let mean = [(sum[0] / count) as f32, (sum[1] / count) as f32, (sum[2] / count) as f32];
    let image = pictures[0].clone();
    let cliffs = std::fs::read(dir.join("cliffs.json")).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
    Ok(Texture {
        image,
        smaller: std::sync::Arc::default(),
        mean,
        surface: 3,
        edge: Edge::Crisp,
        tiles: Some(std::sync::Arc::new(TilePictures { width, pictures, cliffs })),
        metres: TEXTURE_METRES,
        vary: 0.0,
    })
}

/// An 8-bit RGB(A) PNG, rows bottom first (its alpha kept).
pub fn read_png(path: &Path) -> Result<Rgba> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        other => bail!("{}: unsupported PNG colour type {other:?}", path.display()),
    };
    let (w, h) = (info.width as usize, info.height as usize);
    let mut image = Rgba::new(info.width, info.height);
    for y in 0..h {
        // Bottom row first.
        let src = &buf[(h - 1 - y) * info.line_size..][..w * channels];
        for x in 0..w {
            let p = &src[x * channels..][..channels];
            let rgb = if channels >= 3 { [p[0], p[1], p[2]] } else { [p[0]; 3] };
            let a = match channels {
                4 => p[3],
                2 => p[1],
                _ => 255,
            };
            image.data[(y * w + x) * 4..][..4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
        }
    }
    Ok(image)
}

/// A PNG of a picture (rows bottom first, written top first).
pub fn write_png(path: &Path, img: &Rgba) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?);
    let mut encoder = png::Encoder::new(file, img.width, img.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&img.top_down().data)?;
    Ok(())
}

/// A picture from colours (0..1) by stored row, bottom first.
pub fn picture(width: u32, height: u32, colour: impl Fn(u32, u32) -> [f32; 3]) -> Rgba {
    let mut img = Rgba::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let c = colour(x, y);
            let i = ((y * width + x) * 4) as usize;
            for k in 0..3 {
                img.data[i + k] = (c[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            img.data[i + 3] = 255;
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_missing_textures_are_named_together() {
        assert_eq!(
            missing_message(&["sandFine"]),
            "This area is painted with 1 texture the texture library doesn't have: sandFine. Open it in the editor to add it or swap it for others."
        );
        assert!(
            missing_message(&["sandFine", "snowPacked"]).contains(
                "2 textures the texture library doesn't have: sandFine, snowPacked. Open it in the editor to add them or swap them"
            )
        );
    }

    #[test]
    fn an_areas_import_is_found_beside_it_when_it_was_imported_into_a_folder_of_its_own() {
        let dir = std::env::temp_dir().join(format!("ls-import-beside-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lsbesidea.import")).unwrap();
        let default = |n: &str| crate::workspace::exports_dir().join(format!("{n}.import"));
        set_area_folder(Some(dir.clone()));
        // `landscaper import ... lsbesidea <dir>` then `landscaper export <dir>/lsbesidea.json`: its models are beside it.
        assert_eq!(import_dir("lsbesidea"), dir.join("lsbesidea.import"));
        // An import that isn't there (another area's, referred to by this one) is still the exports folder's, and a name that
        // isn't plain never reaches a folder.
        assert_eq!(import_dir("lsbesideb"), default("lsbesideb"));
        assert_eq!(import_dir("../lsbesidea"), default(".not-a-name"));
        set_area_folder(None);
        assert_eq!(import_dir("lsbesidea"), default("lsbesidea"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dds_round_trips_through_moonglow() {
        let img = picture(64, 32, |x, y| [x as f32 / 63.0, y as f32 / 31.0, 0.5]);
        let tex = mg_image::read(mg_core::ResType::DDS, &dds(&img)).unwrap();
        assert_eq!((tex.width, tex.height, tex.mips.len()), (64, 32, 7));
        let back = tex.to_rgba();
        let worst = img.data.iter().zip(&back.data).map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs()).max().unwrap();
        assert!(worst <= 24, "DXT1 error {worst}");
        // Rows keep their order: the first stored row is still the dark one.
        assert!(back.pixel(0, 0)[1] < 40 && back.pixel(0, 31)[1] > 200);
    }

    #[test]
    fn the_catalog_s_edge_wins_over_the_category_s() {
        use serde_json::json;
        assert_eq!(Edge::from_entry(&json!({"category": "Floors"})), Edge::Crisp);
        assert_eq!(Edge::from_entry(&json!({"category": "Earth"})), Edge::Broken);
        assert_eq!(Edge::from_entry(&json!({"category": "Earth", "edge": "crisp"})), Edge::Crisp);
        assert_eq!(Edge::from_entry(&json!({"category": "Walls", "edge": "broken"})), Edge::Broken);
        // (A value it doesn't know: the category's.)
        assert_eq!(Edge::from_entry(&json!({"category": "Cobbles", "edge": "soft"})), Edge::Crisp);
        for (c, e) in [("Grass", Edge::Broken), ("Snow", Edge::Broken), ("Rock", Edge::Broken), ("Wooden planks", Edge::Crisp)] {
            assert_eq!(edge_for(c), e, "{c}");
        }
    }

    #[test]
    fn natural_ground_is_varied_and_laid_surfaces_are_not_unless_the_catalog_says() {
        let r = |v: serde_json::Value| Repeat::of(&v);
        assert_eq!(r(serde_json::json!({"category": "Grass"})).vary, triplanar::VARY_CELLS);
        assert_eq!(r(serde_json::json!({"category": "Earth", "repeats": 2})).vary, 2.0 * triplanar::VARY_CELLS);
        assert_eq!(r(serde_json::json!({"category": "Floors"})).vary, 0.0, "cobbles and planks keep their pattern");
        assert_eq!(r(serde_json::json!({"edge": "crisp"})).vary, 0.0);
        assert_eq!(r(serde_json::json!({"category": "Grass", "vary": false})).vary, 0.0);
        assert_eq!(r(serde_json::json!({"category": "Floors", "vary": true, "metres": 8})), Repeat { metres: 8.0, vary: 1.5 });
    }

    #[test]
    fn points_far_apart_read_a_texture_halved_so_fine_detail_blends() {
        // Black and white stripes a pixel wide: any point of the picture itself is near black or near white.
        let image = picture(512, 512, |x, _| if x % 2 == 0 { [0.0; 3] } else { [1.0; 3] });
        let t = Texture {
            smaller: std::sync::Arc::new(halvings(&image)),
            image,
            mean: [0.5; 3],
            surface: 3,
            edge: Edge::Broken,
            tiles: None,
            metres: TEXTURE_METRES,
            vary: 0.0,
        };
        assert_eq!(t.smaller.len(), 9);
        let up = [0.0, 0.0, 1.0];
        // (A quarter of a pixel in: bilinear between two stripes, still mostly one of them.)
        let at = |x: f32, spacing: f32| t.at([x * TEXTURE_METRES / 512.0, 0.3, 0.0], up, spacing)[0];
        let px = TEXTURE_METRES / 512.0;
        assert!(at(0.75, 0.0) < 0.3 && at(1.75, 0.0) > 0.7, "full detail");
        // Points as close as the pixels (a 512 px bake's) still read the picture itself; 4 pixels apart (a 128 px bake's)
        // read it halved twice, where the stripes are grey.
        assert!(at(0.75, px * 1.25) < 0.3);
        assert!((at(0.75, px * 5.0) - 0.5).abs() < 0.02 && (at(1.75, px * 5.0) - 0.5).abs() < 0.02);
    }

    #[test]
    fn texture_resrefs_are_lower_case_and_fit_16_characters() {
        assert_eq!(resref("autumnLeaves"), "ls_autumnleaves");
        assert!(resref("aVeryLongTextureName").len() <= 16);
    }
}
