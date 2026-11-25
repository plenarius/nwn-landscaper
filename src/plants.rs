// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The plants among the placeables, in two groups the Place and Scatter panels offer as chips: small plants (ground cover a
//! person sees over) and large ones. A placeable is a plant by the words of its name and label (bush, fern, cactus...), and small
//! or large by its model's height at rest, never by its name: a "large bush" a metre tall is small. Trees (and saplings) are not
//! plants here; they keep their own chip. A cactus is a plant like any other: a saguaro (2.3 m to 3.1 m in one module's haks) is
//! large, a barrel cactus (0.6 m) small.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use mg_rules::GameData;

/// From this height (m, the model's top above its origin, at rest) a plant is large: a human's shoulder, about where a player
/// stops seeing over it. A module of 57 haks has 367 plants and no gap to put the line in (from 1.1 m to 1.8 m there are about ten a
/// 0.1 m), so it is set by what it means: 274 of the 367 are under it.
pub const LARGE_FROM: f32 = 1.5;
/// A model wider than a tile (m, its larger side) is no plant to put down one at a time: a ground cover the size of the area
/// ("Grass, 12x12 tiles"). Width says nothing else: a flat grass patch 5 m wide is small.
pub const MAX_WIDTH: f32 = 10.0;

/// The words that make a placeable a plant, by kind, the first kind that matches winning. A word matches a word of the name or
/// label that is it, or it with `s`, `es`, `ing`, `ed` or `y` after it (`flower`: `flowers`, `flowering`; never `grass` in
/// `grassland`, which named a skybox).
const KINDS: &[(&str, &[&str])] = &[
    ("cactus", &["cactus", "cacti", "saguaro", "cholla", "prickly", "succulent"]),
    ("fern", &["fern"]),
    ("reed", &["reed", "cattail", "bulrush", "rushes"]),
    ("flower", &["flower", "daisies", "daisy", "tulip", "poppies", "poppy", "lavender", "blossom"]),
    ("grass", &["grass"]),
    ("bush", &["bush", "shrub", "hedge", "bramble", "briar", "thicket"]),
    ("plant", &["plant", "weed", "thistle", "nettle", "heather", "undergrowth", "foliage"]),
];

/// Words that make a placeable something else: a tree (its own chip), a thing that holds plants (a vase, a planter, a fountain),
/// a plant that grows on a wall, hangs (ivy, vines) or grows under water, which can't stand on open ground, and a few things
/// whose names hold a plant word (a flower cart, a grassland skybox).
const NOT: &[&str] = &[
    "tree",
    "trees",
    "sapling",
    "saplings",
    "stump",
    "trunk",
    "log",
    "logs",
    "vase",
    "pot",
    "pots",
    "potted",
    "planter",
    "planters",
    "basket",
    "bouquet",
    "wreath",
    "garland",
    "painting",
    "tapestry",
    "banner",
    "sign",
    "table",
    "shelf",
    "window",
    "box",
    "statue",
    "wall",
    "hanging",
    "climbing",
    "ivy",
    "vine",
    "vines",
    "column",
    "ruin",
    "ruins",
    "arch",
    "seed",
    "seeds",
    "sack",
    "plantation",
    "rock",
    "rocks",
    "boulder",
    "cart",
    "wagon",
    "skybox",
    "sky",
    "crate",
    "stall",
    "fountain",
    "bottle",
    "urn",
    "shell",
    "underwater",
];

/// The plant kind of a placeable named `name` with label `label` (`bush`, `fern`, ...), None when it is not a plant.
pub fn kind(name: &str, label: &str) -> Option<&'static str> {
    let text = format!("{name} {label}").to_ascii_lowercase();
    let words: Vec<&str> = text.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| NOT.contains(w)) {
        return None;
    }
    let is = |w: &str, s: &str| w.strip_prefix(s).is_some_and(|rest| ["", "s", "es", "ing", "ed", "y"].contains(&rest));
    KINDS.iter().find(|(_, stems)| words.iter().any(|w| stems.iter().any(|s| is(w, s)))).map(|(k, _)| *k)
}

/// Whether a plant `height` tall (m) is a large plant.
pub fn large(height: f32) -> bool {
    height >= LARGE_FROM
}

/// A plant as the panels group it.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Plant {
    /// Its model.
    pub model: String,
    /// What kind of plant its name says it is.
    pub kind: &'static str,
    /// Its top above its origin, at rest (m, to the centimetre).
    pub height: f32,
    /// Its larger side (m, to the centimetre).
    pub width: f32,
    /// [`large`] by its height.
    pub large: bool,
}

/// The plants among the placeables there are models of ([`crate::models::placeables`]), each model once, measured: [`kind`], then
/// the model's size. Measured once per game (the game with the module's haks has other plants than the stand-in), when first
/// asked for, on the import's threads: 367 plants of 57 haks take 0.17 s. A model that can't be read, or is wider than
/// [`MAX_WIDTH`], is left out.
pub fn plants(game: &GameData) -> Result<Arc<Vec<Plant>>> {
    static KEPT: Mutex<Vec<(usize, Arc<Vec<Plant>>)>> = Mutex::new(Vec::new());
    static MEASURING: Mutex<()> = Mutex::new(());
    let key = crate::models::game_key(game);
    let kept = || KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(k, _)| *k == key).map(|(_, l)| l.clone());
    if let Some(l) = kept() {
        return Ok(l);
    }
    // (One measures; the others wait for it and take its answer.)
    let _one = MEASURING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(l) = kept() {
        return Ok(l);
    }
    let list = crate::models::placeables(game)?;
    let mut seen = std::collections::HashSet::new();
    let found: Vec<(&str, &'static str)> = list
        .iter()
        .filter(|p| p.source.is_some() && seen.insert(p.model.as_str()))
        .filter_map(|p| Some((p.model.as_str(), kind(&p.name, &p.label)?)))
        .collect();
    let measured = crate::generator::in_parallel(&found, |(model, kind)| {
        let f = nwn_bake::flatten(game, model).ok()?;
        if f.positions.is_empty() {
            return None;
        }
        let cm = |v: f32| (v * 100.0).round() / 100.0;
        let (height, width) = (cm(f.max[2]), cm((f.max[0] - f.min[0]).max(f.max[1] - f.min[1])));
        (width <= MAX_WIDTH).then(|| Plant { model: model.to_string(), kind, height, width, large: large(height) })
    });
    let out = Arc::new(measured.into_iter().flatten().collect::<Vec<_>>());
    KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((key, out.clone()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{kind, large};

    #[test]
    fn a_plant_is_known_by_the_words_of_its_name_and_label() {
        for (name, label, want) in [
            ("Saguaro cactus, green", "Saguaro_Cactus_Green", "cactus"),
            ("Cacti, small", "", "cactus"),
            ("Bush, Large", "PLC_BushLrg", "bush"),
            ("Leafy shrub, low (10)", "", "bush"),
            ("Flowering shrub", "", "flower"),
            ("Flower patch, pink", "", "flower"),
            ("Fern bush", "", "fern"),
            ("Cattail reeds 13", "", "reed"),
            ("Grass tufts", "", "grass"),
            ("Jungle undergrowth", "", "plant"),
            ("Desert plant, spiky", "", "plant"),
            ("Barrel cactus 2", "", "cactus"),
            ("Grassy weeds", "", "grass"),
            ("Desert plant, spiky", "Succulent_01", "cactus"),
        ] {
            assert_eq!(kind(name, label), Some(want), "{name} / {label}");
        }
    }

    #[test]
    fn trees_vases_and_wall_plants_are_not_plants_here() {
        for (name, label) in [
            ("Tree, oak", ""),
            ("Bushy tree", ""),
            ("Sapling", ""),
            ("Vase of flowers", ""),
            ("Stone planter, red flowers", ""),
            ("Wall ivy, climbing", ""),
            ("Hanging vines", ""),
            ("Wall weed 1, pink", ""),
            ("Vale ruins column, ivy", ""),
            ("Vale water rock 3, with ferns", ""),
            ("Rock, grey", "PLC_Rock"),
            ("Plantation sign", ""),
            ("Flower cart", ""),
            ("Skybox, grassland storm", ""),
            ("Fernando's chair", ""),
            ("Fountain with plants", ""),
            ("Stone urn with flowers", ""),
            ("Sake bottle, black, red flower", ""),
            ("Underwater weed bed 1", ""),
        ] {
            assert_eq!(kind(name, label), None, "{name}");
        }
    }

    #[test]
    fn a_plant_is_large_by_its_height_not_its_name() {
        // A "Garden bush, large" of 0.8 m is small; a "Low bush" of 1.6 m is large.
        assert!(!large(1.1));
        assert!(!large(super::LARGE_FROM - 0.01));
        assert!(large(super::LARGE_FROM));
        assert!(large(1.6));
    }
}
