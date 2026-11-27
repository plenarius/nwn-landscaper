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

#[path = "real_skies_common.rs"]
mod common;
#[path = "../src/dds.rs"]
mod dds;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use mg_core::{ResRef, ResType};
use mg_erf::{Description, Erf, ErfWriter};

use common::{CREDIT, HEIGHT, NWSYNC_LIMIT, WIDTH, bitmaps, find_hak, panorama, rename_model, resource};

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

/// The new hak's bytes from Real Skies' hak.
fn build(erf: &Erf<'_>, jobs: usize) -> Result<Vec<u8>> {
    let resource = |name: &str, kind: ResType| resource(erf, name, kind);
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
                        let dds = dds::dds(&panorama(erf, name)?);
                        ensure!(dds.len() <= NWSYNC_LIMIT, "{name}.dds is {} bytes, over NWSync's limit", dds.len());
                        println!("{name}: {WIDTH} by {HEIGHT}, {:.1} MB", dds.len() as f64 / 1e6);
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
        assert_eq!(dds::dds(&mg_image::Rgba::new(64, 8)).len(), size(64, 8));
        assert_eq!(size(WIDTH, HEIGHT), 5_592_476);
        assert!(size(WIDTH, HEIGHT) < NWSYNC_LIMIT);
    }
}
