// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What texture packs ([`crate::textures::packs`]) and sky packs ([`crate::skies`]) share: the index that lists them, the
//! download with its size and sha256 checked, the zip read into a staging folder with every file's sha256 checked, the files moved
//! in with a way back, and the areas that use what a pack installed.
//!
//! One index lists both kinds. Texture packs are its `packs`, as the first Landscapers with packs read it; sky packs are its
//! `skies` (each `"kind": "sky"`), which those Landscapers don't read, so they never offer a sky pack as a texture pack.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::Digest;

/// The pack and index format this engine reads.
pub const FORMAT: u64 = 1;
/// Where the list of packs is when `LANDSCAPER_PACKS_URL` is not set: an asset of the `texture-packs` release.
pub const DEFAULT_INDEX: &str = "https://github.com/plenarius/nwn-landscaper/releases/download/texture-packs/index.json";
/// The largest pack downloaded, unless `LANDSCAPER_PACK_MAX_MB` says otherwise.
const DEFAULT_MAX_MB: u64 = 512;
/// The largest `pack.json` and index read.
pub const MAX_MANIFEST: u64 = 4 << 20;

/// What a pack holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Textures for the library (docs/TEXTURE-PACKS.md).
    Texture,
    /// Skies: models, their textures and `skyboxes.2da` rows (docs/SKY-PACKS.md).
    Sky,
}

impl Kind {
    /// Its `kind` in the index and in `pack.json`.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Texture => "texture",
            Kind::Sky => "sky",
        }
    }

    /// The index's list of packs of this kind.
    fn list(self) -> &'static str {
        match self {
            Kind::Texture => "packs",
            Kind::Sky => "skies",
        }
    }
}

/// Where an index lists its packs: `LANDSCAPER_PACKS_URL` (a URL or a file), else [`DEFAULT_INDEX`].
pub fn index_url() -> String {
    std::env::var("LANDSCAPER_PACKS_URL").ok().map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).unwrap_or(DEFAULT_INDEX.into())
}

fn max_pack_bytes() -> u64 {
    std::env::var("LANDSCAPER_PACK_MAX_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&n| n > 0).unwrap_or(DEFAULT_MAX_MB)
        << 20
}

/// Whether `s` is one line of text, not empty, at most `max` characters.
pub fn plain_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

/// Whether `id` can be a pack's id: 1 to 40 lower-case letters, digits and `-`, starting with a letter.
pub fn valid_pack_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The fields every pack's `pack.json` has.
pub struct About<'a> {
    /// [`FORMAT`].
    pub format: u64,
    /// [`valid_pack_id`].
    pub id: &'a str,
    /// Letters, digits and `.+-`.
    pub version: &'a str,
    /// What the dialog shows.
    pub name: &'a str,
    /// A sentence or two.
    pub description: &'a str,
    /// An SPDX id or a short name of the licence.
    pub licence: &'a str,
    /// Who made it.
    pub credits: &'a [String],
    /// 2 to 4 lower-case letters and digits, starting with a letter.
    pub prefix: &'a str,
}

/// Checks the fields every pack has.
pub fn check_about(a: &About<'_>) -> Result<()> {
    ensure!(a.format == FORMAT, "pack format {} (this Landscaper reads {FORMAT}): update the Landscaper", a.format);
    ensure!(valid_pack_id(a.id), "'{}' is not a pack id (1 to 40 lower-case letters, digits and -, starting with a letter)", a.id);
    ensure!(
        !a.version.is_empty() && a.version.len() <= 32 && a.version.chars().all(|c| c.is_ascii_alphanumeric() || ".+-".contains(c)),
        "'{}' is not a pack version",
        a.version
    );
    ensure!(plain_text(a.name, 80), "the pack's name is empty or too long");
    ensure!(a.description.chars().count() <= 2000, "the pack's description is over 2000 characters");
    ensure!(plain_text(a.licence, 200), "the pack says no licence");
    ensure!(a.credits.iter().all(|c| plain_text(c, 200)) && a.credits.len() <= 100, "a credit is empty or too long");
    ensure!(
        (2..=4).contains(&a.prefix.len())
            && a.prefix.starts_with(|c: char| c.is_ascii_lowercase())
            && a.prefix.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        "'{}' is not a pack prefix (2 to 4 lower-case letters and digits, starting with a letter)",
        a.prefix
    );
    Ok(())
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `s` is a sha256 as `pack.json` gives it: 64 lower-case hex digits.
pub fn valid_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

/// A folder inside `parent/.incoming` for one install's download and files, removed when dropped (the server never serves it).
pub struct Staging(pub PathBuf);

impl Staging {
    /// A new, empty one.
    pub fn new(parent: &Path) -> Result<Staging> {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = parent.join(".incoming").join(format!("{}-{n}-{t}", std::process::id()));
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

    /// What the catalog records it came from.
    pub fn label(&self) -> String {
        match self {
            Source::Url(u) => u.clone(),
            Source::File(p) => p.file_name().map_or_default(|n| n.to_string_lossy().into_owned()),
        }
    }

    /// Whether `pack` as typed on the command line is a URL or a zip rather than an id in the index.
    pub fn given(pack: &str) -> bool {
        pack.contains("://") || Path::new(pack).extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) || Path::new(pack).is_file()
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
pub fn copy_capped(from: &mut dyn Read, to: &Path, length: Option<u64>, max: u64, progress: Progress<'_>) -> Result<(u64, String)> {
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
pub fn fetch(source: &Source, staging: &Path, expect: &Expect, progress: Progress<'_>) -> Result<PathBuf> {
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
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// Reads the pack `zip` into `staging`: its `pack.json`, which `read` parses and checks into the manifest and the files it lists
/// (name, sha256), then each of those files, at most `max_file` bytes, its sha256 checked. Anything in the zip that the manifest
/// doesn't list, or that isn't a plain file name, is refused, and so is a listed file the zip lacks.
pub fn unzip<M>(
    zip: &Path,
    staging: &Path,
    max_file: u64,
    progress: Progress<'_>,
    read: impl FnOnce(&[u8]) -> Result<(M, HashMap<String, String>)>,
) -> Result<M> {
    let file = std::fs::File::open(zip).with_context(|| format!("opening {}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).context("the pack is not a zip, or it was cut off")?;
    let (manifest, want) = {
        let entry = archive.by_name("pack.json").context("the pack has no pack.json")?;
        let mut text = Vec::new();
        entry.take(MAX_MANIFEST + 1).read_to_end(&mut text)?;
        ensure!(text.len() as u64 <= MAX_MANIFEST, "pack.json is over {} MB", MAX_MANIFEST >> 20);
        read(&text)?
    };
    let (total, mut done) = (want.len() as u64, 0u64);
    let mut seen = std::collections::HashSet::new();
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
        ensure!(entry.size() <= max_file, "{name} is over {} MB", max_file >> 20);
        let mut data = Vec::new();
        (&mut entry).take(max_file + 1).read_to_end(&mut data).with_context(|| format!("reading {name} from the pack"))?;
        ensure!(data.len() as u64 <= max_file, "{name} is over {} MB", max_file >> 20);
        ensure!(hex(&sha2::Sha256::digest(&data)) == *sha, "{name}: its sha256 is not the one pack.json gives (the pack is damaged)");
        let to = staging.join(&name);
        std::fs::write(&to, &data).with_context(|| format!("writing {}", to.display()))?;
        done += 1;
        progress("checking", done, total);
    }
    for name in want.keys() {
        ensure!(seen.contains(&name.to_ascii_lowercase()), "pack.json lists {name}, which the pack doesn't hold");
    }
    Ok(manifest)
}

/// Files moved into a library, each with the file it replaced set aside, so a failure before the catalog is written puts
/// everything back ([`Moves::undo`]).
pub struct Moves {
    aside: PathBuf,
    moved: Vec<(PathBuf, Option<PathBuf>)>,
}

impl Moves {
    /// Sets replaced files aside in `aside` (inside the install's staging folder).
    pub fn new(aside: PathBuf) -> Result<Moves> {
        std::fs::create_dir_all(&aside)?;
        Ok(Moves { aside, moved: Vec::new() })
    }

    /// Moves `from` to `dir/name`, setting a file already there aside.
    pub fn put(&mut self, from: &Path, dir: &Path, name: &str) -> Result<()> {
        let to = dir.join(name);
        let kept = if to.exists() {
            let a = self.aside.join(name);
            std::fs::rename(&to, &a).with_context(|| format!("setting {name} aside"))?;
            Some(a)
        } else {
            None
        };
        self.moved.push((to.clone(), kept));
        std::fs::rename(from, &to).with_context(|| format!("moving {name} into the library"))
    }

    /// How many files were moved.
    pub fn len(&self) -> usize {
        self.moved.len()
    }

    /// Takes the files moved in out again and puts back what they replaced.
    pub fn undo(self) {
        for (to, kept) in self.moved.into_iter().rev() {
            let _ = std::fs::remove_file(&to);
            if let Some(a) = kept {
                let _ = std::fs::rename(a, &to);
            }
        }
    }
}

/// Where areas are kept: this machine's data folder, or on a team server every workspace (labelled `user: `).
pub fn area_roots() -> Vec<(String, PathBuf)> {
    if crate::workspace::team() {
        std::fs::read_dir(crate::workspace::workspaces_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(crate::workspace::clean_user).map(|u| (format!("{u}: "), e.path())))
            .collect()
    } else {
        vec![(String::new(), crate::paint::data_dir())]
    }
}

/// The areas (working copies and exports under `roots`, as `label` + area) whose file `uses` says yes to, given the folder it is in
/// and its bytes.
pub fn areas_where(roots: &[(String, PathBuf)], uses: impl Fn(&Path, &[u8]) -> bool) -> Vec<String> {
    let mut out = BTreeSet::new();
    for (label, root) in roots {
        for d in ["areas", "exports"] {
            let dir = root.join(d);
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let file = e.file_name().to_string_lossy().into_owned();
                let Some(area) = file.strip_suffix(".json").filter(|n| !n.contains('.') && !n.is_empty()) else { continue };
                if std::fs::read(e.path()).is_ok_and(|b| uses(&dir, &b)) {
                    out.insert(format!("{label}{area}"));
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Reads an index's packs of `kind`: `{ "format": 1, "packs": [...], "skies": [...] }` at a URL or in a file. Each pack's `url`
/// comes back absolute (a relative one is beside the index).
pub fn fetch_index(url: &str, kind: Kind) -> Result<Vec<Value>> {
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
    let mut packs = parse_index(&text, url, kind)?;
    // A file's packs are files beside it (the folder as the system writes it: a Windows path has no `/` to cut at).
    if let Some(dir) = beside {
        let raw: Value = serde_json::from_slice(&text)?;
        for p in &mut packs {
            let given = raw[kind.list()].as_array().into_iter().flatten().find(|r| r["id"] == p["id"]).and_then(|r| r["url"].as_str());
            if let Some(at) = given.map(str::trim).filter(|a| !a.contains("://") && !Path::new(a).is_absolute()) {
                p["url"] = dir.join(at).to_string_lossy().into_owned().into();
            }
        }
    }
    Ok(packs)
}

/// How many swatches a texture pack's entry shows.
pub const INDEX_SWATCHES: usize = 6;

fn parse_index(text: &[u8], url: &str, kind: Kind) -> Result<Vec<Value>> {
    let v: Value = serde_json::from_slice(text).context("the index is not JSON")?;
    ensure!(v["format"].as_u64() == Some(FORMAT), "index format {} (this Landscaper reads {FORMAT})", v["format"]);
    ensure!(v["packs"].is_array() || v["skies"].is_array(), "the index has no \"packs\"");
    let base = match url.rfind('/') {
        Some(k) => &url[..=k],
        None => "",
    };
    let mut out = Vec::new();
    for p in v[kind.list()].as_array().into_iter().flatten() {
        let s = |k: &str| p[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        if s("kind").unwrap_or_else(|| kind.name().into()) != kind.name() {
            continue;
        }
        let (Some(id), Some(name), Some(version), Some(at)) = (s("id"), s("name"), s("version"), s("url")) else { continue };
        let Some(sha) = s("sha256").filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())) else { continue };
        let at = if at.contains("://") || at.starts_with('/') { at } else { format!("{base}{at}") };
        let mut entry = json!({
            "id": id, "kind": kind.name(), "name": name, "version": version, "url": at, "sha256": sha.to_ascii_lowercase(),
            "description": s("description").unwrap_or_default(), "licence": s("licence").unwrap_or_default(),
            "credits": p["credits"].as_array().cloned().unwrap_or_default(), "size": p["size"].as_u64(), "homepage": s("homepage"),
        });
        match kind {
            Kind::Texture => {
                let swatches: Vec<&str> = p["swatches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|s| s.starts_with("data:image/png;base64,") && s.len() <= 64 << 10)
                    .take(INDEX_SWATCHES)
                    .collect();
                entry["textures"] = p["textures"].as_u64().into();
                entry["categories"] = p["categories"].as_array().cloned().unwrap_or_default().into();
                entry["swatches"] = swatches.into();
            }
            Kind::Sky => {
                let names: Vec<&str> =
                    p["names"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| plain_text(n, 80)).take(64).collect();
                entry["skies"] = p["skies"].as_u64().into();
                entry["names"] = names.into();
            }
        }
        out.push(entry);
    }
    Ok(out)
}

/// The index entry for `id`.
pub fn find_in_index(index: &[Value], id: &str) -> Result<(Source, Expect)> {
    let p = index.iter().find(|p| p["id"] == id).with_context(|| format!("the index lists no pack '{id}'"))?;
    let url = p["url"].as_str().context("an index entry without a url")?;
    Ok((Source::parse(url), Expect { size: p["size"].as_u64(), sha256: p["sha256"].as_str().map(str::to_string) }))
}

/// A pack's zip: `pack.json`, then its files, with no times in it, so the same files make the same bytes.
pub fn zip_pack(manifest: &[u8], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let at = zip::DateTime::default();
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).last_modified_time(at);
    // (PNG and DDS are compressed already.)
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).last_modified_time(at);
    z.start_file("pack.json", deflated)?;
    z.write_all(manifest)?;
    for (name, data) in files {
        let packed = Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("dds"));
        z.start_file(name, if packed { stored } else { deflated })?;
        z.write_all(data)?;
    }
    Ok(z.finish()?.into_inner())
}

/// Adds `entry` (a pack of `kind`) to the index file `index`, made if missing; an entry of the same id is replaced.
pub fn add_to_index(index: &Path, kind: Kind, mut entry: Value) -> Result<()> {
    // (Only a missing index starts a new one: read as empty, one that failed to open would lose every other pack.)
    let mut v: Value = match crate::atomic::read(index) {
        Ok(b) => serde_json::from_slice(&b).with_context(|| format!("{}: not JSON", index.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({ "format": FORMAT, "packs": [] }),
        Err(e) => return Err(e).with_context(|| format!("reading {}", index.display())),
    };
    ensure!(v.is_object(), "{}: not an index", index.display());
    if kind == Kind::Sky {
        entry["kind"] = kind.name().into();
    }
    if v[kind.list()].is_null() {
        v[kind.list()] = json!([]);
    }
    let packs = v[kind.list()].as_array_mut().with_context(|| format!("the index's \"{}\" is not a list", kind.list()))?;
    packs.retain(|p| p["id"] != entry["id"]);
    packs.push(entry);
    v["format"] = FORMAT.into();
    crate::atomic::write(index, &serde_json::to_vec_pretty(&v)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_is_read_with_its_urls_made_whole() {
        let sha = "ab".repeat(32);
        let text = serde_json::to_vec(&json!({ "format": 1, "packs": [
            { "id": "a", "name": "A", "version": "1", "url": "a-1.zip", "sha256": sha, "swatches": ["data:image/png;base64,AA", "https://x/y.png"] },
            { "id": "b", "name": "B", "version": "1", "url": "https://cdn.example/b.zip", "sha256": sha },
            { "id": "c", "name": "no hash", "version": "1", "url": "c.zip" },
        ] }))
        .unwrap();
        let packs = parse_index(&text, "https://example.org/releases/index.json", Kind::Texture).unwrap();
        assert_eq!(packs.len(), 2);
        assert_eq!(packs[0]["url"], "https://example.org/releases/a-1.zip");
        assert_eq!(packs[0]["swatches"], json!(["data:image/png;base64,AA"]));
        assert_eq!(packs[1]["url"], "https://cdn.example/b.zip");
        assert!(parse_index(br#"{ "format": 2, "packs": [] }"#, "x", Kind::Texture).is_err());
    }

    /// Sky packs are in their own list, which a Landscaper that knows only texture packs doesn't read.
    #[test]
    fn texture_and_sky_packs_share_an_index_without_mixing() {
        let sha = "cd".repeat(32);
        let text = serde_json::to_vec(&json!({ "format": 1,
            "packs": [{ "id": "ground", "name": "Ground", "version": "1", "url": "g.zip", "sha256": sha, "textures": 4 }],
            "skies": [
                { "id": "dusky", "kind": "sky", "name": "Dusky", "version": "2", "url": "d.zip", "sha256": sha, "skies": 2, "names": ["Dusk", "Dawn"] },
                { "id": "odd", "kind": "texture", "name": "Odd", "version": "1", "url": "o.zip", "sha256": sha },
            ],
        }))
        .unwrap();
        let textures = parse_index(&text, "https://example.org/index.json", Kind::Texture).unwrap();
        assert_eq!(textures.iter().map(|p| p["id"].as_str().unwrap()).collect::<Vec<_>>(), ["ground"]);
        let skies = parse_index(&text, "https://example.org/index.json", Kind::Sky).unwrap();
        assert_eq!(skies.len(), 1);
        assert_eq!((skies[0]["kind"].as_str(), skies[0]["skies"].as_u64()), (Some("sky"), Some(2)));
        assert_eq!(skies[0]["names"], json!(["Dusk", "Dawn"]));
        // An index of sky packs alone is an index.
        let only = br#"{ "format": 1, "skies": [] }"#;
        assert!(parse_index(only, "x", Kind::Texture).unwrap().is_empty());
    }

    #[test]
    fn an_entry_goes_into_its_kinds_list() {
        let dir = std::env::temp_dir().join(format!("ls-packs-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let index = dir.join("index.json");
        add_to_index(&index, Kind::Texture, json!({ "id": "a", "version": "1" })).unwrap();
        add_to_index(&index, Kind::Sky, json!({ "id": "s", "version": "1" })).unwrap();
        add_to_index(&index, Kind::Sky, json!({ "id": "s", "version": "2" })).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
        assert_eq!(v["packs"], json!([{ "id": "a", "version": "1" }]));
        assert_eq!(v["skies"], json!([{ "id": "s", "version": "2", "kind": "sky" }]));
    }
}
