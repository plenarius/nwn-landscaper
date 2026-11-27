// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the Real Skies scripts share: finding Real Skies' hak, reading its models and pictures, and scaling a panorama down.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mg_core::{ResRef, ResType};
use mg_erf::Erf;
use mg_image::Rgba;

/// The size of every sky texture made from Real Skies: under NWSync's limit, and within what older graphics cards accept.
pub const WIDTH: u32 = 8192;
pub const HEIGHT: u32 = 1024;

/// `nwn_nwsync_write --limit-file-size`'s default: 15 MiB, and a file of more is refused.
pub const NWSYNC_LIMIT: usize = 15 * 1024 * 1024;

pub const CREDIT: &str = "Real Skies is Savant1974's: download its EE hak version (real_skies_1.02.7z) from \
https://neverwintervault.org/project/nwnee/hakpak/real-skies-hak-version and unpack it with 7-Zip. Its pictures come from free \
wallpapers, blended and stitched together; keep this credit with what you make from it.";

/// The hak itself, or the one in a folder (the unpacked download keeps it in `patch/`).
pub fn find_hak(input: &Path) -> Result<PathBuf> {
    if input.extension().is_some_and(|e| e.eq_ignore_ascii_case("7z")) {
        bail!("{}: unpack the download first (7-Zip), then give its folder", input.display());
    }
    if !input.is_dir() {
        return Ok(input.to_path_buf());
    }
    let mut stack = vec![input.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("real_skies.hak")) {
                return Ok(path);
            }
        }
    }
    bail!("no Real_Skies.hak in {}", input.display())
}

/// Resource `name` of type `kind` from Real Skies' hak.
pub fn resource(erf: &Erf<'_>, name: &str, kind: ResType) -> Result<Vec<u8>> {
    let resref = ResRef::from_str(name)?;
    let entry = erf
        .entries
        .iter()
        .find(|e| e.restype == kind && e.resref.to_lowercase() == resref)
        .with_context(|| format!("{name}.{kind} is not in the hak: is it Real Skies 1.02, the EE hak version?"))?;
    Ok(erf.data(entry)?.into_owned())
}

/// Real Skies' panorama `name` at [`WIDTH`] by [`HEIGHT`] (rows bottom first, as the DDS encoder takes them).
pub fn panorama(erf: &Erf<'_>, name: &str) -> Result<Rgba> {
    let tga = resource(erf, name, ResType::TGA)?;
    let picture = mg_image::tga::read(&tga).with_context(|| format!("reading {name}.tga"))?.to_rgba();
    Ok(shrink(&picture, WIDTH, HEIGHT))
}

/// An ASCII model with its model name (in `newmodel`, `setsupermodel`, its root node and the nodes' `parent`s) set to `name`, and
/// Windows line ends made plain.
pub fn rename_model(text: &str, name: &str) -> Result<String> {
    let old = text
        .lines()
        .find_map(|l| {
            let mut t = l.split_whitespace();
            (t.next().is_some_and(|w| w.eq_ignore_ascii_case("newmodel"))).then(|| t.next()).flatten()
        })
        .context("not an ASCII model (no newmodel line)")?;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let indent = &line[..line.len() - line.trim_start().len()];
        let words: Vec<&str> = line.split_whitespace().map(|w| if w.eq_ignore_ascii_case(old) { name } else { w }).collect();
        out.push_str(indent);
        out.push_str(&words.join(" "));
        out.push('\n');
    }
    Ok(out)
}

/// The textures an ASCII model names, lower case.
pub fn bitmaps(model: &str) -> Vec<String> {
    model
        .lines()
        .filter_map(|l| {
            let mut t = l.split_whitespace();
            (t.next() == Some("bitmap")).then(|| t.next()).flatten().map(str::to_ascii_lowercase)
        })
        .collect()
}

/// For each target pixel along one side: the source pixels it covers and how much of each.
fn spans(from: u32, to: u32) -> Vec<Vec<(usize, f32)>> {
    let scale = f64::from(from) / f64::from(to);
    (0..to)
        .map(|i| {
            let (a, b) = (f64::from(i) * scale, f64::from(i + 1) * scale);
            let first = a.floor() as usize;
            let last = (b.ceil() as usize).min(from as usize);
            (first..last).map(|j| (j, ((b.min(j as f64 + 1.0) - a.max(j as f64)) / scale) as f32)).collect()
        })
        .collect()
}

/// The picture at `width` by `height`, each new pixel the average of the area it covers (rows stay bottom first).
pub fn shrink(img: &Rgba, width: u32, height: u32) -> Rgba {
    let (across, down) = (spans(img.width, width), spans(img.height, height));
    let row = img.width as usize * 4;
    let mut out = Rgba::new(width, height);
    let mut line = vec![0f32; row];
    for (y, rows) in down.iter().enumerate() {
        line.fill(0.0);
        for &(sy, wy) in rows {
            for (l, p) in line.iter_mut().zip(&img.data[sy * row..][..row]) {
                *l += f32::from(*p) * wy;
            }
        }
        for (x, cols) in across.iter().enumerate() {
            let mut sum = [0f32; 4];
            for &(sx, wx) in cols {
                for k in 0..4 {
                    sum[k] += line[sx * 4 + k] * wx;
                }
            }
            let at = (y * width as usize + x) * 4;
            for k in 0..4 {
                out.data[at + k] = (sum[k] + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinking_averages_the_area_each_pixel_covers() {
        let mut img = Rgba::new(7, 2);
        for (i, p) in img.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.copy_from_slice(&[if i % 7 < 3 { 0 } else { 210 }, 77, if i < 7 { 0 } else { 100 }, 255]);
        }
        let out = shrink(&img, 2, 1);
        // Left pixel: 3 of its 3.5 columns are black. Rows (stored bottom first) average into one.
        assert_eq!(out.pixel(0, 0), [30, 77, 50, 255]);
        assert_eq!(out.pixel(1, 0), [210, 77, 50, 255]);
        // An exact half is the plain box filter.
        let stripes = Rgba { width: 4, height: 2, data: [[0, 0, 0, 255], [200, 100, 50, 255]].repeat(4).concat() };
        assert_eq!(shrink(&stripes, 2, 1), crate::dds::half(&stripes));
    }

    #[test]
    fn spans_cover_the_whole_source_once() {
        for (from, to) in [(14_384, 8192), (1798, 1024), (16_384, 8192)] {
            let mut cover = vec![0f64; from as usize];
            for s in spans(from, to) {
                for (j, w) in s {
                    cover[j] += f64::from(w) * f64::from(from) / f64::from(to);
                }
            }
            assert!(cover.iter().all(|c| (c - 1.0).abs() < 1e-4), "{from} to {to}");
        }
    }

    #[test]
    fn a_model_takes_its_files_name_and_loses_windows_line_ends() {
        let real = "# Exported\r\nnewmodel Sky_001\r\nsetsupermodel Sky_001 NULL\r\nbeginmodelgeom Sky_001\r\nnode dummy Sky_001\r\n  \
                    parent null\r\nendnode\r\nnode trimesh Cylinder01\r\n  parent Sky_001\r\n  bitmap Dst_SkyDawn\r\nendnode\r\n\
                    endmodelgeom Sky_001\r\ndonemodel Sky_001\r\n";
        let m = rename_model(real, "iskyda_001").unwrap();
        assert!(!m.contains('\r') && !m.contains("Sky_001"));
        assert!(m.contains("newmodel iskyda_001\nsetsupermodel iskyda_001 NULL\n"));
        assert!(m.contains("node dummy iskyda_001\n  parent null\n") && m.contains("  parent iskyda_001\n"));
        assert!(m.ends_with("endmodelgeom iskyda_001\ndonemodel iskyda_001\n"));
        assert_eq!(bitmaps(&m), ["dst_skydawn"]);
        assert!(rename_model("node dummy x\n", "a").is_err());
    }
}
