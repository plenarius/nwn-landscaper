// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Texture packs: optional sets of textures a user downloads into the library (docs/TEXTURE-PACKS.md has the format).
//!
//! A pack is one zip holding `pack.json` and the pictures in the library's own layout (`<id>.png`, `<id>_128.png`). Installing
//! one reads it into a staging folder inside the library (`.incoming/`, which `/textures/` never serves), checks every file's
//! sha256 and picture, and only then moves the pictures in and writes the catalog in one step ([`crate::atomic::write`]): the
//! catalog is the commit point, so a download or a check that fails leaves the library as it was, and a crash leaves at most
//! pictures that no entry names.
//!
//! Ids never clash. Every texture of a pack starts with the pack's `prefix`, no two installed packs share one, and an id (or
//! its resref in the hak) that is already in the library, or is one of the starter set's, is refused at install: nothing is
//! renamed, because a saved area names its textures by id.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Digest;

use super::{Library, MAX_ID, RENDER, SWATCH};
pub use crate::packs::{Expect, FORMAT, Progress, Source, index_url, valid_pack_id};
use crate::packs::{INDEX_SWATCHES, Kind, Moves, Staging, hex, plain_name};
use crate::paint;

/// The largest picture in a pack (a 512 x 512 RGBA PNG, uncompressed, is 1 MB).
const MAX_FILE: u64 = 16 << 20;
/// The most textures one pack holds.
const MAX_TEXTURES: usize = 4096;
/// The size of the index's swatches.
const INDEX_SWATCH: u32 = 64;

/// One picture of a pack texture: its file name in the zip and the library, and the sha256 of its bytes.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
pub struct PackFile {
    /// `<id>.png` or `<id>_128.png`.
    pub path: String,
    /// Lower-case hex.
    pub sha256: String,
}

/// The two pictures a library texture has.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
pub struct PackFiles {
    /// The picture painted with (at most 512 px a side).
    pub render: PackFile,
    /// The palette's small one (at most 128 px a side).
    pub swatch: PackFile,
}

/// One texture of a pack, as its catalog entry will have it.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
pub struct PackTexture {
    /// Its id in the library: starts with the pack's prefix, letters and digits, at most [`MAX_ID`].
    pub id: String,
    /// The palette's name for it.
    pub name: String,
    /// The palette group it goes in.
    pub category: String,
    /// The `surfacemat.2da` row ground painted with it walks as.
    pub surface: u32,
    /// How its edge meets another texture's, `crisp` or `broken` (unset: what the category suggests, [`paint::edge_for`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    /// Its pictures.
    pub files: PackFiles,
}

/// `pack.json`.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
pub struct Manifest {
    /// [`FORMAT`].
    pub format: u64,
    /// Lower-case letters, digits and `-`: the pack's name in commands and URLs.
    pub id: String,
    /// Any text of letters, digits and `.+-`; a different one replaces the installed pack.
    pub version: String,
    /// What the dialog shows.
    pub name: String,
    #[serde(default)]
    /// A sentence or two.
    pub description: String,
    /// An SPDX id (`CC-BY-4.0`) or a short name of the licence.
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
    /// What every texture id starts with: 2 to 4 lower-case letters and digits, starting with a letter.
    pub prefix: String,
    /// The textures.
    pub textures: Vec<PackTexture>,
}

/// Checks a manifest on its own: its fields, ids and file names (not the library it goes into).
pub fn validate(m: &Manifest) -> Result<()> {
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
    ensure!(!m.textures.is_empty(), "the pack has no textures");
    ensure!(m.textures.len() <= MAX_TEXTURES, "the pack has {} textures; at most {MAX_TEXTURES}", m.textures.len());
    let starter: HashSet<String> = super::starter_ids().into_iter().map(|i| paint::resref(&i)).collect();
    let (mut ids, mut haks) = (HashSet::new(), HashSet::new());
    for t in &m.textures {
        ensure!(
            !t.id.is_empty() && t.id.len() <= MAX_ID && t.id.chars().all(|c| c.is_ascii_alphanumeric()),
            "'{}' is not a texture id (1 to {MAX_ID} letters and digits)",
            t.id
        );
        ensure!(
            t.id.len() > m.prefix.len() && t.id.to_ascii_lowercase().starts_with(&m.prefix),
            "{}: a pack's texture ids start with its prefix '{}'",
            t.id,
            m.prefix
        );
        ensure!(ids.insert(t.id.to_ascii_lowercase()), "the id '{}' is in the pack twice", t.id);
        let hak = paint::resref(&t.id);
        ensure!(haks.insert(hak.clone()), "{}: two textures of the pack would both be {hak} in the hak", t.id);
        ensure!(!starter.contains(&hak), "{}: the starter set has that texture id", t.id);
        ensure!(crate::packs::plain_text(&t.name, 80), "{}: its name is empty or too long", t.id);
        ensure!(crate::packs::plain_text(&t.category, 40), "{}: its category is empty or too long", t.id);
        ensure!(t.surface <= 255, "{}: surface {} is not a surfacemat.2da row", t.id, t.surface);
        ensure!(t.edge.as_deref().is_none_or(|e| e == "crisp" || e == "broken"), "{}: edge is \"crisp\" or \"broken\"", t.id);
        for (f, want) in [(&t.files.render, format!("{}.png", t.id)), (&t.files.swatch, format!("{}_128.png", t.id))] {
            ensure!(f.path == want, "{}: its picture is {}; a pack names it {want}", t.id, f.path);
            ensure!(crate::packs::valid_sha256(&f.sha256), "{}: a bad sha256", f.path);
        }
    }
    Ok(())
}

/// The width and height a PNG says it is, before its pixels are read (a picture that says it is huge is refused unread).
fn png_size(path: &Path) -> Result<(u32, u32)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let reader =
        decoder.read_info().with_context(|| format!("{}: not a PNG", path.file_name().map_or_default(|n| n.display().to_string())))?;
    Ok((reader.info().width, reader.info().height))
}

/// Reads the pack `zip` into `staging` ([`crate::packs::unzip`]): its manifest, checked, and each picture it lists, its sha256
/// and its picture checked.
fn unpack(zip: &Path, staging: &Path, progress: Progress<'_>) -> Result<Manifest> {
    let manifest = crate::packs::unzip(zip, staging, MAX_FILE, progress, |text| {
        let m: Manifest = serde_json::from_slice(text).context("pack.json")?;
        validate(&m)?;
        let mut want = std::collections::HashMap::new();
        for t in &m.textures {
            want.insert(t.files.render.path.clone(), t.files.render.sha256.clone());
            want.insert(t.files.swatch.path.clone(), t.files.swatch.sha256.clone());
        }
        Ok((m, want))
    })?;
    for t in &manifest.textures {
        for (f, max) in [(&t.files.render, RENDER), (&t.files.swatch, SWATCH)] {
            let p = staging.join(&f.path);
            let (w, h) = png_size(&p)?;
            ensure!(w > 0 && h > 0 && w <= max && h <= max, "{}: {w} x {h} px; at most {max} a side", f.path);
            paint::read_png(&p).with_context(|| format!("{}: the picture doesn't read", f.path))?;
        }
    }
    Ok(manifest)
}

/// The catalog's record of an installed pack.
fn record(m: &Manifest, from: &str) -> Value {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut v = json!({
        "id": m.id, "version": m.version, "name": m.name, "description": m.description, "licence": m.licence,
        "credits": m.credits, "prefix": m.prefix, "textures": m.textures.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        "installed": t, "from": from,
    });
    if let Some(l) = &m.licence_text {
        v["licence_text"] = l.clone().into();
    }
    if let Some(h) = &m.homepage {
        v["homepage"] = h.clone().into();
    }
    v
}

impl Library {
    fn pack(&self, id: &str) -> Option<&Value> {
        self.packs.iter().find(|p| p["id"] == id)
    }

    /// The ids the installed pack `id` put in the library.
    fn pack_textures(&self, id: &str) -> Vec<String> {
        self.entries.iter().filter(|e| e["pack"] == id).filter_map(|e| e["id"].as_str().map(str::to_string)).collect()
    }

    /// Why `m` can't go into this library beside what is there (other than an earlier version of itself), if it can't.
    fn clash(&self, m: &Manifest) -> Result<()> {
        if let Some(p) = self.packs.iter().find(|p| p["id"] != m.id.as_str() && p["prefix"].as_str().is_some_and(|x| x == m.prefix)) {
            bail!("the pack {} already uses the prefix '{}': remove it first", p["name"].as_str().unwrap_or("?"), m.prefix);
        }
        for t in &m.textures {
            let hak = paint::resref(&t.id);
            if let Some(e) = self.entries.iter().find(|e| {
                e["pack"] != m.id.as_str() && e["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(&t.id) || paint::resref(i) == hak)
            }) {
                bail!(
                    "{}: the library already has {} (from {}), which would be the same texture in the game; nothing was installed",
                    t.id,
                    e["id"].as_str().unwrap_or("?"),
                    e["source"].as_str().unwrap_or("?")
                );
            }
        }
        Ok(())
    }
}

/// What installing did.
#[derive(Debug, PartialEq)]
pub struct Installed {
    /// The pack's id, name and version.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// The textures it put in the library.
    pub textures: usize,
    /// The version it replaced, if it replaced one.
    pub replaced: Option<String>,
    /// Whether this version was there already (nothing was done).
    pub unchanged: bool,
}

/// Installs a pack into the library: see the module comment. `used` lists the areas that paint with any of some texture ids
/// (a new version that drops a texture an area uses is refused).
pub fn install(source: &Source, expect: &Expect, progress: Progress<'_>) -> Result<Installed> {
    install_into(&paint::texture_dir(), source, expect, progress, &areas_using)
}

fn install_into(
    dir: &Path,
    source: &Source,
    expect: &Expect,
    progress: Progress<'_>,
    used: &dyn Fn(&[String]) -> Vec<String>,
) -> Result<Installed> {
    std::fs::create_dir_all(dir)?;
    let staging = Staging::new(dir)?;
    let zip = crate::packs::fetch(source, &staging.0, expect, progress)?;
    let pictures = staging.0.join("pictures");
    std::fs::create_dir_all(&pictures)?;
    let m = unpack(&zip, &pictures, progress)?;

    let _one = super::lock();
    let mut library = Library::open(dir)?;
    library.clash(&m)?;
    let before = library.pack(&m.id).cloned();
    let replaced = before.as_ref().and_then(|p| p["version"].as_str().map(str::to_string));
    if replaced.as_deref() == Some(m.version.as_str()) {
        return Ok(Installed {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            textures: m.textures.len(),
            replaced,
            unchanged: true,
        });
    }
    // A new version may drop textures: not while an area paints with one.
    let old = library.pack_textures(&m.id);
    let dropped: Vec<String> = old.iter().filter(|o| !m.textures.iter().any(|t| t.id == **o)).cloned().collect();
    if !dropped.is_empty() {
        let areas = used(&dropped);
        if !areas.is_empty() {
            return Err(InUse { pack: m.name.clone(), areas }.into());
        }
    }

    // Move the pictures in, the ones they replace set aside, so a failure puts everything back.
    let mut moves = Moves::new(staging.0.join("aside"))?;
    let total = (m.textures.len() * 2) as u64;
    let result = (|| -> Result<()> {
        for t in &m.textures {
            for f in [&t.files.render, &t.files.swatch] {
                moves.put(&pictures.join(&f.path), dir, &f.path)?;
                progress("installing", moves.len() as u64, total);
            }
        }
        library.entries.retain(|e| e["pack"] != m.id.as_str());
        for t in &m.textures {
            let (w, h) = png_size(&dir.join(&t.files.render.path))?;
            library.entries.push(json!({
                "id": t.id, "name": t.name, "category": t.category, "source": m.name, "file": t.files.render.path,
                "render": t.files.render.path, "swatch": t.files.swatch.path, "surface": t.surface,
                "edge": t.edge.clone().unwrap_or_else(|| paint::edge_for(&t.category).name().into()), "size": [w, h],
                "original": format!("pack:{}", m.id), "pack": m.id,
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
    for id in &dropped {
        let _ = std::fs::remove_file(dir.join(format!("{id}.png")));
        let _ = std::fs::remove_file(dir.join(format!("{id}_128.png")));
    }
    Ok(Installed { id: m.id.clone(), name: m.name.clone(), version: m.version, textures: m.textures.len(), replaced, unchanged: false })
}

/// A pack (or a new version's dropped textures) that areas still paint with.
#[derive(Debug)]
pub struct InUse {
    /// The pack's name.
    pub pack: String,
    /// The areas, as `area` or `user: area` on a team server.
    pub areas: Vec<String>,
}

impl std::fmt::Display for InUse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.areas.len();
        let shown: Vec<&str> = self.areas.iter().take(10).map(String::as_str).collect();
        write!(
            f,
            "{n} area{} with textures of {}: {}{}. Paint over them and export again first: an area whose textures are gone \
             doesn't export, and one exported with them needs them in the texture hak",
            if n == 1 { " paints" } else { "s paint" },
            self.pack,
            shown.join(", "),
            if n > 10 { format!(" and {} more", n - 10) } else { String::new() }
        )
    }
}

impl std::error::Error for InUse {}

/// What removing did.
#[derive(Debug, PartialEq)]
pub struct Removed {
    /// The pack's name.
    pub name: String,
    /// How many textures left the library.
    pub textures: usize,
    /// The areas that still paint with them (only with `force`): they don't export until it is installed again.
    pub left_in: Vec<String>,
}

/// Takes the pack `id` out of the library: its entries, then its pictures. Refused ([`InUse`]) while an area paints with one of
/// its textures, unless `force`.
pub fn remove(id: &str, force: bool) -> Result<Removed> {
    remove_from(&paint::texture_dir(), id, force, &areas_using)
}

fn remove_from(dir: &Path, id: &str, force: bool, used: &dyn Fn(&[String]) -> Vec<String>) -> Result<Removed> {
    let _one = super::lock();
    let mut library = Library::open(dir)?;
    let pack = library.pack(id).cloned().with_context(|| format!("no pack '{id}' is installed"))?;
    let name = pack["name"].as_str().unwrap_or(id).to_string();
    let ids = library.pack_textures(id);
    let areas = used(&ids);
    if !areas.is_empty() && !force {
        return Err(InUse { pack: name, areas }.into());
    }
    library.entries.retain(|e| e["pack"] != id);
    library.packs.retain(|p| p["id"] != id);
    library.save()?;
    // (After the catalog: it never names a picture that is gone.)
    for t in &ids {
        let _ = std::fs::remove_file(dir.join(format!("{t}.png")));
        let _ = std::fs::remove_file(dir.join(format!("{t}_128.png")));
    }
    Ok(Removed { name, textures: ids.len(), left_in: areas })
}

/// The installed packs, as the catalog records them.
pub fn installed() -> Result<Vec<Value>> {
    Ok(Library::open(&paint::texture_dir())?.packs)
}

/// The areas that paint with any of `ids`: working copies and exports, in this machine's data folder, or on a team server in
/// every workspace (as `user: area`).
pub fn areas_using(ids: &[String]) -> Vec<String> {
    areas_using_in(&crate::packs::area_roots(), ids)
}

/// The texture ids an area's ground is painted with: the entries of its `palette` that some sample's material names. A working
/// copy carries its materials as base64, an export's header names its `.materials` file beside it; without materials that read,
/// every entry of the palette counts. (The editor's palette only grows: a texture painted over stays in it.)
fn painted_ids(dir: &Path, palette: &[Value], materials: Option<&str>) -> Vec<String> {
    use base64::Engine;
    let names: Vec<&str> = palette.iter().map(|p| p.as_str().unwrap_or("")).collect();
    let bytes = materials.and_then(|m| {
        if m.ends_with(".materials") && plain_name(m) {
            crate::atomic::read(&dir.join(m)).ok()
        } else {
            base64::engine::general_purpose::STANDARD.decode(m).ok()
        }
    });
    let Some(bytes) = bytes.filter(|b| !b.is_empty()) else { return names.into_iter().map(str::to_string).collect() };
    let mut seen = [false; 256];
    for b in bytes {
        seen[usize::from(b)] = true;
    }
    names.iter().enumerate().filter(|(k, _)| seen.get(*k).copied().unwrap_or(false)).map(|(_, n)| (*n).to_string()).collect()
}

fn areas_using_in(roots: &[(String, PathBuf)], ids: &[String]) -> Vec<String> {
    #[derive(Deserialize)]
    struct Uses {
        #[serde(default)]
        palette: Vec<Value>,
        #[serde(default)]
        materials: Option<String>,
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    crate::packs::areas_where(roots, |dir, bytes| {
        serde_json::from_slice::<Uses>(bytes)
            .is_ok_and(|u| painted_ids(dir, &u.palette, u.materials.as_deref()).iter().any(|p| ids.contains(p.as_str())))
    })
}

/// Reads an index's texture packs ([`crate::packs::fetch_index`]).
pub fn fetch_index(url: &str) -> Result<Vec<Value>> {
    crate::packs::fetch_index(url, Kind::Texture)
}

/// The index entry for `id`.
pub fn find_in_index(index: &[Value], id: &str) -> Result<(Source, Expect)> {
    crate::packs::find_in_index(index, id)
}

// --- Making a pack ----------------------------------------------------------------------------------------------------------

/// The list `landscaper textures pack` reads: the pack's fields as in pack.json, and for each texture a `picture` (PNG, TGA
/// or DDS, relative to the list) instead of its files.
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
    textures: Vec<ListedTexture>,
}

#[derive(Deserialize)]
struct ListedTexture {
    id: String,
    #[serde(default)]
    name: Option<String>,
    category: String,
    #[serde(default)]
    surface: Option<u32>,
    #[serde(default)]
    edge: Option<String>,
    picture: PathBuf,
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
    /// Textures in it.
    pub textures: usize,
}

/// Makes `<out>/<id>-<version>.zip` from `list` (see [`PackList`]): each picture fitted and made opaque as
/// `landscaper textures add` does, its swatch, and pack.json with their sha256. With `index`, adds the pack to that index file
/// (made if missing; an entry of the same id is replaced), its `url` the zip's file name, beside the index.
pub fn make(list: &Path, out: &Path, index: Option<&Path>) -> Result<Made> {
    let text = std::fs::read(list).with_context(|| format!("reading {}", list.display()))?;
    let l: PackList = serde_json::from_slice(&text).with_context(|| format!("{}: not a pack list", list.display()))?;
    let base = list.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(out)?;
    let staging = Staging::new(out)?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut textures = Vec::new();
    let mut swatches: Vec<(String, super::Rgba)> = Vec::new();
    for t in &l.textures {
        let source = base.join(&t.picture);
        let img = super::read_picture(&source)?;
        let (render, swatch) = super::library_pictures(img);
        let mut pf = |name: String, img: &super::Rgba| -> Result<PackFile> {
            let p = staging.0.join(&name);
            paint::write_png(&p, img)?;
            let data = std::fs::read(&p)?;
            let sha256 = hex(&sha2::Sha256::digest(&data));
            files.push((name.clone(), data));
            Ok(PackFile { path: name, sha256 })
        };
        let files = PackFiles { render: pf(format!("{}.png", t.id), &render)?, swatch: pf(format!("{}_128.png", t.id), &swatch)? };
        // (At most as many a category as the index shows: enough to pick from.)
        if swatches.iter().filter(|(c, _)| *c == t.category).count() < INDEX_SWATCHES {
            swatches.push((t.category.clone(), swatch));
        }
        textures.push(PackTexture {
            id: t.id.clone(),
            name: t.name.clone().unwrap_or_else(|| t.id.clone()),
            category: t.category.clone(),
            surface: t.surface.unwrap_or_else(|| super::surface_for(&t.category)),
            edge: t.edge.clone(),
            files,
        });
    }
    let m = Manifest {
        format: FORMAT,
        id: l.id,
        version: l.version,
        name: l.name,
        description: l.description,
        licence: l.licence,
        licence_text: l.licence_text,
        credits: l.credits,
        homepage: l.homepage,
        prefix: l.prefix,
        textures,
    };
    validate(&m)?;
    let zip_path = out.join(format!("{}-{}.zip", m.id, m.version));
    let bytes = crate::packs::zip_pack(&serde_json::to_vec_pretty(&m)?, &files)?;
    crate::atomic::write(&zip_path, &bytes)?;
    let sha256 = hex(&sha2::Sha256::digest(&bytes));
    let made = Made { zip: zip_path.clone(), size: bytes.len() as u64, sha256: sha256.clone(), textures: m.textures.len() };
    if let Some(index) = index {
        let categories: Vec<&str> = m.textures.iter().map(|t| t.category.as_str()).collect::<BTreeSet<_>>().into_iter().collect();
        let shown: Vec<String> = spread(&swatches).into_iter().map(data_uri).collect::<Result<_>>()?;
        let entry = json!({
            "id": m.id, "version": m.version, "name": m.name, "description": m.description, "licence": m.licence,
            "credits": m.credits, "url": zip_path.file_name().map_or_default(|n| n.to_string_lossy().into_owned()),
            "size": made.size, "sha256": sha256, "textures": m.textures.len(), "categories": categories, "swatches": shown,
        });
        crate::packs::add_to_index(index, Kind::Texture, entry)?;
    }
    Ok(made)
}

/// Up to [`INDEX_SWATCHES`] of `swatches` (category, picture), in turn from each category in the order they come, so a pack of
/// many categories shows one of each and a pack of one shows several.
fn spread(swatches: &[(String, super::Rgba)]) -> Vec<&super::Rgba> {
    let mut order: Vec<&str> = Vec::new();
    for (c, _) in swatches {
        if !order.contains(&c.as_str()) {
            order.push(c);
        }
    }
    let mut out = Vec::new();
    for round in 0..INDEX_SWATCHES {
        for c in &order {
            if let Some((_, s)) = swatches.iter().filter(|(k, _)| k == c).nth(round)
                && out.len() < INDEX_SWATCHES
            {
                out.push(s);
            }
        }
    }
    out
}

/// A swatch at the index's size, as a `data:` URI the editor shows without another download.
fn data_uri(img: &super::Rgba) -> Result<String> {
    use base64::Engine;
    let mut small = img.clone();
    while small.width > INDEX_SWATCH || small.height > INDEX_SWATCH {
        small = paint::half(&small);
    }
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, small.width, small.height);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(&small.top_down().data)?;
    }
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ls-packs-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn picture(w: u32, h: u32, shade: u8) -> super::super::Rgba {
        let mut img = super::super::Rgba::new(w, h);
        for (i, px) in img.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            px.copy_from_slice(&[(i % 251) as u8, shade, 30, 255]);
        }
        img
    }

    /// A pack list in `dir` with textures `ids` (category Rock), and its pictures.
    fn list(dir: &Path, id: &str, version: &str, prefix: &str, ids: &[&str]) -> PathBuf {
        let mut textures = Vec::new();
        for (k, t) in ids.iter().enumerate() {
            let p = dir.join(format!("{t}.png"));
            paint::write_png(&p, &picture(1024, 1024, k as u8 * 40)).unwrap();
            textures.push(json!({ "id": t, "name": format!("Name {t}"), "category": "Rock", "picture": p.file_name().unwrap().to_str() }));
        }
        let l = dir.join(format!("{id}-{version}.list.json"));
        std::fs::write(
            &l,
            serde_json::to_vec(&json!({
                "id": id, "version": version, "name": format!("Pack {id}"), "licence": "LicenseRef-placeholder",
                "credits": ["An artist"], "prefix": prefix, "textures": textures,
            }))
            .unwrap(),
        )
        .unwrap();
        l
    }

    fn quiet(_: &'static str, _: u64, _: u64) {}

    fn nobody(_: &[String]) -> Vec<String> {
        Vec::new()
    }

    fn catalog(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join("catalog.json")).unwrap()).unwrap()
    }

    /// A library with one texture of its own (`wallOwn`).
    fn library(name: &str) -> PathBuf {
        let dir = scratch(name);
        let mut lib = Library::open(&dir).unwrap();
        let spec =
            super::super::Spec { id: "wallOwn".into(), name: "Own".into(), category: "Walls".into(), surface: 4, resref: String::new() };
        lib.put(&spec, "test", "x", picture(8, 8, 0)).unwrap();
        lib.save().unwrap();
        dir
    }

    /// The zip `made`, with `pack.json` changed by `edit` and `extra` entries added.
    fn rezip(made: &Path, to: &Path, edit: impl Fn(&mut Value), extra: &[(&str, &[u8])]) {
        let mut a = zip::ZipArchive::new(std::fs::File::open(made).unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(std::fs::File::create(to).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        for k in 0..a.len() {
            let mut e = a.by_index(k).unwrap();
            let name = e.name().to_string();
            let mut data = Vec::new();
            e.read_to_end(&mut data).unwrap();
            if name == "pack.json" {
                let mut v: Value = serde_json::from_slice(&data).unwrap();
                edit(&mut v);
                data = serde_json::to_vec(&v).unwrap();
            }
            z.start_file(name, o).unwrap();
            z.write_all(&data).unwrap();
        }
        for (name, data) in extra {
            z.start_file(*name, o).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn a_pack_is_made_installed_upgraded_and_removed() {
        let (src, dir) = (scratch("make-src"), library("make-lib"));
        let index = src.join("out/index.json");
        let made = make(&list(&src, "demo", "1.0", "dm", &["dmMoss", "dmSlate"]), &src.join("out"), Some(&index)).unwrap();
        assert_eq!(made.textures, 2);
        // The same list makes the same bytes.
        let again = make(&list(&src, "demo", "1.0", "dm", &["dmMoss", "dmSlate"]), &src.join("out2"), None).unwrap();
        assert_eq!(made.sha256, again.sha256);
        let entries = fetch_index(index.to_str().unwrap()).unwrap();
        assert_eq!((entries.len(), entries[0]["textures"].as_u64()), (1, Some(2)));
        // One category of two textures: both shown.
        assert_eq!(entries[0]["swatches"].as_array().unwrap().len(), 2);
        assert!(entries[0]["swatches"][0].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(Path::new(entries[0]["url"].as_str().unwrap()), src.join("out").join("demo-1.0.zip"));

        let (source, expect) = find_in_index(&entries, "demo").unwrap();
        let steps = std::cell::RefCell::new(Vec::new());
        let r = install_into(&dir, &source, &expect, &|s, d, t| steps.borrow_mut().push((s, d, t)), &nobody).unwrap();
        assert_eq!((r.textures, r.unchanged, r.replaced.clone()), (2, false, None));
        assert!(steps.borrow().iter().any(|s| s.0 == "checking" && s.1 == 4 && s.2 == 4));
        let c = catalog(&dir);
        let moss = c["textures"].as_array().unwrap().iter().find(|e| e["id"] == "dmMoss").unwrap();
        assert_eq!((moss["pack"].as_str(), moss["size"].clone(), moss["surface"].as_u64()), (Some("demo"), json!([512, 512]), Some(4)));
        assert_eq!(c["packs"][0]["textures"], json!(["dmMoss", "dmSlate"]));
        // What the exporter reads: the library loads it.
        let lib = Library::open(&dir).unwrap();
        assert!(lib.has("wallOwn") && lib.has("dmSlate"));
        assert_eq!(paint::read_png(&dir.join("dmSlate_128.png")).unwrap().width, 128);
        assert!(!dir.join(".incoming").exists(), "the staging folder is cleared");
        // The same version again changes nothing.
        assert!(install_into(&dir, &source, &expect, &quiet, &nobody).unwrap().unchanged);

        // Version 2 drops dmSlate: refused while an area paints with it, done once none does.
        let v2 = make(&list(&src, "demo", "2.0", "dm", &["dmMoss", "dmClay"]), &src.join("out"), None).unwrap();
        let painting = |ids: &[String]| if ids.iter().any(|i| i == "dmSlate") { vec!["cave".to_string()] } else { vec![] };
        let e = install_into(&dir, &Source::File(v2.zip.clone()), &Expect::default(), &quiet, &painting).unwrap_err();
        assert!(e.downcast_ref::<InUse>().is_some_and(|u| u.areas == ["cave"]), "{e}");
        assert!(dir.join("dmSlate.png").is_file() && Library::open(&dir).unwrap().has("dmSlate"), "a refused upgrade changes nothing");
        let r = install_into(&dir, &Source::File(v2.zip), &Expect::default(), &quiet, &nobody).unwrap();
        assert_eq!(r.replaced.as_deref(), Some("1.0"));
        assert!(!dir.join("dmSlate.png").exists() && dir.join("dmClay.png").is_file());

        // Removing: refused while used, then done.
        let e = remove_from(&dir, "demo", false, &|_: &[String]| vec!["alice: cave".to_string()]).unwrap_err();
        assert!(e.to_string().contains("alice: cave"), "{e}");
        let r = remove_from(&dir, "demo", false, &nobody).unwrap();
        assert_eq!(r.textures, 2);
        let c = catalog(&dir);
        assert_eq!(c["textures"].as_array().unwrap().len(), 1);
        assert!(c.get("packs").is_none());
        assert!(!dir.join("dmMoss.png").exists() && dir.join("wallOwn.png").is_file());
        assert!(remove_from(&dir, "demo", false, &nobody).is_err());
    }

    #[test]
    fn a_damaged_or_unsafe_pack_changes_nothing() {
        let (src, dir) = (scratch("bad-src"), library("bad-lib"));
        let made = make(&list(&src, "demo", "1.0", "dm", &["dmMoss"]), &src.join("out"), None).unwrap();
        let before = std::fs::read(dir.join("catalog.json")).unwrap();
        let files_before: BTreeSet<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        let check = |zip: &Path, why: &str| {
            let e = install_into(&dir, &Source::File(zip.to_path_buf()), &Expect::default(), &quiet, &nobody).unwrap_err();
            assert!(format!("{e:#}").contains(why), "{why}: {e:#}");
            assert_eq!(std::fs::read(dir.join("catalog.json")).unwrap(), before, "{why}");
            let now: BTreeSet<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
            assert_eq!(now, files_before, "{why}: no picture left behind");
        };
        // A wrong sha256.
        let bad = src.join("hash.zip");
        rezip(&made.zip, &bad, |v| v["textures"][0]["files"]["render"]["sha256"] = "0".repeat(64).into(), &[]);
        check(&bad, "sha256");
        // Paths that leave the folder, hidden files, files pack.json doesn't list.
        for (name, why) in [
            ("../evil.png", "plain file names"),
            ("sub/dmMoss.png", "plain file names"),
            (".hidden", "plain file names"),
            ("extra.png", "doesn't list"),
        ] {
            let bad = src.join("extra.zip");
            rezip(&made.zip, &bad, |_| {}, &[(name, b"x")]);
            check(&bad, why);
        }
        // A cut-off zip, and one with a manifest that lies about a picture's name.
        let data = std::fs::read(&made.zip).unwrap();
        let cut = src.join("cut.zip");
        std::fs::write(&cut, &data[..data.len() / 2]).unwrap();
        check(&cut, "cut off");
        let bad = src.join("name.zip");
        rezip(&made.zip, &bad, |v| v["textures"][0]["files"]["render"]["path"] = "../../x.png".into(), &[]);
        check(&bad, "a pack names it");
        // The index's hash and size are checked too.
        let e = install_into(&dir, &Source::File(made.zip.clone()), &Expect { size: None, sha256: Some("a".repeat(64)) }, &quiet, &nobody)
            .unwrap_err();
        assert!(e.to_string().contains("index"), "{e}");
    }

    #[test]
    fn ids_never_clash() {
        let (src, dir) = (scratch("clash-src"), library("clash-lib"));
        // The library's own texture, by id or by its name in the hak.
        let own = make(&list(&src, "own", "1", "wa", &["wallOwn"]), &src.join("out"), None).unwrap();
        let e = install_into(&dir, &Source::File(own.zip), &Expect::default(), &quiet, &nobody).unwrap_err();
        assert!(e.to_string().contains("already has wallOwn"), "{e}");
        // The starter set's, even when it isn't installed.
        assert!(make(&list(&src, "st", "1", "gr", &["grassMedium"]), &src.join("out"), None).unwrap_err().to_string().contains("starter"));
        // Ids must start with the prefix.
        assert!(make(&list(&src, "pre", "1", "dm", &["xxMoss"]), &src.join("out"), None).unwrap_err().to_string().contains("prefix"));
        // Two packs with one prefix.
        let a = make(&list(&src, "alpha", "1", "dm", &["dmMoss"]), &src.join("out"), None).unwrap();
        install_into(&dir, &Source::File(a.zip), &Expect::default(), &quiet, &nobody).unwrap();
        let b = make(&list(&src, "beta", "1", "dm", &["dmOther"]), &src.join("out"), None).unwrap();
        let e = install_into(&dir, &Source::File(b.zip), &Expect::default(), &quiet, &nobody).unwrap_err();
        assert!(e.to_string().contains("prefix 'dm'"), "{e}");
    }

    /// A one-shot HTTP server on a free port answering with `head` and then `body`, closing after.
    fn serve_once(head: String, body: Vec<u8>) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}/pack.zip")
    }

    #[test]
    fn a_download_that_stops_or_runs_over_installs_nothing() {
        let (src, dir) = (scratch("dl-src"), library("dl-lib"));
        let made = make(&list(&src, "demo", "1.0", "dm", &["dmMoss"]), &src.join("out"), None).unwrap();
        let data = std::fs::read(&made.zip).unwrap();
        let before = std::fs::read(dir.join("catalog.json")).unwrap();
        // Says its whole length, sends half.
        let url = serve_once(
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len()),
            data[..data.len() / 2].to_vec(),
        );
        let e = install_into(&dir, &Source::Url(url), &Expect::default(), &quiet, &nobody).unwrap_err();
        assert!(format!("{e:#}").contains("stopped"), "{e:#}");
        assert_eq!(std::fs::read(dir.join("catalog.json")).unwrap(), before);
        // Over the cap: refused from its length, before any of it is kept.
        let e = crate::packs::copy_capped(&mut &data[..], &src.join("x"), Some(10 << 30), 1 << 20, &quiet).unwrap_err();
        assert!(e.to_string().contains("most"), "{e}");
        // And without a length, as it passes the cap.
        let e = crate::packs::copy_capped(&mut &vec![0u8; 3 << 20][..], &src.join("x"), None, 1 << 20, &quiet).unwrap_err();
        assert!(e.to_string().contains("over 1 MB"), "{e}");
        // A whole one installs, its progress in bytes.
        let url = serve_once(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len()), data.clone());
        let last = std::cell::Cell::new((0, 0));
        let expect = Expect { size: Some(data.len() as u64), sha256: Some(made.sha256.clone()) };
        install_into(
            &dir,
            &Source::Url(url),
            &expect,
            &|s, d, t| {
                if s == "downloading" {
                    last.set((d, t));
                }
            },
            &nobody,
        )
        .unwrap();
        assert_eq!(last.get(), (data.len() as u64, data.len() as u64));
        assert!(Library::open(&dir).unwrap().has("dmMoss"));
    }

    #[test]
    fn the_index_shows_swatches_of_every_category_first() {
        let pic = |k: u8| picture(4, 4, k);
        let cat = |c: &str, k: u8| (c.to_string(), pic(k));
        let shade = |v: Vec<&super::super::Rgba>| v.iter().map(|p| p.data[1]).collect::<Vec<_>>();
        let many = [cat("Grass", 1), cat("Grass", 2), cat("Rock", 3), cat("Sand", 4), cat("Rock", 5)];
        assert_eq!(shade(spread(&many)), [1, 3, 4, 2, 5]);
        let one: Vec<_> = (0..9).map(|k| cat("Grass", k)).collect();
        assert_eq!(spread(&one).len(), INDEX_SWATCHES);
    }

    #[test]
    fn areas_using_a_texture_are_the_ones_painted_with_it() {
        use base64::Engine;
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let root = scratch("areas");
        std::fs::create_dir_all(root.join("areas")).unwrap();
        std::fs::create_dir_all(root.join("exports")).unwrap();
        let write = |d: &str, name: &str, v: Value| {
            std::fs::write(root.join(d).join(format!("{name}.json")), serde_json::to_vec(&v).unwrap()).unwrap();
        };
        // Painted with dmMoss (index 1); one with dmMoss in its palette but painted over (no sample is 1); one with no materials.
        write("areas", "cave", json!({ "palette": ["rock", "dmMoss"], "materials": b64(&[0, 1, 1, 0]) }));
        write("areas", "field", json!({ "palette": ["grassMedium", "dmMoss"], "materials": b64(&[0, 0, 0, 0]) }));
        write("areas", "old", json!({ "palette": ["dmSlate"] }));
        write("areas", "cave.bak", json!({ "palette": ["dmMoss"] }));
        // An export names its materials file.
        write("exports", "road", json!({ "palette": ["grassMedium", "dmClay"], "materials": "road.materials" }));
        std::fs::write(root.join("exports/road.materials"), [1u8, 0]).unwrap();
        let roots = [("alice: ".to_string(), root.clone())];
        assert_eq!(areas_using_in(&roots, &["dmMoss".into()]), ["alice: cave"]);
        assert_eq!(areas_using_in(&roots, &["dmSlate".into()]), ["alice: old"]);
        assert_eq!(areas_using_in(&roots, &["dmClay".into()]), ["alice: road"]);
        assert!(areas_using_in(&roots, &["dmOther".into()]).is_empty());
    }
}
