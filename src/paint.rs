// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Painted textures: the editor's texture library (filled by textures.rs) and what the hak needs from it: textures as BioWare DXT1
//! DDS, sampled the way the editor shows them.
//!
//! Pictures are kept as [`mg_image::Rgba`], rows bottom first as the game stores them, so texture coordinate v = 0 is the first stored
//! row; the library PNGs (top row first) are flipped on load. A texture repeats every [`TEXTURE_METRES`] on the ground, north being
//! the picture's up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mg_image::Rgba;

/// Metres one texture repeat covers on the ground (the editor's too).
pub const TEXTURE_METRES: f32 = 4.0;

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
    /// Average colour (0..1), for minimaps.
    pub mean: [f32; 3],
    /// surfacemat.2da row of walkable ground painted with it (footsteps).
    pub surface: u32,
    /// An imported area's ground (`@import:<name>`): a picture per tile, drawn over its square instead of repeating.
    pub tiles: Option<std::sync::Arc<TilePictures>>,
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
        let (w, h) = (self.image.width as usize, self.image.height as usize);
        let fx = u.rem_euclid(1.0) * w as f32 - 0.5;
        let fy = v.rem_euclid(1.0) * h as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let wrap = |a: f32, n: usize| (a as i64).rem_euclid(n as i64) as usize;
        let (x0, x1, y0, y1) = (wrap(x0, w), wrap(x0 + 1.0, w), wrap(y0, h), wrap(y0 + 1.0, h));
        let px = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            let d = &self.image.data;
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

    /// Colour at area point (x, y) on level ground.
    pub fn ground(&self, x: f32, y: f32) -> [f32; 3] {
        let Some(t) = &self.tiles else { return self.sample(x / TEXTURE_METRES, y / TEXTURE_METRES) };
        let rows = t.pictures.len() / t.width.max(1);
        let (tx, ty) = (((x / 10.0) as usize).min(t.width - 1), ((y / 10.0) as usize).min(rows.max(1) - 1));
        let p = &t.pictures[ty * t.width + tx];
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

/// Loads the library textures `ids` (their 512 px render copies).
pub fn load_library(ids: &[String]) -> Result<HashMap<String, Texture>> {
    let dir = texture_dir();
    let catalog: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("catalog.json"))
            .with_context(|| format!("no texture library in {} (run `landscaper textures init`)", dir.display()))?,
    )?;
    let entries = catalog["textures"].as_array().context("catalog.json: textures")?;
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
        out.insert(id.clone(), Texture { image, mean, surface, tiles: None });
    }
    Ok(out)
}

/// An import's pictures as a texture (its mean colour over them all).
fn load_import(name: &str) -> Result<Texture> {
    let dir = import_dir(name);
    let mut pictures = Vec::new();
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
            let img = read_png(&p)?;
            for px in img.data.as_chunks::<4>().0 {
                for k in 0..3 {
                    sum[k] += f64::from(px[k]);
                }
            }
            count += f64::from(img.width * img.height) * 255.0;
            pictures.push(img);
        }
    }
    anyhow::ensure!(width > 0 && !pictures.is_empty(), "no imported pictures in {}", dir.display());
    let mean = [(sum[0] / count) as f32, (sum[1] / count) as f32, (sum[2] / count) as f32];
    let image = pictures[0].clone();
    let cliffs = std::fs::read(dir.join("cliffs.json")).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
    Ok(Texture { image, mean, surface: 3, tiles: Some(std::sync::Arc::new(TilePictures { width, pictures, cliffs })) })
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

/// Half the size (box filter), at least 1 x 1.
pub(crate) fn half(img: &Rgba) -> Rgba {
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
fn dxt1_block(px: &[[f32; 3]; 16]) -> [u8; 8] {
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
    fn texture_resrefs_are_lower_case_and_fit_16_characters() {
        assert_eq!(resref("autumnLeaves"), "ls_autumnleaves");
        assert!(resref("aVeryLongTextureName").len() <= 16);
    }
}
