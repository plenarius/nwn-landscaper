// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Preset skies, the first sky pack: a sky for each Sky & fog preset that shows one (docs/PRESET-SKIES.md). Five re-use Real
//! Skies' pictures in other times of day, so they are made from your own Real Skies download, as `real-skies` makes its hak;
//! Thick mist and Swamp are gradients painted here from their presets' colours.
//!
//! ```text
//! cargo run --release --example preset-skies -- <Real_Skies.hak, or the folder it was unpacked into> <folder>
//! landscaper skies pack <folder>/skies.json --out dist
//! ```

#[path = "real_skies_common.rs"]
mod common;
#[path = "../src/dds.rs"]
mod dds;

use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use mg_erf::Erf;
use mg_image::Rgba;
use serde_json::json;

use common::{CREDIT, NWSYNC_LIMIT, bitmaps, find_hak, panorama, rename_model, resource};

/// What every file of the pack starts with.
const PREFIX: &str = "psk";

/// Real Skies' pictures the pack re-uses: the Real Skies model that draws with the picture, the pack's name for its model and
/// texture, and whether its colours are flattened (Overcast's, from Grass_Storm). The picture is the one texture that model names.
const PICTURES: [(&str, &str, bool); 11] = [
    ("skyd_001", "psk_grass_dusk", false),
    ("skyn_001", "psk_grass_night", false),
    ("dskyda_001", "psk_desert_dawn", false),
    ("dskyn_001", "psk_desert_night", false),
    ("wskyda_001", "psk_winter_dawn", false),
    ("wskyd_001", "psk_winter_dusk", false),
    ("wskyn_001", "psk_winter_night", false),
    ("sskyda_001", "psk_overcast_da", true),
    ("ssky_001", "psk_overcast_d", true),
    ("sskyd_001", "psk_overcast_du", true),
    ("sskyn_001", "psk_overcast_n", true),
];

/// A generated sky: its id and the colours of its preset (`js/atmosphere.js` in nwn-landscaper-web), the sun's and the moon's
/// fog and diffuse, as `0xrrggbb`.
struct Gradient {
    id: &'static str,
    sun_fog: u32,
    sun_diffuse: u32,
    moon_fog: u32,
    moon_diffuse: u32,
}

const GRADIENTS: [Gradient; 2] = [
    Gradient { id: "mist", sun_fog: 0xc8c8c8, sun_diffuse: 0xb0b0b0, moon_fog: 0x303030, moon_diffuse: 0x404040 },
    Gradient { id: "swamp", sun_fog: 0x6b844a, sun_diffuse: 0xb4c594, moon_fog: 0x183117, moon_diffuse: 0x425238 },
];

/// The pack's skies: id, name, the preset it suits, and its models at dawn, by day, at dusk and by night.
const SKIES: [(&str, &str, &str, [&str; 4]); 7] = [
    ("golden-haze", "Golden haze", "golden", ["psk_desert_dawn", "psk_desert_dawn", "psk_grass_dusk", "psk_desert_night"]),
    ("dusk", "Dusk", "dusk", ["psk_grass_dusk", "psk_grass_dusk", "psk_grass_dusk", "psk_grass_night"]),
    ("overcast", "Overcast", "overcast", ["psk_overcast_da", "psk_overcast_d", "psk_overcast_du", "psk_overcast_n"]),
    ("starry-night", "Starry night", "night", ["psk_desert_night", "psk_desert_night", "psk_desert_night", "psk_desert_night"]),
    ("dawn-haze", "Dawn haze", "dawn", ["psk_desert_dawn", "psk_winter_dawn", "psk_winter_dusk", "psk_winter_night"]),
    ("thick-mist", "Thick mist", "mist", ["psk_mist_twi", "psk_mist_day", "psk_mist_twi", "psk_mist_night"]),
    ("swamp", "Swamp", "swamp", ["psk_swamp_twi", "psk_swamp_day", "psk_swamp_twi", "psk_swamp_night"]),
];

/// Makes the Preset skies pack's files and its list for `landscaper skies pack`: Golden haze, Dusk, Overcast, Starry night and
/// Dawn haze from Real Skies' pictures, Thick mist and Swamp painted from their presets' colours.
#[derive(Parser)]
#[command(name = "preset-skies", after_help = CREDIT)]
struct Args {
    /// `Real_Skies.hak`, or a folder that holds it (the unpacked download)
    input: PathBuf,
    /// The folder the files and `skies.json` go in
    output: PathBuf,
    /// Pictures re-encoded at once (each takes about 150 MB of memory)
    #[arg(long, default_value_t = 4)]
    jobs: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let input = find_hak(&args.input)?;
    println!("reading {}", input.display());
    let bytes = std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
    let erf = Erf::read(&bytes).with_context(|| format!("{} is not a hak", input.display()))?;
    std::fs::create_dir_all(&args.output).with_context(|| format!("making {}", args.output.display()))?;
    let files = build(&erf, &args.output, args.jobs.max(1))?;
    let list = args.output.join("skies.json");
    std::fs::write(&list, serde_json::to_string_pretty(&pack_list(&files))? + "\n")?;
    println!("wrote {} and {} files. Next: landscaper skies pack {} --out <folder>", list.display(), files.len(), list.display());
    println!("{CREDIT}");
    Ok(())
}

/// Writes every model and texture into `out`, and returns their file names.
fn build(erf: &Erf<'_>, out: &Path, jobs: usize) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let mut write = |name: String, data: &[u8]| -> Result<()> {
        ensure!(data.len() <= NWSYNC_LIMIT, "{name} is {} bytes, over NWSync's limit", data.len());
        std::fs::write(out.join(&name), data).with_context(|| format!("writing {name}"))?;
        files.push(name);
        Ok(())
    };
    let mut pictures = Vec::new();
    for (model, name, flat) in PICTURES {
        let text = String::from_utf8_lossy(&resource(erf, model, mg_core::ResType::MDL)?).into_owned();
        let [texture] = bitmaps(&text).try_into().map_err(|_| anyhow::anyhow!("{model}.mdl draws with more than one texture"))?;
        write(format!("{name}.mdl"), retexture(&rename_model(&text, name)?, name).as_bytes())?;
        pictures.push((texture, name, flat));
    }
    for g in &GRADIENTS {
        let sun = [colour(g.sun_fog), colour(g.sun_diffuse)];
        let moon = [colour(g.moon_fog), colour(g.moon_diffuse)];
        let twilight = [mix(sun[0], moon[0], 0.5), mix(sun[1], moon[1], 0.5)];
        for (time, [fog, diffuse]) in [("day", sun), ("twi", twilight), ("night", moon)] {
            let name = format!("{PREFIX}_{}_{time}", g.id);
            write(format!("{name}.mdl"), dome(&name).as_bytes())?;
            write(format!("{name}.dds"), &dds::dds(&gradient(fog, diffuse)))?;
        }
    }

    // Real Skies' pictures are the slow part: a few at once.
    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::new());
    std::thread::scope(|s| -> Result<()> {
        let workers: Vec<_> = (0..jobs.min(pictures.len()))
            .map(|_| {
                s.spawn(|| -> Result<()> {
                    while let Some((texture, name, flat)) = pictures.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let mut picture = panorama(erf, texture)?;
                        if *flat {
                            flatten(&mut picture);
                        }
                        let dds = dds::dds(&picture);
                        println!("{name}: from {texture}{}, {:.1} MB", if *flat { ", flattened" } else { "" }, dds.len() as f64 / 1e6);
                        std::fs::write(out.join(format!("{name}.dds")), &dds).with_context(|| format!("writing {name}.dds"))?;
                        done.lock().expect("no worker panicked").push(format!("{name}.dds"));
                    }
                    Ok(())
                })
            })
            .collect();
        for w in workers {
            w.join().expect("a worker panicked")?;
        }
        Ok(())
    })?;
    files.extend(done.into_inner().expect("no worker panicked"));
    files.sort();
    Ok(files)
}

/// The list `landscaper skies pack` takes (docs/SKY-PACKS.md, "Making a pack").
fn pack_list(files: &[String]) -> serde_json::Value {
    let skies: Vec<_> = SKIES
        .iter()
        .map(|(id, name, preset, m)| {
            json!({ "id": id, "name": name, "preset": preset, "models": { "dawn": m[0], "day": m[1], "dusk": m[2], "night": m[3] } })
        })
        .collect();
    json!({
        "id": "preset-skies",
        "version": "1.0.2",
        "name": "Preset skies",
        "description": "A sky for each Sky & fog preset that shows one. Golden haze, Dusk, Overcast, Starry night and Dawn haze \
            re-use the pictures of Savant1974's Real Skies; Thick mist and Swamp are painted from their presets' colours.",
        "licence": "Real Skies' pictures: Savant1974's. Thick mist and Swamp: GPL-3.0-or-later",
        "credits": ["Savant1974: Real Skies", "The Landscaper: Thick mist and Swamp"],
        "homepage": "https://neverwintervault.org/project/nwnee/hakpak/real-skies-hak-version",
        "prefix": PREFIX,
        "files": files,
        "skies": skies,
    })
}

/// An ASCII model drawing with `bitmap` instead of its own textures.
fn retexture(model: &str, bitmap: &str) -> String {
    model
        .lines()
        .map(|l| {
            let indent = &l[..l.len() - l.trim_start().len()];
            if l.split_whitespace().next() == Some("bitmap") { format!("{indent}bitmap {bitmap}\n") } else { format!("{l}\n") }
        })
        .collect()
}

/// `0xrrggbb` as 0 to 255 a channel.
fn colour(rgb: u32) -> [f32; 3] {
    [(rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255].map(|c| c as f32)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// Where the horizon is on a generated sky's texture (0 the bottom, 1 the top): below it, the fog colour.
const HORIZON: f32 = 0.25;

/// A generated sky's texture: the fog colour up to the horizon, easing into the sky's top colour overhead. The top is the
/// preset's diffuse colour half greyed (full, it paints a green sky over the swamp), or the fog colour a little lighter where the
/// diffuse is darker than the fog (thick mist by day), so the sky is never darker overhead than at the horizon. It leaves the
/// horizon flat: a steep start drew a line along it. 16 by 256 px: it changes only with height.
fn gradient(fog: [f32; 3], diffuse: [f32; 3]) -> Rgba {
    let top = if luma(diffuse) > luma(fog) { mix([luma(diffuse); 3], diffuse, 0.5) } else { mix(fog, [255.0; 3], 0.12) };
    let (w, h) = (16, 256);
    let mut img = Rgba::new(w, h);
    for y in 0..h {
        // Rows bottom first, so row 0 is the bottom of the texture (v = 0).
        let v = (y as f32 + 0.5) / h as f32;
        let t = ((v - HORIZON) / (1.0 - HORIZON)).clamp(0.0, 1.0);
        let c = mix(fog, top, t * t * (3.0 - 2.0 * t));
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                img.data[i + k] = (c[k] + 0.5).clamp(0.0, 255.0) as u8;
            }
            img.data[i + 3] = 255;
        }
    }
    img
}

/// Overcast from a storm picture: its colours a quarter as strong and its contrast two thirds, around its mean brightness, so
/// the sky reads as a flat grey cover over the preset's flat grey light.
fn flatten(img: &mut Rgba) {
    let n = (img.width * img.height) as usize;
    let mean = img.data.as_chunks::<4>().0.iter().map(|p| luma([p[0], p[1], p[2]].map(f32::from))).sum::<f32>() / n as f32;
    for p in img.data.as_chunks_mut::<4>().0 {
        let c = [p[0], p[1], p[2]].map(f32::from);
        let l = luma(c);
        let grey = mix([l; 3], c, 0.25);
        for k in 0..3 {
            p[k] = (mean + (grey[k] - mean) * 0.67 + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
}

/// The rings of a generated sky's dome, from the bottom: height and distance out in metres, and the texture's height there.
fn rings() -> Vec<(f32, f32, f32)> {
    const R: f32 = 600.0;
    let mut r = vec![(-300.0, 560.0, 0.0), (0.0, R, HORIZON)];
    for deg in [8.0f32, 18.0, 32.0, 48.0, 64.0, 78.0] {
        let e = deg.to_radians();
        r.push((R * e.sin(), R * e.cos(), HORIZON + (1.0 - HORIZON) * e / FRAC_PI_2));
    }
    r
}

/// Segments around a generated sky's dome.
const AROUND: usize = 24;

/// A generated sky's model: a dome 600 m out, from 300 m below the horizon to a cap overhead, drawn from inside with texture
/// `name`. Its own mesh, so the generated skies need nothing of Real Skies'.
fn dome(name: &str) -> String {
    let rings = rings();
    let mut verts = Vec::new();
    for &(z, r, _) in &rings {
        for i in 0..AROUND {
            let a = TAU * i as f32 / AROUND as f32;
            verts.push([r * a.cos(), r * a.sin(), z]);
        }
    }
    let top = verts.len();
    verts.push([0.0, 0.0, 600.0]);
    // One texture vertex a ring (and the top): the texture changes only with height.
    let mut tverts: Vec<f32> = rings.iter().map(|r| r.2).collect();
    tverts.push(1.0);
    let mut faces = Vec::new();
    for k in 0..rings.len() - 1 {
        for i in 0..AROUND {
            let j = (i + 1) % AROUND;
            let (li, lj, ui, uj) = (k * AROUND + i, k * AROUND + j, (k + 1) * AROUND + i, (k + 1) * AROUND + j);
            faces.push(([li, ui, uj], [k, k + 1, k + 1]));
            faces.push(([uj, lj, li], [k + 1, k, k]));
        }
    }
    let last = rings.len() - 1;
    for i in 0..AROUND {
        let j = (i + 1) % AROUND;
        faces.push(([last * AROUND + i, top, last * AROUND + j], [last, rings.len(), last]));
    }
    let mut m = String::new();
    let _ = write!(
        m,
        "# Preset skies: a generated sky (scripts/preset-skies.rs)\nnewmodel {name}\nsetsupermodel {name} NULL\nclassification EFFECT\n\
         setanimationscale 1.0\nbeginmodelgeom {name}\nnode dummy {name}\n  parent null\nendnode\nnode trimesh dome\n  parent {name}\n  \
         position 0 0 0\n  orientation 0 0 0 0\n  ambient 1 1 1\n  diffuse 1 1 1\n  specular 0 0 0\n  shininess 1\n  \
         selfillumcolor 0 0 0\n  bitmap {name}\n  render 1\n  shadow 0\n  beaming 0\n  inheritcolor 0\n  transparencyhint 0\n"
    );
    let _ = writeln!(m, "  verts {}", verts.len());
    for v in &verts {
        let _ = writeln!(m, "    {:.3} {:.3} {:.3}", v[0], v[1], v[2]);
    }
    let _ = writeln!(m, "  faces {}", faces.len());
    for (v, t) in &faces {
        let _ = writeln!(m, "    {} {} {} 1 {} {} {} 0", v[0], v[1], v[2], t[0], t[1], t[2]);
    }
    let _ = writeln!(m, "  tverts {}", tverts.len());
    for v in &tverts {
        let _ = writeln!(m, "    0.5 {v:.4} 0");
    }
    let _ = write!(m, "endnode\nendmodelgeom {name}\ndonemodel {name}\n");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sky_draws_with_the_packs_own_models_named_as_the_game_allows() {
        let made: Vec<String> = PICTURES
            .iter()
            .map(|p| p.1.to_string())
            .chain(GRADIENTS.iter().flat_map(|g| ["day", "twi", "night"].map(|t| format!("{PREFIX}_{}_{t}", g.id))))
            .collect();
        for name in &made {
            assert!(name.len() <= 16 && name.starts_with(PREFIX), "{name}");
        }
        for (id, _, _, models) in SKIES {
            for m in models {
                assert!(made.iter().any(|n| n == m), "{id}: {m} is not made");
            }
        }
        for name in &made {
            assert!(SKIES.iter().any(|s| s.3.contains(&name.as_str())), "{name} is made but no sky uses it");
        }
    }

    #[test]
    fn a_gradient_is_the_fog_colour_to_the_horizon_and_lighter_overhead() {
        for g in &GRADIENTS {
            for (fog, diffuse) in [(g.sun_fog, g.sun_diffuse), (g.moon_fog, g.moon_diffuse)] {
                let img = gradient(colour(fog), colour(diffuse));
                let at = |y: u32| img.pixel(0, y);
                let f = colour(fog).map(|c| c as u8);
                assert_eq!(at(0)[..3], f, "{}", g.id);
                assert_eq!(at((HORIZON * 256.0) as u32 - 1)[..3], f, "{}", g.id);
                let px = |p: [u8; 4]| luma([p[0], p[1], p[2]].map(f32::from));
                assert!(px(at(255)) > px(at(0)), "{}: darker overhead", g.id);
                assert!((1..256).all(|y| px(at(y)) >= px(at(y - 1)) - 0.5), "{}: not rising", g.id);
            }
        }
        // Swamp by day goes up to its diffuse colour, half greyed.
        let top = gradient(colour(0x6b844a), colour(0xb4c594)).pixel(3, 255);
        assert!(top[..3].iter().zip([183u8, 192, 167]).all(|(a, b)| a.abs_diff(b) <= 1), "{top:?}");
    }

    #[test]
    fn the_dome_faces_inwards_and_its_texture_rises_with_it() {
        let m = dome("psk_mist_day");
        let section = |head: &str| -> Vec<Vec<f32>> {
            let mut lines = m.lines().skip_while(|l| !l.trim_start().starts_with(head));
            let n: usize = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
            lines.take(n).map(|l| l.split_whitespace().map(|w| w.parse().unwrap()).collect()).collect()
        };
        let (verts, faces, tverts) = (section("verts"), section("faces"), section("tverts"));
        assert_eq!(faces.len(), (rings().len() - 1) * AROUND * 2 + AROUND);
        for f in &faces {
            let p = |i: usize| -> [f32; 3] {
                let v = &verts[f[i] as usize];
                [v[0], v[1], v[2]]
            };
            let (a, b, c) = (p(0), p(1), p(2));
            let (u, w) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| c[k] - a[k]));
            let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
            let mid = [0, 1, 2].map(|k| (a[k] + b[k] + c[k]) / 3.0);
            // Towards the axis, where the camera is.
            assert!(n[0] * -mid[0] + n[1] * -mid[1] + n[2] * (100.0 - mid[2]) > 0.0, "a face looks outwards: {f:?}");
            // Higher vertices take higher texture rows.
            for (i, j) in [(0, 1), (1, 2), (0, 2)] {
                let (zi, zj) = (p(i)[2], p(j)[2]);
                let (vi, vj) = (tverts[f[4 + i] as usize][1], tverts[f[4 + j] as usize][1]);
                assert!((zi - zj).abs() < 1e-3 || (zi < zj) == (vi < vj), "{f:?}");
            }
        }
        assert!(m.starts_with("# Preset skies") && m.contains("\nnewmodel psk_mist_day\n") && m.contains("  bitmap psk_mist_day\n"));
    }

    #[test]
    fn a_real_skies_model_takes_the_packs_texture() {
        let real = "newmodel Sky_001\nnode trimesh Cylinder01\n  parent Sky_001\n  bitmap tmp_skydusk\nendnode\ndonemodel Sky_001\n";
        let m = retexture(&rename_model(real, "psk_grass_dusk").unwrap(), "psk_grass_dusk");
        assert_eq!(bitmaps(&m), ["psk_grass_dusk"]);
        assert!(m.contains("newmodel psk_grass_dusk\n") && !m.contains("Sky_001") && !m.contains("tmp_"));
    }

    #[test]
    fn flattening_greys_the_colours_and_keeps_the_mean() {
        let mut img = Rgba { width: 2, height: 1, data: vec![200, 60, 40, 255, 40, 60, 200, 255] };
        let before = luma([200.0, 60.0, 40.0]);
        flatten(&mut img);
        let (a, b) = (img.pixel(0, 0), img.pixel(1, 0));
        let spread = |p: [u8; 4]| i32::from(p[0].max(p[1]).max(p[2])) - i32::from(p[0].min(p[1]).min(p[2]));
        assert!(spread(a) < 160 / 3 && spread(b) < 160 / 3, "{a:?} {b:?}");
        assert!((luma([a[0], a[1], a[2]].map(f32::from)) - before).abs() < 30.0);
    }

    #[test]
    fn the_list_names_every_file_and_sky() {
        let files = vec!["psk_grass_dusk.dds".to_string()];
        let l = pack_list(&files);
        assert_eq!(l["prefix"], PREFIX);
        assert_eq!(l["files"][0], "psk_grass_dusk.dds");
        assert_eq!(l["skies"].as_array().unwrap().len(), SKIES.len());
        assert_eq!(l["skies"][3]["models"]["dusk"], "psk_desert_night");
        assert!(l["licence"].as_str().unwrap().len() <= 200);
    }
}
