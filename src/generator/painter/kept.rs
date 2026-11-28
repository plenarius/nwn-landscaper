// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Files an export makes (baked ground textures, shadow maps), kept for the next export by a digest of everything they are made
//! from: in memory while the program runs, and on disk in the data folder's `cache/`, so a command-line export, a restarted
//! `landscaper serve` and the browser page's engine (one run a request) don't make again what hasn't changed. On lsforest the
//! bake and the shadow maps are 1,400 of the tiles' 1,500 s of thread time. A file on disk is named by its key and this build's
//! source digest ([`SOURCE`]), so a build with any other code never reads it; it holds its contents' digest, so a file cut short
//! is a miss. Both sides are bounded: the least recently used go first, and on disk anything unused for [`MAX_AGE_DAYS`] days.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use sha1::Digest;

/// A digest of this build's source and locked dependencies (build.rs): a cache file from other code may differ.
const SOURCE: &str = env!("LANDSCAPER_SOURCE_DIGEST");

/// Days a file on disk may go unused before it goes.
const MAX_AGE_DAYS: u64 = 30;

/// What a file on disk starts with, then its contents' SHA-1, then its contents.
const MAGIC: &[u8; 4] = b"LSK1";

/// One kind of kept file: its folder under the data folder's `cache/`, kept up to a number of bytes in memory and on disk.
pub(in crate::generator) struct Store {
    name: &'static str,
    memory: Mutex<Kept>,
    disk_max: u64,
    /// Bytes written to disk since the folder was last pruned; None until the first write of the run prunes it.
    written: Mutex<Option<u64>>,
}

impl Store {
    pub(in crate::generator) const fn new(name: &'static str, memory_max: usize, disk_max: u64) -> Store {
        Store { name, memory: Mutex::new(Kept::new(memory_max)), disk_max, written: Mutex::new(None) }
    }

    /// The folder its files are kept in on disk.
    fn dir(&self) -> PathBuf {
        crate::paint::data_dir().join("cache").join(self.name)
    }

    /// The file kept for `key`: from memory, else from disk.
    pub(in crate::generator) fn get(&self, key: &[u8; 20]) -> Option<Vec<u8>> {
        if let Some(file) = self.memory.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(key) {
            return Some(file);
        }
        let file = read(&self.dir(), key)?;
        self.memory.lock().unwrap_or_else(std::sync::PoisonError::into_inner).put(*key, &file);
        Some(file)
    }

    /// Keeps `file` for `key`, in memory and on disk. A disk that can't be written to only means it isn't kept there.
    pub(in crate::generator) fn put(&self, key: [u8; 20], file: &[u8]) {
        self.memory.lock().unwrap_or_else(std::sync::PoisonError::into_inner).put(key, file);
        let dir = self.dir();
        if write(&dir, &key, file).is_err() {
            return;
        }
        // (Pruned on the run's first write, then each time a quarter of the limit has been written.)
        let due = {
            let mut written = self.written.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let w = written.map_or(u64::MAX, |w| w + file.len() as u64 + 24);
            *written = Some(if w >= self.disk_max / 4 { 0 } else { w });
            w >= self.disk_max / 4
        };
        if due {
            prune(&dir, self.disk_max, Duration::from_secs(MAX_AGE_DAYS * 86_400));
        }
    }
}

/// The name on disk of the file kept for `key` by a build of source digest `source`.
fn file_name(source: &str, key: &[u8; 20]) -> String {
    let mut h = sha1::Sha1::new();
    h.update(source.as_bytes());
    h.update(key);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The file kept for `key` in folder `dir`, if it is there whole. Reading it counts as a use (its modification time is set
/// to now, for [`prune`]); one that isn't whole is removed.
fn read(dir: &Path, key: &[u8; 20]) -> Option<Vec<u8>> {
    let path = dir.join(file_name(SOURCE, key));
    let data = std::fs::read(&path).ok()?;
    let whole = data.len() >= 24 && &data[..4] == MAGIC && data[4..24] == *sha1::Sha1::digest(&data[24..]);
    if !whole {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    if let Ok(f) = std::fs::File::options().append(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(data[24..].to_vec())
}

/// Writes `file` for `key` into folder `dir`: to a file of its own first, then renamed, so another export reading the folder never
/// sees half of it.
fn write(dir: &Path, key: &[u8; 20], file: &[u8]) -> std::io::Result<()> {
    // (This run's start in nanoseconds tells it from other exports writing the folder: WASI has no process id.)
    static RUN: std::sync::OnceLock<u128> = std::sync::OnceLock::new();
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir)?;
    let name = file_name(SOURCE, key);
    let run = RUN.get_or_init(|| SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    let tmp = dir.join(format!("{name}.{run:x}-{}.tmp", SERIAL.fetch_add(1, Ordering::Relaxed)));
    let mut data = Vec::with_capacity(file.len() + 24);
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&sha1::Sha1::digest(file));
    data.extend_from_slice(file);
    std::fs::write(&tmp, &data)?;
    std::fs::rename(&tmp, dir.join(name)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Removes from folder `dir` the files unused for longer than `max_age`, then the least recently used until the rest fit in
/// three quarters of `max` bytes (so the next prune is a while off). A half-written file over an hour old is a crashed export's.
fn prune(dir: &Path, max: u64, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    let age = |t: SystemTime| now.duration_since(t).unwrap_or_default();
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        // (A file system without modification times: as if used now.)
        let used = meta.modified().unwrap_or(now);
        let path = e.path();
        let tmp = path.extension().is_some_and(|x| x == "tmp");
        if age(used) > max_age || tmp && age(used) > Duration::from_secs(3600) {
            let _ = std::fs::remove_file(&path);
        } else if !tmp {
            files.push((used, meta.len(), path));
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= max / 4 * 3 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// Files kept in memory by key, up to `max` bytes; the least recently used go first.
struct Kept {
    /// (key, file, when last used).
    entries: Vec<([u8; 20], Vec<u8>, u64)>,
    bytes: usize,
    max: usize,
    clock: u64,
}

impl Kept {
    const fn new(max: usize) -> Kept {
        Kept { entries: Vec::new(), bytes: 0, max, clock: 0 }
    }

    fn get(&mut self, key: &[u8; 20]) -> Option<Vec<u8>> {
        self.clock += 1;
        let e = self.entries.iter_mut().find(|e| e.0 == *key)?;
        e.2 = self.clock;
        Some(e.1.clone())
    }

    fn put(&mut self, key: [u8; 20], file: &[u8]) {
        if file.len() > self.max || self.entries.iter().any(|e| e.0 == key) {
            return;
        }
        self.clock += 1;
        self.bytes += file.len();
        self.entries.push((key, file.to_vec(), self.clock));
        while self.bytes > self.max {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].2).expect("over the limit, so not empty");
            self.bytes -= self.entries.swap_remove(oldest).1.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("landscaper-kept-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn kept_files_go_least_recently_used_first_once_over_the_limit() {
        let mut kept = Kept::new(10);
        kept.put([1; 20], &[1; 4]);
        kept.put([2; 20], &[2; 4]);
        // (Using the first makes the second the oldest.)
        assert_eq!(kept.get(&[1; 20]), Some(vec![1; 4]));
        kept.put([3; 20], &[3; 4]);
        assert_eq!((kept.get(&[2; 20]), kept.get(&[1; 20]), kept.get(&[3; 20])), (None, Some(vec![1; 4]), Some(vec![3; 4])));
        assert_eq!(kept.bytes, 8);
        // Too big to keep at all: nothing else goes.
        kept.put([4; 20], &[4; 11]);
        assert_eq!((kept.get(&[4; 20]), kept.bytes), (None, 8));
    }

    #[test]
    fn a_file_written_to_disk_reads_back_by_its_key_alone() {
        let dir = temp("hit");
        write(&dir, &[1; 20], b"baked").unwrap();
        assert_eq!(read(&dir, &[1; 20]).as_deref(), Some(&b"baked"[..]), "a hit");
        assert_eq!(read(&dir, &[2; 20]), None, "another key misses");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_cut_short_or_changed_on_disk_is_a_miss_and_goes() {
        let dir = temp("torn");
        write(&dir, &[1; 20], b"baked ground").unwrap();
        let path = dir.join(file_name(SOURCE, &[1; 20]));
        let data = std::fs::read(&path).unwrap();
        std::fs::write(&path, &data[..data.len() - 3]).unwrap();
        assert_eq!(read(&dir, &[1; 20]), None);
        assert!(!path.exists(), "removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn another_build_names_the_same_key_differently() {
        assert_ne!(file_name("a", &[1; 20]), file_name("b", &[1; 20]));
        assert_ne!(file_name("a", &[1; 20]), file_name("a", &[2; 20]));
        assert_eq!(file_name("a", &[1; 20]).len(), 40);
    }

    #[test]
    fn pruning_removes_the_stale_then_the_least_recently_used() {
        let dir = temp("prune");
        let ago = |days: u64| SystemTime::now() - Duration::from_secs(days * 86_400);
        for (k, days) in [(1u8, 40), (2, 3), (3, 2), (4, 1)] {
            write(&dir, &[k; 20], &[k; 100]).unwrap();
            let f = std::fs::File::options().append(true).open(dir.join(file_name(SOURCE, &[k; 20]))).unwrap();
            f.set_modified(ago(days)).unwrap();
        }
        // 4 files of 124 bytes: the 40-day-old one goes for its age, then the oldest until 3/4 of 300 is left.
        prune(&dir, 300, Duration::from_secs(MAX_AGE_DAYS * 86_400));
        let left: Vec<bool> = (1..=4u8).map(|k| dir.join(file_name(SOURCE, &[k; 20])).exists()).collect();
        assert_eq!(left, [false, false, false, true]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
