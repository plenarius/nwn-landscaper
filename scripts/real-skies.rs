// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Real Skies for modules: turns the EE hak version of Savant1974's Real Skies into one hak a module can serve through NWSync.
//! A step outside the Landscaper (docs/REAL-SKIES.md); the pictures are Real Skies', so this repository ships none of them.
//!
//! ```text
//! cargo run --release --example real-skies -- <Real_Skies.hak, or the folder it was unpacked into> [output.hak]
//! ```
//!
//! - Every texture becomes a DXT1 DDS of 8,192 by 1,024 with its mipmaps, about 5.6 MB: NWSync refuses files over 15 MB, and
//!   Real Skies' 24-bit TGAs are 77 MB and more.
//! - Icy, which Real Skies covers by day only, gets Winter_Clear's dawn, dusk and night.
//! - Each model is named after its file: Real Skies names most of them `Sky_001` inside, whatever their file is called.
//! - The models replace the stock ones by name, so `skyboxes.2da` stays as it is.

#[path = "../src/dds.rs"]
mod dds;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use mg_core::{ResRef, ResType};
use mg_erf::{Description, Erf, ErfWriter};
use mg_image::Rgba;

/// The size of every sky texture: under NWSync's limit, and within what older graphics cards accept.
const WIDTH: u32 = 8192;
const HEIGHT: u32 = 1024;

/// `nwn_nwsync_write --limit-file-size`'s default: 15 MiB, and a file of more is refused.
const NWSYNC_LIMIT: usize = 15 * 1024 * 1024;

/// The models Real Skies 1.02 replaces: the stock rows Grass_Clear, Grass_Storm, Desert_Clear, Winter_Clear (dawn, day, dusk,
/// night) and Icy (day).
const MODELS: [&str; 17] = [
    "skyda_001",
    "sky_001",
    "skyd_001",
    "skyn_001",
    "sskyda_001",
    "ssky_001",
    "sskyd_001",
    "sskyn_001",
    "dskyda_001",
    "dsky_001",
    "dskyd_001",
    "dskyn_001",
    "wskyda_001",
    "wsky_001",
    "wskyd_001",
    "wskyn_001",
    "isky_001",
];

/// Icy's times Real Skies leaves stock, and the Winter_Clear model each copies.
const ICY_FILL: [(&str, &str); 3] = [("iskyda_001", "wskyda_001"), ("iskyd_001", "wskyd_001"), ("iskyn_001", "wskyn_001")];

const CREDIT: &str = "Real Skies is Savant1974's: download its EE hak version (real_skies_1.02.7z) from \
https://neverwintervault.org/project/nwnee/hakpak/real-skies-hak-version and unpack it with 7-Zip. Its pictures come from free \
wallpapers, blended and stitched together; keep this credit with the hak.";

/// Turns Savant1974's Real Skies (EE hak version) into one hak a module can serve through NWSync: every sky texture as a DXT1 DDS
/// of 8,192 by 1,024 (about 5.6 MB, under NWSync's 15 MB), and Icy's dawn, dusk and night from Winter_Clear.
#[derive(Parser)]
#[command(name = "real-skies", after_help = CREDIT)]
struct Args {
    /// `Real_Skies.hak`, or a folder that holds it (the unpacked download)
    input: PathBuf,
    /// The hak to write
    #[arg(default_value = "realskies_dds.hak")]
    output: PathBuf,
    /// Textures re-encoded at once (each takes about 150 MB of memory)
    #[arg(long, default_value_t = 4)]
    jobs: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let input = find_hak(&args.input)?;
    let name = args.output.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    ensure!(
        args.output.extension().is_some_and(|e| e.eq_ignore_ascii_case("hak")) && ResRef::from_str(name).is_ok(),
        "{}: a hak's name is at most 16 letters, digits and underscores, ending in .hak",
        args.output.display()
    );
    println!("reading {}", input.display());
    let bytes = std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
    let erf = Erf::read(&bytes).with_context(|| format!("{} is not a hak", input.display()))?;
    let hak = build(&erf, args.jobs.max(1))?;
    let tmp = args.output.with_extension("hak.part");
    std::fs::write(&tmp, &hak).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &args.output).with_context(|| format!("writing {}", args.output.display()))?;
    println!("wrote {} ({:.1} MB). {CREDIT}", args.output.display(), hak.len() as f64 / 1e6);
    Ok(())
}

/// The hak itself, or the one in a folder (the unpacked download keeps it in `patch/`).
fn find_hak(input: &Path) -> Result<PathBuf> {
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

/// The new hak's bytes from Real Skies' hak.
fn build(erf: &Erf<'_>, jobs: usize) -> Result<Vec<u8>> {
    let resource = |name: &str, kind: ResType| -> Result<Vec<u8>> {
        let resref = ResRef::from_str(name)?;
        let entry = erf
            .entries
            .iter()
            .find(|e| e.restype == kind && e.resref.to_lowercase() == resref)
            .with_context(|| format!("{name}.{kind} is not in the hak: is it Real Skies 1.02, the EE hak version?"))?;
        Ok(erf.data(entry)?.into_owned())
    };
    let mut out: Vec<(ResRef, ResType, Vec<u8>)> = Vec::new();
    let mut textures: Vec<String> = Vec::new();
    for name in MODELS {
        let text = String::from_utf8_lossy(&resource(name, ResType::MDL)?).into_owned();
        let model = rename_model(&text, name)?;
        for bitmap in bitmaps(&model) {
            if !textures.contains(&bitmap) {
                textures.push(bitmap);
            }
        }
        out.push((ResRef::from_str(name)?, ResType::MDL, model.into_bytes()));
    }
    for (name, from) in ICY_FILL {
        let text = String::from_utf8_lossy(&resource(from, ResType::MDL)?).into_owned();
        out.push((ResRef::from_str(name)?, ResType::MDL, rename_model(&text, name)?.into_bytes()));
    }

    // The textures are the slow part: a few at once.
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(ResRef, ResType, Vec<u8>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| -> Result<()> {
        let workers: Vec<_> = (0..jobs.min(textures.len()))
            .map(|_| {
                s.spawn(|| -> Result<()> {
                    loop {
                        let Some(name) = textures.get(next.fetch_add(1, Ordering::Relaxed)) else { return Ok(()) };
                        let tga = resource(name, ResType::TGA)?;
                        let picture = mg_image::tga::read(&tga).with_context(|| format!("reading {name}.tga"))?.to_rgba();
                        let dds = dds::dds(&shrink(&picture, WIDTH, HEIGHT));
                        ensure!(dds.len() <= NWSYNC_LIMIT, "{name}.dds is {} bytes, over NWSync's limit", dds.len());
                        println!(
                            "{name}: {} by {} to {WIDTH} by {HEIGHT}, {:.1} MB",
                            picture.width,
                            picture.height,
                            dds.len() as f64 / 1e6
                        );
                        done.lock().expect("no worker panicked").push((ResRef::from_str(name)?, ResType::DDS, dds));
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().expect("a worker panicked")?;
        }
        Ok(())
    })?;
    let mut done = done.into_inner().expect("no worker panicked");
    done.sort_by_key(|(r, _, _)| r.to_string_lossy());
    out.extend(done);

    let mut hak = ErfWriter::new(*b"HAK ");
    hak.description = Description { strings: vec![(0, CREDIT.as_bytes().to_vec())], ..Description::default() };
    for (resref, kind, data) in out {
        hak.add(resref, kind, data)?;
    }
    Ok(hak.to_bytes()?)
}

/// An ASCII model with its model name (in `newmodel`, `setsupermodel`, its root node and the nodes' `parent`s) set to `name`, and
/// Windows line ends made plain.
fn rename_model(text: &str, name: &str) -> Result<String> {
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
fn bitmaps(model: &str) -> Vec<String> {
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
fn shrink(img: &Rgba, width: u32, height: u32) -> Rgba {
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
    fn a_sky_texture_fits_nwsync_at_the_size_chosen() {
        // A DXT1 level is a 4 by 4 block in 8 bytes, down to 1 by 1, after BioWare's 20-byte header.
        let size = |mut w: u32, mut h: u32| {
            let mut n = 20;
            loop {
                n += (w.div_ceil(4) * h.div_ceil(4) * 8) as usize;
                if w == 1 && h == 1 {
                    return n;
                }
                (w, h) = ((w / 2).max(1), (h / 2).max(1));
            }
        };
        assert_eq!(dds::dds(&Rgba::new(64, 8)).len(), size(64, 8));
        assert_eq!(size(WIDTH, HEIGHT), 5_592_476);
        assert!(size(WIDTH, HEIGHT) < NWSYNC_LIMIT);
    }

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
        assert_eq!(shrink(&stripes, 2, 1), dds::half(&stripes));
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
