// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An import's files: the heights and header the editor and export read, the tile and area pictures, the objects' models and
//! ground patches, and the pick map (which texture is where) that [`pick_at`] reads back.

use std::path::Path;

use anyhow::{Context, Result};
use mg_image::Rgba;

use super::{Imported, PICK, Pick, SPACING, TILE, span, threads};
use crate::paint;

/// `pick.json` (the textures, their footsteps and the map's size) and `pick.bin` (the map) beside an import's pictures.
fn write_pick(pick: &Pick, dir: &Path) -> Result<()> {
    std::fs::write(
        dir.join("pick.json"),
        serde_json::to_vec(
            &serde_json::json!({ "per": PICK, "width": pick.width, "height": pick.height, "names": pick.names, "surface": pick.surface }),
        )?,
    )?;
    std::fs::write(dir.join("pick.bin"), &pick.map)?;
    Ok(())
}

/// The tileset texture that covers the ground at (x, y) metres of an import (in its folder `dir`), with the surfacemat row it walks
/// as (0: unknown): the nearest covered sample within half a metre when that one has none (a hole under a rock). An import made
/// before this existed has no map: it has to be imported again.
pub fn pick_at(dir: &Path, x: f32, y: f32) -> Result<(String, u32)> {
    let head: serde_json::Value = std::fs::read(dir.join("pick.json"))
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .context("this import has no texture map: import the area again to pick its textures")?;
    let (per, width, height) = (
        head["per"].as_u64().unwrap_or(0) as usize,
        head["width"].as_u64().unwrap_or(0) as usize,
        head["height"].as_u64().unwrap_or(0) as usize,
    );
    let map = std::fs::read(dir.join("pick.bin")).unwrap_or_default();
    anyhow::ensure!(per > 0 && width > 0 && map.len() == width * height, "this import's texture map is damaged: import the area again");
    let metres = TILE / per as f32;
    let (cx, cy) = ((x / metres).floor() as i64, (y / metres).floor() as i64);
    anyhow::ensure!(x >= 0.0 && y >= 0.0 && cx < width as i64 && cy < height as i64, "that point is outside the area");
    let reach = (0.5 / metres).ceil() as i64;
    // Rings outwards: the nearest sample with a texture.
    let mut best: Option<(i64, u8)> = None;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (i, j) = (cx + dx, cy + dy);
            if i < 0 || j < 0 || i >= width as i64 || j >= height as i64 {
                continue;
            }
            let v = map[j as usize * width + i as usize];
            let d = dx * dx + dy * dy;
            if v != 0 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, v));
            }
        }
    }
    let (_, v) = best.context("no tileset texture is under that spot (it may be hidden under something): try beside it")?;
    let at = v as usize - 1;
    let name = head["names"][at].as_str().context("this import's texture map is damaged: import the area again")?.to_string();
    Ok((name, head["surface"][at].as_u64().unwrap_or(0) as u32))
}

/// Writes the import where the editor and export find it: the heights and a heightmap header in `dir` (as an export's), each tile's
/// picture as `<name>.import/<xx><yy>.png`, and the editor's smaller picture of the whole area as `<name>.import.view.png`.
pub(super) fn write(imp: &Imported, name: &str, dir: &Path) -> Result<()> {
    let (nx, ny) = (imp.width as usize * 16 + 1, imp.height as usize * 16 + 1);
    std::fs::create_dir_all(dir)?;
    let bytes: Vec<u8> = imp.heights.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(dir.join(format!("{name}.f32")), bytes)?;
    std::fs::write(dir.join(format!("{name}.materials")), vec![0u8; nx * ny])?;
    let pics = dir.join(format!("{name}.import"));
    std::fs::create_dir_all(&pics)?;
    // For the editor: at most 128 pixels a tile, 4096 in all.
    let per = (4096 / imp.width.max(imp.height)).clamp(16, 128);
    // Each tile's picture, and the tile shrunk for the editor's.
    let tiles: Vec<usize> = (0..imp.pictures.len()).collect();
    let written = span("  tile pngs", || {
        crate::generator::in_parallel_on(threads(), &tiles, |t| -> Result<Rgba> {
            let (tx, ty) = (t % imp.width as usize, t / imp.width as usize);
            paint::write_png(&pics.join(format!("{tx:02}{ty:02}.png")), &imp.pictures[t])?;
            Ok(shrink(&imp.pictures[t], per, per))
        })
    });
    let mut view = Rgba::new(per * imp.width, per * imp.height);
    let (s, row) = (per as usize, (per * imp.width * 4) as usize);
    for (t, small) in written.into_iter().enumerate() {
        let small = small?;
        let (tx, ty) = (t % imp.width as usize, t / imp.width as usize);
        for y in 0..s {
            let at = (ty * s + y) * row + tx * s * 4;
            view.data[at..at + s * 4].copy_from_slice(&small.data[y * s * 4..(y + 1) * s * 4]);
        }
    }
    span("  view png", || paint::write_png(&dir.join(format!("{name}.import.view.png")), &view))?;
    // What stands on each tile, as models to bake (objects the editor can
    // select, move and delete like any other), and their ground patches.
    let ids: Vec<usize> = (0..imp.objects.len()).collect();
    let written = span("  objects", || {
        crate::generator::in_parallel_on(threads(), &ids, |id| -> Result<()> {
            let o = &imp.objects[id];
            std::fs::write(pics.join(format!("obj_{}.json", o.key)), serde_json::to_vec(&o.flat)?)?;
            if let (Some(d), Some(decal)) = (&o.decal, &o.flat.decal) {
                paint::write_png(&pics.join(&decal.file), d)?;
            }
            Ok(())
        })
    });
    written.into_iter().collect::<Result<()>>()?;
    let mut objects = Vec::new();
    for (id, o) in imp.objects.iter().enumerate() {
        objects.push(serde_json::json!({
            "model": format!("{}{name}:{}", paint::IMPORT, o.key), "x": o.x, "y": o.y, "z": o.z,
            "bearing": 0.0, "scale": 1.0, "solid": true, "baked": true, "id": id + 1,
        }));
    }
    std::fs::write(pics.join("cliffs.json"), serde_json::to_vec(&imp.cliffs)?)?;
    write_pick(&imp.pick, &pics)?;
    let header = serde_json::json!({
        "name": name, "tiles": imp.width, "tilesY": imp.height, "spacing": SPACING, "width": nx, "height": ny,
        "data": format!("{name}.f32"), "materials": format!("{name}.materials"),
        "palette": [format!("{}{name}", paint::IMPORT)],
        "kind": imp.kind,
        "objects": objects,
        "atmosphere": imp.atmosphere,
        "settings": imp.settings,
        "imported": true,
        "water": imp
            .water
            .iter()
            .map(|b| serde_json::json!({ "i": b.i, "j": b.j, "level": b.level, "bounds": b.bounds }))
            .collect::<Vec<_>>(),
        "import": { "tileset": imp.tileset, "pictures": format!("{name}.import") },
    });
    std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(&header)?)?;
    Ok(())
}

/// A picture scaled down to w by h (box filter).
fn shrink(img: &Rgba, w: u32, h: u32) -> Rgba {
    let mut out = Rgba::new(w, h);
    let (fx, fy) = (img.width / w, img.height / h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for dy in 0..fy {
                for dx in 0..fx {
                    let k = (((y * fy + dy) * img.width + x * fx + dx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += u32::from(img.data[k + c]);
                    }
                }
            }
            let k = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                out.data[k + c] = (sum[c] / (fx * fy)) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spot_finds_the_texture_under_it_or_the_nearest_beside_it() {
        // Two tiles wide, one high: the left tile grass (1), the right stone (2) but for a hole (0) at its middle.
        let (width, height) = (2 * PICK, PICK);
        let mut map = vec![1u8; width * height];
        for j in 0..height {
            for i in PICK..width {
                map[j * width + i] = 2;
            }
        }
        let mid = (PICK / 2) * width + PICK + PICK / 2;
        map[mid] = 0;
        let pick = Pick { names: vec!["tcn01_grass01".into(), "tcn01_rock01".into()], surface: vec![3, 4], map, width, height };
        let dir = std::env::temp_dir().join(format!("ls-pick-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_pick(&pick, &dir).unwrap();
        assert_eq!(pick_at(&dir, 2.0, 3.0).unwrap(), ("tcn01_grass01".to_string(), 3));
        assert_eq!(pick_at(&dir, 15.0, 5.0).unwrap(), ("tcn01_rock01".to_string(), 4));
        // The hole: the sample beside it.
        let m = TILE / PICK as f32;
        assert_eq!(pick_at(&dir, 10.0 + (PICK / 2) as f32 * m + 0.01, (PICK / 2) as f32 * m + 0.01).unwrap().0, "tcn01_rock01");
        // Off the area, and an import with no map.
        assert!(pick_at(&dir, -1.0, 2.0).is_err());
        assert!(pick_at(&dir, 25.0, 2.0).is_err());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(pick_at(&dir, 2.0, 2.0).unwrap_err().to_string().contains("import the area again"));
    }
}
