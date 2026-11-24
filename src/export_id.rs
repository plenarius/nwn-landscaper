// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An export's id and its files' SHA-256s: what a trusted service compares to tell one export of an area from the next
//! (docs/REVIEW-SERVICE.md, "Fetching exports").
//!
//! The id is the SHA-256 of the export's header (`exports/<name>.json`, as on disk) followed by the hex SHA-256 of its hak
//! (`nwn/hak/<name>.hak`). Every export writes both, so it changes with every export. Publish sends it (`export`), and the
//! service routes list it.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use sha2::Digest;

/// What a kept SHA-256 was taken of: the file as it was.
#[derive(PartialEq, Eq, Clone)]
struct Seen {
    path: PathBuf,
    modified: Option<std::time::SystemTime>,
    len: u64,
}

/// The most SHA-256s kept (one a hak: a team's tile haks and texture libraries).
const KEPT: usize = 256;

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// The hex SHA-256 of `file` (open at `path`), and its length. Kept while its length and modification time stay the same: a
/// tile hak is up to 81 MB, and the list, the download and Publish all ask for it. Read from the open file, so a download
/// streams the file it hashed even when an export replaces it meanwhile; the file is left at its start.
pub fn sha256_of(path: &Path, file: &mut std::fs::File) -> Result<(String, u64)> {
    static SEEN: Mutex<Vec<(Seen, String)>> = Mutex::new(Vec::new());
    let meta = file.metadata().with_context(|| format!("reading {}", path.display()))?;
    let key = Seen { path: path.to_path_buf(), modified: meta.modified().ok(), len: meta.len() };
    let lock = || SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, sha)) = lock().iter().find(|(k, _)| *k == key) {
        return Ok((sha.clone(), key.len));
    }
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    file.rewind()?;
    let sha = hex(&h.finalize());
    let mut seen = lock();
    seen.retain(|(k, _)| k.path != key.path);
    seen.push((key.clone(), sha.clone()));
    if seen.len() > KEPT {
        seen.remove(0);
    }
    Ok((sha, key.len))
}

/// [`sha256_of`] the file at `path`.
pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut f = std::fs::File::open(path).with_context(|| format!("no {}", path.display()))?;
    sha256_of(path, &mut f)
}

/// The id of an export from its header's bytes and its hak's hex SHA-256.
pub fn of(header: &[u8], hak_sha: &str) -> String {
    let mut h = sha2::Sha256::new();
    h.update(header);
    h.update(hak_sha.as_bytes());
    hex(&h.finalize())
}

/// `name`'s hak in this workspace (`nwn/hak/<name>.hak`).
pub fn hak_path(name: &str) -> Option<PathBuf> {
    crate::workspace::nwn_user_dir().map(|d| d.join("hak").join(format!("{name}.hak")))
}

/// The id of `name`'s last export in this workspace.
pub fn export_id(name: &str) -> Result<String> {
    let header = std::fs::read(crate::workspace::exports_dir().join(format!("{name}.json")))
        .with_context(|| format!("{name} hasn't been exported yet"))?;
    let hak = hak_path(name).context("no module folder")?;
    Ok(of(&header, &sha256_file(&hak)?.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha256(bytes: &[u8]) -> String {
        hex(&sha2::Sha256::digest(bytes))
    }

    #[test]
    fn a_files_sha_is_taken_again_once_it_changes_and_the_id_follows_the_header_and_the_hak() {
        let dir = std::env::temp_dir().join(format!("landscaper-export-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.hak");
        std::fs::write(&p, b"abc").unwrap();
        let (sha, len) = sha256_file(&p).unwrap();
        assert_eq!((sha.as_str(), len), ("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", 3));
        // Another length: read again, not the kept one.
        std::fs::write(&p, b"abcd").unwrap();
        assert_eq!(sha256_file(&p).unwrap().0, sha256(b"abcd"));
        let id = of(b"{}", &sha);
        assert_eq!(id, sha256(format!("{{}}{sha}").as_bytes()));
        assert_ne!(id, of(b"{ }", &sha));
        assert_ne!(id, of(b"{}", &sha256(b"abcd")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
