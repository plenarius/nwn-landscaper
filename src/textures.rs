// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The texture library the editor paints with: the `textures` folder in the data folder, holding a `catalog.json` and, for each
//! texture, its picture (`<id>.png`, at most 512 px) and a small one (`<id>_128.png`) for the palette.
//!
//! Where the pictures come from is not the engine's business, so there are three ways to fill the folder, all writing the same files:
//!
//! - **The starter set** (`starter_textures.json`): ground, rock, sand, snow, floors and walls read from the player's own copy of the
//!   game. It is built the first time the Landscaper runs on a machine with no library, so a fresh install works. None of the game's
//!   art is in this repository: the pictures are made on the machine, from its install.
//! - **`landscaper textures add <list.json>`**: the same, from a list of your own: textures of the game or of the module's haks, by
//!   name (see [`add_listed`]).
//! - **`landscaper textures build <folder>`**: a folder of your own PNG, TGA or DDS pictures (a subfolder is a category).
//! - **`landscaper textures get <pack>`**: a texture pack, downloaded (see [`packs`]).
//!
//! A team's own art, whatever its licence, stays in its own library folder; it never goes through this repository.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mg_core::{ResRef, ResType};
use mg_image::Rgba;
use mg_rules::GameData;
use serde_json::{Value, json};

use crate::paint;

pub mod packs;

const STARTER: &str = include_str!("starter_textures.json");

/// Starter textures whose game texture changed, as (id, the resref it was made from before). A library still holding the old
/// one, untouched, gets the new one ([`refresh_starter`]); one the builder replaced or changed is left alone.
/// `dirt` was `tcm02_dirt02`, a near-black stone that made Dig's and Room's floors black.
const RETIRED: [(&str, &str); 1] = [("dirt", "tcm02_dirt02")];

/// The longest id the Landscaper makes itself: with the `ls_` prefix it is the texture's resref in the hak (16 characters at most).
pub const MAX_ID: usize = 13;
/// The longest id a list may give. The hak's resref takes `ls_` and the first [`MAX_ID`] (see [`paint::resref`]), so two ids
/// that start alike are refused (see `Library::put`).
const MAX_LISTED_ID: usize = 16;

/// The texture ids the editor (landscaper-web) uses without asking the library: a new area's ground (`DEFAULT_TEXTURE` in
/// `js/area.js`), a new interior's walls and a new cave's rock (`Area.create`), dug ground and a room's floor (`js/strokes.js`),
/// and the rock of an entrance's slope (`rockFor` in `js/entrance.js`, `js/view.js`). The starter set has every one, so a fresh
/// install paints and exports a new area as the editor makes it.
pub const EDITOR_DEFAULTS: [&str; 9] =
    ["grassMedium", "wallFieldstone", "rock", "dirt", "rockCliff", "swampRock", "canyonRock", "winterRock", "woodOld"];
/// The picture the editor and the hak use, and the palette's small one, as their longest side.
const RENDER: u32 = 512;
const SWATCH: u32 = 128;

/// One texture to put in the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// Its id in the library: what the editor's paint and a saved area name it by.
    pub id: String,
    /// The name the editor's palette shows.
    pub name: String,
    /// The palette group it goes in (Grass, Rock, Tileset...).
    pub category: String,
    /// The `surfacemat.2da` row of walkable ground painted with it (footsteps): 3 grass, 1 dirt, 4 stone, 5 wood, 19 snow, 20 sand...
    pub surface: u32,
    /// For a texture of the game or a hak: its resref. Empty for a picture from a folder.
    pub resref: String,
    /// How many times its picture repeats each way across the [`paint::TEXTURE_METRES`] a library texture covers: 1, 2 or 4. A
    /// close-up of pebbles or cobbles shown once across 4 m makes every stone huge; repeated, they come out their real size.
    pub repeats: u32,
}

/// The repeats a list may give a texture ([`Spec::repeats`]): the picture still fits the library's size in whole halvings.
const REPEATS: [u32; 3] = [1, 2, 4];

/// `{ "haks": [...], "textures": [{ id, name, category, surface, resref }] }`: the haks (optional) are added to the game's files first.
fn parse_list(text: &str) -> Result<(Vec<String>, Vec<Spec>)> {
    let v: Value = serde_json::from_str(text).context("the list is not JSON")?;
    let haks = v["haks"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect();
    let mut specs = Vec::new();
    for t in v["textures"].as_array().context("the list has no \"textures\"")? {
        let field = |k: &str| t[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let id = field("id").context("a texture without an id")?;
        anyhow::ensure!(valid_id(&id), "'{id}' is not a texture id (1 to {MAX_LISTED_ID} letters and digits)");
        let resref = field("resref").with_context(|| format!("{id}: no resref"))?;
        let repeats = t["repeats"].as_u64().unwrap_or(1) as u32;
        anyhow::ensure!(REPEATS.contains(&repeats), "{id}: repeats is 1, 2 or 4, not {}", t["repeats"]);
        specs.push(Spec {
            repeats,
            name: field("name").unwrap_or_else(|| id.clone()),
            category: field("category").unwrap_or_else(|| "Custom".into()),
            surface: t["surface"].as_u64().unwrap_or(3) as u32,
            resref: resref.to_ascii_lowercase(),
            id,
        });
    }
    let mut seen = std::collections::HashSet::new();
    for s in &specs {
        anyhow::ensure!(seen.insert(s.id.to_ascii_lowercase()), "the id '{}' is in the list twice", s.id);
    }
    Ok((haks, specs))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_LISTED_ID && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// What the library has: `<dir>/catalog.json` as the editor and the exporter read it (`{ "textures": [...] }`), and the texture
/// packs installed in it (`"packs"`, only when there are some).
struct Library {
    dir: PathBuf,
    entries: Vec<Value>,
    packs: Vec<Value>,
}

/// One change to the library at a time: two builders picking a tileset texture, or a pack installed meanwhile, must not write
/// over each other's catalog.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static BUSY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    BUSY.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The starter set's ids (whether or not they are in the library): a pack may not take one.
fn starter_ids() -> Vec<String> {
    parse_list(STARTER).map_or_default(|(_, specs)| specs.into_iter().map(|s| s.id).collect())
}

impl Library {
    /// The library in `dir`: empty when it has no catalog yet, an error when its catalog can't be read ([`crate::packs::read_catalog`]).
    fn open(dir: &Path) -> Result<Library> {
        let [entries, packs] = crate::packs::read_catalog(dir, "texture", ["textures", "packs"])?;
        Ok(Library { dir: dir.to_path_buf(), entries, packs })
    }

    /// Writes `spec`'s pictures and its entry (replacing one with the same id). An id whose resref in the hak is another
    /// entry's is refused: the two would be one texture in the game.
    fn put(&mut self, spec: &Spec, source: &str, original: &str, picture: Rgba) -> Result<()> {
        let hak_name = paint::resref(&spec.id);
        if let Some(other) =
            self.entries.iter().filter_map(|e| e["id"].as_str()).find(|i| !i.eq_ignore_ascii_case(&spec.id) && paint::resref(i) == hak_name)
        {
            bail!("'{}' and '{other}' would both be {hak_name} in the hak: give one another id", spec.id);
        }
        let (render, swatch) = library_pictures(picture);
        std::fs::create_dir_all(&self.dir).with_context(|| format!("creating {}", self.dir.display()))?;
        let (file, small) = (format!("{}.png", spec.id), format!("{}_128.png", spec.id));
        paint::write_png(&self.dir.join(&file), &render)?;
        paint::write_png(&self.dir.join(&small), &swatch)?;
        let entry = json!({
            "id": spec.id, "name": spec.name, "category": spec.category, "source": source, "file": file, "render": file, "swatch": small,
            "surface": spec.surface, "edge": paint::edge_for(&spec.category).name(), "size": [render.width, render.height],
            "original": original,
        });
        let mut entry = entry;
        if spec.repeats > 1 {
            entry["repeats"] = spec.repeats.into();
        }
        match self.entries.iter().position(|e| e["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(&spec.id))) {
            Some(at) => self.entries[at] = entry,
            None => self.entries.push(entry),
        }
        Ok(())
    }

    fn has(&self, id: &str) -> bool {
        self.entries.iter().any(|e| e["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(id)))
    }

    /// The id of the entry made from `original`, if there is one (the same texture under another name).
    fn id_from(&self, original: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e["original"].as_str().is_some_and(|o| o == original))
            .and_then(|e| e["id"].as_str())
            .map(str::to_string)
    }

    /// Where the entry `id` came from (`game:<resref>`, a file name...), if there is one.
    fn original_of(&self, id: &str) -> Option<&str> {
        self.entries.iter().find(|e| e["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(id))).and_then(|e| e["original"].as_str())
    }

    /// How many times the entry `id`'s picture repeats its source ([`Spec::repeats`]; 1 when the entry doesn't say).
    fn repeats_of(&self, id: &str) -> u32 {
        self.entries
            .iter()
            .find(|e| e["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(id)))
            .and_then(|e| e["repeats"].as_u64())
            .map_or(1, |r| r as u32)
    }

    /// The picture file of the entry `id`, if there is one.
    fn render_of(&self, id: &str) -> Option<&str> {
        let e = self.entries.iter().find(|e| e["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(id)))?;
        e["render"].as_str().or(e["file"].as_str())
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("catalog.json");
        // In one step: on a team server everyone's exports read it, and a cut-off catalog fails every one of them.
        let mut catalog = json!({ "textures": self.entries });
        if !self.packs.is_empty() {
            catalog["packs"] = self.packs.clone().into();
        }
        crate::atomic::write(&path, &serde_json::to_vec_pretty(&catalog)?)
    }

    /// The pack the entry `id` came from, if it came from one.
    fn pack_of(&self, id: &str) -> Option<&str> {
        self.entries.iter().find(|e| e["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(id))).and_then(|e| e["pack"].as_str())
    }
}

/// A picture as the library keeps it: opaque, fitted to the render size, and its swatch.
fn library_pictures(picture: Rgba) -> (Rgba, Rgba) {
    let render = fit(opaque(picture), RENDER);
    let swatch = fit(render.clone(), SWATCH);
    (render, swatch)
}

/// `img` repeated `n` times each way (n x n copies side by side): a game picture as a library texture of [`Spec::repeats`] `n`.
fn repeated(img: Rgba, n: u32) -> Rgba {
    if n <= 1 {
        return img;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let mut data = vec![0u8; w * h * 4 * (n * n) as usize];
    let row = w * 4;
    for y in 0..h * n as usize {
        let src = &img.data[(y % h) * row..(y % h + 1) * row];
        for k in 0..n as usize {
            let at = (y * w * n as usize + k * w) * 4;
            data[at..at + row].copy_from_slice(src);
        }
    }
    Rgba { width: img.width * n, height: img.height * n, data }
}

/// Library pictures have no transparency (the game's own can carry a specular mask in alpha).
fn opaque(mut img: Rgba) -> Rgba {
    for px in img.data.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    img
}

/// Halved until no side is longer than `max` (never enlarged).
fn fit(mut img: Rgba, max: u32) -> Rgba {
    while img.width > max || img.height > max {
        img = paint::half(&img);
    }
    img
}

/// A texture of the game or of the haks added to it, as a picture (rows from the bottom, like the library's).
fn game_picture(game: &GameData, resref: &str) -> Option<Rgba> {
    let (t, d) = game.resman.texture(ResRef::from_str(resref).ok()?)?;
    mg_image::read(t, &d).ok().map(|i| i.to_rgba())
}

/// What a build did: how many textures went in, and what could not.
#[derive(Debug, Default)]
pub struct Report {
    /// Textures put in the library.
    pub added: usize,
    /// Textures already there, left as they were.
    pub kept: usize,
    /// What could not go in, and why.
    pub problems: Vec<String>,
}

/// Adds textures of the game and of `haks` (hak names in the user folder's `hak/`) to the library, by resref. A texture already in
/// the library under that id is replaced only with `replace`; one the game does not have is reported, not fatal.
fn add_specs(library: &mut Library, haks: &[String], specs: &[Spec], replace: bool) -> Result<Report> {
    let game = crate::models::open_game(nwn_bake::game::Haks::List(haks.to_vec()), nwn_bake::game::Tlk::None)
        .context("opening the game (set NWN_ROOT, and NWN_HOME for the haks)")?;
    let mut report = Report::default();
    for h in game.missing_haks() {
        report.problems.push(format!("hak {h} was not found in the user folder"));
    }
    for s in specs {
        if library.has(&s.id) && !replace {
            report.kept += 1;
            continue;
        }
        // A pack's texture is the pack's: removing it again must not take one of yours with it.
        if let Some(pack) = library.pack_of(&s.id) {
            report.problems.push(format!("{}: the id is the texture pack {pack}'s; give yours another", s.id));
            continue;
        }
        match game_picture(&game, &s.resref) {
            Some(img) => {
                library.put(s, "Neverwinter Nights", &format!("game:{}", s.resref), repeated(img, s.repeats))?;
                report.added += 1;
            }
            None => report.problems.push(format!("{} ({}): the game has no such texture", s.id, s.resref)),
        }
    }
    Ok(report)
}

/// Builds the starter set from the game's files, adding what the library lacks.
pub fn init() -> Result<Report> {
    let (_, specs) = parse_list(STARTER)?;
    let mut library = Library::open(&paint::texture_dir())?;
    let mut report = add_specs(&mut library, &[], &specs, false)?;
    anyhow::ensure!(
        report.added > 0 || report.kept > 0,
        "none of the starter textures were found in the game's files ({})",
        report.problems.first().cloned().unwrap_or_default()
    );
    library.save()?;
    let replaced = refresh_starter()?.len();
    report.added += replaced;
    report.kept = report.kept.saturating_sub(replaced);
    Ok(report)
}

/// Builds the starter set from `folder`'s pictures instead of the game's files (the web demo, which has no game): `<id>.png`,
/// `.tga` or `.dds` for each starter id, with the starter list's names, groups and footsteps. A missing picture is reported.
pub fn init_from_pictures(folder: &Path) -> Result<Report> {
    let (_, specs) = parse_list(STARTER)?;
    let mut library = Library::open(&paint::texture_dir())?;
    let report = add_pictures(&mut library, folder, &specs)?;
    anyhow::ensure!(
        report.added > 0 || report.kept > 0,
        "none of the starter textures' pictures are in {} ({})",
        folder.display(),
        report.problems.first().cloned().unwrap_or_default()
    );
    library.save()?;
    Ok(report)
}

/// Adds `specs` to `library` from the pictures in `folder` named by their ids, leaving an id the library has as it is.
fn add_pictures(library: &mut Library, folder: &Path, specs: &[Spec]) -> Result<Report> {
    let mut report = Report::default();
    for s in specs {
        if library.has(&s.id) {
            report.kept += 1;
            continue;
        }
        let Some(path) = ["png", "tga", "dds"].iter().map(|e| folder.join(format!("{}.{e}", s.id))).find(|p| p.is_file()) else {
            report.problems.push(format!("{}: no {}.png in {}", s.id, s.id, folder.display()));
            continue;
        };
        match read_picture(&path) {
            Ok(img) => {
                let file = path.file_name().map_or_default(|n| n.to_string_lossy().into_owned());
                // (A picture of the starter id is already the 4 m it covers, as the demo pack's are.)
                library.put(&Spec { repeats: 1, ..s.clone() }, "Your pictures", &file, img)?;
                report.added += 1;
            }
            Err(e) => report.problems.push(format!("{e:#}")),
        }
    }
    Ok(report)
}

/// The starter set as the editor reads it: each texture's id, name, category and game texture, so an area painted with one the
/// library lacks can offer to add it, or a library texture made from the same game texture in its place.
pub fn starter_list() -> Vec<Value> {
    parse_list(STARTER).map_or_default(|(_, specs)| {
        specs.into_iter().map(|s| json!({ "id": s.id, "name": s.name, "category": s.category, "resref": s.resref })).collect()
    })
}

/// The starter specs named by `ids` (any case), and the ids that are not starter textures.
fn starter_named(ids: &[String]) -> (Vec<Spec>, Vec<String>) {
    let specs = parse_list(STARTER).map_or_default(|(_, s)| s);
    let mut found: Vec<Spec> = Vec::new();
    let mut unknown = Vec::new();
    for id in ids {
        match specs.iter().find(|s| s.id.eq_ignore_ascii_case(id)) {
            Some(s) if !found.contains(s) => found.push(s.clone()),
            Some(_) => {}
            None => unknown.push(id.clone()),
        }
    }
    (found, unknown)
}

/// Adds the starter textures `ids` from the game's files, as `textures add` would with their lines of the starter list: for an
/// area painted with them on a library that never had them (a team's own). One the library has is left as it is.
pub fn add_starter(ids: &[String]) -> Result<Report> {
    let _one = lock();
    let (specs, unknown) = starter_named(ids);
    let mut library = Library::open(&paint::texture_dir())?;
    let mut report = if specs.is_empty() { Report::default() } else { add_specs(&mut library, &[], &specs, false)? };
    report.problems.extend(unknown.iter().map(|id| format!("{id}: not a starter texture")));
    if report.added > 0 {
        library.save()?;
    }
    Ok(report)
}

/// Adds the textures a list names (`{ "haks": [...], "textures": [...] }`, see [`parse_list`]); an existing id is replaced.
pub fn add_listed(list: &Path) -> Result<Report> {
    let text = std::fs::read_to_string(list).with_context(|| format!("reading {}", list.display()))?;
    let (haks, specs) = parse_list(&text)?;
    let mut library = Library::open(&paint::texture_dir())?;
    let report = add_specs(&mut library, &haks, &specs, true)?;
    library.save()?;
    Ok(report)
}

/// The library id of the tileset texture `resref` (`tcn01_grass01` is `tcn01grass01`): letters and digits only, and when that is
/// too long its start and four hex digits of the whole name. An id that is already another picture's gets different digits.
fn tileset_id(resref: &str, library: &Library) -> String {
    let base: String = resref.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    let original = format!("game:{}", resref.to_ascii_lowercase());
    for n in 0u32.. {
        let id = if n == 0 && base.len() <= MAX_ID {
            base.clone()
        } else {
            // FNV-1a of the resref (and the try), the same on every machine.
            let h = resref
                .to_ascii_lowercase()
                .bytes()
                .chain(n.to_le_bytes())
                .fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
            format!("{}{:04x}", &base[..base.len().min(MAX_ID - 4)], h & 0xffff)
        };
        if library.original_of(&id).is_none_or(|o| o == original) {
            return id;
        }
    }
    unreachable!()
}

/// What picking a tileset's texture in the editor did: its library entry, and whether it is new.
pub struct Picked {
    /// Its id in the library.
    pub id: String,
    /// Its name in the palette.
    pub name: String,
    /// Whether this pick put it there (else it was there already).
    pub added: bool,
}

/// Puts the tileset texture `resref` (found in the game or `haks`) in the library as a "Tileset" texture, unless it is there
/// already. `surface` is the surfacemat row the ground it was picked from walks as (0: not known, so the name decides).
pub fn add_picked(haks: &[String], resref: &str, surface: u32) -> Result<Picked> {
    let _one = lock();
    anyhow::ensure!(ResRef::from_str(resref).is_ok(), "'{resref}' is not a texture name");
    let mut library = Library::open(&paint::texture_dir())?;
    let name = resref.to_ascii_lowercase();
    // The library may have it already, as the starter set's or an earlier pick.
    if let Some(id) = library.id_from(&format!("game:{name}")) {
        return Ok(Picked { id, name, added: false });
    }
    let id = tileset_id(resref, &library);
    let surface = if surface == 0 { surface_for(&name) } else { surface };
    let spec = Spec { id: id.clone(), name: name.clone(), category: "Tileset".into(), surface, resref: name.clone(), repeats: 1 };
    let report = add_specs(&mut library, haks, &[spec], false)?;
    anyhow::ensure!(report.added == 1, "{}", report.problems.first().cloned().unwrap_or_else(|| format!("{resref} was not added")));
    library.save()?;
    Ok(Picked { id, name, added: true })
}

/// The footsteps row for a category's name.
pub fn surface_for(category: &str) -> u32 {
    let c = category.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| c.contains(w));
    if has(&["mud"]) {
        13
    } else if has(&["snow", "ice"]) {
        19
    } else if has(&["sand"]) {
        20
    } else if has(&["wood", "plank"]) {
        5
    } else if has(&["carpet", "rug"]) {
        9
    } else if has(&["metal"]) {
        10
    } else if has(&["dirt", "earth", "soil", "gravel", "path", "road"]) {
        1
    } else if has(&["rock", "stone", "wall", "floor", "tile", "brick", "cobble"]) {
        4
    } else {
        3
    }
}

/// An id for a picture's file name: letters and digits only, at most [`MAX_ID`], and not one of `taken`'s (ignoring case).
fn folder_id(stem: &str, taken: &std::collections::HashSet<String>) -> String {
    let mut base: String = stem.chars().filter(char::is_ascii_alphanumeric).collect();
    if base.is_empty() {
        base = "texture".into();
    }
    let mut n = 1;
    let mut id: String = base.chars().take(MAX_ID).collect();
    while taken.contains(&id.to_ascii_lowercase()) {
        n += 1;
        let tail = n.to_string();
        id = format!("{}{tail}", base.chars().take(MAX_ID - tail.len()).collect::<String>());
    }
    id
}

/// A file name as a name for people: separators to spaces, first letter up.
fn folder_name(stem: &str) -> String {
    let s: String = stem.chars().map(|c| if c == '_' || c == '-' { ' ' } else { c }).collect();
    let s = s.trim();
    let mut cs = s.chars();
    cs.next().map_or_default(|f| f.to_uppercase().collect::<String>() + cs.as_str())
}

fn read_picture(path: &Path) -> Result<Rgba> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => paint::read_png(path),
        "tga" | "dds" => {
            let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let kind = if ext == "tga" { ResType::TGA } else { ResType::DDS };
            Ok(mg_image::read(kind, &data).with_context(|| format!("decoding {}", path.display()))?.to_rgba())
        }
        _ => bail!("{}: only PNG, TGA and DDS pictures are read", path.display()),
    }
}

/// Puts the PNG, TGA and DDS pictures of `folder` in the library. A picture directly in `folder` gets `category` (default "Custom"),
/// one in a subfolder gets the subfolder's name; `surface` overrides the footsteps row that the category's name suggests.
pub fn build_from_folder(folder: &Path, category: Option<&str>, surface: Option<u32>) -> Result<Report> {
    build_into(&paint::texture_dir(), folder, category, surface)
}

fn build_into(library_dir: &Path, folder: &Path, category: Option<&str>, surface: Option<u32>) -> Result<Report> {
    let mut found: Vec<(PathBuf, String)> = Vec::new();
    let mut scan = |dir: &Path, cat: String| -> Result<()> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for p in files {
            if p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "tga", "dds"].contains(&e.to_ascii_lowercase().as_str())) {
                found.push((p, cat.clone()));
            }
        }
        Ok(())
    };
    scan(folder, category.unwrap_or("Custom").to_string())?;
    let mut subs: Vec<PathBuf> = std::fs::read_dir(folder)?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    subs.sort();
    for d in subs {
        scan(&d, d.file_name().map_or_default(|n| n.to_string_lossy().into_owned()))?;
    }
    anyhow::ensure!(!found.is_empty(), "no PNG, TGA or DDS pictures in {}", folder.display());

    let mut library = Library::open(library_dir)?;
    let mut taken: std::collections::HashSet<String> =
        library.entries.iter().filter_map(|e| e["id"].as_str().map(str::to_ascii_lowercase)).collect();
    let mut report = Report::default();
    for (path, cat) in found {
        let stem = path.file_stem().map_or_default(|s| s.to_string_lossy().into_owned());
        let id = folder_id(&stem, &taken);
        taken.insert(id.to_ascii_lowercase());
        let spec = Spec {
            name: folder_name(&stem),
            surface: surface.unwrap_or_else(|| surface_for(&cat)),
            category: cat,
            resref: String::new(),
            repeats: 1,
            id,
        };
        match read_picture(&path) {
            Ok(img) => {
                library.put(&spec, "Your pictures", &path.to_string_lossy(), img)?;
                report.added += 1;
            }
            Err(e) => report.problems.push(format!("{}: {e:#}", path.display())),
        }
    }
    library.save()?;
    Ok(report)
}

/// Whether `library` holds a [`RETIRED`] starter texture as it was made: its entry names the old game texture and is no pack's.
fn has_retired(library: &Library) -> bool {
    RETIRED.iter().any(|(id, old)| library.pack_of(id).is_none() && library.original_of(id) == Some(&format!("game:{old}")))
}

/// Whether `library` holds a starter texture made from the game at another [`Spec::repeats`] than the starter set's now.
fn has_rescaled(library: &Library) -> bool {
    parse_list(STARTER).map_or_default(|(_, specs)| {
        specs.iter().any(|s| {
            library.pack_of(&s.id).is_none()
                && library.original_of(&s.id) == Some(&format!("game:{}", s.resref))
                && library.repeats_of(&s.id) != s.repeats
        })
    })
}

/// Replaces each [`RETIRED`] starter texture `library` still holds, untouched (its entry names the old game texture and its
/// picture is the one that texture makes), with the starter set's texture for that id, and each starter texture made at another
/// scale (untouched, likewise) at the starter set's. `picture` reads a game texture by resref.
/// Returns the ids replaced; running it again replaces nothing.
fn refresh_retired(library: &mut Library, picture: impl Fn(&str) -> Option<Rgba>) -> Result<Vec<String>> {
    let (_, specs) = parse_list(STARTER)?;
    let mut replaced = Vec::new();
    for (id, old) in RETIRED {
        if library.pack_of(id).is_some() || library.original_of(id) != Some(&format!("game:{old}")) {
            continue;
        }
        let Some(spec) = specs.iter().find(|s| s.id == id) else { continue };
        // (A picture edited in place keeps the old entry: it is the builder's now.)
        let Some(file) = library.render_of(id) else { continue };
        let (Some(was), Ok(now)) = (picture(old), paint::read_png(&library.dir.join(file))) else { continue };
        let was = library_pictures(was).0;
        if (was.width, was.height, &was.data) != (now.width, now.height, &now.data) {
            continue;
        }
        let Some(new) = picture(&spec.resref) else { continue };
        library.put(spec, "Neverwinter Nights", &format!("game:{}", spec.resref), repeated(new, spec.repeats))?;
        replaced.push(id.to_string());
    }
    // Starter textures made at another scale ([`has_rescaled`]), untouched: made again at the starter set's.
    for spec in &specs {
        let id = spec.id.as_str();
        if library.pack_of(id).is_some() || library.original_of(id) != Some(&format!("game:{}", spec.resref)) {
            continue;
        }
        let had = library.repeats_of(id);
        if had == spec.repeats {
            continue;
        }
        let Some(file) = library.render_of(id) else { continue };
        let (Some(source), Ok(now)) = (picture(&spec.resref), paint::read_png(&library.dir.join(file))) else { continue };
        let was = library_pictures(repeated(source.clone(), had)).0;
        if (was.width, was.height, &was.data) != (now.width, now.height, &now.data) {
            continue;
        }
        library.put(spec, "Neverwinter Nights", &format!("game:{}", spec.resref), repeated(source, spec.repeats))?;
        replaced.push(id.to_string());
    }
    Ok(replaced)
}

/// [`refresh_retired`] on the library, reading the game only when it holds a retired or rescaled texture. Its catalog is saved when one
/// was replaced, so the shared hak and the next export of an area painted with it take the new picture.
pub fn refresh_starter() -> Result<Vec<String>> {
    let _one = lock();
    let mut library = Library::open(&paint::texture_dir())?;
    if !has_retired(&library) && !has_rescaled(&library) {
        return Ok(Vec::new());
    }
    let game = crate::models::open_game(nwn_bake::game::Haks::List(Vec::new()), nwn_bake::game::Tlk::None)
        .context("opening the game (set NWN_ROOT)")?;
    let replaced = refresh_retired(&mut library, |resref| game_picture(&game, resref))?;
    if !replaced.is_empty() {
        library.save()?;
    }
    Ok(replaced)
}

/// The first run on a machine: with no library, builds the starter set so painting and exporting work. A failure is reported, not
/// fatal (a team server whose game files are not mounted yet should still start).
pub fn ensure_starter() {
    if paint::texture_dir().join("catalog.json").is_file() {
        match refresh_starter() {
            Ok(ids) if !ids.is_empty() => println!("textures: {} replaced with the starter set's new picture", ids.join(", ")),
            Ok(_) => {}
            Err(e) => eprintln!("The texture library's old starter textures could not be refreshed: {e:#}"),
        }
        // A library made before the starter set had all of them: say how to add the editor's defaults it lacks.
        let library = match Library::open(&paint::texture_dir()) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{e:#}");
                return;
            }
        };
        let missing: Vec<&str> = EDITOR_DEFAULTS.into_iter().filter(|id| !library.has(id)).collect();
        if !missing.is_empty() && !library.entries.is_empty() {
            eprintln!(
                "The texture library has no {}, which the editor uses by default; `landscaper textures init` adds them from your \
                 game files",
                missing.join(", ")
            );
        }
        return;
    }
    println!("No texture library in {}: building the starter set from your game files...", paint::texture_dir().display());
    match init() {
        Ok(r) => print_report(&r),
        Err(e) => eprintln!(
            "The starter texture library could not be built: {e:#}\n  (set NWN_ROOT or --root to the game's folder, or build a \
             library with `landscaper textures build <folder>`)"
        ),
    }
}

fn print_report(r: &Report) {
    println!("textures: {} added, {} already there", r.added, r.kept);
    for p in &r.problems {
        eprintln!("  skipped: {p}");
    }
}

/// `landscaper textures init | add <list.json> | build <folder> [--category NAME] [--surface N] | list | get | list-packs |
/// remove-pack | pack`
pub fn run(command: crate::cli::Textures) -> Result<()> {
    use crate::cli::Textures;
    match command {
        Textures::Init { pictures: None } => print_report(&init()?),
        Textures::Init { pictures: Some(folder) } => print_report(&init_from_pictures(&folder)?),
        Textures::Add { list } => print_report(&add_listed(&list)?),
        Textures::Build { folder, category, surface } => print_report(&build_from_folder(&folder, category.as_deref(), surface)?),
        Textures::List => {
            let library = Library::open(&paint::texture_dir())?;
            println!("{} textures in {}", library.entries.len(), library.dir.display());
            for e in &library.entries {
                println!(
                    "  {:<14} {:<10} {}",
                    e["id"].as_str().unwrap_or(""),
                    e["category"].as_str().unwrap_or(""),
                    e["name"].as_str().unwrap_or("")
                );
            }
        }
        Textures::Get { pack, index } => {
            let progress = |step: &'static str, done: u64, total: u64| {
                if step == "downloading" && total > 0 {
                    eprint!("\r  downloading {} of {} KB   ", done >> 10, total >> 10);
                }
            };
            let (source, expect) = if pack.contains("://")
                || Path::new(&pack).extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
                || Path::new(&pack).is_file()
            {
                (packs::Source::parse(&pack), packs::Expect::default())
            } else {
                let url = index.unwrap_or_else(packs::index_url);
                packs::find_in_index(&packs::fetch_index(&url)?, &pack)?
            };
            let r = packs::install(&source, &expect, &progress)?;
            eprintln!();
            if r.unchanged {
                println!("{} {} is installed already", r.name, r.version);
            } else {
                let was = r.replaced.map_or(String::new(), |v| format!(" (replacing {v})"));
                println!("installed {} {}{was}: {} textures", r.name, r.version, r.textures);
            }
        }
        Textures::ListPacks { index } => {
            let installed = packs::installed()?;
            println!("installed ({}):", installed.len());
            for p in &installed {
                println!(
                    "  {:<24} {:<10} {}",
                    p["id"].as_str().unwrap_or(""),
                    p["version"].as_str().unwrap_or(""),
                    p["name"].as_str().unwrap_or("")
                );
            }
            let url = index.unwrap_or_else(packs::index_url);
            match packs::fetch_index(&url) {
                Ok(list) => {
                    println!("available from {url} ({}):", list.len());
                    for p in &list {
                        println!(
                            "  {:<24} {:<10} {} ({} textures, {} KB; {})",
                            p["id"].as_str().unwrap_or(""),
                            p["version"].as_str().unwrap_or(""),
                            p["name"].as_str().unwrap_or(""),
                            p["textures"].as_u64().unwrap_or(0),
                            p["size"].as_u64().unwrap_or(0) >> 10,
                            p["licence"].as_str().unwrap_or("")
                        );
                    }
                }
                Err(e) => eprintln!("the list of packs ({url}) could not be read: {e:#}"),
            }
        }
        Textures::RemovePack { id, force } => {
            let r = packs::remove(&id, force)?;
            println!("removed {}: {} textures", r.name, r.textures);
            if !r.left_in.is_empty() {
                eprintln!("  these areas paint with them and won't export until it is installed again: {}", r.left_in.join(", "));
            }
        }
        Textures::Pack { list, out, index } => {
            let m = packs::make(&list, &out, index.as_deref())?;
            println!("{}: {} textures, {} KB, sha256 {}", m.zip.display(), m.textures, m.size >> 10, m.sha256);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_starter_list_is_well_formed() {
        let (haks, specs) = parse_list(STARTER).unwrap();
        assert!(haks.is_empty() && specs.len() >= 30);
        let mut resrefs = HashSet::new();
        let mut hak_names = HashSet::new();
        for s in &specs {
            assert!(valid_id(&s.id), "{}", s.id);
            assert!(
                s.resref.len() <= 16 && s.resref.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{}",
                s.resref
            );
            assert!(["Grass", "Earth", "Sand", "Rock", "Snow", "Floors", "Walls"].contains(&s.category.as_str()), "{}", s.category);
            assert!([1, 3, 4, 5, 13, 19, 20].contains(&s.surface), "{} surface {}", s.id, s.surface);
            assert!(resrefs.insert(s.resref.clone()), "{} is listed twice", s.resref);
            // The hak's name for it must not collide either.
            assert!(hak_names.insert(paint::resref(&s.id)), "{} collides in the hak", s.id);
        }
    }

    #[test]
    fn the_starter_set_has_every_texture_the_editor_uses_by_default() {
        let (_, specs) = parse_list(STARTER).unwrap();
        for id in EDITOR_DEFAULTS {
            assert!(specs.iter().any(|s| s.id == id), "the editor uses '{id}' by default, and the starter set hasn't got it");
        }
    }

    #[test]
    fn starter_ids_are_found_in_any_case_and_others_are_named() {
        let ids = ["sandfine", "SNOWPACKED", "sandFine", "myOwn"].map(str::to_string);
        let (specs, unknown) = starter_named(&ids);
        assert_eq!(specs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sandFine", "snowPacked"]);
        assert_eq!(unknown, ["myOwn"]);
        let listed = starter_list();
        assert!(listed.iter().any(|t| t["id"] == "gravelPebbles" && t["resref"].as_str().is_some_and(|r| !r.is_empty())));
    }

    #[test]
    fn a_list_is_checked() {
        assert!(parse_list("not json").is_err());
        assert!(parse_list(r#"{ "textures": [{ "id": "has space", "resref": "x" }] }"#).is_err());
        assert!(parse_list(r#"{ "textures": [{ "id": "waytoolongforanid", "resref": "x" }] }"#).is_err());
        assert!(parse_list(r#"{ "textures": [{ "id": "a" }] }"#).is_err());
        assert!(parse_list(r#"{ "textures": [{ "id": "a", "resref": "x" }, { "id": "A", "resref": "y" }] }"#).is_err());
        let (haks, specs) = parse_list(r#"{ "haks": ["my_hak"], "textures": [{ "id": "a1", "resref": "TTR01_Grass02" }] }"#).unwrap();
        assert_eq!(haks, ["my_hak"]);
        assert_eq!(
            (specs[0].resref.as_str(), specs[0].category.as_str(), specs[0].surface, specs[0].name.as_str()),
            ("ttr01_grass02", "Custom", 3, "a1")
        );
    }

    #[test]
    fn footsteps_follow_the_category() {
        for (c, s) in [
            ("Grass", 3),
            ("Earth", 1),
            ("Gravel path", 1),
            ("Mud flats", 13),
            ("Sand", 20),
            ("Snow & Ice", 19),
            ("Rock", 4),
            ("Stone floors", 4),
            ("Wood", 5),
            ("Carpets", 9),
            ("Custom", 3),
        ] {
            assert_eq!(surface_for(c), s, "{c}");
        }
    }

    #[test]
    fn file_names_become_ids_and_names() {
        let mut taken = HashSet::new();
        let mut ids = Vec::new();
        for stem in ["grass_01", "grass-01", "Grass 01", "a_very_long_picture_name_indeed", "a_very_long_picture_name_again", "!!!"] {
            let id = folder_id(stem, &taken);
            assert!(valid_id(&id), "{stem} -> {id}");
            assert!(taken.insert(id.to_ascii_lowercase()), "{id} twice");
            ids.push(id);
        }
        assert_eq!(ids[..3].iter().map(|i| i.to_ascii_lowercase()).collect::<Vec<_>>(), ["grass01", "grass012", "grass013"]);
        assert_eq!(ids[5], "texture");
        assert_eq!(folder_name("dark_oak-planks"), "Dark oak planks");
    }

    fn picture(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        for (i, px) in img.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            px.copy_from_slice(&[(i % 251) as u8, 90, 30, 0]);
        }
        img
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ls-textures-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_picture_is_fitted_written_and_listed_the_way_the_loader_reads_it() {
        let dir = scratch("put");
        let spec = Spec {
            id: "grassTest".into(),
            name: "Test grass".into(),
            category: "Grass".into(),
            surface: 3,
            resref: String::new(),
            repeats: 1,
        };
        let mut lib = Library::open(&dir).unwrap();
        lib.put(&spec, "test", "x", picture(1024, 1024)).unwrap();
        lib.save().unwrap();
        // A second library object sees it, and putting the same id again replaces it.
        let mut again = Library::open(&dir).unwrap();
        assert_eq!(again.entries.len(), 1);
        again.put(&Spec { name: "Renamed".into(), ..spec.clone() }, "test", "x", picture(128, 64)).unwrap();
        again.save().unwrap();
        let cat: Value = serde_json::from_slice(&std::fs::read(dir.join("catalog.json")).unwrap()).unwrap();
        let entries = cat["textures"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0]["name"].as_str(), entries[0]["size"].clone()), (Some("Renamed"), json!([128, 64])));
        // What load_library and the editor read: the picture named by `render`, the `swatch`, the `surface`.
        let render = paint::read_png(&dir.join(entries[0]["render"].as_str().unwrap())).unwrap();
        assert_eq!((render.width, render.height), (128, 64));
        assert!(dir.join(entries[0]["swatch"].as_str().unwrap()).is_file());
        assert_eq!(entries[0]["surface"], 3);
        // A bigger one is halved to the render size, and alpha is dropped.
        let mut big = Library::open(&dir).unwrap();
        big.put(&Spec { id: "big".into(), ..spec }, "test", "x", picture(2048, 1024)).unwrap();
        let e = big.entries.iter().find(|e| e["id"] == "big").unwrap();
        assert_eq!(e["size"], json!([512, 256]));
        let p = paint::read_png(&dir.join("big.png")).unwrap();
        assert!(p.data.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_missing_catalog_is_an_empty_library_and_one_that_cant_be_read_is_never_written_over() {
        let (src, data) = (scratch("catalog-src"), scratch("catalog-data"));
        std::fs::create_dir_all(&src).unwrap();
        paint::write_png(&src.join("moss_patch.png"), &picture(64, 64)).unwrap();
        // Missing: an empty library, which the build starts.
        assert!(Library::open(&data).unwrap().entries.is_empty());
        assert_eq!(build_into(&data, &src, None, None).unwrap().added, 1);
        // Damaged: refused, by name, and the build writes nothing over it.
        let catalog = data.join("catalog.json");
        let whole = std::fs::read(&catalog).unwrap();
        for (why, damaged) in
            [("cut off", &whole[..whole.len() / 2]), ("not a catalog", b"[]".as_slice()), ("a list that isn't one", br#"{"textures": {}}"#)]
        {
            std::fs::write(&catalog, damaged).unwrap();
            let e = format!("{:#}", build_into(&data, &src, None, None).unwrap_err());
            assert!(e.contains("the texture library's catalog.json") && e.contains("nothing was changed"), "{why}: {e}");
            assert_eq!(std::fs::read(&catalog).unwrap(), damaged, "{why}: not written over");
        }
        // Unreadable (a folder where the file is: no system reads that as a file, nor as a missing one): refused, and left.
        std::fs::remove_file(&catalog).unwrap();
        std::fs::create_dir(&catalog).unwrap();
        let e = format!("{:#}", build_into(&data, &src, None, None).unwrap_err());
        assert!(e.contains("the texture library's catalog.json can't be read"), "{e}");
        assert!(catalog.is_dir() && std::fs::read_dir(&catalog).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn two_ids_that_would_share_a_hak_name_are_refused() {
        let dir = scratch("hak-names");
        let mut library = Library::open(&dir).unwrap();
        let spec =
            |id: &str| Spec { id: id.into(), name: id.into(), category: "Walls".into(), surface: 4, resref: String::new(), repeats: 1 };
        library.put(&spec("wallFieldstone"), "test", "a", picture(8, 8)).unwrap();
        // The same id again replaces it; another that starts with the same 13 letters can't go in beside it.
        library.put(&spec("wallFieldstone"), "test", "a", picture(8, 8)).unwrap();
        let e = library.put(&spec("wallFieldston2"), "test", "b", picture(8, 8)).unwrap_err().to_string();
        assert!(e.contains("ls_wallfieldston"), "{e}");
        library.put(&spec("wallField"), "test", "c", picture(8, 8)).unwrap();
        assert_eq!(library.entries.len(), 2);
    }

    #[test]
    fn a_folder_of_pictures_builds_a_library_with_categories() {
        let (src, data) = (scratch("src"), scratch("data"));
        std::fs::create_dir_all(src.join("Sand")).unwrap();
        paint::write_png(&src.join("moss_patch.png"), &picture(64, 64)).unwrap();
        paint::write_png(&src.join("Sand").join("beach.png"), &picture(64, 64)).unwrap();
        std::fs::write(src.join("notes.txt"), "not a picture").unwrap();
        let report = build_into(&data, &src, None, None).unwrap();
        assert_eq!((report.added, report.problems.len()), (2, 0));
        let lib = Library::open(&data).unwrap();
        let by = |id: &str| lib.entries.iter().find(|e| e["id"] == id).unwrap().clone();
        assert_eq!((by("mosspatch")["category"].as_str(), by("mosspatch")["surface"].as_u64()), (Some("Custom"), Some(3)));
        assert_eq!((by("beach")["category"].as_str(), by("beach")["surface"].as_u64()), (Some("Sand"), Some(20)));
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn the_starter_set_builds_from_pictures_with_its_names_and_footsteps() {
        let (src, data) = (scratch("starter-src"), scratch("starter-data"));
        std::fs::create_dir_all(&src).unwrap();
        paint::write_png(&src.join("grassMedium.png"), &picture(64, 64)).unwrap();
        paint::write_png(&src.join("mudSwamp.png"), &picture(64, 64)).unwrap();
        paint::write_png(&src.join("notAStarter.png"), &picture(64, 64)).unwrap();
        let (_, specs) = parse_list(STARTER).unwrap();
        let mut lib = Library::open(&data).unwrap();
        let report = add_pictures(&mut lib, &src, &specs).unwrap();
        // Only starter ids are read; each one without a picture is reported.
        assert_eq!((report.added, report.problems.len()), (2, specs.len() - 2));
        let by = |id: &str| lib.entries.iter().find(|e| e["id"] == id).unwrap().clone();
        assert_eq!((by("grassMedium")["name"].as_str(), by("grassMedium")["category"].as_str()), (Some("Medium grass"), Some("Grass")));
        assert_eq!(by("mudSwamp")["surface"], 13);
        assert!(!lib.has("notAStarter"));
        // Run again: what the library has is kept.
        assert_eq!(add_pictures(&mut lib, &src, &specs).unwrap().kept, 2);
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn the_demo_pack_has_every_starter_texture_within_its_budget() {
        // The web demo (packs/demo) has no game: its pictures stand in for the whole starter set, and are downloaded on a first visit.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/demo");
        let pack: Value = serde_json::from_slice(&std::fs::read(dir.join("pack.json")).unwrap()).unwrap();
        let listed = pack["textures"].as_array().unwrap();
        let (_, specs) = parse_list(STARTER).unwrap();
        assert_eq!(listed.len(), specs.len(), "one picture for each starter texture, and no others");
        let mut total = 0;
        for s in &specs {
            let t = listed.iter().find(|t| t["id"] == s.id.as_str()).unwrap_or_else(|| panic!("the demo pack has no {}", s.id));
            let (sha, size) = crate::export_id::sha256_file(&dir.join("pictures").join(t["file"].as_str().unwrap())).unwrap();
            assert_eq!(t["sha256"].as_str(), Some(sha.as_str()), "{}: run packs/demo/make.py again", s.id);
            total += size;
        }
        assert!(total <= 3 << 20, "the demo pack is {} KB, over its 3 MB budget (packs/demo/README.md)", total >> 10);
    }

    #[test]
    fn a_tileset_texture_gets_an_id_that_fits_and_does_not_clash() {
        let dir = scratch("tileset-ids");
        let mut lib = Library::open(&dir).unwrap();
        assert_eq!(tileset_id("TCN01_Grass01", &lib), "tcn01grass01");
        // Too long for an id: its start and four hex digits of the whole name.
        let long = tileset_id("tdc01_floor_stone01_a", &lib);
        assert!(long.len() <= MAX_ID && long.starts_with("tdc01floo"), "{long}");
        assert_eq!(long, tileset_id("TDC01_FLOOR_STONE01_A", &lib));
        assert_ne!(long, tileset_id("tdc01_floor_stone01_b", &lib));
        // Another picture holding the id: a different one; the same texture: the same.
        lib.entries.push(json!({ "id": "tcn01grass01", "original": "game:something_else" }));
        let other = tileset_id("tcn01_grass01", &lib);
        assert!(other != "tcn01grass01" && valid_id(&other), "{other}");
        lib.entries.push(json!({ "id": "tcn02grass01", "original": "game:tcn02_grass01" }));
        assert_eq!(tileset_id("tcn02_grass01", &lib), "tcn02grass01");
        assert_eq!(lib.id_from("game:tcn02_grass01").as_deref(), Some("tcn02grass01"));
        assert_eq!(lib.id_from("game:nothing"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The starter's `dirt` as an old library holds it: made from the retired texture, `old` its picture.
    fn old_dirt(dir: &Path, old: &Rgba) -> Library {
        let mut lib = Library::open(dir).unwrap();
        let (_, specs) = parse_list(STARTER).unwrap();
        let spec = Spec { resref: RETIRED[0].1.into(), ..specs.into_iter().find(|s| s.id == "dirt").unwrap() };
        lib.put(&spec, "Neverwinter Nights", &format!("game:{}", RETIRED[0].1), old.clone()).unwrap();
        lib.save().unwrap();
        Library::open(dir).unwrap()
    }

    /// Two game textures, the retired dirt and its replacement, as `refresh_retired` reads them.
    fn two_dirts() -> (Rgba, Rgba, impl Fn(&str) -> Option<Rgba>) {
        let old = picture(64, 64);
        let mut new = picture(64, 64);
        new.data.iter_mut().for_each(|v| *v = v.wrapping_add(40));
        let (o, n) = (old.clone(), new.clone());
        let read = move |resref: &str| match resref {
            "tcm02_dirt02" => Some(o.clone()),
            "tni01_dirt02" => Some(n.clone()),
            _ => None,
        };
        (old, new, read)
    }

    #[test]
    fn an_old_starter_dirt_is_replaced_once() {
        let dir = scratch("retired");
        let (old, new, read) = two_dirts();
        let mut lib = old_dirt(&dir, &old);
        assert!(has_retired(&lib));
        assert_eq!(refresh_retired(&mut lib, &read).unwrap(), vec!["dirt".to_string()]);
        lib.save().unwrap();
        let lib = Library::open(&dir).unwrap();
        assert_eq!(lib.original_of("dirt"), Some("game:tni01_dirt02"));
        assert_eq!(paint::read_png(&dir.join("dirt.png")).unwrap().data, library_pictures(new).0.data);
        // Again: nothing left to replace.
        let mut again = Library::open(&dir).unwrap();
        assert!(!has_retired(&again));
        assert!(refresh_retired(&mut again, &read).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_repeated_is_copies_side_by_side() {
        let p = picture(3, 2);
        let r = repeated(p.clone(), 2);
        assert_eq!((r.width, r.height), (6, 4));
        let px = |img: &Rgba, x: u32, y: u32| img.data[((y * img.width + x) * 4) as usize..][..4].to_vec();
        for (x, y) in [(0, 0), (2, 1), (4, 3), (5, 2)] {
            assert_eq!(px(&r, x, y), px(&p, x % 3, y % 2), "({x}, {y})");
        }
        assert_eq!(repeated(p.clone(), 1).data, p.data);
    }

    #[test]
    fn a_starter_texture_made_at_the_old_scale_is_made_again_at_the_new_once_and_an_edited_one_is_kept() {
        let (_, specs) = parse_list(STARTER).unwrap();
        let pebbles = specs.iter().find(|s| s.id == "gravelPebbles").unwrap();
        assert_eq!(pebbles.repeats, 2, "the pebbles' close-up repeats, so its stones come out a hand across, not a foot");
        let game = picture(128, 128);
        let g = game.clone();
        let read = move |resref: &str| (resref == "ttf01_gravel01").then(|| g.clone());
        for edited in [false, true] {
            let dir = scratch(if edited { "rescaled-edited" } else { "rescaled" });
            let mut lib = Library::open(&dir).unwrap();
            // As a library made before `repeats`: the game's picture once, and no repeats in its entry.
            lib.put(&Spec { repeats: 1, ..pebbles.clone() }, "Neverwinter Nights", "game:ttf01_gravel01", game.clone()).unwrap();
            lib.save().unwrap();
            if edited {
                paint::write_png(&dir.join("gravelPebbles.png"), &picture(64, 64)).unwrap();
            }
            let mut lib = Library::open(&dir).unwrap();
            assert!(has_rescaled(&lib));
            let replaced = refresh_retired(&mut lib, &read).unwrap();
            if edited {
                assert!(replaced.is_empty(), "the builder's picture is theirs");
            } else {
                assert_eq!(replaced, vec!["gravelPebbles".to_string()]);
                lib.save().unwrap();
                let lib = Library::open(&dir).unwrap();
                assert_eq!(lib.repeats_of("gravelPebbles"), 2);
                assert_eq!(
                    paint::read_png(&dir.join("gravelPebbles.png")).unwrap().data,
                    library_pictures(repeated(game.clone(), 2)).0.data
                );
                assert!(!has_rescaled(&lib));
                let mut again = Library::open(&dir).unwrap();
                assert!(refresh_retired(&mut again, &read).unwrap().is_empty());
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_dirt_the_builder_changed_or_replaced_is_kept() {
        let (old, _, read) = two_dirts();
        // The old entry with its picture painted over in place.
        let dir = scratch("retired-edited");
        let mut lib = old_dirt(&dir, &old);
        let mut edited = library_pictures(old.clone()).0;
        edited.data[0] ^= 0xff;
        paint::write_png(&dir.join("dirt.png"), &edited).unwrap();
        assert!(refresh_retired(&mut lib, &read).unwrap().is_empty());
        assert_eq!(lib.original_of("dirt"), Some("game:tcm02_dirt02"));
        assert_eq!(paint::read_png(&dir.join("dirt.png")).unwrap().data, edited.data);
        let _ = std::fs::remove_dir_all(&dir);
        // Replaced with another game texture (`textures add`).
        let dir = scratch("retired-replaced");
        let mut lib = Library::open(&dir).unwrap();
        let spec = Spec {
            id: "dirt".into(),
            name: "Dirt".into(),
            category: "Earth".into(),
            surface: 1,
            resref: "ttr01_dirt03".into(),
            repeats: 1,
        };
        lib.put(&spec, "Neverwinter Nights", "game:ttr01_dirt03", old.clone()).unwrap();
        assert!(!has_retired(&lib));
        assert!(refresh_retired(&mut lib, &read).unwrap().is_empty());
        assert_eq!(lib.original_of("dirt"), Some("game:ttr01_dirt03"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_retired_starter_texture_is_no_longer_in_the_starter_set() {
        let (_, specs) = parse_list(STARTER).unwrap();
        for (id, old) in RETIRED {
            let now = specs.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("{id} left the starter set"));
            assert_ne!(now.resref, old, "{id} is still {old}");
        }
    }
}
