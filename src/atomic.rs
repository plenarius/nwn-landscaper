// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing a file whole or not at all. The server's own records (`assignments.json`, `markers.json`) are read, changed and
//! written back whole; a crash or a full disk halfway through a plain `fs::write` leaves a cut-off file, which then doesn't
//! parse, and nothing that needs the records works until it is restored. Here the new content goes to a temporary file beside
//! the old one, is flushed to the disk, and is renamed over it: a rename within one folder replaces the file in one step, so a
//! reader sees the old file or the new one.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.with_context(|| format!("writing {}", path.display()))
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

    /// The file at `path`, read while other threads replace it. On Windows an open at the moment a rename replaces the file
    /// can fail with "Access is denied" (or not find it), and succeeds when tried again; Go's own tools retry the same
    /// (cmd/internal/robustio, golang/go#31247). The file is never cut off: what is read is still checked whole.
    fn read_while_replaced(path: &Path) -> Vec<u8> {
        let mut tries = 0;
        loop {
            match std::fs::read(path) {
                Ok(d) => return d,
                Err(e)
                    if cfg!(windows)
                        && matches!(e.kind(), std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound)
                        && tries < 200 =>
                {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("{}: {e}", path.display()),
            }
        }
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
                        let now = read_while_replaced(path);
                        assert_eq!(now.len(), 256 * 1024);
                        assert!(now.iter().all(|c| *c == now[0]));
                    }
                });
            }
        });
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary files left");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
