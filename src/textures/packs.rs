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
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Digest;

use super::{Library, MAX_ID, RENDER, SWATCH};
use crate::paint;

/// The pack and index format this engine reads.
pub const FORMAT: u64 = 1;
/// Where the list of packs is when `LANDSCAPER_PACKS_URL` is not set: an asset of the `texture-packs` release.
pub const DEFAULT_INDEX: &str = "https://github.com/plenarius/nwn-landscaper/releases/download/texture-packs/index.json";
/// The largest pack downloaded, unless `LANDSCAPER_PACK_MAX_MB` says otherwise. A 512 px texture and its swatch are under 1 MB.
const DEFAULT_MAX_MB: u64 = 512;
/// The largest `pack.json` and index read.
const MAX_MANIFEST: u64 = 4 << 20;
/// The largest picture in a pack (a 512 x 512 RGBA PNG, uncompressed, is 1 MB).
const MAX_FILE: u64 = 16 << 20;
/// The most textures one pack holds.
const MAX_TEXTURES: usize = 4096;
/// How many swatches an index entry shows, and their size.
const INDEX_SWATCHES: usize = 6;
const INDEX_SWATCH: u32 = 64;

/// Where an index lists its packs: `LANDSCAPER_PACKS_URL` (a URL or a file), else [`DEFAULT_INDEX`].
pub fn index_url() -> String {
    std::env::var("LANDSCAPER_PACKS_URL").ok().map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).unwrap_or(DEFAULT_INDEX.into())
}

fn max_pack_bytes() -> u64 {
    std::env::var("LANDSCAPER_PACK_MAX_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&n| n > 0).unwrap_or(DEFAULT_MAX_MB)
        << 20
}

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

fn plain_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

/// Whether `id` can be a pack's id: 1 to 40 lower-case letters, digits and `-`, starting with a letter.
pub fn valid_pack_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Checks a manifest on its own: its fields, ids and file names (not the library it goes into).
pub fn validate(m: &Manifest) -> Result<()> {
    ensure!(m.format == FORMAT, "pack format {} (this Landscaper reads {FORMAT}): update the Landscaper", m.format);
    ensure!(valid_pack_id(&m.id), "'{}' is not a pack id (1 to 40 lower-case letters, digits and -, starting with a letter)", m.id);
    ensure!(
        !m.version.is_empty() && m.version.len() <= 32 && m.version.chars().all(|c| c.is_ascii_alphanumeric() || ".+-".contains(c)),
        "'{}' is not a pack version",
        m.version
    );
    ensure!(plain_text(&m.name, 80), "the pack's name is empty or too long");
    ensure!(m.description.chars().count() <= 2000, "the pack's description is over 2000 characters");
    ensure!(plain_text(&m.licence, 200), "the pack says no licence");
    ensure!(m.credits.iter().all(|c| plain_text(c, 200)) && m.credits.len() <= 100, "a credit is empty or too long");
    ensure!(
        (2..=4).contains(&m.prefix.len())
            && m.prefix.starts_with(|c: char| c.is_ascii_lowercase())
            && m.prefix.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        "'{}' is not a pack prefix (2 to 4 lower-case letters and digits, starting with a letter)",
        m.prefix
    );
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
        ensure!(plain_text(&t.name, 80), "{}: its name is empty or too long", t.id);
        ensure!(plain_text(&t.category, 40), "{}: its category is empty or too long", t.id);
        ensure!(t.surface <= 255, "{}: surface {} is not a surfacemat.2da row", t.id, t.surface);
        for (f, want) in [(&t.files.render, format!("{}.png", t.id)), (&t.files.swatch, format!("{}_128.png", t.id))] {
            ensure!(f.path == want, "{}: its picture is {}; a pack names it {want}", t.id, f.path);
            ensure!(f.sha256.len() == 64 && f.sha256.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')), "{}: a bad sha256", f.path);
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A folder inside the library for one install's download and pictures, removed when dropped.
struct Staging(PathBuf);

impl Staging {
    fn new(library: &Path) -> Result<Staging> {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = library.join(".incoming").join(format!("{}-{n}-{t}", std::process::id()));
        std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
        Ok(Staging(dir))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        // (`.incoming` itself too, once no other install is using it: removing a folder that isn't empty fails.)
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Where a pack comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// Downloaded (http or https).
    Url(String),
    /// A zip on this machine.
    File(PathBuf),
}

impl Source {
    /// A URL (`http://`, `https://`), else a file (`file://` taken off).
    pub fn parse(s: &str) -> Source {
        if s.starts_with("http://") || s.starts_with("https://") {
            Source::Url(s.to_string())
        } else {
            Source::File(PathBuf::from(s.strip_prefix("file://").unwrap_or(s)))
        }
    }

    fn label(&self) -> String {
        match self {
            Source::Url(u) => u.clone(),
            Source::File(p) => p.file_name().map_or_default(|n| n.to_string_lossy().into_owned()),
        }
    }
}

/// What the index says a pack must be: its zip's size and sha256.
#[derive(Debug, Clone, Default)]
pub struct Expect {
    /// The zip's size in bytes.
    pub size: Option<u64>,
    /// The zip's sha256 (hex).
    pub sha256: Option<String>,
}

/// Steps an install reports: (`downloading` | `checking` | `installing`, done, total), in bytes or files.
pub type Progress<'a> = &'a dyn Fn(&'static str, u64, u64);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent(concat!("landscaper/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn http_error(url: &str, e: &ureq::Error) -> anyhow::Error {
    anyhow::anyhow!("{url}: {}", crate::remote::why(e))
}

/// Copies `from` to `to`, at most `max` bytes, reporting progress; the sha256 of what it copied. `length` is what the sender
/// said it would send: fewer bytes is a download that stopped.
fn copy_capped(from: &mut dyn Read, to: &Path, length: Option<u64>, max: u64, progress: Progress<'_>) -> Result<(u64, String)> {
    if let Some(n) = length {
        ensure!(n <= max, "the pack is {} MB; the most this server takes is {} MB (LANDSCAPER_PACK_MAX_MB)", n >> 20, max >> 20);
    }
    let mut out = std::io::BufWriter::new(std::fs::File::create(to).with_context(|| format!("creating {}", to.display()))?);
    let mut h = sha2::Sha256::new();
    let (mut done, mut buf) = (0u64, vec![0u8; 64 << 10]);
    let total = length.unwrap_or(0);
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => bail!("the download stopped after {done} bytes: {e}"),
        };
        done += n as u64;
        ensure!(done <= max, "the pack is over {} MB, the most this server takes (LANDSCAPER_PACK_MAX_MB)", max >> 20);
        h.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        progress("downloading", done, total);
    }
    out.flush()?;
    if let Some(n) = length {
        ensure!(done == n, "the download stopped after {done} of {n} bytes: try again");
    }
    Ok((done, hex(&h.finalize())))
}

/// The pack's zip on this machine: the file itself, or the download in `staging`. Checked against `expect`.
fn fetch(source: &Source, staging: &Path, expect: &Expect, progress: Progress<'_>) -> Result<PathBuf> {
    let max = max_pack_bytes();
    let (path, size, sha) = match source {
        Source::File(p) => {
            let mut f = std::fs::File::open(p).with_context(|| format!("opening {}", p.display()))?;
            let len = f.metadata()?.len();
            ensure!(
                len <= max,
                "{} is {} MB; the most this server takes is {} MB (LANDSCAPER_PACK_MAX_MB)",
                p.display(),
                len >> 20,
                max >> 20
            );
            let mut h = sha2::Sha256::new();
            std::io::copy(&mut f, &mut h)?;
            (p.clone(), len, hex(&h.finalize()))
        }
        Source::Url(url) => {
            let resp = agent().get(url).call().map_err(|e| http_error(url, &e))?;
            let length = resp.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok());
            let to = staging.join("pack.zip");
            let (size, sha) = copy_capped(&mut resp.into_reader(), &to, length.or(expect.size), max, progress)?;
            (to, size, sha)
        }
    };
    if let Some(want) = expect.size {
        ensure!(size == want, "the pack is {size} bytes; the index says {want}: try again, or tell whoever publishes it");
    }
    if let Some(want) = &expect.sha256 {
        ensure!(sha.eq_ignore_ascii_case(want), "the pack's sha256 is not the one the index gives: it was changed or cut off");
    }
    Ok(path)
}

/// Whether `name` (an entry of a pack's zip) is a plain file name: no folder, nothing hidden, nothing a file system reads as a
/// path of its own.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// The width and height a PNG says it is, before its pixels are read (a picture that says it is huge is refused unread).
fn png_size(path: &Path) -> Result<(u32, u32)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let reader =
        decoder.read_info().with_context(|| format!("{}: not a PNG", path.file_name().map_or_default(|n| n.display().to_string())))?;
    Ok((reader.info().width, reader.info().height))
}

/// Reads the pack `zip` into `staging`: its manifest, checked, and each picture it lists, its sha256 and its picture checked.
/// Anything in the zip that the manifest doesn't list, or that isn't a plain file name, is refused.
fn unpack(zip: &Path, staging: &Path, progress: Progress<'_>) -> Result<Manifest> {
    let file = std::fs::File::open(zip).with_context(|| format!("opening {}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).context("the pack is not a zip, or it was cut off")?;
    let manifest: Manifest = {
        let entry = archive.by_name("pack.json").context("the pack has no pack.json")?;
        let mut text = Vec::new();
        entry.take(MAX_MANIFEST + 1).read_to_end(&mut text)?;
        ensure!(text.len() as u64 <= MAX_MANIFEST, "pack.json is over {} MB", MAX_MANIFEST >> 20);
        serde_json::from_slice(&text).context("pack.json")?
    };
    validate(&manifest)?;
    let mut want: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for t in &manifest.textures {
        want.insert(t.files.render.path.clone(), t.files.render.sha256.clone());
        want.insert(t.files.swatch.path.clone(), t.files.swatch.sha256.clone());
    }
    let total = want.len() as u64;
    let mut seen = HashSet::new();
    let mut extracted = 0u64;
    for k in 0..archive.len() {
        let mut entry = archive.by_index(k)?;
        let name = entry.name().to_string();
        ensure!(plain_name(&name) && entry.enclosed_name().is_some(), "the pack holds '{name}': a pack's files are plain file names");
        ensure!(!entry.is_dir() && !entry.is_symlink(), "the pack holds '{name}', which is not a file");
        ensure!(seen.insert(name.to_ascii_lowercase()), "the pack holds '{name}' twice");
        if name == "pack.json" {
            continue;
        }
        let sha = want.get(&name).with_context(|| format!("the pack holds '{name}', which pack.json doesn't list"))?;
        ensure!(entry.size() <= MAX_FILE, "{name} is over {} MB", MAX_FILE >> 20);
        let to = staging.join(&name);
        let mut out = std::fs::File::create(&to).with_context(|| format!("creating {}", to.display()))?;
        let mut h = sha2::Sha256::new();
        let mut data = Vec::new();
        (&mut entry).take(MAX_FILE + 1).read_to_end(&mut data).with_context(|| format!("reading {name} from the pack"))?;
        ensure!(data.len() as u64 <= MAX_FILE, "{name} is over {} MB", MAX_FILE >> 20);
        h.update(&data);
        ensure!(hex(&h.finalize()) == *sha, "{name}: its sha256 is not the one pack.json gives (the pack is damaged)");
        out.write_all(&data)?;
        extracted += 1;
        progress("checking", extracted, total);
    }
    for t in &manifest.textures {
        for (f, max) in [(&t.files.render, RENDER), (&t.files.swatch, SWATCH)] {
            ensure!(seen.contains(&f.path.to_ascii_lowercase()), "pack.json lists {}, which the pack doesn't hold", f.path);
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
    let zip = fetch(source, &staging.0, expect, progress)?;
    let pictures = staging.0.join("pictures");
    std::fs::create_dir_all(&pictures)?;
    let m = unpack(&zip, &pictures, progress)?;

    let _one = super::lock();
    let mut library = Library::open(dir);
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
    let aside = staging.0.join("aside");
    std::fs::create_dir_all(&aside)?;
    let mut moved: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let total = (m.textures.len() * 2) as u64;
    let result = (|| -> Result<()> {
        for t in &m.textures {
            for f in [&t.files.render, &t.files.swatch] {
                let to = dir.join(&f.path);
                let kept = if to.exists() {
                    let a = aside.join(&f.path);
                    std::fs::rename(&to, &a).with_context(|| format!("setting {} aside", f.path))?;
                    Some(a)
                } else {
                    None
                };
                moved.push((to.clone(), kept));
                std::fs::rename(pictures.join(&f.path), &to).with_context(|| format!("moving {} into the library", f.path))?;
                progress("installing", moved.len() as u64, total);
            }
        }
        library.entries.retain(|e| e["pack"] != m.id.as_str());
        for t in &m.textures {
            let (w, h) = png_size(&dir.join(&t.files.render.path))?;
            library.entries.push(json!({
                "id": t.id, "name": t.name, "category": t.category, "source": m.name, "file": t.files.render.path,
                "render": t.files.render.path, "swatch": t.files.swatch.path, "surface": t.surface, "size": [w, h],
                "original": format!("pack:{}", m.id), "pack": m.id,
            }));
        }
        library.packs.retain(|p| p["id"] != m.id.as_str());
        library.packs.push(record(&m, &source.label()));
        library.save()
    })();
    if let Err(e) = result {
        for (to, kept) in moved.into_iter().rev() {
            let _ = std::fs::remove_file(&to);
            if let Some(a) = kept {
                let _ = std::fs::rename(a, &to);
            }
        }
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
    let mut library = Library::open(dir);
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
pub fn installed() -> Vec<Value> {
    Library::open(&paint::texture_dir()).packs
}

/// The areas that paint with any of `ids`: working copies and exports, in this machine's data folder, or on a team server in
/// every workspace (as `user: area`).
pub fn areas_using(ids: &[String]) -> Vec<String> {
    let roots: Vec<(String, PathBuf)> = if crate::workspace::team() {
        std::fs::read_dir(crate::workspace::workspaces_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(crate::workspace::clean_user).map(|u| (format!("{u}: "), e.path())))
            .collect()
    } else {
        vec![(String::new(), paint::data_dir())]
    };
    areas_using_in(&roots, ids)
}

/// The texture ids an area's ground is painted with: the entries of its `palette` that some sample's material names. A working
/// copy carries its materials as base64, an export's header names its `.materials` file beside it; without materials that read,
/// every entry of the palette counts. (The editor's palette only grows: a texture painted over stays in it.)
fn painted_ids(dir: &Path, palette: &[Value], materials: Option<&str>) -> Vec<String> {
    use base64::Engine;
    let names: Vec<&str> = palette.iter().map(|p| p.as_str().unwrap_or("")).collect();
    let bytes = materials.and_then(|m| {
        if m.ends_with(".materials") && plain_name(m) {
            std::fs::read(dir.join(m)).ok()
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
    let mut out = BTreeSet::new();
    for (label, root) in roots {
        for d in ["areas", "exports"] {
            for e in std::fs::read_dir(root.join(d)).into_iter().flatten().flatten() {
                let file = e.file_name().to_string_lossy().into_owned();
                let Some(area) = file.strip_suffix(".json").filter(|n| !n.contains('.') && !n.is_empty()) else { continue };
                let Some(uses) = std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<Uses>(&b).ok()) else { continue };
                if painted_ids(&root.join(d), &uses.palette, uses.materials.as_deref()).iter().any(|p| ids.contains(p.as_str())) {
                    out.insert(format!("{label}{area}"));
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Reads an index: `{ "format": 1, "packs": [...] }` at a URL or in a file. Each pack's `url` comes back absolute (a relative
/// one is beside the index).
pub fn fetch_index(url: &str) -> Result<Vec<Value>> {
    let (text, beside) = match Source::parse(url) {
        Source::Url(u) => {
            let resp = agent().get(&u).call().map_err(|e| http_error(&u, &e))?;
            let mut b = Vec::new();
            resp.into_reader().take(MAX_MANIFEST + 1).read_to_end(&mut b)?;
            (b, None)
        }
        Source::File(p) => (std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?, p.parent().map(Path::to_path_buf)),
    };
    ensure!(text.len() as u64 <= MAX_MANIFEST, "the index is over {} MB", MAX_MANIFEST >> 20);
    let mut packs = parse_index(&text, url)?;
    // A file's packs are files beside it (the folder as the system writes it: a Windows path has no `/` to cut at).
    if let Some(dir) = beside {
        let raw: Value = serde_json::from_slice(&text)?;
        for p in &mut packs {
            let given = raw["packs"].as_array().into_iter().flatten().find(|r| r["id"] == p["id"]).and_then(|r| r["url"].as_str());
            if let Some(at) = given.map(str::trim).filter(|a| !a.contains("://") && !Path::new(a).is_absolute()) {
                p["url"] = dir.join(at).to_string_lossy().into_owned().into();
            }
        }
    }
    Ok(packs)
}

fn parse_index(text: &[u8], url: &str) -> Result<Vec<Value>> {
    let v: Value = serde_json::from_slice(text).context("the index is not JSON")?;
    ensure!(v["format"].as_u64() == Some(FORMAT), "index format {} (this Landscaper reads {FORMAT})", v["format"]);
    let base = match url.rfind('/') {
        Some(k) => &url[..=k],
        None => "",
    };
    let mut out = Vec::new();
    for p in v["packs"].as_array().context("the index has no \"packs\"")? {
        let s = |k: &str| p[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let (Some(id), Some(name), Some(version), Some(at)) = (s("id"), s("name"), s("version"), s("url")) else { continue };
        let sha = s("sha256").filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()));
        let Some(sha) = sha else { continue };
        let at = if at.contains("://") || at.starts_with('/') { at } else { format!("{base}{at}") };
        let swatches: Vec<&str> = p["swatches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|s| s.starts_with("data:image/png;base64,") && s.len() <= 64 << 10)
            .take(INDEX_SWATCHES)
            .collect();
        out.push(json!({
            "id": id, "name": name, "version": version, "url": at, "sha256": sha.to_ascii_lowercase(),
            "description": s("description").unwrap_or_default(), "licence": s("licence").unwrap_or_default(),
            "credits": p["credits"].as_array().cloned().unwrap_or_default(), "size": p["size"].as_u64(),
            "textures": p["textures"].as_u64(), "categories": p["categories"].as_array().cloned().unwrap_or_default(),
            "swatches": swatches, "homepage": s("homepage"),
        }));
    }
    Ok(out)
}

/// The index entry for `id`.
pub fn find_in_index(index: &[Value], id: &str) -> Result<(Source, Expect)> {
    let p = index.iter().find(|p| p["id"] == id).with_context(|| format!("the index lists no pack '{id}'"))?;
    let url = p["url"].as_str().context("an index entry without a url")?;
    Ok((Source::parse(url), Expect { size: p["size"].as_u64(), sha256: p["sha256"].as_str().map(str::to_string) }))
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
    let bytes = zip_pack(&m, &files)?;
    crate::atomic::write(&zip_path, &bytes)?;
    let sha256 = hex(&sha2::Sha256::digest(&bytes));
    let made = Made { zip: zip_path.clone(), size: bytes.len() as u64, sha256: sha256.clone(), textures: m.textures.len() };
    if let Some(index) = index {
        let mut v: Value = match std::fs::read(index) {
            Ok(b) => serde_json::from_slice(&b).with_context(|| format!("{}: not JSON", index.display()))?,
            Err(_) => json!({ "format": FORMAT, "packs": [] }),
        };
        let categories: Vec<&str> = m.textures.iter().map(|t| t.category.as_str()).collect::<BTreeSet<_>>().into_iter().collect();
        let shown: Vec<String> = spread(&swatches).into_iter().map(data_uri).collect::<Result<_>>()?;
        let entry = json!({
            "id": m.id, "version": m.version, "name": m.name, "description": m.description, "licence": m.licence,
            "credits": m.credits, "url": zip_path.file_name().map_or_default(|n| n.to_string_lossy().into_owned()),
            "size": made.size, "sha256": sha256, "textures": m.textures.len(), "categories": categories, "swatches": shown,
        });
        let packs = v["packs"].as_array_mut().context("the index has no \"packs\" list")?;
        packs.retain(|p| p["id"] != m.id.as_str());
        packs.push(entry);
        v["format"] = FORMAT.into();
        crate::atomic::write(index, &serde_json::to_vec_pretty(&v)?)?;
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

/// A pack's zip: pack.json, then the pictures, with no times in it, so the same list makes the same bytes.
fn zip_pack(m: &Manifest, files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let at = zip::DateTime::default();
    let json = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).last_modified_time(at);
    // (PNGs are compressed already.)
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).last_modified_time(at);
    z.start_file("pack.json", json)?;
    z.write_all(&serde_json::to_vec_pretty(m)?)?;
    for (name, data) in files {
        z.start_file(name, stored)?;
        z.write_all(data)?;
    }
    Ok(z.finish()?.into_inner())
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
        let mut lib = Library::open(&dir);
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
        let lib = Library::open(&dir);
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
        assert!(dir.join("dmSlate.png").is_file() && Library::open(&dir).has("dmSlate"), "a refused upgrade changes nothing");
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
        let e = copy_capped(&mut &data[..], &src.join("x"), Some(10 << 30), 1 << 20, &quiet).unwrap_err();
        assert!(e.to_string().contains("most"), "{e}");
        // And without a length, as it passes the cap.
        let e = copy_capped(&mut &vec![0u8; 3 << 20][..], &src.join("x"), None, 1 << 20, &quiet).unwrap_err();
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
        assert!(Library::open(&dir).has("dmMoss"));
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

    #[test]
    fn an_index_is_read_with_its_urls_made_whole() {
        let sha = "ab".repeat(32);
        let text = serde_json::to_vec(&json!({ "format": 1, "packs": [
            { "id": "a", "name": "A", "version": "1", "url": "a-1.zip", "sha256": sha, "swatches": ["data:image/png;base64,AA", "https://x/y.png"] },
            { "id": "b", "name": "B", "version": "1", "url": "https://cdn.example/b.zip", "sha256": sha },
            { "id": "c", "name": "no hash", "version": "1", "url": "c.zip" },
        ] }))
        .unwrap();
        let packs = parse_index(&text, "https://example.org/releases/index.json").unwrap();
        assert_eq!(packs.len(), 2);
        assert_eq!(packs[0]["url"], "https://example.org/releases/a-1.zip");
        assert_eq!(packs[0]["swatches"], json!(["data:image/png;base64,AA"]));
        assert_eq!(packs[1]["url"], "https://cdn.example/b.zip");
        assert!(parse_index(br#"{ "format": 2, "packs": [] }"#, "x").is_err());
    }
}
