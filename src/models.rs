// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Placeable models for baking into the terrain: the game's (and haks') placeables as flat textured triangles in the model's space, at
//! rest, and the footprint of their walkmesh (`.pwk`) where it can't be walked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_rules::GameData;

/// The game's data, opened once (it takes a moment): the install and user folder (`--root`, `--user`, else as the game finds them:
/// NWN_ROOT, NWN_HOME), no haks.
pub fn game() -> Result<&'static GameData> {
    static GAME: OnceLock<nwn_bake::game::Game> = OnceLock::new();
    if let Some(g) = GAME.get() {
        return Ok(&g.data);
    }
    // The module's own talk table (blueprint and area names are in it), when the user folder has one (--tlk, LANDSCAPER_TLK).
    let g = open_game(nwn_bake::game::Haks::None, nwn_bake::game::Tlk::named_or_auto(crate::config::tlk()))?;
    Ok(&GAME.get_or_init(|| g).data)
}

/// Opens the game where `--root` and `--user` say, else where the game finds it (NWN_ROOT, NWN_HOME), with `haks` and talk table
/// `tlk`: the one way the Landscaper opens it.
pub fn open_game(haks: nwn_bake::game::Haks, tlk: nwn_bake::game::Tlk) -> Result<nwn_bake::game::Game> {
    let (root, user) = (crate::config::game_root(), crate::config::game_user());
    nwn_bake::game::open(&nwn_bake::game::GameOptions { root, user, haks, tlk })
}

/// The game install and user folder the Landscaper uses, as `--root`, `--user` or the game's own rule find them, without opening
/// the game (the same rule as `nwn_bake::game::open`).
pub fn game_folders() -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let given = crate::config::game_user();
    match crate::config::game_root() {
        Some(root) => (Some(root), given.or_else(mg_resman::GameInstall::detect_user_dir)),
        None => {
            let detected = mg_resman::GameInstall::detect();
            (detected.as_ref().map(|g| g.root.clone()), given.or_else(|| detected.and_then(|g| g.user_dir)))
        }
    }
}

pub use nwn_bake::placeables;

/// A model as the editor shows and places it: nwn-bake's flattened model plus what an import adds to the things it finds standing
/// on a tile. Written to and read from an import's `obj_<xxyy>*.json` in exactly the format these files had when the two fields
/// were part of `nwn_bake::Flat` (the model's fields, then `decal` when there is one, then `fixed` when it is set). Read through
/// [`FlatFile`], not `flatten`: serde buffers a flattened struct whole before reading it, about three times slower on a large model.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "FlatFile")]
pub struct Flat {
    /// The model's triangles, as nwn-bake flattens them.
    #[serde(flatten)]
    pub mesh: nwn_bake::Flat,
    /// An imported thing's patch of ground (the earth under a tree), drawn on the ground wherever it stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decal: Option<Decal>,
    /// Imported cliffs and blocked ground: fixed to their tile, heights their own (not set on the ground when touched).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fixed: bool,
}

/// An `obj_*.json` file as read: every field of `nwn_bake::Flat` and the two of [`Flat`]. (A field added to `nwn_bake::Flat` stops
/// the conversion below from compiling, so it cannot be left out here.)
#[derive(serde::Deserialize)]
struct FlatFile {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    groups: Vec<(String, usize, usize)>,
    transparency: Vec<u32>,
    footprint: Vec<[[f32; 2]; 3]>,
    min: [f32; 3],
    max: [f32; 3],
    #[serde(default)]
    decal: Option<Decal>,
    #[serde(default)]
    fixed: bool,
}

impl From<FlatFile> for Flat {
    fn from(f: FlatFile) -> Self {
        let FlatFile { positions, uvs, groups, transparency, footprint, min, max, decal, fixed } = f;
        Flat { mesh: nwn_bake::Flat { positions, uvs, groups, transparency, footprint, min, max }, decal, fixed }
    }
}

impl From<nwn_bake::Flat> for Flat {
    fn from(mesh: nwn_bake::Flat) -> Self {
        Flat { mesh, decal: None, fixed: false }
    }
}

impl std::ops::Deref for Flat {
    type Target = nwn_bake::Flat;
    fn deref(&self) -> &nwn_bake::Flat {
        &self.mesh
    }
}

impl std::ops::DerefMut for Flat {
    fn deref_mut(&mut self) -> &mut nwn_bake::Flat {
        &mut self.mesh
    }
}

/// A picture laid on the ground under a model: `file` (in its import's folder, rows from the south) over `rect` (x0, y0, x1, y1,
/// metres about the model's origin).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Decal {
    /// The picture's file, in its import's folder.
    pub file: String,
    /// Where it lies: x0, y0, x1, y1 (m, about the model's origin).
    pub rect: [f32; 4],
}

/// The model `resref`, flattened (cached).
pub fn flat(game: &GameData, resref: &str) -> Result<Arc<Flat>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Flat>>>> = OnceLock::new();
    let key = resref.to_ascii_lowercase();
    // An imported tile's standing things (`@import:<name>:<xxyy>`) belong to a person's workspace and change when they re-import or
    // lose the area: read from their folder each time, never shared between requests (two people can each have an import of the same
    // name).
    if let Some(rest) = key.strip_prefix(crate::paint::IMPORT) {
        let (name, tile) = rest.split_once(':').with_context(|| format!("bad imported model {key}"))?;
        let path = crate::paint::import_dir(name).join(format!("obj_{tile}.json"));
        return Ok(Arc::new(serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("no imported model {name}:{tile}"))?)?));
    }
    let cache = CACHE.get_or_init(Default::default);
    if let Some(f) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
        return Ok(f.clone());
    }
    let f = Arc::new(Flat::from(nwn_bake::flatten(game, &key)?));
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, f.clone());
    Ok(f)
}

/// The game's placeable blueprint (`.utp`) with appearance `row`, else any (its appearance set to `row`): (its resref, its fields).
pub fn blueprint(game: &GameData, row: u32) -> Result<(String, mg_gff::Struct)> {
    static INDEX: OnceLock<HashMap<u32, String>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let mut by_row = HashMap::new();
        for r in game.resman.list(ResType::UTP) {
            let name = r.to_string();
            if let Ok(data) = game.resman.get_named(&name, ResType::UTP)
                && let Ok(g) = mg_gff::Gff::read(&data)
                && let Some(a) = g.root.integer("Appearance")
            {
                by_row.entry(a as u32).or_insert(name);
            }
        }
        by_row
    });
    let (name, exact) = match index.get(&row) {
        Some(n) => (n.clone(), true),
        None => (index.values().min().cloned().context("the game has no placeable blueprints")?, false),
    };
    let data = game.resman.get_named(&name, ResType::UTP)?;
    let mut s = mg_gff::Gff::read(&data)?.root;
    if !exact {
        let v = match s.get("Appearance") {
            Some(mg_gff::Value::Dword(_)) | None => mg_gff::Value::Dword(row),
            Some(mg_gff::Value::Int(_)) => mg_gff::Value::Int(row as i32),
            Some(_) => mg_gff::Value::Dword(row),
        };
        s.set("Appearance", v);
    }
    Ok((name, s))
}

/// A tileset texture an import copied (exports/*.import/tex), the module's haks' version of it.
pub fn import_texture(name: &str) -> Option<(ResType, Vec<u8>)> {
    let dir = crate::workspace::exports_dir();
    for e in std::fs::read_dir(&dir).ok()?.flatten() {
        let tex = e.path().join("tex");
        if !e.file_name().to_string_lossy().ends_with(".import") || !tex.is_dir() {
            continue;
        }
        for (t, ext) in [(ResType::DDS, "dds"), (ResType::TGA, "tga")] {
            if let Ok(d) = std::fs::read(tex.join(format!("{name}.{ext}"))) {
                return Some((t, d));
            }
        }
    }
    None
}

/// A texture as PNG (rows top first), from an import's copy of it, else the image the game means by it ([`nwn_bake::texture::png`]).
pub fn texture_png(game: &GameData, name: &str) -> Result<Vec<u8>> {
    nwn_bake::texture::png(game, name, import_texture(name))
}

/// The door looks the game has: generic doors (`genericdoors.2da`, chosen with `Appearance` 0 and `GenericType_New`) and tile doors
/// (`doortypes.2da`, chosen with `Appearance`): `[{generic, row, name, model}]`.
pub fn door_appearances(game: &GameData) -> Result<Vec<serde_json::Value>> {
    let mut out = Vec::new();
    for (table, generic, model_col, name_col) in
        [("genericdoors", true, "ModelName", "Name"), ("doortypes", false, "Model", "StringRefGame")]
    {
        let Ok(t) = game.table(table) else { continue };
        for row in 0..t.len() {
            let model = t.get(row, model_col).map(str::trim).filter(|m| !m.is_empty() && *m != "****").map(str::to_lowercase);
            if model.is_none() {
                continue;
            }
            let label = t.get(row, "Label").unwrap_or("").to_string();
            let name = t
                .get(row, name_col)
                .and_then(|s| s.parse::<u32>().ok())
                .and_then(|s| game.string(mg_core::StrRef(s)))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| label.clone());
            out.push(serde_json::json!({ "generic": generic, "row": row, "label": label, "name": name, "model": model }));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Decal, Flat};

    /// `nwn_bake::Flat` as it was before `decal` and `fixed` moved here: the format an import's `obj_*.json` files are in.
    #[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
    struct Before {
        positions: Vec<[f32; 3]>,
        uvs: Vec<[f32; 2]>,
        groups: Vec<(String, usize, usize)>,
        transparency: Vec<u32>,
        footprint: Vec<[[f32; 2]; 3]>,
        min: [f32; 3],
        max: [f32; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decal: Option<Decal>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        fixed: bool,
    }

    fn both(decal: Option<Decal>, fixed: bool) -> (Flat, Before) {
        // Awkward floats on purpose: the flattened field is read through serde's buffer, and must come out bit for bit the same.
        let positions = vec![[0.1, -0.0, 1e-7], [123.456_79, -7.25, 3.402_823_5e38], [1.0 / 3.0, 2.0 / 3.0, -1e-30]];
        let uvs = vec![[0.0, 1.0], [0.333_333_34, 0.999_999_9], [-2.5, 17.125]];
        let groups = vec![("tdm01_rock".to_string(), 0, 1)];
        let footprint = vec![[[0.1, 0.2], [0.3, 0.4], [-0.5, 0.6]]];
        let (min, max) = ([-7.25, -0.0, -1e-30], [123.456_79, 2.0 / 3.0, 3.402_823_5e38]);
        let mesh = nwn_bake::Flat {
            positions: positions.clone(),
            uvs: uvs.clone(),
            groups: groups.clone(),
            transparency: vec![2],
            footprint: footprint.clone(),
            min,
            max,
        };
        let new = Flat { mesh, decal: decal.clone(), fixed };
        let old = Before { positions, uvs, groups, transparency: vec![2], footprint, min, max, decal, fixed };
        (new, old)
    }

    fn cases() -> Vec<(Flat, Before)> {
        let decal = || Some(Decal { file: "decal_0102_3.png".into(), rect: [-1.25, -0.1, 2.5, 0.7] });
        vec![both(None, false), both(None, true), both(decal(), false), both(decal(), true)]
    }

    /// The import writes `obj_*.json` with `serde_json::to_vec`: the bytes are the same as before the move, in every combination of
    /// the two fields.
    #[test]
    fn obj_json_is_written_as_before() {
        for (new, old) in cases() {
            assert_eq!(serde_json::to_vec(&new).unwrap(), serde_json::to_vec(&old).unwrap());
            assert_eq!(serde_json::to_vec_pretty(&new).unwrap(), serde_json::to_vec_pretty(&old).unwrap());
        }
    }

    /// Files written before the move read back to the same model, and write back byte for byte.
    #[test]
    fn obj_json_written_before_reads_the_same() {
        for (_, old) in cases() {
            let bytes = serde_json::to_vec(&old).unwrap();
            let read: Flat = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(serde_json::to_vec(&read).unwrap(), bytes);
            let bits = |v: &[[f32; 3]]| v.iter().flatten().map(|x| x.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&read.positions), bits(&old.positions));
            assert_eq!(read.min.map(f32::to_bits), old.min.map(f32::to_bits));
            assert_eq!(read.max.map(f32::to_bits), old.max.map(f32::to_bits));
            assert_eq!((read.decal.is_some(), read.fixed), (old.decal.is_some(), old.fixed));
        }
    }

    /// The exact text, so a change to `nwn_bake::Flat` itself (a field renamed, added or reordered) shows here too.
    #[test]
    fn a_flat_mesh_is_written_as_the_exact_json_the_editor_reads() {
        let decal = Decal { file: "decal_0102_3.png".into(), rect: [-1.25, -0.5, 2.5, 0.75] };
        let mesh = nwn_bake::Flat {
            positions: vec![[0.5, 1.0, 2.0], [1.5, 1.0, 2.0], [0.5, 2.0, 2.25]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            groups: vec![("tdm01_rock".into(), 0, 1)],
            transparency: vec![0],
            footprint: vec![[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
            min: [0.5, 1.0, 2.0],
            max: [1.5, 2.0, 2.25],
        };
        let flat = Flat { mesh, decal: Some(decal), fixed: true };
        assert_eq!(
            serde_json::to_string(&flat).unwrap(),
            concat!(
                r#"{"positions":[[0.5,1.0,2.0],[1.5,1.0,2.0],[0.5,2.0,2.25]],"uvs":[[0.0,0.0],[1.0,0.0],[0.0,1.0]],"#,
                r#""groups":[["tdm01_rock",0,1]],"transparency":[0],"footprint":[[[0.0,0.0],[1.0,0.0],[0.0,1.0]]],"#,
                r#""min":[0.5,1.0,2.0],"max":[1.5,2.0,2.25],"decal":{"file":"decal_0102_3.png","rect":[-1.25,-0.5,2.5,0.75]},"fixed":true}"#
            )
        );
        let plain = Flat { decal: None, fixed: false, ..flat };
        assert!(!serde_json::to_string(&plain).unwrap().contains("decal"));
        assert!(!serde_json::to_string(&plain).unwrap().contains("fixed"));
    }
}
