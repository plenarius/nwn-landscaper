// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A builder's own module or ERF (an area made in the toolset), kept in their workspace so it can be imported
//! like an existing area. It is theirs, so no assignment is involved; builders never bring haks (their areas use
//! the shared tilesets), so only `.mod` and `.erf` are taken.

use anyhow::{Context, Result};
use mg_core::ResType;
use mg_module::Module;
use mg_resman::ResKey;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The most a file may be. Production's whole module is about 70 MB.
pub const MAX_BYTES: usize = 150 * 1024 * 1024;

/// The builder's uploaded modules and ERFs: `uploads/` in their workspace.
pub fn dir() -> PathBuf {
    crate::workspace::root().join("uploads")
}

/// A file name that is safe as a path: letters, digits and `_ . -`, a `.mod` or `.erf`.
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "`lower` is the name in lower case already")]
pub fn file_ok(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (1..=64).contains(&name.len())
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && (lower.ends_with(".mod") || lower.ends_with(".erf"))
}

/// The file of the upload `name`, once the name is one it could have been kept under (never a path).
pub fn path_of(name: &str) -> Result<PathBuf> {
    anyhow::ensure!(file_ok(name), "{name:?} isn't a file name I can keep (letters, digits, _ . - and ending .mod or .erf)");
    Ok(dir().join(name))
}

/// The areas in the file: [{ area, tag, tileset, width, height, flags }]. Plain ERFs have no module.ifo, so the
/// areas are the ARE resources.
pub fn areas_of(path: &std::path::Path) -> Result<Vec<Value>> {
    areas_in(path, &path.display().to_string())
}

/// [`areas_of`] `path`, which errors call `name`.
fn areas_in(path: &std::path::Path, name: &str) -> Result<Vec<Value>> {
    // (The reader's own message starts with the path it opened: `name` takes its place.)
    let m = Module::open(path).map_err(|e| {
        let why = format!("{e:#}");
        let why = why.strip_prefix(&format!("{}: ", path.display())).unwrap_or(&why);
        anyhow::anyhow!("{name} isn't a module or ERF I can read: {why}")
    })?;
    let mut keys: Vec<_> = m.keys_of(ResType::ARE).map(|k| k.resref).collect();
    keys.sort_by_key(ToString::to_string);
    let mut out = Vec::new();
    for a in keys {
        let Some(Ok(are)) = m.gff(&ResKey { resref: a, restype: ResType::ARE }) else { continue };
        out.push(json!({
            "area": a.to_string(),
            "tag": are.root.string("Tag").map(|b| String::from_utf8_lossy(b).into_owned()),
            "tileset": are.root.resref("Tileset").map_or_default(|t| t.to_string()),
            "width": are.root.integer("Width"), "height": are.root.integer("Height"), "flags": are.root.integer("Flags"),
            "from": "your upload",
        }));
    }
    Ok(out)
}

/// The most files a workspace keeps: a builder's own areas come one module or ERF at a time, and 20 files of production's size
/// (70 MB) is more than anyone has needed.
pub const MAX_FILES: usize = 20;
/// The most bytes a workspace's uploads may take in all.
pub const MAX_TOTAL: u64 = 500 * 1024 * 1024;

/// The most uploads the server takes at once (from different builders). The body is written to disk as it arrives, but
/// checking the file opens it whole (about its size in memory: 157 MB at the most for a 140 MB file), so this bounds what
/// uploads can hold at once to about 600 MB.
const AT_ONCE: usize = 4;

/// A builder's turn to upload (see [`begin`]); the next upload into the same workspace may start once it is dropped.
pub struct Turn {
    dir: PathBuf,
}

impl Drop for Turn {
    fn drop(&mut self) {
        uploading().remove(&self.dir);
    }
}

/// The workspaces (their uploads folders) with an upload going.
fn uploading() -> std::sync::MutexGuard<'static, std::collections::BTreeSet<PathBuf>> {
    static NOW: std::sync::Mutex<std::collections::BTreeSet<PathBuf>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
    NOW.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether the builder may upload `name`, `len` bytes, now: one upload at a time per workspace and [`AT_ONCE`] in all (else
/// 429), and the file within
/// [`MAX_BYTES`] and their uploads within [`MAX_FILES`] and [`MAX_TOTAL`] once it is in (else 413; a file of the same name is
/// replaced, so it doesn't count). The refusal is (HTTP status, why).
pub fn begin(name: &str, len: u64) -> std::result::Result<Turn, (u16, String)> {
    let dir = dir();
    if len > MAX_BYTES as u64 {
        return Err((413, format!("a file up to {} MB, please", MAX_BYTES >> 20)));
    }
    let (count, total) = kept(&dir, name);
    if count >= MAX_FILES || total + len > MAX_TOTAL {
        return Err((
            413,
            format!("your uploads are at their limit ({MAX_FILES} files, {} MB in all): delete one you no longer need", MAX_TOTAL >> 20),
        ));
    }
    let mut now = uploading();
    if now.contains(&dir) {
        return Err((429, "an upload of yours is still going: wait for it to finish".into()));
    }
    if now.len() >= AT_ONCE {
        return Err((429, "the server is taking other uploads: try again in a minute".into()));
    }
    now.insert(dir.clone());
    Ok(Turn { dir })
}

/// How many files the uploads folder `dir` keeps and their bytes, leaving out `but` (a file about to be replaced).
fn kept(dir: &std::path::Path, but: &str) -> (usize, u64) {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| file_ok(n) && n != but))
        .fold((0, 0), |(n, b), e| (n + 1, b + e.metadata().map_or(0, |m| m.len())))
}

/// Keeps what `body` holds (read as it comes, never whole in memory) as the builder's file `name` after checking it reads as a
/// module; the areas it holds and its size. `_turn` is the builder's turn ([`begin`]).
pub fn store(_turn: &Turn, name: &str, body: &mut dyn std::io::Read) -> Result<(Vec<Value>, u64)> {
    store_in(&dir(), name, body)
}

fn store_in(dir: &std::path::Path, name: &str, body: &mut dyn std::io::Read) -> Result<(Vec<Value>, u64)> {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    anyhow::ensure!(file_ok(name), "{name:?} isn't a file name I can keep (letters, digits, _ . - and ending .mod or .erf)");
    std::fs::create_dir_all(dir)?;
    // A name of its own for each upload, so two uploads of the same name never write into one file.
    let tmp = dir.join(format!(".{name}.{}-{}.part", crate::pid(), SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let kept = (|| -> Result<(Vec<Value>, u64)> {
        let mut f = std::fs::File::create(&tmp)?;
        let len = std::io::copy(&mut std::io::Read::take(body, MAX_BYTES as u64 + 1), &mut f).context("reading the upload")?;
        anyhow::ensure!(len > 0 && len <= MAX_BYTES as u64, "a file up to {} MB, please", MAX_BYTES >> 20);
        f.sync_all()?;
        let mut magic = [0u8; 4];
        let start = std::fs::File::open(&tmp).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic)).is_ok();
        anyhow::ensure!(start && matches!(&magic, b"MOD " | b"ERF "), "that isn't a module or ERF (it should start with MOD or ERF)");
        // (An error names the file as the builder sent it, not the server's path to its part file.)
        let areas = areas_in(&tmp, name)?;
        anyhow::ensure!(!areas.is_empty(), "there is no area in that file");
        std::fs::rename(&tmp, dir.join(name))?;
        Ok((areas, len))
    })();
    if kept.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    kept
}

/// [`areas_of`], read once per version of the file (its size and time): the listing of a builder's uploads re-opened and
/// parsed every one of them on every request.
fn areas_cached(path: &std::path::Path) -> Vec<Value> {
    type Seen = std::collections::HashMap<PathBuf, (u64, std::time::SystemTime, Vec<Value>)>;
    static SEEN: std::sync::Mutex<Option<Seen>> = std::sync::Mutex::new(None);
    let Ok(m) = std::fs::metadata(path) else { return Vec::new() };
    let version = (m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH));
    if let Some((_, _, areas)) =
        SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().and_then(|s| s.get(path)).filter(|e| (e.0, e.1) == version)
    {
        return areas.clone();
    }
    let areas = areas_of(path).unwrap_or_default();
    SEEN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(Default::default)
        .insert(path.to_path_buf(), (version.0, version.1, areas.clone()));
    areas
}

/// The builder's files with their areas.
pub fn list() -> Vec<Value> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if file_ok(&name) {
            out.push(json!({ "file": name, "areas": areas_cached(&e.path()) }));
        }
    }
    out.sort_by_key(|v| v["file"].as_str().map(str::to_string));
    out
}

/// Deletes the builder's upload `name`.
pub fn delete(name: &str) -> Result<()> {
    std::fs::remove_file(path_of(name)?).with_context(|| format!("no such file {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_module_and_erf_file_names_are_taken() {
        for ok in ["cave.mod", "My_Cave-2.ERF", "a.mod"] {
            assert!(file_ok(ok), "{ok}");
        }
        for bad in ["", ".hidden.mod", "../x.mod", "a/b.mod", "cave.hak", "cave", "a b.mod", &format!("{}.mod", "x".repeat(70))] {
            assert!(!file_ok(bad), "{bad}");
        }
    }

    #[test]
    fn rejects_what_is_not_a_module_and_leaves_nothing() {
        let dir = std::env::temp_dir().join(format!("ls-upload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = |name: &str, body: &[u8]| store_in(&dir, name, &mut &body[..]);
        assert!(store("x.mod", b"").is_err());
        assert!(store("x.mod", b"HAK V1.0 whatever").is_err());
        assert!(store("x.hak", b"MOD V1.0").is_err());
        // (An error names the file as sent, never the server's path to its part file.)
        let e = format!("{:#}", store("x.mod", b"MOD V1.0 but truncated").unwrap_err());
        assert!(e.starts_with("x.mod isn't a module or ERF") && !e.contains(&*dir.to_string_lossy()), "{e}");
        assert!(store("x.mod", b"MOD").is_err());
        // Over the limit, read as it comes: refused without holding it.
        let mut endless = std::io::Read::chain(&b"MOD V1.0"[..], std::io::repeat(0));
        let e = store_in(&dir, "big.mod", &mut endless).unwrap_err();
        assert!(e.to_string().contains("up to 150 MB"), "{e}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no part file left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_files_kept_are_counted_leaving_out_the_one_replaced() {
        let dir = std::env::temp_dir().join(format!("ls-upload-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (f, n) in [("a.mod", 10), ("b.erf", 5), (".a.mod.1-0.part", 100), ("notes.txt", 7)] {
            std::fs::write(dir.join(f), vec![0u8; n]).unwrap();
        }
        assert_eq!(kept(&dir, "c.mod"), (2, 15));
        assert_eq!(kept(&dir, "a.mod"), (1, 5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
