// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Skies the Landscaper adds: rows of `skyboxes.2da`, each four models (dawn, day, dusk, night) and their textures, installed from
//! sky packs (docs/SKY-PACKS.md has the format). A sky pack is a texture pack's sibling: the same index, download, hash and
//! atomic install ([`crate::packs`]), its own checks, and its own library, the `skies` folder in the data folder.
//!
//! - **Rows never clash.** The module's `skyboxes.2da` (the one its haks give, read as the placeables are) decides: at install a
//!   pack's new skies take the first free run of rows from [`FIRST_ROW`] up, and the catalog records each sky's row. An area keeps
//!   the sky's id (its atmosphere's `"sky"`), and the export turns it into the row.
//! - **The hak.** Every installed sky's files go into [`HAK`], with a `skyboxes.2da` that is the module's table with the skies'
//!   rows added. It must come before every other hak of the module that has a `skyboxes.2da`: the export refuses otherwise.
//! - **A pack adds, it never replaces.** Its file names start with its prefix, and a file the game or the module's haks already
//!   have is refused, as is a sky id another pack has.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use mg_core::{ResRef, ResType};
use mg_resman::ResKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest;

use crate::packs::{Expect, Kind, Moves, Progress, Source, Staging, hex, plain_text, valid_sha256};

/// The hak the installed skies go in.
pub const HAK: &str = "ls_skies";
/// The first row a pack's sky may take: below it are the game's rows and room for a module's own.
pub const FIRST_ROW: usize = 32;
/// The last row an area can name (its `.are`'s `SkyBox` is a byte).
const LAST_ROW: usize = 255;
/// The largest file of a sky pack: NWSync refuses a file over 15 MB.
const MAX_FILE: u64 = 15 << 20;
/// The widest (and tallest) texture: older graphics cards stop at 8,192 px.
const MAX_SIDE: u32 = 8192;
/// The most files and skies one pack holds.
const MAX_FILES: usize = 512;
const MAX_SKIES: usize = 64;
/// The table and its columns of the four models, in the order of the times of day.
const TABLE: &str = "skyboxes";
const TIMES: [(&str, &str); 4] = [("dawn", "DAWN"), ("day", "DAY"), ("dusk", "DUSK"), ("night", "NIGHT")];

/// The sky library: `skies` in the data folder.
pub fn sky_dir() -> PathBuf {
    crate::paint::data_dir().join("skies")
}

/// One file of a sky pack: `<resref>.mdl` or `<resref>.dds`, and the sha256 of its bytes.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PackFile {
    /// Its name in the zip and the hak.
    pub path: String,
    /// Lower-case hex.
    pub sha256: String,
}

/// A sky's four models, by resref.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Models {
    /// From dawn.
    pub dawn: String,
    /// By day.
    pub day: String,
    /// From dusk.
    pub dusk: String,
    /// By night.
    pub night: String,
}

impl Models {
    fn all(&self) -> [&str; 4] {
        [&self.dawn, &self.day, &self.dusk, &self.night]
    }
}

/// One sky of a pack.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PackSky {
    /// What an area names it by: [`valid_sky_id`].
    pub id: String,
    /// What the editor shows.
    pub name: String,
    /// The Sky & fog preset it suits, if one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Its models: the pack's or the game's.
    pub models: Models,
}

/// A sky pack's `pack.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Manifest {
    /// [`crate::packs::FORMAT`].
    pub format: u64,
    /// `sky`.
    pub kind: String,
    /// Lower-case letters, digits and `-`.
    pub id: String,
    /// A different one replaces the installed pack.
    pub version: String,
    /// What the editor shows.
    pub name: String,
    /// A sentence or two.
    #[serde(default)]
    pub description: String,
    /// An SPDX id or the licence's short name.
    pub licence: String,
    /// The licence's text, when it isn't a well-known one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence_text: Option<String>,
    /// Who made the art.
    #[serde(default)]
    pub credits: Vec<String>,
    /// Where to read more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// What every file name starts with.
    pub prefix: String,
    /// The models and textures.
    pub files: Vec<PackFile>,
    /// The skies.
    pub skies: Vec<PackSky>,
}

/// Whether `id` can be a sky's id: 1 to 32 lower-case letters, digits and `-`, starting with a letter.
pub fn valid_sky_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether `r` is a resref as the game names files: 1 to 16 lower-case letters, digits and `_`.
fn valid_resref(r: &str) -> bool {
    !r.is_empty() && r.len() <= 16 && r.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A pack file's resref and type, from its name.
fn file_key(path: &str) -> Option<(&str, ResType)> {
    let (stem, ext) = path.rsplit_once('.')?;
    let t = match ext {
        "mdl" => ResType::MDL,
        "dds" => ResType::DDS,
        _ => return None,
    };
    valid_resref(stem).then_some((stem, t))
}

/// Checks a manifest on its own: its fields, ids and file names (not the game or the library it goes into).
pub fn validate(m: &Manifest) -> Result<()> {
    ensure!(m.kind == Kind::Sky.name(), "pack.json says kind '{}': a sky pack's is 'sky'", m.kind);
    crate::packs::check_about(&crate::packs::About {
        format: m.format,
        id: &m.id,
        version: &m.version,
        name: &m.name,
        description: &m.description,
        licence: &m.licence,
        credits: &m.credits,
        prefix: &m.prefix,
    })?;
    // (The Landscaper's own resources start with `ls_`.)
    ensure!(!m.prefix.starts_with("ls"), "'{}': a sky pack's prefix doesn't start with ls", m.prefix);
    ensure!(!m.skies.is_empty(), "the pack has no skies");
    ensure!(m.skies.len() <= MAX_SKIES, "the pack has {} skies; at most {MAX_SKIES}", m.skies.len());
    ensure!(m.files.len() <= MAX_FILES, "the pack has {} files; at most {MAX_FILES}", m.files.len());
    let mut names = BTreeSet::new();
    for f in &m.files {
        let Some((stem, _)) = file_key(&f.path) else {
            bail!("{}: a sky pack's files are <name>.mdl or <name>.dds, the name 1 to 16 lower-case letters, digits and _", f.path)
        };
        ensure!(
            stem.starts_with(&m.prefix) && stem.len() > m.prefix.len(),
            "{}: a pack's file names start with its prefix '{}'",
            f.path,
            m.prefix
        );
        ensure!(valid_sha256(&f.sha256), "{}: a bad sha256", f.path);
        ensure!(names.insert(f.path.as_str()), "the file {} is in the pack twice", f.path);
    }
    let mut ids = BTreeSet::new();
    for s in &m.skies {
        ensure!(valid_sky_id(&s.id), "'{}' is not a sky id (1 to 32 lower-case letters, digits and -, starting with a letter)", s.id);
        ensure!(ids.insert(s.id.as_str()), "the sky '{}' is in the pack twice", s.id);
        ensure!(plain_text(&s.name, 80), "{}: its name is empty or too long", s.id);
        ensure!(s.preset.as_deref().is_none_or(|p| plain_text(p, 40)), "{}: its preset is empty or too long", s.id);
        for model in s.models.all() {
            ensure!(valid_resref(model), "{}: '{model}' is not a model's name (1 to 16 lower-case letters, digits and _)", s.id);
        }
    }
    Ok(())
}

/// What the game, with the module's haks and without [`HAK`], has: the module's `skyboxes.2da`, and whether a resource is there.
pub struct Module<'a> {
    /// The module's `skyboxes.2da` (the game's, when no hak has one).
    pub table: mg_2da::TwoDa,
    /// Whether the game or a module hak has the resource (resref, type).
    pub has: &'a dyn Fn(&str, ResType) -> bool,
}

/// Whether the game has texture `name` in any format the client draws.
fn has_texture(has: &dyn Fn(&str, ResType) -> bool, name: &str) -> bool {
    [ResType::DDS, ResType::TGA, ResType::PLT].into_iter().any(|t| has(name, t))
}

/// Checks the files of a pack read into `dir`: each DDS reads and is at most [`MAX_SIDE`] a side, each model reads and is named
/// as its file, every texture a model draws with is in the pack or the game, and every sky's four models are. `has` is the game
/// ([`Module::has`]); without it, only what the pack holds is checked.
fn check_files(m: &Manifest, dir: &Path, has: Option<&dyn Fn(&str, ResType) -> bool>) -> Result<()> {
    let mut models = BTreeSet::new();
    let mut textures = BTreeSet::new();
    for f in &m.files {
        let Some((stem, t)) = file_key(&f.path) else { continue };
        let data = std::fs::read(dir.join(&f.path)).with_context(|| format!("reading {}", f.path))?;
        if t == ResType::DDS {
            let tex = mg_image::read(ResType::DDS, &data).with_context(|| format!("{}: not a DDS the game reads", f.path))?;
            ensure!(
                tex.width <= MAX_SIDE && tex.height <= MAX_SIDE,
                "{}: {} x {} px; at most {MAX_SIDE} a side (older graphics cards draw nothing larger)",
                f.path,
                tex.width,
                tex.height
            );
            textures.insert(stem.to_string());
        } else {
            models.insert(stem.to_string());
        }
    }
    for f in &m.files {
        let Some((stem, ResType::MDL)) = file_key(&f.path) else { continue };
        let data = std::fs::read(dir.join(&f.path))?;
        let model = mg_mdl::Model::read(&data).map_err(|e| anyhow::anyhow!("{}: not a model ({e})", f.path))?;
        ensure!(
            model.name.eq_ignore_ascii_case(stem),
            "{}: the model inside is named {}; the game finds it by its file's name",
            f.path,
            model.name
        );
        for n in &model.nodes {
            let Some(bitmap) = n.mesh().and_then(|mesh| mesh.textures[0].as_deref()) else { continue };
            let b = bitmap.trim().to_ascii_lowercase();
            if b.is_empty() || b == "null" || textures.contains(&b) {
                continue;
            }
            if let Some(has) = has {
                ensure!(has_texture(has, &b), "{}: it draws with the texture {b}, which neither the pack nor the game has", f.path);
            }
        }
    }
    for s in &m.skies {
        for model in s.models.all() {
            if models.contains(model) {
                continue;
            }
            if let Some(has) = has {
                ensure!(has(model, ResType::MDL), "{}: its model {model} is neither in the pack nor in the game", s.id);
            }
        }
    }
    Ok(())
}

/// Whether row `row` of `table` is free: past its end, or every cell empty.
fn free(table: &mg_2da::TwoDa, row: usize) -> bool {
    table.rows.get(row).is_none_or(|r| r.iter().all(Option::is_none))
}

/// The first of `n` rows in a run, from [`FIRST_ROW`] up, that the module's `table` leaves free and no installed sky (`taken`) has.
fn choose_rows(table: &mg_2da::TwoDa, taken: &BTreeSet<usize>, n: usize) -> Result<usize> {
    if n == 0 {
        return Ok(FIRST_ROW);
    }
    let ok = |r: usize| free(table, r) && !taken.contains(&r);
    (FIRST_ROW..=LAST_ROW + 1 - n).find(|&first| (first..first + n).all(ok)).with_context(|| {
        format!("skyboxes.2da has no {n} free rows in a run between {FIRST_ROW} and {LAST_ROW} (an area's sky is a byte): remove a sky pack first")
    })
}

/// The sky library: `<dir>/catalog.json` (`{ "skies": [...], "packs": [...] }`) and the packs' files.
struct Library {
    dir: PathBuf,
    skies: Vec<Value>,
    packs: Vec<Value>,
}

/// One change to the sky library at a time.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static BUSY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    BUSY.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Library {
    /// The library in `dir`: empty when it has no catalog yet, an error when its catalog can't be read
    /// ([`crate::packs::read_catalog`]).
    fn open(dir: &Path) -> Result<Library> {
        let [skies, packs] = crate::packs::read_catalog(dir, "sky", ["skies", "packs"])?;
        Ok(Library { dir: dir.to_path_buf(), skies, packs })
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        // In one step: it is the commit point of an install, and every export reads it.
        crate::atomic::write(
            &self.dir.join("catalog.json"),
            &serde_json::to_vec_pretty(&json!({ "skies": self.skies, "packs": self.packs }))?,
        )
    }

    fn pack(&self, id: &str) -> Option<&Value> {
        self.packs.iter().find(|p| p["id"] == id)
    }

    fn sky(&self, id: &str) -> Option<&Value> {
        self.skies.iter().find(|s| s["id"] == id)
    }

    /// The files the installed pack `id` put in the library.
    fn pack_files(&self, id: &str) -> Vec<String> {
        self.pack(id).and_then(|p| p["files"].as_array()).into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect()
    }

    /// Every installed sky's row, but `except` pack's.
    fn rows_but(&self, except: &str) -> BTreeSet<usize> {
        self.skies.iter().filter(|s| s["pack"] != except).filter_map(|s| s["row"].as_u64()).map(|r| r as usize).collect()
    }

    /// Why `m` can't go into this library beside what is there (other than an earlier version of itself), if it can't.
    fn clash(&self, m: &Manifest) -> Result<()> {
        for p in self.packs.iter().filter(|p| p["id"] != m.id.as_str()) {
            if p["prefix"] == m.prefix.as_str() {
                bail!("the sky pack {} already uses the prefix '{}': remove it first", p["name"].as_str().unwrap_or("?"), m.prefix);
            }
            if let Some(f) =
                p["files"].as_array().into_iter().flatten().find(|f| m.files.iter().any(|g| f.as_str() == Some(g.path.as_str())))
            {
                bail!("the sky pack {} already has {}", p["name"].as_str().unwrap_or("?"), f.as_str().unwrap_or("?"));
            }
        }
        for s in &m.skies {
            if let Some(o) = self.sky(&s.id).filter(|o| o["pack"] != m.id.as_str()) {
                bail!("the sky '{}' is installed already, from {}: two skies can't share an id", s.id, o["pack"].as_str().unwrap_or("?"));
            }
        }
        Ok(())
    }
}

/// What installing did.
#[derive(Debug, PartialEq)]
pub struct Installed {
    /// The pack's id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Its skies and their rows, in its order.
    pub rows: Vec<(String, usize)>,
    /// The version it replaced, if it replaced one.
    pub replaced: Option<String>,
    /// Whether this version was there already (nothing was done).
    pub unchanged: bool,
}

/// Installs a sky pack into the sky library, its rows chosen in the module's `skyboxes.2da` (see the module comment).
pub fn install(source: &Source, expect: &Expect, progress: Progress<'_>) -> Result<Installed> {
    let game = module_game()?;
    let has = |r: &str, t: ResType| below_ours(game, r, t).is_some();
    let module = Module { table: module_table(game)?, has: &has };
    install_into(&sky_dir(), source, expect, progress, &module, &areas_using)
}

fn install_into(
    dir: &Path,
    source: &Source,
    expect: &Expect,
    progress: Progress<'_>,
    module: &Module<'_>,
    used: &dyn Fn(&[String]) -> Vec<String>,
) -> Result<Installed> {
    std::fs::create_dir_all(dir)?;
    let staging = Staging::new(dir)?;
    let zip = crate::packs::fetch(source, &staging.0, expect, progress)?;
    let files = staging.0.join("files");
    std::fs::create_dir_all(&files)?;
    let m = crate::packs::unzip(&zip, &files, MAX_FILE, progress, |text| {
        let m: Manifest = serde_json::from_slice(text).context("pack.json")?;
        validate(&m)?;
        let want: HashMap<String, String> = m.files.iter().map(|f| (f.path.clone(), f.sha256.clone())).collect();
        Ok((m, want))
    })?;
    check_files(&m, &files, Some(module.has))?;
    for f in &m.files {
        if let Some((stem, t)) = file_key(&f.path) {
            ensure!(
                !(module.has)(stem, t),
                "{}: the game or one of the module's haks has it already. A sky pack adds skies; it doesn't replace files",
                f.path
            );
        }
    }

    let _one = lock();
    let mut library = Library::open(dir)?;
    library.clash(&m)?;
    let replaced = library.pack(&m.id).and_then(|p| p["version"].as_str().map(str::to_string));
    let row_of =
        |l: &Library, id: &str| l.sky(id).filter(|s| s["pack"] == m.id.as_str()).and_then(|s| s["row"].as_u64()).map(|r| r as usize);
    if replaced.as_deref() == Some(m.version.as_str()) {
        let rows = m.skies.iter().map(|s| (s.id.clone(), row_of(&library, &s.id).unwrap_or(0))).collect();
        return Ok(Installed { id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), rows, replaced, unchanged: true });
    }
    // A new version may drop skies: not while an area uses one.
    let old: Vec<String> =
        library.skies.iter().filter(|s| s["pack"] == m.id.as_str()).filter_map(|s| s["id"].as_str().map(str::to_string)).collect();
    let dropped: Vec<String> = old.iter().filter(|o| !m.skies.iter().any(|s| s.id == **o)).cloned().collect();
    let areas = used(&dropped);
    if !areas.is_empty() {
        bail!("{}", in_use(&m.name, &areas));
    }

    // Rows: a sky the pack had keeps its own (areas exported with it name it), while the module leaves it free; the others take
    // the first free run.
    let mut taken = library.rows_but(&m.id);
    let mut rows: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &m.skies {
        // (The module's row may be this sky's own already: a publish put the skies' table into the module.)
        let had = library.sky(&s.id).cloned();
        if let Some(r) =
            row_of(&library, &s.id).filter(|&r| had.as_ref().is_some_and(|h| free_for(&module.table, r, h)) && !taken.contains(&r))
        {
            rows.insert(&s.id, r);
            taken.insert(r);
        }
    }
    let new: Vec<&PackSky> = m.skies.iter().filter(|s| !rows.contains_key(s.id.as_str())).collect();
    let first = choose_rows(&module.table, &taken, new.len())?;
    for (k, s) in new.iter().enumerate() {
        rows.insert(&s.id, first + k);
    }

    let mut moves = Moves::new(staging.0.join("aside"))?;
    let total = m.files.len() as u64;
    let before = library.pack_files(&m.id);
    let result = (|| -> Result<()> {
        for f in &m.files {
            moves.put(&files.join(&f.path), dir, &f.path)?;
            progress("installing", moves.len() as u64, total);
        }
        library.skies.retain(|s| s["pack"] != m.id.as_str());
        for s in &m.skies {
            library.skies.push(json!({
                "id": s.id, "name": s.name, "preset": s.preset, "models": s.models, "pack": m.id, "row": rows[s.id.as_str()],
            }));
        }
        library.packs.retain(|p| p["id"] != m.id.as_str());
        library.packs.push(record(&m, &source.label()));
        library.save()
    })();
    if let Err(e) = result {
        moves.undo();
        return Err(e);
    }
    for f in before.iter().filter(|f| !m.files.iter().any(|g| g.path == **f)) {
        let _ = std::fs::remove_file(dir.join(f));
    }
    Ok(Installed {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        rows: m.skies.iter().map(|s| (s.id.clone(), rows[s.id.as_str()])).collect(),
        replaced,
        unchanged: false,
    })
}

/// The catalog's record of an installed pack.
fn record(m: &Manifest, from: &str) -> Value {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut v = json!({
        "id": m.id, "version": m.version, "name": m.name, "description": m.description, "licence": m.licence,
        "credits": m.credits, "prefix": m.prefix, "skies": m.skies.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        "files": m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), "installed": t, "from": from,
    });
    if let Some(l) = &m.licence_text {
        v["licence_text"] = l.clone().into();
    }
    if let Some(h) = &m.homepage {
        v["homepage"] = h.clone().into();
    }
    v
}

fn in_use(pack: &str, areas: &[String]) -> String {
    let n = areas.len();
    let shown: Vec<&str> = areas.iter().take(10).map(String::as_str).collect();
    format!(
        "{n} area{} skies of {pack}: {}{}. Give them another sky first: an area whose sky is gone doesn't export",
        if n == 1 { " has one of the" } else { "s have" },
        shown.join(", "),
        if n > 10 { format!(" and {} more", n - 10) } else { String::new() }
    )
}

/// What removing did.
#[derive(Debug, PartialEq)]
pub struct Removed {
    /// The pack's name.
    pub name: String,
    /// How many skies left the library.
    pub skies: usize,
    /// The areas that still use them (only with `force`): they don't export until it is installed again.
    pub left_in: Vec<String>,
}

/// Takes the sky pack `id` out of the library. Refused while an area uses one of its skies, unless `force`.
pub fn remove(id: &str, force: bool) -> Result<Removed> {
    remove_from(&sky_dir(), id, force, &areas_using)
}

fn remove_from(dir: &Path, id: &str, force: bool, used: &dyn Fn(&[String]) -> Vec<String>) -> Result<Removed> {
    let _one = lock();
    let mut library = Library::open(dir)?;
    let pack = library.pack(id).cloned().with_context(|| format!("no sky pack '{id}' is installed"))?;
    let name = pack["name"].as_str().unwrap_or(id).to_string();
    let ids: Vec<String> = library.skies.iter().filter(|s| s["pack"] == id).filter_map(|s| s["id"].as_str().map(str::to_string)).collect();
    let areas = used(&ids);
    if !areas.is_empty() && !force {
        bail!("{}", in_use(&name, &areas));
    }
    let files = library.pack_files(id);
    library.skies.retain(|s| s["pack"] != id);
    library.packs.retain(|p| p["id"] != id);
    library.save()?;
    // (After the catalog: it never names a file that is gone.)
    for f in &files {
        let _ = std::fs::remove_file(dir.join(f));
    }
    Ok(Removed { name, skies: ids.len(), left_in: areas })
}

/// The installed skies (id, name, preset, models, pack, row) and packs, as the catalog records them.
pub fn installed() -> Result<(Vec<Value>, Vec<Value>)> {
    let l = Library::open(&sky_dir())?;
    Ok((l.skies, l.packs))
}

/// Whether `row` of `skyboxes.2da` is an installed sky's: an area with that `SkyBox` needs [`HAK`] wherever it goes. With a
/// sky catalog that can't be read, any row from [`FIRST_ROW`] up, where only Landscaper skies go, counts (said in the log).
pub fn is_skys_row(row: u8) -> bool {
    rows_in(&sky_dir()).map_or_else(
        |e| {
            eprintln!("{e:#}: taking skyboxes.2da row {row} as a Landscaper sky's if it is {FIRST_ROW} or more");
            usize::from(row) >= FIRST_ROW
        },
        |rows| rows.contains(&usize::from(row)),
    )
}

/// Every installed sky's row, in the sky library `dir`.
fn rows_in(dir: &Path) -> Result<BTreeSet<usize>> {
    Ok(Library::open(dir)?.rows_but(""))
}

/// The sky an area's atmosphere names (`"sky": "<id>"`), if it names one.
fn sky_of(atmosphere: &Value) -> Option<&str> {
    atmosphere["sky"].as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// The areas whose atmosphere names any of the skies `ids`.
pub fn areas_using(ids: &[String]) -> Vec<String> {
    areas_using_in(&crate::packs::area_roots(), ids)
}

fn areas_using_in(roots: &[(String, PathBuf)], ids: &[String]) -> Vec<String> {
    #[derive(Deserialize)]
    struct Uses {
        #[serde(default)]
        atmosphere: Value,
    }
    if ids.is_empty() {
        return Vec::new();
    }
    crate::packs::areas_where(roots, |_, bytes| {
        serde_json::from_slice::<Uses>(bytes).is_ok_and(|u| sky_of(&u.atmosphere).is_some_and(|s| ids.iter().any(|i| i == s)))
    })
}

// --- The module's table and the export ---------------------------------------------------------------------------------------

/// The game the module's table is read from: the editor's, once it has the module's haks.
fn module_game() -> Result<&'static mg_rules::GameData> {
    let haks = crate::models::editor_haks();
    ensure!(
        haks.settled,
        "the module's haks aren't known yet (the review service hasn't answered): its skyboxes.2da decides a sky's row, so try again \
         in a minute"
    );
    crate::models::game()
}

/// Whether a resource manager layer is [`HAK`].
fn is_ours(label: &str) -> bool {
    label.strip_prefix("hak:").is_some_and(|h| h.eq_ignore_ascii_case(HAK))
}

/// The first layer of `game` that has resource `name` of type `t`, [`HAK`] left out.
fn below_ours(game: &mg_rules::GameData, name: &str, t: ResType) -> Option<usize> {
    let key = ResKey::new(ResRef::cut(name)?, t);
    game.resman.copies(&key).into_iter().find(|&i| !is_ours(&game.resman.layers()[i].label))
}

/// The module's `skyboxes.2da`: the first copy the game's layers have, [`HAK`] left out.
fn module_table(game: &mg_rules::GameData) -> Result<mg_2da::TwoDa> {
    let at = below_ours(game, TABLE, ResType::TWODA).context("the game has no skyboxes.2da")?;
    let key = ResKey::new(ResRef::from_str(TABLE)?, ResType::TWODA);
    let data = game.resman.get_in(at, &key)?;
    mg_2da::TwoDa::parse(&data, game.codepage()).map_err(|e| anyhow::anyhow!("skyboxes.2da ({}): {e}", game.resman.layers()[at].label))
}

/// The hak above [`HAK`] in the module's list (`haks`, first highest) that has its own `skyboxes.2da`, if one is: the module would
/// show that hak's table, without the skies' rows.
fn hak_above(haks: &[String], has_table: impl Fn(&str) -> bool) -> Option<String> {
    let ours = haks.iter().position(|h| h.eq_ignore_ascii_case(HAK))?;
    haks[..ours].iter().find(|h| has_table(h)).cloned()
}

/// `table` with the installed `skies`' rows: label, no name in the talk table, cycling with the time of day, and the four models.
fn merged(table: &mg_2da::TwoDa, skies: &[Value]) -> Result<mg_2da::TwoDa> {
    let mut t = table.clone();
    if t.columns().is_empty() {
        let cols = ["LABEL", "STRING_REF", "CYCLICAL", "DAWN", "DAY", "DUSK", "NIGHT"];
        t = mg_2da::TwoDa::new(cols.iter().map(|c| (*c).to_string()).collect())?;
    }
    let width = t.columns().len();
    for s in skies {
        let (Some(_), Some(row)) = (s["id"].as_str(), s["row"].as_u64().map(|r| r as usize)) else { continue };
        while t.rows.len() <= row {
            t.rows.push(vec![None; width]);
        }
        t.rows[row] = sky_cells(&t, s)?;
    }
    Ok(t)
}

/// The row an installed sky `s` has in `table`'s columns: its label, no name in the talk table, cycling with the time of day, and
/// its four models.
fn sky_cells(table: &mg_2da::TwoDa, s: &Value) -> Result<Vec<Option<String>>> {
    let id = s["id"].as_str().context("a sky without an id")?;
    let mut cells = vec![None; table.columns().len()];
    let mut put = |col: &str, v: &str| {
        if let Some(c) = table.column(col) {
            cells[c] = Some(v.to_string());
        }
    };
    put("LABEL", &format!("LS_{}", id.replace('-', "_")));
    put("CYCLICAL", "1");
    for (time, col) in TIMES {
        let model = s["models"][time].as_str().with_context(|| format!("the sky {id} has no {time} model"))?;
        put(col, model);
    }
    Ok(cells)
}

/// Whether row `row` of the module's `table` is free for the installed sky `s`: free, or already the sky's own row (as after a
/// publish put the skies' table into the module), cells compared without regard to case.
fn free_for(table: &mg_2da::TwoDa, row: usize, s: &Value) -> bool {
    if free(table, row) {
        return true;
    }
    let (Some(have), Ok(want)) = (table.rows.get(row), sky_cells(table, s)) else { return false };
    have.len() == want.len()
        && have.iter().zip(&want).all(|(a, b)| match (a, b) {
            (Some(a), Some(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
            (a, b) => a.is_none() && b.is_none(),
        })
}

/// The sky the loaded area names, if it names one and isn't underground (a cave has no sky).
pub fn area_sky() -> Option<String> {
    let h = crate::heightmap::current()?;
    if h.underground {
        return None;
    }
    h.atmosphere.as_ref().and_then(sky_of).map(str::to_string)
}

/// Makes the export's [`HAK`] for the sky `id` (every installed sky's files and the merged `skyboxes.2da`) in `hak_dir`, after
/// the checks in the module comment; the row `id` is.
pub fn prepare(id: &str, hak_dir: &Path) -> Result<u8> {
    let game = module_game()?;
    let haks = crate::models::editor_haks().list;
    let layer_has = |hak: &str| {
        let key = ResKey::new(ResRef::from_str(TABLE).expect("a resref"), ResType::TWODA);
        game.resman
            .layers()
            .iter()
            .any(|l| l.label.strip_prefix("hak:").is_some_and(|h| h.eq_ignore_ascii_case(hak)) && l.container.contains(&key))
    };
    if let Some(h) = hak_above(&haks, layer_has) {
        bail!(
            "the module lists {h}.hak above {HAK}.hak, and {h} has its own skyboxes.2da: the game would read that one, without the \
             Landscaper's skies. Move {HAK} above {h} in the module's haks"
        );
    }
    if !haks.is_empty() && !haks.iter().any(|h| h.eq_ignore_ascii_case(HAK)) {
        eprintln!("note: the module's haks don't list {HAK}: the exported module does, but the module this area goes into needs it too");
    }
    prepare_in(&sky_dir(), id, &module_table(game)?, game.codepage(), hak_dir)
}

fn prepare_in(dir: &Path, id: &str, table: &mg_2da::TwoDa, codepage: mg_core::Codepage, hak_dir: &Path) -> Result<u8> {
    let library = Library::open(dir)?;
    let sky = library.sky(id).with_context(|| {
        format!("the area's sky '{id}' is not installed: install the sky pack that has it (landscaper skies get), or choose another sky")
    })?;
    let row = sky["row"].as_u64().context("the sky has no row: install its pack again")? as usize;
    for s in &library.skies {
        let r = s["row"].as_u64().unwrap_or(0) as usize;
        if !free_for(table, r, s) {
            let label = table.get(r, "LABEL").unwrap_or("?");
            bail!(
                "the module's skyboxes.2da has its own row {r} ({label}) now, which the sky {} took when it was installed. Remove its \
                 pack (landscaper skies remove-pack {} --force) and install it again for new rows, then export the areas with it again",
                s["id"].as_str().unwrap_or("?"),
                s["pack"].as_str().unwrap_or("?")
            );
        }
    }
    let table = merged(table, &library.skies)?;
    let data = table.to_bytes(codepage)?;
    let mut owned: Vec<(ResRef, ResType, PathBuf)> = Vec::new();
    for p in &library.packs {
        for f in p["files"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let (stem, t) = file_key(f).with_context(|| format!("the sky library names {f}"))?;
            owned.push((ResRef::from_str(stem)?, t, dir.join(f)));
        }
    }
    // Rebuilt when the table or a file changes: a sky's textures run to megabytes.
    let mut h = sha2::Sha256::new();
    h.update(&data);
    for (r, t, p) in &owned {
        let m = std::fs::metadata(p).with_context(|| format!("the sky library's {} is missing: install its pack again", p.display()))?;
        h.update(format!("{r}.{t:?} {} {:?}\n", m.len(), m.modified().ok()).as_bytes());
    }
    let stamp = hex(&h.finalize());
    let path = hak_dir.join(format!("{HAK}.hak"));
    let stamp_path = dir.join(format!("{HAK}.stamp"));
    if !(path.exists() && std::fs::read_to_string(&stamp_path).is_ok_and(|s| s.trim() == stamp)) {
        let mut hak = mg_erf::ErfWriter::new(*b"HAK ");
        let mut files = Vec::with_capacity(owned.len());
        for (r, t, p) in &owned {
            files.push((*r, *t, std::fs::read(p)?));
        }
        for (r, t, d) in &files {
            hak.add(*r, *t, d.as_slice())?;
        }
        hak.add(ResRef::from_str(TABLE)?, ResType::TWODA, data.as_slice())?;
        std::fs::create_dir_all(hak_dir)?;
        crate::atomic::write(&path, &hak.to_bytes()?)?;
        std::fs::write(&stamp_path, &stamp)?;
        println!("skies hak: {} ({} skies)", path.display(), library.skies.len());
    }
    u8::try_from(row).context("a sky's row is over 255")
}

// --- Making a pack -----------------------------------------------------------------------------------------------------------

/// The list `landscaper skies pack` reads: the pack's fields as in pack.json, with `files` the paths of its models and pictures
/// (relative to the list): a `.mdl` or `.dds` goes in as it is, a `.tga` or `.png` picture as a DDS of the same name.
#[derive(Deserialize)]
struct PackList {
    id: String,
    version: String,
    name: String,
    #[serde(default)]
    description: String,
    licence: String,
    #[serde(default)]
    licence_text: Option<String>,
    #[serde(default)]
    credits: Vec<String>,
    #[serde(default)]
    homepage: Option<String>,
    prefix: String,
    files: Vec<PathBuf>,
    skies: Vec<PackSky>,
}

/// What `make` wrote.
#[derive(Debug)]
pub struct Made {
    /// The pack.
    pub zip: PathBuf,
    /// Its size.
    pub size: u64,
    /// Its sha256.
    pub sha256: String,
    /// Skies in it.
    pub skies: usize,
}

/// A picture as the DDS a sky pack holds: halved until it is at most [`MAX_SIDE`] a side.
fn picture_dds(path: &Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut img = match ext.as_str() {
        "png" => crate::paint::read_png(path)?,
        _ => mg_image::read(ResType::TGA, &data).with_context(|| format!("decoding {}", path.display()))?.to_rgba(),
    };
    while img.width > MAX_SIDE || img.height > MAX_SIDE {
        img = crate::paint::half(&img);
    }
    Ok(crate::paint::dds(&img))
}

/// Makes `<out>/<id>-<version>.zip` from `list` (see [`PackList`]), checked as an install checks it (with the game when it opens);
/// with `index`, adds it to that index's sky packs.
pub fn make(list: &Path, out: &Path, index: Option<&Path>) -> Result<Made> {
    let game = crate::models::game().ok();
    let has = |r: &str, t: ResType| game.is_some_and(|g| below_ours(g, r, t).is_some());
    make_with(list, out, index, game.map(|_| &has as &dyn Fn(&str, ResType) -> bool))
}

/// [`make`], what the pack doesn't hold looked up with `has` (none: not checked).
fn make_with(list: &Path, out: &Path, index: Option<&Path>, has: Option<&dyn Fn(&str, ResType) -> bool>) -> Result<Made> {
    let text = std::fs::read(list).with_context(|| format!("reading {}", list.display()))?;
    let l: PackList = serde_json::from_slice(&text).with_context(|| format!("{}: not a sky pack list", list.display()))?;
    let base = list.parent().unwrap_or(Path::new("."));
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for f in &l.files {
        let path = base.join(f);
        let stem =
            path.file_stem().and_then(|s| s.to_str()).with_context(|| format!("{}: no file name", f.display()))?.to_ascii_lowercase();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let (name, data) = match ext.as_str() {
            "mdl" | "dds" => (format!("{stem}.{ext}"), std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?),
            "tga" | "png" => (format!("{stem}.dds"), picture_dds(&path)?),
            _ => bail!("{}: a sky pack takes .mdl and .dds files, and .tga or .png pictures to make a DDS of", f.display()),
        };
        ensure!(data.len() as u64 <= MAX_FILE, "{name} is over {} MB, the most NWSync takes", MAX_FILE >> 20);
        files.push((name, data));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let m = Manifest {
        format: crate::packs::FORMAT,
        kind: Kind::Sky.name().into(),
        id: l.id,
        version: l.version,
        name: l.name,
        description: l.description,
        licence: l.licence,
        licence_text: l.licence_text,
        credits: l.credits,
        homepage: l.homepage,
        prefix: l.prefix,
        files: files.iter().map(|(n, d)| PackFile { path: n.clone(), sha256: hex(&sha2::Sha256::digest(d)) }).collect(),
        skies: l.skies,
    };
    validate(&m)?;
    std::fs::create_dir_all(out)?;
    let staging = Staging::new(out)?;
    for (n, d) in &files {
        std::fs::write(staging.0.join(n), d)?;
    }
    check_files(&m, &staging.0, has)?;
    let zip_path = out.join(format!("{}-{}.zip", m.id, m.version));
    let bytes = crate::packs::zip_pack(&serde_json::to_vec_pretty(&m)?, &files)?;
    crate::atomic::write(&zip_path, &bytes)?;
    let sha256 = hex(&sha2::Sha256::digest(&bytes));
    let made = Made { zip: zip_path.clone(), size: bytes.len() as u64, sha256: sha256.clone(), skies: m.skies.len() };
    if let Some(index) = index {
        let entry = json!({
            "id": m.id, "version": m.version, "name": m.name, "description": m.description, "licence": m.licence,
            "credits": m.credits, "url": zip_path.file_name().map_or_default(|n| n.to_string_lossy().into_owned()),
            "size": made.size, "sha256": sha256, "skies": m.skies.len(), "names": m.skies.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        });
        crate::packs::add_to_index(index, Kind::Sky, entry)?;
    }
    Ok(made)
}

/// `landscaper skies`.
pub fn run(command: crate::cli::Skies) -> Result<()> {
    use crate::cli::Skies;
    match command {
        Skies::Get { pack, index } => {
            let progress = |step: &'static str, done: u64, total: u64| {
                if step == "downloading" && total > 0 {
                    eprint!("\r  downloading {} of {} KB   ", done >> 10, total >> 10);
                }
            };
            let (source, expect) = if Source::given(&pack) {
                (Source::parse(&pack), Expect::default())
            } else {
                let url = index.unwrap_or_else(crate::packs::index_url);
                crate::packs::find_in_index(&crate::packs::fetch_index(&url, Kind::Sky)?, &pack)?
            };
            let r = install(&source, &expect, &progress)?;
            eprintln!();
            if r.unchanged {
                println!("{} {} is installed already", r.name, r.version);
            } else {
                let was = r.replaced.map_or(String::new(), |v| format!(" (replacing {v})"));
                println!("installed {} {}{was}:", r.name, r.version);
            }
            for (id, row) in &r.rows {
                println!("  {id:<24} skyboxes.2da row {row}");
            }
        }
        Skies::List { index } => {
            let (skies, packs) = installed()?;
            println!("installed skies ({}):", skies.len());
            for s in &skies {
                println!(
                    "  {:<24} row {:<4} {:<24} {}",
                    s["id"].as_str().unwrap_or(""),
                    s["row"].as_u64().unwrap_or(0),
                    s["name"].as_str().unwrap_or(""),
                    s["pack"].as_str().unwrap_or("")
                );
            }
            println!("installed sky packs ({}):", packs.len());
            for p in &packs {
                println!(
                    "  {:<24} {:<10} {}",
                    p["id"].as_str().unwrap_or(""),
                    p["version"].as_str().unwrap_or(""),
                    p["name"].as_str().unwrap_or("")
                );
            }
            let url = index.unwrap_or_else(crate::packs::index_url);
            match crate::packs::fetch_index(&url, Kind::Sky) {
                Ok(list) => {
                    println!("available from {url} ({}):", list.len());
                    for p in &list {
                        println!(
                            "  {:<24} {:<10} {} ({} skies, {} KB; {})",
                            p["id"].as_str().unwrap_or(""),
                            p["version"].as_str().unwrap_or(""),
                            p["name"].as_str().unwrap_or(""),
                            p["skies"].as_u64().unwrap_or(0),
                            p["size"].as_u64().unwrap_or(0) >> 10,
                            p["licence"].as_str().unwrap_or("")
                        );
                    }
                }
                Err(e) => eprintln!("the list of packs ({url}) could not be read: {e:#}"),
            }
        }
        Skies::RemovePack { id, force } => {
            let r = remove(&id, force)?;
            println!("removed {}: {} skies", r.name, r.skies);
            if !r.left_in.is_empty() {
                eprintln!("  these areas use them and won't export until it is installed again: {}", r.left_in.join(", "));
            }
        }
        Skies::Pack { list, out, index } => {
            let m = make(&list, &out, index.as_deref())?;
            println!("{}: {} skies, {} KB, sha256 {}", m.zip.display(), m.skies, m.size >> 10, m.sha256);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ls-skies-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn quiet(_: &'static str, _: u64, _: u64) {}

    fn nobody(_: &[String]) -> Vec<String> {
        Vec::new()
    }

    /// A sky model as text: a dome of one mesh drawn with `bitmap`.
    fn model(name: &str, bitmap: &str) -> String {
        format!(
            "newmodel {name}\nsetsupermodel {name} NULL\nclassification effect\nsetanimationscale 1\nbeginmodelgeom {name}\nnode dummy {name}\n  parent NULL\nendnode\nnode trimesh dome\n  parent {name}\n  bitmap {bitmap}\n  verts 3\n    0 0 0\n    1 0 0\n    0 1 0\n  faces 1\n    0 1 2 1 0 1 2 1\n  tverts 3\n    0 0 0\n    1 0 0\n    0 1 0\nendnode\nendmodelgeom {name}\ndonemodel {name}\n"
        )
    }

    /// A picture `w` by `h`.
    fn picture(w: u32, h: u32) -> mg_image::Rgba {
        crate::paint::picture(w, h, |x, y| [x as f32 / w as f32, y as f32 / h as f32, 0.5])
    }

    /// A pack list in `dir`: `skies` (id, its four models), each model drawn with the pack's `<prefix>_sky` texture.
    fn list(dir: &Path, id: &str, version: &str, prefix: &str, skies: &[(&str, [&str; 4])]) -> PathBuf {
        let mut files = BTreeSet::new();
        let tex = format!("{prefix}_sky");
        crate::paint::write_png(&dir.join(format!("{tex}.png")), &picture(64, 16)).unwrap();
        files.insert(format!("{tex}.png"));
        let mut listed = Vec::new();
        for (sky, models) in skies {
            for m in models {
                if m.starts_with(prefix) {
                    std::fs::write(dir.join(format!("{m}.mdl")), model(m, &tex)).unwrap();
                    files.insert(format!("{m}.mdl"));
                }
            }
            let [dawn, day, dusk, night] = models;
            listed.push(json!({ "id": sky, "name": format!("Sky {sky}"), "preset": "dusk", "models": { "dawn": dawn, "day": day, "dusk": dusk, "night": night } }));
        }
        let l = dir.join(format!("{id}-{version}.list.json"));
        let v = json!({
            "id": id, "version": version, "name": format!("Pack {id}"), "licence": "LicenseRef-placeholder",
            "prefix": prefix, "files": files, "skies": listed,
        });
        std::fs::write(&l, serde_json::to_vec(&v).unwrap()).unwrap();
        l
    }

    /// The module's table: the game's seven rows, and a module row at 32.
    fn table(with_32: bool) -> mg_2da::TwoDa {
        let mut text = String::from("2DA V2.0\n\nLABEL STRING_REF CYCLICAL DAWN DAY DUSK NIGHT\n");
        for (k, l) in ["(None)", "Grass_Clear", "Grass_Storm", "Desert_Clear", "Winter_Clear", "Icy", "Black"].iter().enumerate() {
            text += &format!("{k} {l} **** 1 sky_00{k} sky_00{k} sky_00{k} sky_00{k}\n");
        }
        if with_32 {
            for k in 7..32 {
                text += &format!("{k} **** **** **** **** **** **** ****\n");
            }
            text += "32 Module_Own **** 1 m_a m_b m_c m_d\n";
        }
        mg_2da::TwoDa::parse(text.as_bytes(), mg_core::Codepage::default()).unwrap()
    }

    fn stock(name: &str, t: ResType) -> bool {
        (t == ResType::MDL && name.starts_with("sky_")) || (t == ResType::TGA && name.starts_with("tsky"))
    }

    fn install_with(dir: &Path, zip: &Path, module: &Module<'_>) -> Result<Installed> {
        install_into(dir, &Source::File(zip.to_path_buf()), &Expect::default(), &quiet, module, &nobody)
    }

    #[test]
    fn a_pack_is_made_installed_and_takes_free_rows() {
        let (src, dir) = (scratch("make-src"), scratch("make-lib"));
        let index = src.join("out/index.json");
        let l = list(
            &src,
            "dusky",
            "1.0",
            "dk",
            &[("dusk", ["dk_dawn", "dk_day", "dk_dusk", "sky_005"]), ("glow", ["dk_dawn", "dk_day", "dk_day", "dk_day"])],
        );
        let made = make_with(&l, &src.join("out"), Some(&index), None).unwrap();
        assert_eq!(made.skies, 2);
        // The same list makes the same bytes.
        assert_eq!(make_with(&l, &src.join("out2"), None, None).unwrap().sha256, made.sha256);
        let entries = crate::packs::fetch_index(index.to_str().unwrap(), Kind::Sky).unwrap();
        assert_eq!((entries.len(), entries[0]["skies"].as_u64()), (1, Some(2)));
        assert!(crate::packs::fetch_index(index.to_str().unwrap(), Kind::Texture).unwrap().is_empty());

        // The module has its own row 32: the pack's two take 33 and 34.
        let module = Module { table: table(true), has: &stock };
        let r = install_with(&dir, &made.zip, &module).unwrap();
        assert_eq!(r.rows, [("dusk".to_string(), 33), ("glow".to_string(), 34)]);
        assert!(dir.join("dk_sky.dds").is_file() && dir.join("dk_dawn.mdl").is_file());
        assert!(!dir.join(".incoming").exists(), "the staging folder is cleared");
        assert!(install_with(&dir, &made.zip, &module).unwrap().unchanged);

        // Another pack takes the next free run.
        let l2 = list(&src, "misty", "1", "ms", &[("mist", ["ms_a", "ms_a", "ms_a", "ms_a"])]);
        let r2 = install_with(&dir, &make_with(&l2, &src.join("out"), None, None).unwrap().zip, &module).unwrap();
        assert_eq!(r2.rows, [("mist".to_string(), 35)]);

        // Version 2 drops glow and adds dawn: dusk keeps 33, dawn takes the first free row (34, glow's).
        let l3 = list(
            &src,
            "dusky",
            "2.0",
            "dk",
            &[("dusk", ["dk_dawn", "dk_day", "dk_dusk", "sky_005"]), ("dawn", ["dk_dawn", "dk_dawn", "dk_dawn", "dk_dawn"])],
        );
        let v2 = make_with(&l3, &src.join("out"), None, None).unwrap();
        let painting = |ids: &[String]| if ids.iter().any(|i| i == "glow") { vec!["field".to_string()] } else { vec![] };
        let e = install_into(&dir, &Source::File(v2.zip.clone()), &Expect::default(), &quiet, &module, &painting).unwrap_err();
        assert!(e.to_string().contains("field"), "{e}");
        let r3 = install_with(&dir, &v2.zip, &module).unwrap();
        assert_eq!((r3.rows.clone(), r3.replaced.as_deref()), (vec![("dusk".to_string(), 33), ("dawn".to_string(), 34)], Some("1.0")));

        // Removing: refused while used, then done, its files gone.
        let e = remove_from(&dir, "dusky", false, &|_: &[String]| vec!["alice: field".to_string()]).unwrap_err();
        assert!(e.to_string().contains("alice: field"), "{e}");
        assert_eq!(remove_from(&dir, "dusky", false, &nobody).unwrap().skies, 2);
        assert!(!dir.join("dk_dawn.mdl").exists() && dir.join("ms_a.mdl").is_file());
    }

    #[test]
    fn a_pack_adds_and_never_replaces() {
        let (src, dir) = (scratch("clash-src"), scratch("clash-lib"));
        let module = Module { table: table(false), has: &stock };
        // A file the game has.
        let bad = list(&src, "bad", "1", "sky", &[("x", ["sky_001", "sky_001", "sky_001", "sky_001"])]);
        std::fs::write(src.join("sky_001.mdl"), model("sky_001", "sky_sky")).unwrap();
        let zip = make_with(&bad, &src.join("out"), None, None).unwrap().zip;
        let e = install_with(&dir, &zip, &module).unwrap_err();
        assert!(e.to_string().contains("doesn't replace"), "{e}");
        // Two packs with one sky id, or one prefix.
        let a = make_with(&list(&src, "alpha", "1", "al", &[("dusk", ["al_a", "al_a", "al_a", "al_a"])]), &src.join("out"), None, None)
            .unwrap();
        install_with(&dir, &a.zip, &module).unwrap();
        let b =
            make_with(&list(&src, "beta", "1", "be", &[("dusk", ["be_a", "be_a", "be_a", "be_a"])]), &src.join("out"), None, None).unwrap();
        assert!(install_with(&dir, &b.zip, &module).unwrap_err().to_string().contains("share an id"));
        let c = make_with(&list(&src, "gamma", "1", "al", &[("other", ["al_b", "al_b", "al_b", "al_b"])]), &src.join("out"), None, None)
            .unwrap();
        assert!(install_with(&dir, &c.zip, &module).unwrap_err().to_string().contains("prefix 'al'"));
        // A model that is neither in the pack nor in the game; a model named for another file.
        let d = make_with(&list(&src, "delta", "1", "de", &[("d", ["de_a", "de_a", "de_a", "nowhere"])]), &src.join("out"), None, None)
            .unwrap();
        assert!(install_with(&dir, &d.zip, &module).unwrap_err().to_string().contains("nowhere"));
        let l = list(&src, "eps", "1", "ep", &[("e", ["ep_a", "ep_a", "ep_a", "ep_a"])]);
        std::fs::write(src.join("ep_a.mdl"), model("ep_other", "ep_sky")).unwrap();
        assert!(make_with(&l, &src.join("out"), None, None).unwrap_err().to_string().contains("named ep_other"));
    }

    #[test]
    fn textures_are_checked_and_made_to_fit() {
        let src = scratch("tex-src");
        // A picture over 8,192 px is halved to fit; a DDS over it is refused.
        let big = src.join("bg_wide.png");
        crate::paint::write_png(&big, &picture(16384, 8)).unwrap();
        let dds = picture_dds(&big).unwrap();
        let tex = mg_image::read(ResType::DDS, &dds).unwrap();
        assert_eq!((tex.width, tex.height), (8192, 4));
        let m = Manifest {
            format: 1,
            kind: "sky".into(),
            id: "big".into(),
            version: "1".into(),
            name: "Big".into(),
            description: String::new(),
            licence: "CC0-1.0".into(),
            licence_text: None,
            credits: vec![],
            homepage: None,
            prefix: "bg".into(),
            files: vec![PackFile { path: "bg_wide.dds".into(), sha256: "0".repeat(64) }],
            skies: vec![PackSky {
                id: "w".into(),
                name: "W".into(),
                preset: None,
                models: Models { dawn: "sky_001".into(), day: "sky_001".into(), dusk: "sky_001".into(), night: "sky_001".into() },
            }],
        };
        std::fs::write(src.join("bg_wide.dds"), crate::paint::dds(&picture(16384, 4))).unwrap();
        let e = check_files(&m, &src, None).unwrap_err();
        assert!(e.to_string().contains("at most 8192"), "{e}");
        std::fs::write(src.join("bg_wide.dds"), b"not a dds").unwrap();
        assert!(check_files(&m, &src, None).is_err());
    }

    #[test]
    fn rows_are_the_first_free_run_from_32() {
        let t = table(true);
        assert_eq!(choose_rows(&t, &BTreeSet::new(), 3).unwrap(), 33);
        assert_eq!(choose_rows(&table(false), &BTreeSet::new(), 2).unwrap(), 32);
        // A gap too small for the run is passed over.
        let taken: BTreeSet<usize> = [34].into();
        assert_eq!(choose_rows(&t, &taken, 2).unwrap(), 35);
        assert!(choose_rows(&t, &BTreeSet::new(), 250).is_err());
    }

    #[test]
    fn the_export_writes_the_modules_table_with_the_skies_and_turns_the_id_into_the_row() {
        let (src, dir, haks) = (scratch("exp-src"), scratch("exp-lib"), scratch("exp-hak"));
        let module = Module { table: table(true), has: &stock };
        let made = make_with(
            &list(&src, "dusky", "1.0", "dk", &[("dusk", ["dk_dawn", "dk_day", "dk_dusk", "sky_005"])]),
            &src.join("out"),
            None,
            None,
        )
        .unwrap();
        install_with(&dir, &made.zip, &module).unwrap();
        let cp = mg_core::Codepage::default();
        assert_eq!(prepare_in(&dir, "dusk", &module.table, cp, &haks).unwrap(), 33);
        let bytes = std::fs::read(haks.join("ls_skies.hak")).unwrap();
        let hak = mg_erf::Erf::read(&bytes).unwrap();
        let names: BTreeSet<String> = hak.entries.iter().map(mg_erf::Entry::filename).collect();
        for f in ["dk_dawn.mdl", "dk_day.mdl", "dk_dusk.mdl", "dk_sky.dds", "skyboxes.2da"] {
            assert!(names.contains(f), "{f} in {names:?}");
        }
        let key = hak.entries.iter().find(|e| e.restype == ResType::TWODA).unwrap();
        let t = mg_2da::TwoDa::parse(&hak.data(key).unwrap(), cp).unwrap();
        assert_eq!(t.get(32, "LABEL"), Some("Module_Own"), "the module's rows stay");
        assert_eq!(t.get(1, "DAY"), Some("sky_001"));
        assert_eq!(
            (t.get(33, "LABEL"), t.get(33, "CYCLICAL"), t.get(33, "DAY"), t.get(33, "NIGHT")),
            (Some("LS_dusk"), Some("1"), Some("dk_day"), Some("sky_005"))
        );
        assert!(prepare_in(&dir, "nope", &module.table, cp, &haks).unwrap_err().to_string().contains("not installed"));
        // The module took row 33 since: refused, saying which.
        let mut later = module.table.clone();
        later.rows.push(vec![Some("Team_New".into()), None, Some("1".into()), None, None, None, None]);
        let e = prepare_in(&dir, "dusk", &later, cp, &haks).unwrap_err();
        assert!(e.to_string().contains("row 33 (Team_New)"), "{e}");
        // After a publish the module's table has the sky's own row: that is ours, not a clash (case aside).
        let published = t.clone();
        assert_eq!(prepare_in(&dir, "dusk", &published, cp, &haks).unwrap(), 33);
        let mut shouting = published.clone();
        shouting.rows[33][4] = Some("DK_DAY".into());
        assert_eq!(prepare_in(&dir, "dusk", &shouting, cp, &haks).unwrap(), 33);
        // A row that differs (another night model) still refuses.
        let mut changed = published.clone();
        changed.rows[33][6] = Some("sky_001".into());
        let e = prepare_in(&dir, "dusk", &changed, cp, &haks).unwrap_err();
        assert!(e.to_string().contains("row 33 (LS_dusk)"), "{e}");
        // A new version installed over a module that has the sky's own row keeps the row.
        let published_module = Module { table: published, has: &stock };
        let v2 = make_with(
            &list(
                &src,
                "dusky",
                "2.0",
                "dk",
                &[("dusk", ["dk_dawn", "dk_day", "dk_dusk", "sky_005"]), ("dawn", ["dk_dawn", "dk_dawn", "dk_dawn", "dk_dawn"])],
            ),
            &src.join("out"),
            None,
            None,
        )
        .unwrap();
        let r = install_with(&dir, &v2.zip, &published_module).unwrap();
        assert_eq!(r.rows, [("dusk".to_string(), 33), ("dawn".to_string(), 34)]);
    }

    #[test]
    fn only_an_installed_skys_row_is_ours() {
        let (src, dir) = (scratch("ours-src"), scratch("ours-lib"));
        let module = Module { table: table(false), has: &stock };
        let l = list(&src, "dusky", "1", "dk", &[("dusk", ["dk_a", "dk_a", "dk_a", "dk_a"])]);
        install_with(&dir, &make_with(&l, &src.join("out"), None, None).unwrap().zip, &module).unwrap();
        assert_eq!(rows_in(&dir).unwrap(), BTreeSet::from([32]));
        assert!(rows_in(&scratch("ours-none")).unwrap().is_empty(), "no sky library: no row is ours");
        // A catalog that doesn't read is an error, not a library with no skies.
        let damaged = scratch("ours-damaged");
        std::fs::create_dir_all(&damaged).unwrap();
        std::fs::write(damaged.join("catalog.json"), "{\"skies\": [").unwrap();
        let e = format!("{:#}", rows_in(&damaged).unwrap_err());
        assert!(e.contains("the sky library's catalog.json doesn't parse"), "{e}");
    }

    #[test]
    fn a_hak_with_its_own_table_above_ours_is_named() {
        let s = |v: &[&str]| v.iter().map(|h| (*h).to_string()).collect::<Vec<_>>();
        let has = |h: &str| h == "team_2da" || h == "low_2da";
        assert_eq!(hak_above(&s(&["team_2da", "Ls_Skies", "low_2da"]), has), Some("team_2da".into()));
        assert_eq!(hak_above(&s(&["art", "ls_skies", "team_2da"]), has), None);
        assert_eq!(hak_above(&s(&["team_2da"]), has), None, "not listed: the export's own list puts it first");
    }

    #[test]
    fn areas_using_a_sky_are_the_ones_whose_atmosphere_names_it() {
        let root = scratch("areas");
        std::fs::create_dir_all(root.join("areas")).unwrap();
        std::fs::write(root.join("areas/cove.json"), br#"{ "atmosphere": { "sky": "dusk" } }"#).unwrap();
        std::fs::write(root.join("areas/hill.json"), br#"{ "atmosphere": { "time": "cycle" } }"#).unwrap();
        let roots = [("bob: ".to_string(), root)];
        assert_eq!(areas_using_in(&roots, &["dusk".into()]), ["bob: cove"]);
        assert!(areas_using_in(&roots, &["dawn".into()]).is_empty());
    }
}
