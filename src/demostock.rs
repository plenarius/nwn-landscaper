// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The game's stock placeables for the web demo (docs/DESIGN-web-demo.md, "The stock placeables"): each placeable model, its
//! walkmesh and its textures as the game has them, the tables and shaders the engine reads with them, and the Place panel's
//! lists as the engine gives them; and the game's skies, each row of `skyboxes.2da` with its models and textures. The page
//! fetches a model's files when an area or a panel first needs it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use mg_core::ResType;

/// What the page puts in the engine's `override` before anything else: the tables a model's footprint and an object-mode
/// placeable read, the skies' table (which decides a sky pack's rows, and an export's `ls_skies` copies), and the three
/// shaders an export's tiles are made from.
const START: [(&str, ResType); 6] = [
    ("surfacemat", ResType::TWODA),
    ("placeables", ResType::TWODA),
    ("skyboxes", ResType::TWODA),
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
    // The game's skies: `row-<n>` (as /api/sky-picture names it) to its models, each model's files under `models`.
    let table = game.resman.get_named("skyboxes", ResType::TWODA)?;
    let table = mg_2da::TwoDa::parse(&table, game.codepage()).map_err(|e| anyhow::anyhow!("skyboxes.2da: {e}"))?;
    let mut skies: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in 0..table.rows.len() {
        let mut row_models = Vec::new();
        for col in ["DAWN", "DAY", "DUSK", "NIGHT"] {
            let Some(model) = table.get(row, col).map(|m| m.trim().to_ascii_lowercase()).filter(|m| !m.is_empty() && m != "****") else {
                continue;
            };
            if !models.contains_key(&model) {
                let Ok(bytes) = game.resman.get_named(&model, ResType::MDL) else {
                    unread.insert(model);
                    continue;
                };
                let Ok(mdl) = mg_mdl::Model::read(&bytes) else {
                    unread.insert(model);
                    continue;
                };
                let mut files = vec![put(&model, ResType::MDL, &bytes)?];
                for node in &mdl.nodes {
                    let Some(texture) = node.mesh().and_then(|m| m.textures[0].as_deref()).map(|t| t.trim().to_ascii_lowercase()) else {
                        continue;
                    };
                    if texture.is_empty() || texture == "null" {
                        continue;
                    }
                    if let Some(file) = textures.get(&texture) {
                        files.push(file.clone());
                    } else if let Some((t, data)) = nwn_bake::texture::image(game, &texture) {
                        let file = put(&texture, t, &data)?;
                        textures.insert(texture.clone(), file.clone());
                        files.push(file);
                    }
                }
                files.sort();
                files.dedup();
                models.insert(model.clone(), files);
            }
            row_models.push(model);
        }
        row_models.dedup();
        if !row_models.is_empty() {
            skies.insert(format!("row-{row}"), row_models);
        }
    }
    let index = serde_json::json!({ "start": start, "models": models, "textures": textures, "skies": skies });
    std::fs::write(out.join("index.json"), serde_json::to_vec(&index)?)?;
    std::fs::write(out.join("placeables.json"), serde_json::to_vec(&serde_json::json!({ "placeables": &*list }))?)?;
    let plants = crate::plants::plants(game)?;
    let groups = serde_json::json!({ "large_from": crate::plants::LARGE_FROM, "max_width": crate::plants::MAX_WIDTH, "plants": &*plants });
    std::fs::write(out.join("placeable-groups.json"), serde_json::to_vec(&groups)?)?;
    println!(
        "{}: {} models, {} textures, {} skies, {:.1} MB{}",
        out.display(),
        models.len(),
        textures.len(),
        skies.len(),
        bytes as f64 / 1e6,
        if unread.is_empty() { String::new() } else { format!("; not read: {}", unread.into_iter().collect::<Vec<_>>().join(", ")) }
    );
    Ok(())
}
