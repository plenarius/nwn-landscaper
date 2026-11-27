// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What a builder's workspace may hold on a team server's disk: [`MAX_AREAS`] areas (a canvas counts as one) and [`MAX_BYTES`]
//! in all, its uploads aside (they have limits of their own, `upload.rs`). Saves, imports, exports and checks are refused with
//! 413 once the workspace is full. Without this a signed-in builder could fill the server's disk at 8 MB a request: an area's
//! name has 37^9 values, and every `/api/save?replace=1` kept another copy of the area it replaced.
//!
//! On your own machine there are no limits: the disk is yours.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The most areas a workspace may have (its working copies, exports and imports, by name).
pub const MAX_AREAS: usize = 100;

/// The most bytes a workspace may take, its uploads aside: about 50 MB for each of [`MAX_AREAS`] areas (an 8 MB working copy,
/// its export's heights, paint, module and hak).
pub const MAX_BYTES: u64 = 5 << 30;

/// How many copies of an area a workspace keeps set aside, of each kind, the newest: those another area of the same name
/// replaced (`areas/<name>.json.replaced-<seconds>`), and those the builder deleted (`deleted/<name>-<seconds>/`).
pub const KEPT_ASIDE: usize = 3;

/// How long a workspace's measure is used before its folders are walked again. Between walks, what a save writes is added.
const FRESH: Duration = Duration::from_secs(60);

/// A workspace that is full (answered 413).
#[derive(Debug)]
pub struct Full(String);

impl std::fmt::Display for Full {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Full {}

/// Whether the current workspace has room for `adding` more bytes and for an area called `name` (which it may have already).
/// Always on your own machine.
pub fn room(name: &str, adding: u64) -> Result<(), Full> {
    if !crate::workspace::team() {
        return Ok(());
    }
    let root = crate::workspace::root();
    room_in(&root, name, adding, used(&root))
}

/// [`room`] in the workspace `root`, which takes `used` bytes now.
fn room_in(root: &Path, name: &str, adding: u64, used: u64) -> Result<(), Full> {
    let names = area_names(root);
    if !names.contains(name) && names.len() >= MAX_AREAS {
        return Err(Full(format!(
            "your workspace has {MAX_AREAS} areas, the most it may hold: open one of them, or delete one you no longer need"
        )));
    }
    if used.saturating_add(adding) > MAX_BYTES {
        return Err(Full(format!(
            "your workspace is full ({} GB): delete areas you no longer need, or ask an admin to clear their deleted copies",
            MAX_BYTES >> 30
        )));
    }
    Ok(())
}

/// The areas of the workspace `root`, by name: working copies (`areas/<name>.json`), exports and imports
/// (`exports/<name>.json`, `exports/<name>.import`), and canvases (`canvases/<name>/`, one each however many pieces it has).
fn area_names(root: &Path) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = std::fs::read_dir(root.join("canvases"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().filter(|n| crate::canvas::name_ok(n)).map(str::to_string))
        .collect();
    for dir in [root.join("areas"), root.join("exports")] {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if let Some(name) = file.strip_suffix(".json").or_else(|| file.strip_suffix(".import"))
                && !name.is_empty()
                && !name.starts_with('.')
            {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// The workspaces' last measures: when, and how many bytes.
fn measures() -> std::sync::MutexGuard<'static, Option<HashMap<PathBuf, (Instant, u64)>>> {
    static SEEN: Mutex<Option<HashMap<PathBuf, (Instant, u64)>>> = Mutex::new(None);
    SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The bytes the workspace `root` takes, its uploads aside: its last measure while [`FRESH`], else walked again.
fn used(root: &Path) -> u64 {
    if let Some((at, bytes)) = measures().as_ref().and_then(|m| m.get(root))
        && at.elapsed() < FRESH
    {
        return *bytes;
    }
    let bytes = size_of(root, true, &mut HashSet::new());
    measures().get_or_insert_with(HashMap::new).insert(root.to_path_buf(), (Instant::now(), bytes));
    bytes
}

/// The bytes of the files under `dir` (links not followed; a file with two names, such as a working copy and its hard-linked
/// `.bak`, counted once), leaving out `uploads/` when `top`.
fn size_of(dir: &Path, top: bool, seen: &mut HashSet<(u64, u64)>) -> u64 {
    let mut total = 0;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if top && e.file_name() == "uploads" {
            continue;
        }
        let Ok(m) = e.path().symlink_metadata() else { continue };
        if m.is_dir() {
            total += size_of(&e.path(), false, seen);
        } else if m.is_file() && file_id(&m).is_none_or(|id| seen.insert(id)) {
            total += m.len();
        }
    }
    total
}

/// A file's device and inode, which its every name shares.
#[cfg(unix)]
fn file_id(m: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((m.dev(), m.ino()))
}

/// None where the standard library can't say (Windows): each name of a file is counted. (A team server runs on Linux.)
#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// Counts `bytes` the current workspace took in the request now ending, until its next walk.
pub fn wrote(bytes: u64) {
    if !crate::workspace::team() {
        return;
    }
    if let Some((_, used)) = measures().as_mut().and_then(|m| m.get_mut(&crate::workspace::root())) {
        *used = used.saturating_add(bytes);
    }
}

/// Forgets the current workspace's measure, so the next [`room`] walks it again: after an export or an import, which write
/// files whose size isn't known in advance.
pub fn forget() {
    if crate::workspace::team()
        && let Some(m) = measures().as_mut()
    {
        m.remove(&crate::workspace::root());
    }
}

/// Removes all but the newest [`KEPT_ASIDE`] copies of area `name` that other areas of the same name replaced, in the
/// working copies' folder `dir`. A team server only: on your own machine they stay until you remove them.
pub fn prune_replaced(dir: &Path, name: &str) {
    if crate::workspace::team() {
        prune(dir, &format!("{name}.json.replaced-"));
    }
}

/// Removes all but the newest [`KEPT_ASIDE`] deleted copies of area `name` (`<name>-<seconds>/`), in the workspace's folder of
/// deleted areas `dir`. A team server only, as [`prune_replaced`].
pub fn prune_deleted(dir: &Path, name: &str) {
    if crate::workspace::team() {
        prune(dir, &format!("{name}-"));
    }
}

/// Removes all but the newest [`KEPT_ASIDE`] of the files and folders in `dir` named `<prefix><Unix seconds>`. (An area's name
/// has no `-` nor `.`, so one area's prefix never takes in another's copies.)
fn prune(dir: &Path, prefix: &str) {
    let mut copies: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            file.strip_prefix(prefix).and_then(|s| s.parse().ok()).map(|at| (at, e.path()))
        })
        .collect();
    copies.sort_unstable_by_key(|c| std::cmp::Reverse(c.0));
    for (_, old) in copies.into_iter().skip(KEPT_ASIDE) {
        let _ = if old.is_dir() { std::fs::remove_dir_all(old) } else { std::fs::remove_file(old) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ls-quota-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["areas", "exports/cave.import", "uploads"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        dir
    }

    #[test]
    fn a_workspace_is_measured_without_its_uploads_and_a_hard_link_counts_once() {
        let dir = scratch("size");
        std::fs::write(dir.join("areas/town.json"), vec![0u8; 1000]).unwrap();
        std::fs::hard_link(dir.join("areas/town.json"), dir.join("areas/town.json.bak")).unwrap();
        std::fs::write(dir.join("exports/cave.import/area.are"), vec![0u8; 200]).unwrap();
        std::fs::write(dir.join("uploads/big.mod"), vec![0u8; 5000]).unwrap();
        // (On Windows each name counts: see `file_id`.)
        assert_eq!(size_of(&dir, true, &mut HashSet::new()), if cfg!(unix) { 1200 } else { 2200 });
        assert_eq!(area_names(&dir).into_iter().collect::<Vec<_>>(), ["cave", "town"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_workspace_refuses_new_bytes_and_new_areas_but_not_its_own() {
        let dir = scratch("room");
        // (With the scratch folder's import, `cave`: MAX_AREAS in all.)
        for i in 1..MAX_AREAS {
            std::fs::write(dir.join(format!("areas/a{i}.json")), b"{}").unwrap();
        }
        assert!(room_in(&dir, "a7", 1, 0).is_ok(), "an area it has");
        let e = room_in(&dir, "fresh", 1, 0).unwrap_err();
        assert!(e.to_string().contains("100 areas"), "{e}");
        let _ = std::fs::remove_file(dir.join("areas/a1.json"));
        assert!(room_in(&dir, "fresh", 1, 0).is_ok());
        assert!(room_in(&dir, "a7", 8 << 20, MAX_BYTES - (8 << 20)).is_ok(), "up to the limit");
        let e = room_in(&dir, "a7", 8 << 20, MAX_BYTES - 1).unwrap_err();
        assert!(e.to_string().contains("full (5 GB)"), "{e}");
        assert!(room_in(&dir, "a7", u64::MAX, u64::MAX).is_err(), "no overflow");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_newest_replaced_copies_are_kept() {
        let dir = scratch("replaced");
        for at in [100, 300, 200, 400, 50] {
            std::fs::write(dir.join(format!("areas/town.json.replaced-{at}")), b"x").unwrap();
        }
        std::fs::write(dir.join("areas/town.json"), b"x").unwrap();
        std::fs::write(dir.join("areas/townhall.json.replaced-10"), b"x").unwrap();
        prune(&dir.join("areas"), "town.json.replaced-");
        let mut left: Vec<String> =
            std::fs::read_dir(dir.join("areas")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(
            left,
            ["town.json", "town.json.replaced-200", "town.json.replaced-300", "town.json.replaced-400", "townhall.json.replaced-10"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_newest_deleted_copies_of_an_area_are_kept() {
        let dir = scratch("deleted");
        let deleted = dir.join("deleted");
        for at in [100, 300, 200, 400] {
            std::fs::create_dir_all(deleted.join(format!("town-{at}/areas"))).unwrap();
            std::fs::write(deleted.join(format!("town-{at}/areas/town.json")), b"x").unwrap();
        }
        std::fs::create_dir_all(deleted.join("town_2-50")).unwrap();
        prune(&deleted, "town-");
        let mut left: Vec<String> =
            std::fs::read_dir(&deleted).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["town-200", "town-300", "town-400", "town_2-50"]);
        // (A deleted copy is no area of the workspace's: deleting frees its place.)
        assert!(area_names(&dir).iter().all(|n| !n.starts_with("town")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
