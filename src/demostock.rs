// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The game's stock placeables for the web demo (docs/DESIGN-web-demo.md, "The stock placeables"): each placeable model, its
//! walkmesh and its textures as the game has them, the tables and shaders the engine reads with them, and the Place panel's
//! lists as the engine gives them. The page fetches a model's files when an area or a panel first needs it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use mg_core::ResType;

/// What the page puts in the engine's `override` before anything else: the tables a model's footprint and an object-mode
/// placeable read, and the three shaders an export's tiles are made from.
const START: [(&str, ResType); 5] = [
    ("surfacemat", ResType::TWODA),
    ("placeables", ResType::TWODA),
    ("fslit", ResType::SHD),
    ("vslit", ResType::SHD),
    ("inc_lighting", ResType::SHD),
];

/// Writes the stock placeables of the game (`NWN_ROOT`, no haks) into `out`: `res/` (the files), `index.json` (each model's
/// files, each texture's file, the files to load at start), `placeables.json` and `placeable-groups.json` (the answers of
/// `/api/placeables` and `/api/placeable-groups`). Prints the count and size.
pub fn write(out: &Path) -> Result<()> {
    let game = crate::models::game()?;
    anyhow::ensure!(crate::models::editor_haks().list.is_empty(), "the demo's placeables are the game's own: unset LANDSCAPER_HAKS");
    let res = out.join("res");
    std::fs::create_dir_all(&res)?;
    let mut bytes = 0u64;
    let mut put = |name: &str, t: ResType, data: &[u8]| -> Result<String> {
        let file = format!("{name}.{}", t.extension().context("a type with no extension")?);
        std::fs::write(res.join(&file), data).with_context(|| format!("writing {file}"))?;
        bytes += data.len() as u64;
        Ok(file)
    };
    let mut start = Vec::new();
    for (name, t) in START {
        start.push(put(name, t, &game.resman.get_named(name, t).with_context(|| format!("the game has no {name}"))?)?);
    }
    let list = crate::models::placeables(game)?;
    let mut models: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut textures: BTreeMap<String, String> = BTreeMap::new();
    let mut unread = BTreeSet::new();
    let wanted: BTreeSet<&str> = list.iter().filter(|p| p.source.is_some()).map(|p| p.model.as_str()).collect();
    for model in wanted {
        let Ok(flat) = nwn_bake::flatten(game, model) else {
            unread.insert(model.to_string());
            continue;
        };
        let mut files = vec![put(model, ResType::MDL, &game.resman.get_named(model, ResType::MDL)?)?];
        if let Ok(pwk) = game.resman.get_named(model, ResType::PWK) {
            files.push(put(model, ResType::PWK, &pwk)?);
        }
        for (texture, _, _) in &flat.groups {
            if let Some(file) = textures.get(texture) {
                files.push(file.clone());
            } else if let Some((t, data)) = nwn_bake::texture::image(game, texture) {
                let file = put(texture, t, &data)?;
                textures.insert(texture.clone(), file.clone());
                files.push(file);
            }
        }
        files.dedup();
        models.insert(model.to_string(), files);
    }
    let index = serde_json::json!({ "start": start, "models": models, "textures": textures });
    std::fs::write(out.join("index.json"), serde_json::to_vec(&index)?)?;
    std::fs::write(out.join("placeables.json"), serde_json::to_vec(&serde_json::json!({ "placeables": &*list }))?)?;
    let plants = crate::plants::plants(game)?;
    let groups = serde_json::json!({ "large_from": crate::plants::LARGE_FROM, "max_width": crate::plants::MAX_WIDTH, "plants": &*plants });
    std::fs::write(out.join("placeable-groups.json"), serde_json::to_vec(&groups)?)?;
    println!(
        "{}: {} models, {} textures, {:.1} MB{}",
        out.display(),
        models.len(),
        textures.len(),
        bytes as f64 / 1e6,
        if unread.is_empty() { String::new() } else { format!("; not read: {}", unread.into_iter().collect::<Vec<_>>().join(", ")) }
    );
    Ok(())
}
