// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing a file whole or not at all. The server's own records (`assignments.json`, `markers.json`) are read, changed and
//! written back whole; a crash or a full disk halfway through a plain `fs::write` leaves a cut-off file, which then doesn't
//! parse, and nothing that needs the records works until it is restored. Here the new content goes to a temporary file beside
//! the old one, is flushed to the disk, and is renamed over it: a rename within one folder replaces the file in one step, so a
//! reader sees the old file or the new one.
//!
//! On Windows, opening a file at the moment a rename replaces it can fail ("Access is denied", or not found), and so can the
//! rename while another program has the file open; tried again a moment later, each works. A reader of a file [`write()`]
//! replaces opens it with [`open`], [`read`] or [`read_to_string`], which try again for a short while, as Go's own tools do
//! (`cmd/internal/robustio`, golang/go#31247); [`write()`]'s rename does the same. Elsewhere they are the plain calls.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Writes `bytes` to `path` in one step (see the module comment), making its folder if needed.
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    // A name of its own for each write, so two threads writing the same file never share a temporary file.
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().with_context(|| format!("{}: no folder", path.display()))?;
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let name = path.file_name().with_context(|| format!("{}: no file name", path.display()))?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}-{}.tmp", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
    let written = (|| -> Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        retried(|| std::fs::rename(&tmp, path))?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.with_context(|| format!("writing {}", path.display()))
}

/// Opens `path` to read it, trying again on Windows while a replace may be under way (see the module comment).
pub fn open(path: &Path) -> std::io::Result<std::fs::File> {
    retried(|| std::fs::File::open(path))
}

/// The bytes of `path`, as [`std::fs::read`], trying again on Windows while a replace may be under way.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut f = open(path)?;
    let mut out = Vec::new();
    f.read_to_end(&mut out)?;
    Ok(out)
}

/// The text of `path`, as [`std::fs::read_to_string`], trying again on Windows while a replace may be under way.
pub fn read_to_string(path: &Path) -> std::io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// How long [`open`] and [`write()`] keep trying while Windows refuses a file (access denied, sharing violation). A file that is
/// missing is said after [`NOT_FOUND_WAIT`] only: most readers ask for files that may not be there.
const REFUSED_WAIT: Duration = Duration::from_secs(1);
const NOT_FOUND_WAIT: Duration = Duration::from_millis(50);

/// `f`, tried again on Windows while it fails as a file being replaced makes it fail.
fn retried<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    if cfg!(windows) { again(f, REFUSED_WAIT, NOT_FOUND_WAIT) } else { f() }
}

/// `f`, tried again while it fails with Windows's ERROR_ACCESS_DENIED (5) or ERROR_SHARING_VIOLATION (32), for `refused`
/// at most, or ERROR_FILE_NOT_FOUND (2), for `not_found`; after a pause of 1 ms, doubled up to 50 ms. (By number: the
/// error kinds group other errors with them.)
fn again<T>(mut f: impl FnMut() -> std::io::Result<T>, refused: Duration, not_found: Duration) -> std::io::Result<T> {
    let start = Instant::now();
    let mut pause = Duration::from_millis(1);
    loop {
        let r = f();
        let wait = match r.as_ref().err().and_then(std::io::Error::raw_os_error) {
            Some(5 | 32) => refused,
            Some(2) => not_found,
            _ => return r,
        };
        if start.elapsed() >= wait {
            return r;
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_replaces_the_file_whole_and_leaves_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("landscaper-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("records.json");
        write(&path, b"[1]").unwrap();
        write(&path, b"[1,2]").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"[1,2]");
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["records.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writers_on_many_threads_never_leave_a_cut_off_file() {
        let dir = std::env::temp_dir().join(format!("landscaper-atomic-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("records.json");
        let big: Vec<Vec<u8>> = (0..8u8).map(|k| vec![b'a' + k; 256 * 1024]).collect();
        std::thread::scope(|s| {
            for b in &big {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..5 {
                        write(path, b).unwrap();
                        // Whatever is there now is one writer's content, whole.
                        let now = read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                        assert_eq!(now.len(), 256 * 1024);
                        assert!(now.iter().all(|c| *c == now[0]));
                    }
                });
            }
        });
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary files left");
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// A call that fails with the Windows error `code` its first `times` calls, and how many calls it had.
    fn failing(code: i32, times: usize) -> (std::rc::Rc<std::cell::Cell<usize>>, impl FnMut() -> std::io::Result<usize>) {
        let tries = std::rc::Rc::new(std::cell::Cell::new(0));
        let counted = tries.clone();
        let f = move || {
            counted.set(counted.get() + 1);
            if counted.get() <= times { Err(std::io::Error::from_raw_os_error(code)) } else { Ok(counted.get()) }
        };
        (tries, f)
    }

    #[test]
    fn a_file_windows_refuses_is_tried_again_for_a_while_and_a_missing_one_hardly() {
        let (refused, missing) = (Duration::from_millis(300), Duration::from_millis(5));
        // Refused (access denied, sharing violation) a few times, then read.
        for code in [5, 32] {
            assert_eq!(again(failing(code, 3).1, refused, missing).unwrap(), 4);
        }
        // Refused for good: said once the wait is over, not before.
        let t = Instant::now();
        assert_eq!(again(failing(5, usize::MAX).1, refused, missing).unwrap_err().raw_os_error(), Some(5));
        assert!(t.elapsed() >= refused && t.elapsed() < refused * 3, "{:?}", t.elapsed());
        // Missing: a moment only.
        let t = Instant::now();
        assert_eq!(again(failing(2, usize::MAX).1, refused, missing).unwrap_err().raw_os_error(), Some(2));
        assert!(t.elapsed() < refused, "{:?}", t.elapsed());
        // Any other error at once.
        let (tries, f) = failing(13, usize::MAX);
        assert!(again(f, refused, missing).is_err());
        assert_eq!(tries.get(), 1);
    }

    #[test]
    fn readers_on_other_threads_read_the_file_whole_while_it_is_replaced() {
        let dir = std::env::temp_dir().join(format!("landscaper-atomic-readers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("catalog.json");
        let big: Vec<Vec<u8>> = (0..4u8).map(|k| vec![b'a' + k; 64 * 1024]).collect();
        write(&path, &big[0]).unwrap();
        let writing = std::sync::atomic::AtomicUsize::new(big.len());
        std::thread::scope(|s| {
            for b in &big {
                let (path, writing) = (&path, &writing);
                s.spawn(move || {
                    for _ in 0..20 {
                        write(path, b).unwrap();
                    }
                    writing.fetch_sub(1, Ordering::SeqCst);
                });
            }
            for _ in 0..4 {
                let (path, writing) = (&path, &writing);
                s.spawn(move || {
                    while writing.load(Ordering::SeqCst) > 0 {
                        let now = read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                        assert_eq!(now.len(), 64 * 1024);
                        assert!(now.iter().all(|c| *c == now[0]));
                    }
                });
            }
        });
        assert_eq!(read_to_string(&path).unwrap().len(), 64 * 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
