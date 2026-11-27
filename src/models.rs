// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Placeable models for baking into the terrain: the game's (and haks') placeables as flat textured triangles in the model's space, at
//! rest, and the footprint of their walkmesh (`.pwk`) where it can't be walked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_rules::GameData;

/// The game's data as the editor reads it: the install and user folder (`--root`, `--user`, else as the game finds them: NWN_ROOT,
/// NWN_HOME) with the editor's haks over them ([`editor_haks`]), opened once. The haks are only mapped and their tables read: with
/// a module's 57 haks (16 GB) it opens and reads its 21,201 placeables in 0.16 s.
pub fn game() -> Result<&'static GameData> {
    Ok(&opened()?.data)
}

/// The game [`game`] gives, opened. While a review service is set but hasn't given the module's haks (it is down as the server
/// starts), the game without haks stands in, and each call asks again (the answer is kept: see `review::module_haks`).
pub fn opened() -> Result<&'static nwn_bake::game::Game> {
    static GAME: OnceLock<nwn_bake::game::Game> = OnceLock::new();
    static STAND_IN: OnceLock<nwn_bake::game::Game> = OnceLock::new();
    static OPENING: Mutex<()> = Mutex::new(());
    if let Some(g) = GAME.get() {
        return Ok(g);
    }
    let haks = editor_haks();
    let _one = OPENING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(g) = GAME.get() {
        return Ok(g);
    }
    if !haks.settled {
        if let Some(g) = STAND_IN.get() {
            return Ok(g);
        }
        eprintln!("the review service hasn't given the module's haks yet: the editor offers the game's own placeables until it does");
        let g = open_editor(&[])?;
        return Ok(STAND_IN.get_or_init(|| g));
    }
    let g = open_editor(&haks.list)?;
    if !haks.list.is_empty() {
        eprintln!("haks: {} ({}), first highest", haks.list.join(", "), haks.from);
    }
    if !g.missing_haks().is_empty() {
        eprintln!("haks not found in the user folder's hak/ or the game's data/hk/: {}", g.missing_haks().join(", "));
    }
    Ok(GAME.get_or_init(|| g))
}

fn open_editor(haks: &[String]) -> Result<nwn_bake::game::Game> {
    let list = if haks.is_empty() { nwn_bake::game::Haks::None } else { nwn_bake::game::Haks::List(haks.to_vec()) };
    // The module's own talk table (blueprint and area names are in it), when the user folder has one (--tlk, LANDSCAPER_TLK).
    open_game(list, nwn_bake::game::Tlk::named_or_auto(crate::config::tlk()))
}

/// The talk table the editor's game read (its file name without `.tlk`), when it was opened with haks: an exported module names
/// it with the haks, or the names of their placeables (rows of that table) show blank in the game. None without haks.
pub fn game_tlk() -> Option<String> {
    let g = opened().ok()?;
    if g.haks().is_empty() {
        return None;
    }
    g.tlk_path().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned())
}

/// Sets module.ifo root `info`'s talk table (`Mod_CustomTlk`) to the editor's ([`game_tlk`]) when it names none.
pub fn give_tlk(info: &mut mg_gff::Struct) {
    let named = info.get("Mod_CustomTlk").is_some_and(|v| matches!(v, mg_gff::Value::String(b) if !b.trim_ascii().is_empty()));
    if let (false, Some(t)) = (named, game_tlk()) {
        info.set("Mod_CustomTlk", mg_gff::Value::String(t.into_bytes()));
    }
}

/// A key for what is kept per opened game (its models, its blueprints): the game's address, which never changes (it is never
/// dropped) and differs between the stand-in and the game with the module's haks.
pub fn game_key(game: &GameData) -> usize {
    std::ptr::from_ref(game) as usize
}

/// The haks the editor reads the game with, and where the list came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorHaks {
    /// The haks, first highest, as a module lists them.
    pub list: Vec<String>,
    /// Where they came from, for the start line.
    pub from: &'static str,
    /// False while a review service is set and hasn't given the module's list: ask again later.
    pub settled: bool,
}

/// `--haks` or `LANDSCAPER_HAKS` (a list, or `module` for the checkout's), else on a team server the module's haks from the
/// review service, else none: the editor offers the game's own placeables, as before there was a setting.
pub fn editor_haks() -> EditorHaks {
    let review = crate::config::review_service().is_some();
    editor_haks_from(crate::config::haks(), review, crate::review::module_haks, checkout_haks)
}

fn editor_haks_from(
    setting: crate::config::Haks,
    review: bool,
    from_review: impl FnOnce() -> Vec<String>,
    from_checkout: impl FnOnce() -> Vec<String>,
) -> EditorHaks {
    use crate::config::Haks;
    match setting {
        Haks::List(list) => EditorHaks { list, from: "LANDSCAPER_HAKS", settled: true },
        Haks::Module => EditorHaks { list: from_checkout(), from: "the module checkout's module.ifo.json", settled: true },
        Haks::Unset if review => {
            let list = from_review();
            let settled = !list.is_empty();
            EditorHaks { list, from: "the review service", settled }
        }
        Haks::Unset => EditorHaks { list: Vec::new(), from: "none", settled: true },
    }
}

/// The module checkout's hak list (`Mod_HakList` of its `module.ifo.json`), first highest; empty when it has none.
pub fn checkout_haks() -> Vec<String> {
    let ifo =
        std::fs::read(crate::config::Layout::from_env().ifo_path()).ok().and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok());
    ifo.and_then(|v| {
        v["Mod_HakList"]["value"].as_array().map(|a| a.iter().filter_map(|h| h["Mod_Hak"]["value"].as_str().map(str::to_string)).collect())
    })
    .unwrap_or_default()
}

/// The haks the editor's game was opened with and found (first highest): an exported module lists them, so the placeables and
/// baked models taken from them are there in the game. Empty before the game is opened, and for the stand-in.
pub fn game_haks() -> Vec<String> {
    match opened() {
        Ok(g) => g.haks().iter().filter(|h| !g.missing_haks().iter().any(|m| m.eq_ignore_ascii_case(h))).cloned().collect(),
        Err(_) => Vec::new(),
    }
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

/// A placeable as the Place and Scatter panels offer it: placeables.2da's row, its name (the talk table's, else its label), its
/// model and where the model comes from.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Placeable {
    /// Its row in placeables.2da.
    pub row: usize,
    /// The row's `Label` column.
    pub label: String,
    /// The name shown for it.
    pub name: String,
    /// Its model's resref, lower case.
    pub model: String,
    /// The hak its model is in, `game` for the game's own files (its data, `override/`, `development/`), None when there is no such
    /// model (it can't be drawn or baked).
    pub source: Option<String>,
}

/// placeables.2da's rows with a model, from the haks' table when they have one, each with its model's [`Placeable::source`]. Read
/// once per game: 21,201 rows with a module's haks.
pub fn placeables(game: &GameData) -> Result<Arc<Vec<Placeable>>> {
    static KEPT: Mutex<Vec<(usize, Arc<Vec<Placeable>>)>> = Mutex::new(Vec::new());
    let key = game_key(game);
    if let Some((_, l)) = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(k, _)| *k == key) {
        return Ok(l.clone());
    }
    let mut sources: HashMap<String, Option<String>> = HashMap::new();
    let list: Vec<Placeable> = nwn_bake::placeables(game)?
        .into_iter()
        .map(|p| {
            let source = sources.entry(p.model.clone()).or_insert_with(|| source_of(game, &p.model)).clone();
            Placeable { row: p.row, label: p.label, name: p.name, model: p.model, source }
        })
        .collect();
    let list = Arc::new(list);
    KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((key, list.clone()));
    Ok(list)
}

/// Where model `model` comes from: the hak's name, `game`, or None when no layer has it.
fn source_of(game: &GameData, model: &str) -> Option<String> {
    let r = mg_core::ResRef::cut(model)?;
    let label = game.resman.origin(&mg_resman::ResKey::new(r, ResType::MDL))?;
    Some(source_name(label))
}

/// A resource manager layer's label as a source: `hak:<name>` is the hak, anything else (the game's keys, `override/`,
/// `development/`, the workshop) is the game's own.
fn source_name(label: &str) -> String {
    label.strip_prefix("hak:").map_or_else(|| "game".to_string(), str::to_ascii_lowercase)
}

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
    /// Each group's self-illumination colour (the model's `selfillumcolor`), so a glowing mesh still glows when baked; empty
    /// when no group glows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glow: Vec<[f32; 3]>,
}

/// An `obj_*.json` file as read: every field of `nwn_bake::Flat` and the three of [`Flat`]. (A field added to `nwn_bake::Flat` stops
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
    #[serde(default)]
    glow: Vec<[f32; 3]>,
}

impl From<FlatFile> for Flat {
    fn from(f: FlatFile) -> Self {
        let FlatFile { positions, uvs, groups, transparency, footprint, min, max, decal, fixed, glow } = f;
        Flat { mesh: nwn_bake::Flat { positions, uvs, groups, transparency, footprint, min, max }, decal, fixed, glow }
    }
}

impl From<nwn_bake::Flat> for Flat {
    fn from(mesh: nwn_bake::Flat) -> Self {
        Flat { mesh, decal: None, fixed: false, glow: Vec::new() }
    }
}

/// The self-illumination colour of each of `groups` (nwn-bake's: one a drawn, textured mesh, in node order) in `model`, the
/// model they were flattened from; empty when none glows, or when the meshes don't line up with the groups (then nothing glows
/// rather than the wrong mesh).
pub fn glow_of(model: &mg_mdl::Model, groups: &[(String, usize, usize)]) -> Vec<[f32; 3]> {
    let mut glow = Vec::with_capacity(groups.len());
    for node in &model.nodes {
        let Some(mesh) = node.mesh() else { continue };
        // (nwn-bake's choice of meshes: drawn, with vertices, a texture, and at least one triangle)
        let Some(texture) = mesh.textures[0].as_deref().filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("null")) else { continue };
        if !mesh.render || mesh.vertices.is_empty() {
            continue;
        }
        let count = mesh.triangles().count();
        if count == 0 {
            continue;
        }
        match groups.get(glow.len()) {
            Some((t, _, n)) if t.eq_ignore_ascii_case(texture) && *n == count => {}
            _ => return Vec::new(),
        }
        let c = node.value("selfillumcolor").filter(|v| v.len() >= 3).map_or([0.0; 3], |v| [v[0], v[1], v[2]]);
        glow.push(c.map(|x| x.clamp(0.0, 1.0)));
    }
    if glow.len() != groups.len() || glow.iter().all(|c| *c == [0.0; 3]) {
        return Vec::new();
    }
    glow
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
    static CACHE: OnceLock<Mutex<HashMap<(usize, String), Arc<Flat>>>> = OnceLock::new();
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
    let kept = (game_key(game), key);
    if let Some(f) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&kept) {
        return Ok(f.clone());
    }
    let mut f = Flat::from(nwn_bake::flatten(game, &kept.1)?);
    // (nwn-bake leaves self-illumination out: read again here, cached with the rest)
    if let Ok(data) = game.resman.get_named(&kept.1, ResType::MDL)
        && let Ok(model) = mg_mdl::Model::read(&data)
    {
        f.glow = glow_of(&model, &f.groups);
    }
    let f = Arc::new(f);
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(kept, f.clone());
    Ok(f)
}

/// The game's placeable blueprint (`.utp`) with appearance `row`, else the plainest there is (no scripts, no inventory, no
/// conversation) made into one of that row: its appearance set to `row` and its name and description to the row's (placeables.2da's
/// `StrRef`, else its label). Most of a module's hak placeables have no blueprint of their own, and the first blueprint by name
/// gave a cactus a skeleton's name and scripts. (Its resref, its fields.)
pub fn blueprint(game: &GameData, row: u32) -> Result<(String, mg_gff::Struct)> {
    /// Per game: the first blueprint of each appearance, and the plainest blueprint (fewest of the things above, then by name).
    type Index = (HashMap<u32, String>, Option<(usize, String)>);
    static INDEX: Mutex<Vec<(usize, Arc<Index>)>> = Mutex::new(Vec::new());
    let key = game_key(game);
    let kept = INDEX.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(k, _)| *k == key).map(|(_, i)| i.clone());
    let index = kept.unwrap_or_else(|| {
        let mut by_row = HashMap::new();
        let mut plainest: Option<(usize, String)> = None;
        for r in game.resman.list(ResType::UTP) {
            let name = r.to_string();
            if let Ok(data) = game.resman.get_named(&name, ResType::UTP)
                && let Ok(g) = mg_gff::Gff::read(&data)
                && let Some(a) = g.root.integer("Appearance")
            {
                let fuss = fuss(&g.root);
                if plainest.as_ref().is_none_or(|(f, n)| (fuss, &name) < (*f, n)) {
                    plainest = Some((fuss, name.clone()));
                }
                by_row.entry(a as u32).or_insert(name);
            }
        }
        let index = Arc::new((by_row, plainest));
        INDEX.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((key, index.clone()));
        index
    });
    let (name, exact) = match index.0.get(&row) {
        Some(n) => (n.clone(), true),
        None => (index.1.as_ref().map_or_else(String::new, |(_, n)| n.clone()), false),
    };
    // (A game with no placeable blueprints at all, as the web demo's: an empty one, which the module's placing fills in.)
    let mut s =
        if name.is_empty() { mg_gff::Struct::new(u32::MAX) } else { mg_gff::Gff::read(&game.resman.get_named(&name, ResType::UTP)?)?.root };
    if !exact {
        let v = match s.get("Appearance") {
            Some(mg_gff::Value::Dword(_)) | None => mg_gff::Value::Dword(row),
            Some(mg_gff::Value::Int(_)) => mg_gff::Value::Int(row as i32),
            Some(_) => mg_gff::Value::Dword(row),
        };
        s.set("Appearance", v);
        s.set("LocName", mg_gff::Value::LocString(row_name(game, row as usize)));
        s.set("Description", mg_gff::Value::LocString(mg_core::LocString::default()));
    }
    Ok((name, s))
}

/// How much a blueprint does besides look like its model: its scripts, an inventory, a conversation. The plainest stands in for a
/// placeables.2da row with no blueprint of its own.
fn fuss(s: &mg_gff::Struct) -> usize {
    let scripts =
        s.fields.iter().filter(|f| f.label.as_bytes().starts_with(b"On") && f.value.as_resref().is_some_and(|r| !r.is_empty())).count();
    let conversation = s.resref("Conversation").is_some_and(|r| !r.is_empty());
    scripts + usize::from(s.integer("HasInventory").unwrap_or(0) != 0) + usize::from(conversation)
}

/// placeables.2da row `row`'s name: its `StrRef` (a talk table row), else its label as text.
fn row_name(game: &GameData, row: usize) -> mg_core::LocString {
    let t = game.table("placeables").ok();
    let strref = t.as_ref().and_then(|t| t.get(row, "StrRef")).and_then(|v| v.trim().parse::<u32>().ok());
    match strref {
        Some(r) => mg_core::LocString::from_strref(mg_core::StrRef(r)),
        None => {
            let label = t.as_ref().and_then(|t| t.get(row, "Label")).unwrap_or("").replace('_', " ");
            mg_core::LocString::from_text(mg_core::Language::ENGLISH, mg_core::Gender::Male, label.into_bytes())
        }
    }
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
    use super::{Decal, EditorHaks, Flat};
    use crate::config::Haks;

    /// A game of `layers` (label, files), the first highest, each a placeables.2da (its rows after the header) and models.
    fn small_game(layers: &[(&str, Option<&str>, &[&str])]) -> mg_rules::GameData {
        let mut rm = mg_resman::ResMan::new();
        for (label, table, models) in layers.iter().rev() {
            let mut mem = mg_resman::MemContainer::new();
            if let Some(rows) = table {
                let text = format!("2DA V2.0\n\n   Label StrRef ModelName\n{rows}");
                mem.insert(mg_resman::ResKey::parse("placeables", mg_core::ResType::TWODA).unwrap(), text.into_bytes());
            }
            for m in *models {
                mem.insert(mg_resman::ResKey::parse(m, mg_core::ResType::MDL).unwrap(), b"model".to_vec());
            }
            // (A hak over the game, as `nwn_bake::game::open` stacks them.)
            let (priority, class) = match label.strip_prefix("hak:") {
                Some(_) => (mg_resman::priority::HAK, mg_resman::LayerClass::Erf),
                None => (mg_resman::priority::KEY, mg_resman::LayerClass::Key),
            };
            rm.add(priority, *label, class, mem);
        }
        mg_rules::GameData::new(rm, mg_tlk::Tlk::new(mg_core::Language::ENGLISH))
    }

    /// With a hak's placeables.2da over the game's, its rows are offered (the game's own table is not), each saying where its
    /// model is: the hak, the game, or nowhere.
    #[test]
    fn a_haks_placeables_are_offered_with_where_their_models_are() {
        let game = small_game(&[
            (
                "hak:My_Cacti",
                Some("0 Rock **** plc_rock\n1 Saguaro_cactus **** my_cactus01\n2 Lost **** my_gone\n3 Blank **** ****"),
                &["my_cactus01"],
            ),
            ("key:data/nwn_base", Some("0 Rock **** plc_rock"), &["plc_rock"]),
        ]);
        let list = super::placeables(&game).unwrap();
        let got: Vec<(usize, &str, &str, Option<&str>)> =
            list.iter().map(|p| (p.row, p.name.as_str(), p.model.as_str(), p.source.as_deref())).collect();
        assert_eq!(
            got,
            [(0, "Rock", "plc_rock", Some("game")), (1, "Saguaro_cactus", "my_cactus01", Some("my_cacti")), (2, "Lost", "my_gone", None)]
        );
        // Kept for that game: the same list, not read again.
        assert!(std::sync::Arc::ptr_eq(&list, &super::placeables(&game).unwrap()));
        // Without the hak, the game's own.
        let base = small_game(&[("key:data/nwn_base", Some("0 Rock **** plc_rock"), &["plc_rock"])]);
        assert_eq!(super::placeables(&base).unwrap().iter().map(|p| p.model.as_str()).collect::<Vec<_>>(), ["plc_rock"]);
    }

    /// A placeables.2da row with no blueprint of its own gets the plainest blueprint (not the first by name, which has a script),
    /// with the row's appearance and name.
    #[test]
    fn a_row_without_a_blueprint_gets_the_plainest_named_for_the_row() {
        use mg_gff::Value;
        let utp = |appearance: u32, script: &str| {
            let mut g = mg_gff::Gff::new(*b"UTP ");
            g.root.set("Appearance", Value::Dword(appearance));
            g.root.set("OnUsed", Value::resref(mg_core::ResRef::from_str(script).unwrap()));
            g.root.set("LocName", Value::LocString(mg_core::LocString::from_strref(mg_core::StrRef(14530))));
            g.to_bytes().unwrap()
        };
        let rows: String = (0..4).map(|r| format!("{r} Row_{r} **** plc_{r}\n")).collect::<String>() + "4 Saguaro_cactus **** my_cactus\n";
        let mut game = small_game(&[("key:data/nwn_base", Some(&rows), &[])]);
        let mut mem = mg_resman::MemContainer::new();
        mem.insert(mg_resman::ResKey::parse("a_skeleton", mg_core::ResType::UTP).unwrap(), utp(1, "skel_used"));
        mem.insert(mg_resman::ResKey::parse("z_plain", mg_core::ResType::UTP).unwrap(), utp(2, ""));
        game.resman.add(mg_resman::priority::HAK, "hak:bp", mg_resman::LayerClass::Erf, mem);
        // Its own blueprint, as it is.
        let (name, s) = super::blueprint(&game, 1).unwrap();
        assert_eq!((name.as_str(), s.integer("Appearance")), ("a_skeleton", Some(1)));
        // None of its own: the plain one, made into the row's.
        let (name, s) = super::blueprint(&game, 4).unwrap();
        assert_eq!((name.as_str(), s.integer("Appearance")), ("z_plain", Some(4)));
        let l = s.locstring("LocName").unwrap();
        assert_eq!(l.text(mg_core::Language::ENGLISH, mg_core::Gender::Male).as_deref(), Some("Saguaro cactus"));
        assert!(s.locstring("Description").is_some_and(|d| d.text(mg_core::Language::ENGLISH, mg_core::Gender::Male).is_none()));
    }

    #[test]
    fn a_layer_is_a_hak_or_the_games_own() {
        assert_eq!(super::source_name("hak:L64_Placeables"), "l64_placeables");
        for label in ["key:data/nwn_base.key", "override", "development", "workshop:123"] {
            assert_eq!(super::source_name(label), "game", "{label}");
        }
    }

    /// `--haks` or LANDSCAPER_HAKS wins; `module` reads the checkout; unset, a team server asks its review service (and asks
    /// again while it gives none), else none at all, as before the setting.
    #[test]
    fn the_editors_haks_come_from_the_setting_then_the_review_service() {
        let list = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let never = || -> Vec<String> { panic!("not asked") };
        let given = super::editor_haks_from(Haks::List(list(&["a", "b"])), true, never, never);
        assert_eq!(given, EditorHaks { list: list(&["a", "b"]), from: "LANDSCAPER_HAKS", settled: true });
        let module = super::editor_haks_from(Haks::Module, true, never, || list(&["m1", "m2"]));
        assert_eq!((module.list, module.settled), (list(&["m1", "m2"]), true));
        let service = super::editor_haks_from(Haks::Unset, true, || list(&["s1"]), never);
        assert_eq!((service.list, service.from, service.settled), (list(&["s1"]), "the review service", true));
        // The service down (or not answering yet): no haks for now, and not settled, so the next call asks again.
        let down = super::editor_haks_from(Haks::Unset, true, Vec::new, never);
        assert_eq!((down.list.len(), down.settled), (0, false));
        let alone = super::editor_haks_from(Haks::Unset, false, never, never);
        assert_eq!((alone.list.len(), alone.settled), (0, true));
    }

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
        let new = Flat { mesh, decal: decal.clone(), fixed, glow: Vec::new() };
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

    /// A model of two textured meshes, the second glowing orange, and an untextured one between them (nwn-bake leaves it out).
    const GLOWING: &str = "newmodel g\nbeginmodelgeom g\nnode dummy g\n  parent NULL\nendnode\n\
        node trimesh stem\n  parent g\n  bitmap plc_stem\n  verts 3\n    0 0 0\n    1 0 0\n    0 1 0\n  faces 1\n    0 1 2 1 0 0 0 0\nendnode\n\
        node trimesh bare\n  parent g\n  bitmap NULL\n  verts 3\n    0 0 0\n    1 0 0\n    0 1 0\n  faces 1\n    0 1 2 1 0 0 0 0\nendnode\n\
        node trimesh cap\n  parent g\n  bitmap plc_cap\n  selfillumcolor 1 0.5 0\n  verts 3\n    0 0 1\n    1 0 1\n    0 1 1\n  faces 1\n    0 1 2 1 0 0 0 0\nendnode\n\
        endmodelgeom g\ndonemodel g\n";

    #[test]
    fn a_glowing_mesh_keeps_its_glow() {
        let model = mg_mdl::Model::read(GLOWING.as_bytes()).unwrap();
        let groups = vec![("plc_stem".to_string(), 0, 1), ("plc_cap".to_string(), 1, 1)];
        assert_eq!(super::glow_of(&model, &groups), [[0.0; 3], [1.0, 0.5, 0.0]]);
        // Meshes that don't line up with the groups: no glow rather than the wrong mesh's.
        assert!(super::glow_of(&model, &groups[1..]).is_empty());
        assert!(super::glow_of(&model, &[("plc_stem".into(), 0, 1), ("plc_cap".into(), 1, 2)]).is_empty());
        // Nothing glowing: none.
        let dull = mg_mdl::Model::read(GLOWING.replace("  selfillumcolor 1 0.5 0\n", "").as_bytes()).unwrap();
        assert!(super::glow_of(&dull, &groups).is_empty());
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
        let flat = Flat { mesh, decal: Some(decal), fixed: true, glow: Vec::new() };
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
