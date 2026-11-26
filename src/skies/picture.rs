// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pictures of a sky for the editor. The game draws a sky's model around the camera, so a picture is the model's textured
//! triangles seen from its origin, laid out by direction: a column per sixth of a degree of bearing (column 0 looks east, and
//! they turn north from there), a row per sixth of a degree of height, the zenith at the top. The Sky & fog panel shows a
//! strip of the horizon ([`Kind::Strip`]); the 3D view wraps the whole panorama around its camera ([`Kind::Panorama`]).
//! Where no triangle is, the picture is clear: the game shows its fog colour there.
//!
//! A picture is made once for a model and the bytes of its textures, and kept in the sky library's `pictures` folder: an
//! 8,192 px DDS takes about a second to read and project, the kept PNG a few milliseconds.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use mg_core::ResType;
use mg_image::Rgba;
use sha2::Digest;

/// Pixels a degree, across and up.
const PER_DEGREE: usize = 6;
/// The panorama's top and bottom, in degrees above the horizon: the zenith, and as far below the horizon as the view's camera
/// looks over an area.
const TOP: f32 = 90.0;
const BOTTOM: f32 = -30.0;
/// The panorama's size: 360° across.
pub const WIDTH: usize = 360 * PER_DEGREE;
pub const HEIGHT: usize = (TOP - BOTTOM) as usize * PER_DEGREE;
/// The strip: from 25° above the horizon to 5° below it, where every sky has its picture (Real Skies' band reaches 30°), at a
/// third of the panorama's size (720 by 60 px).
const STRIP_TOP: f32 = 25.0;
const STRIP_BOTTOM: f32 = -5.0;
const STRIP_SCALE: usize = 3;
/// Changes with the way pictures are made, so the kept ones are made again.
const MADE_BY: &[u8] = b"sky picture 1";

/// Which picture of a sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The horizon: 720 by 60 px.
    Strip,
    /// All around: [`WIDTH`] by [`HEIGHT`] px.
    Panorama,
}

impl Kind {
    /// `strip` or `panorama`.
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "strip" => Some(Kind::Strip),
            "panorama" => Some(Kind::Panorama),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Strip => "strip",
            Kind::Panorama => "panorama",
        }
    }
}

/// Where a sky's model and textures are read from: the sky library's files first, then the game's (when there is one).
pub struct Files<'a> {
    /// The sky library: `<model>.mdl` and `<texture>.dds`.
    pub library: &'a Path,
    /// The game with the module's haks.
    pub game: Option<&'a mg_rules::GameData>,
}

impl Files<'_> {
    fn model(&self, name: &str) -> Result<Vec<u8>> {
        if let Ok(d) = std::fs::read(self.library.join(format!("{name}.mdl"))) {
            return Ok(d);
        }
        let game = self.game.with_context(|| format!("no model {name} in the sky library, and the game isn't open"))?;
        Ok(game.resman.get_named(name, ResType::MDL).with_context(|| format!("no model {name}"))?.into_owned())
    }

    fn texture(&self, name: &str) -> Result<(ResType, Vec<u8>)> {
        if let Ok(d) = std::fs::read(self.library.join(format!("{name}.dds"))) {
            return Ok((ResType::DDS, d));
        }
        let game = self.game.with_context(|| format!("no texture {name} in the sky library, and the game isn't open"))?;
        let (t, d) = nwn_bake::texture::image(game, name).with_context(|| format!("no texture {name}"))?;
        Ok((t, d.into_owned()))
    }
}

/// Whether `name` can be a model's or texture's resref (and so a file name in the library).
fn resref_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The picture `kind` of the sky model `model` as PNG, kept in `kept` (made there first if it isn't).
pub fn picture(files: &Files<'_>, model: &str, kind: Kind, kept: &Path) -> Result<Vec<u8>> {
    let model = model.trim().to_ascii_lowercase();
    ensure!(resref_ok(&model), "{model} isn't a model's name");
    let bytes = files.model(&model)?;
    let mdl = mg_mdl::Model::read(&bytes).map_err(|e| anyhow::anyhow!("{model}: not a model ({e})"))?;
    // The textures, read once each, and the key: the model's and textures' bytes.
    let mut key = sha2::Sha256::new();
    key.update(MADE_BY);
    key.update(&bytes);
    let mut textures = BTreeMap::new();
    for node in &mdl.nodes {
        let Some(name) = node.mesh().and_then(|m| m.textures[0].as_deref()).map(|t| t.trim().to_ascii_lowercase()) else { continue };
        if name.is_empty() || name == "null" || textures.contains_key(&name) || !resref_ok(&name) {
            continue;
        }
        let (t, data) = files.texture(&name)?;
        key.update(name.as_bytes());
        key.update(&data);
        textures.insert(name, (t, data));
    }
    let hex: String = key.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect();
    let file = |k: Kind| kept.join(format!("{hex}-{}.png", k.name()));
    if let Ok(png) = crate::atomic::read(&file(kind)) {
        return Ok(png);
    }
    let mut decoded = BTreeMap::new();
    for (name, (t, data)) in textures {
        let img = mg_image::read(t, &data).map_err(|e| anyhow::anyhow!("{name}: {e}"))?.to_rgba();
        decoded.insert(name, fit(img));
    }
    let pano = panorama(&mdl, &decoded);
    let made = [(Kind::Panorama, png(&pano, WIDTH, HEIGHT)?), (Kind::Strip, png(&strip(&pano), WIDTH / STRIP_SCALE, strip_rows())?)];
    // (Kept for next time; a library that can't be written to still gets its picture.)
    if std::fs::create_dir_all(kept).is_ok() {
        for (k, p) in &made {
            let _ = crate::atomic::write(&file(*k), p);
        }
    }
    Ok(made.into_iter().find_map(|(k, p)| (k == kind).then_some(p)).unwrap_or_default())
}

/// `img` halved until it is no more than twice the panorama's width: a sky's texture goes once around, so a texel then covers
/// about half a pixel and needs no more filtering than [`sample`]'s.
fn fit(mut img: Rgba) -> Rgba {
    while img.width as usize > 2 * WIDTH {
        img = crate::dds::half(&img);
    }
    img
}

/// Rows of the strip.
fn strip_rows() -> usize {
    ((STRIP_TOP - STRIP_BOTTOM) as usize * PER_DEGREE) / STRIP_SCALE
}

/// The model's triangles around its origin: RGBA, rows from the top ([`WIDTH`] by [`HEIGHT`]), clear where none is.
fn panorama(model: &mg_mdl::Model, textures: &BTreeMap<String, Rgba>) -> Vec<u8> {
    let pose = nwn_bake::rest_pose(model);
    let mut out = vec![0u8; WIDTH * HEIGHT * 4];
    for (i, node) in model.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh() else { continue };
        let Some(tex) = mesh.textures[0].as_deref().and_then(|t| textures.get(&t.trim().to_ascii_lowercase())) else { continue };
        if !mesh.render || mesh.vertices.is_empty() {
            continue;
        }
        let corners: Vec<Corner> = mesh
            .vertices
            .iter()
            .enumerate()
            .map(|(k, v)| {
                let p = pose[i].transform_point3(glam::Vec3::from_array(*v)).to_array();
                Corner::at(p, mesh.uvs[0].get(k).copied().unwrap_or([0.0, 0.0]))
            })
            .collect();
        for t in mesh.triangles() {
            let Some(c) = t.map(|v| corners.get(v as usize).copied()).into_iter().collect::<Option<Vec<_>>>() else { continue };
            triangle(&mut out, [c[0], c[1], c[2]], tex);
        }
    }
    out
}

/// A triangle's corner in the panorama: column and row (pixels, not wrapped), its texture coordinates, and whether it is
/// straight above or below the origin (its bearing is any).
#[derive(Clone, Copy, Debug)]
struct Corner {
    x: f32,
    y: f32,
    uv: [f32; 2],
    pole: bool,
}

impl Corner {
    /// The corner at `p` (metres from the origin; x east, y north, z up).
    fn at(p: [f32; 3], uv: [f32; 2]) -> Corner {
        let flat = p[0].hypot(p[1]);
        let height = p[2].atan2(flat).to_degrees();
        let bearing = p[1].atan2(p[0]).to_degrees().rem_euclid(360.0);
        let d = PER_DEGREE as f32;
        Corner { x: bearing * d, y: (TOP - height) * d, uv, pole: flat <= 1e-4 * p[2].abs() }
    }
}

/// Draws a triangle into `out`, its texture `tex`. A triangle across the panorama's seam (east) is drawn whole and wrapped; one
/// with a corner straight overhead is a fan's wedge, drawn up to the top along both its other corners' bearings.
fn triangle(out: &mut [u8], mut c: [Corner; 3], tex: &Rgba) {
    let w = WIDTH as f32;
    let Some(first) = c.iter().position(|k| !k.pole) else { return };
    let base = c[first].x;
    for k in c.iter_mut().filter(|k| !k.pole) {
        if k.x - base > w / 2.0 {
            k.x -= w;
        } else if base - k.x > w / 2.0 {
            k.x += w;
        }
    }
    match c.iter().filter(|k| k.pole).count() {
        0 => fill(out, c, tex),
        1 => {
            let p = c.iter().position(|k| k.pole).unwrap_or(0);
            let (a, b, pole) = (c[(p + 1) % 3], c[(p + 2) % 3], c[p]);
            let a2 = Corner { x: a.x, ..pole };
            let b2 = Corner { x: b.x, ..pole };
            fill(out, [a, b, b2], tex);
            fill(out, [a, b2, a2], tex);
        }
        _ => {}
    }
}

/// Fills a triangle of the panorama, its texture coordinates spread across it.
fn fill(out: &mut [u8], c: [Corner; 3], tex: &Rgba) {
    let area = (c[1].x - c[0].x) * (c[2].y - c[0].y) - (c[2].x - c[0].x) * (c[1].y - c[0].y);
    if area.abs() < 1e-6 {
        return;
    }
    let (x0, x1) = (c.iter().map(|k| k.x).fold(f32::MAX, f32::min), c.iter().map(|k| k.x).fold(f32::MIN, f32::max));
    let (y0, y1) = (c.iter().map(|k| k.y).fold(f32::MAX, f32::min), c.iter().map(|k| k.y).fold(f32::MIN, f32::max));
    let ys = (y0.floor().max(0.0) as usize)..(y1.ceil().min(HEIGHT as f32).max(0.0) as usize);
    for py in ys {
        for px in (x0.floor() as i64)..(x1.ceil() as i64) {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            // Barycentric weights; inside when all three are (allowing for rounding, so neighbours leave no gap).
            let w0 = ((c[1].x - x) * (c[2].y - y) - (c[2].x - x) * (c[1].y - y)) / area;
            let w1 = ((c[2].x - x) * (c[0].y - y) - (c[0].x - x) * (c[2].y - y)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let uv = [0, 1].map(|k| w0 * c[0].uv[k] + w1 * c[1].uv[k] + w2 * c[2].uv[k]);
            let col = px.rem_euclid(WIDTH as i64) as usize;
            let i = (py * WIDTH + col) * 4;
            let s = sample(tex, uv);
            out[i..i + 3].copy_from_slice(&s[..3]);
            out[i + 3] = 255;
        }
    }
}

/// `tex` at `uv` (v up, as the game maps a texture: rows are stored from the bottom), between its four nearest texels; it wraps
/// across and stops at the top and bottom.
fn sample(tex: &Rgba, uv: [f32; 2]) -> [u8; 4] {
    let (w, h) = (i64::from(tex.width), i64::from(tex.height));
    if w == 0 || h == 0 {
        return [0; 4];
    }
    let fx = uv[0] * w as f32 - 0.5;
    let fy = (uv[1] * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let at = |x: i64, y: i64| tex.pixel(x.rem_euclid(w) as u32, y.clamp(0, h - 1) as u32);
    let (a, b, c, d) = (at(ix, iy), at(ix + 1, iy), at(ix, iy + 1), at(ix + 1, iy + 1));
    std::array::from_fn(|k| {
        let top = f32::from(a[k]) * (1.0 - tx) + f32::from(b[k]) * tx;
        let bottom = f32::from(c[k]) * (1.0 - tx) + f32::from(d[k]) * tx;
        (top * (1.0 - ty) + bottom * ty).round() as u8
    })
}

/// The strip from the panorama: its rows from [`STRIP_TOP`] to [`STRIP_BOTTOM`], each [`STRIP_SCALE`] square averaged (the
/// clear pixels left out of the colour).
fn strip(pano: &[u8]) -> Vec<u8> {
    let first = (TOP - STRIP_TOP) as usize * PER_DEGREE;
    let (w, h, s) = (WIDTH / STRIP_SCALE, strip_rows(), STRIP_SCALE);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for dy in 0..s {
                for dx in 0..s {
                    let i = ((first + y * s + dy) * WIDTH + x * s + dx) * 4;
                    let a = u32::from(pano[i + 3]);
                    for k in 0..3 {
                        sum[k] += u32::from(pano[i + k]) * a;
                    }
                    sum[3] += a;
                }
            }
            let o = (y * w + x) * 4;
            for k in 0..3 {
                out[o + k] = (sum[k] + sum[3] / 2).checked_div(sum[3]).unwrap_or(0) as u8;
            }
            out[o + 3] = (sum[3] / (s * s) as u32) as u8;
        }
    }
    out
}

/// RGBA rows from the top as PNG.
fn png(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, width as u32, height as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(rgba)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A band 600 m out from 5.7° below the horizon (-60 m) to 30° above it (346 m), 36 sides, its texture once around: u
    /// from east turning north, v up.
    fn band(name: &str, bitmap: &str) -> String {
        let n = 36;
        let mut m = format!(
            "newmodel {name}\nsetsupermodel {name} NULL\nclassification EFFECT\nbeginmodelgeom {name}\nnode dummy {name}\n  parent null\n\
             endnode\nnode trimesh band\n  parent {name}\n  bitmap {bitmap}\n  render 1\n  verts {}\n",
            2 * (n + 1)
        );
        for i in 0..=n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            m += &format!("    {} {} -60\n    {} {} 346.4\n", 600.0 * a.cos(), 600.0 * a.sin(), 600.0 * a.cos(), 600.0 * a.sin());
        }
        m += &format!("  faces {}\n", 2 * n);
        for i in 0..n {
            let (l, u, l2, u2) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            m += &format!("    {l} {l2} {u2} 1 {l} {l2} {u2} 0\n    {u2} {u} {l} 1 {u2} {u} {l} 0\n");
        }
        m += &format!("  tverts {}\n", 2 * (n + 1));
        for i in 0..=n {
            let u = i as f32 / n as f32;
            m += &format!("    {u} 0 0\n    {u} 1 0\n");
        }
        m + &format!("endnode\nendmodelgeom {name}\ndonemodel {name}\n")
    }

    /// A texture whose west half (u 0.25 to 0.75) is red, the rest blue, with a green bottom row.
    fn texture() -> Rgba {
        let (w, h) = (64, 16);
        let mut t = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let west = (16..48).contains(&x);
                let c = if y == 0 {
                    [0, 255, 0]
                } else if west {
                    [255, 0, 0]
                } else {
                    [0, 0, 255]
                };
                t.data[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        t
    }

    fn px(pano: &[u8], bearing: f32, height: f32) -> [u8; 4] {
        let (x, y) = ((bearing * PER_DEGREE as f32) as usize, ((TOP - height) * PER_DEGREE as f32) as usize);
        let i = (y * WIDTH + x) * 4;
        [pano[i], pano[i + 1], pano[i + 2], pano[i + 3]]
    }

    #[test]
    fn a_band_is_drawn_by_bearing_and_height_and_clear_elsewhere() {
        let model = mg_mdl::Model::read(band("tst_band", "tst_tex").as_bytes()).unwrap();
        let pano = panorama(&model, &BTreeMap::from([("tst_tex".to_string(), texture())]));
        // East and north (u 0 and 0.25): blue; west (u 0.5): red.
        assert_eq!(px(&pano, 10.0, 10.0), [0, 0, 255, 255]);
        assert_eq!(px(&pano, 180.0, 10.0), [255, 0, 0, 255]);
        assert_eq!(px(&pano, 350.0, 20.0), [0, 0, 255, 255]);
        // Across the seam (east) without a gap.
        assert_eq!(px(&pano, 0.0, 10.0)[3], 255);
        assert_eq!(px(&pano, 359.9, 10.0)[3], 255);
        // The bottom row of the texture at the band's foot, below the horizon.
        assert_eq!(px(&pano, 180.0, -5.5)[..3], [0, 255, 0]);
        // Nothing overhead or far below.
        assert_eq!(px(&pano, 90.0, 60.0)[3], 0);
        assert_eq!(px(&pano, 90.0, -20.0)[3], 0);
    }

    #[test]
    fn a_fan_at_the_zenith_covers_the_top() {
        let n = 12;
        let mut m =
            format!("newmodel tst_cap\nbeginmodelgeom tst_cap\nnode trimesh cap\n  parent null\n  bitmap tst_tex\n  verts {}\n", n + 1);
        for i in 0..n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            m += &format!("    {} {} 300\n", 500.0 * a.cos(), 500.0 * a.sin());
        }
        m += &format!("    0 0 600\n  faces {n}\n");
        for i in 0..n {
            m += &format!("    {i} {} {n} 1 0 0 0 0\n", (i + 1) % n);
        }
        m += "  tverts 1\n    0.5 0.5 0\nendnode\nendmodelgeom tst_cap\ndonemodel tst_cap\n";
        let model = mg_mdl::Model::read(m.as_bytes()).unwrap();
        let pano = panorama(&model, &BTreeMap::from([("tst_tex".to_string(), texture())]));
        for bearing in [0.5, 45.0, 170.0, 300.0, 359.5] {
            for height in [89.9, 70.0, 45.0] {
                assert_eq!(px(&pano, bearing, height), [255, 0, 0, 255], "{bearing} {height}");
            }
            assert_eq!(px(&pano, bearing, 25.0)[3], 0, "{bearing}");
        }
    }

    #[test]
    fn pictures_are_made_once_and_kept_by_their_files_bytes() {
        let dir = std::env::temp_dir().join(format!("ls-sky-picture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tst_band.mdl"), band("tst_band", "tst_tex")).unwrap();
        std::fs::write(dir.join("tst_tex.dds"), crate::dds::dds(&texture())).unwrap();
        let files = Files { library: &dir, game: None };
        let kept = dir.join("pictures");
        let strip = picture(&files, "TST_BAND", Kind::Strip, &kept).unwrap();
        let read = |png: &[u8]| {
            let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size()];
            let info = r.next_frame(&mut buf).unwrap();
            (info.width, info.height, buf)
        };
        let (w, h, _) = read(&strip);
        assert_eq!((w, h), (720, 60));
        let (w, h, _) = read(&picture(&files, "tst_band", Kind::Panorama, &kept).unwrap());
        assert_eq!((w as usize, h as usize), (WIDTH, HEIGHT));
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 2, "both made at once");
        // Another texture under the same name: made again.
        let mut other = texture();
        other.data.iter_mut().step_by(4).for_each(|r| *r = 9);
        std::fs::write(dir.join("tst_tex.dds"), crate::dds::dds(&other)).unwrap();
        assert_ne!(picture(&files, "tst_band", Kind::Strip, &kept).unwrap(), strip);
        assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 4);
        assert!(picture(&files, "../x", Kind::Strip, &kept).is_err());
        assert!(picture(&files, "nothere", Kind::Strip, &kept).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
