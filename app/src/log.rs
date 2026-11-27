// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The log: what the app and the engine print goes to `landscaper.log` in the data folder, the start before it kept as
//! `landscaper.previous.log`. Started from a terminal, it stays in the terminal.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use stdio_override::{StderrOverride, StdoutOverride};

/// The log while it is written; dropped, the output goes back where it was.
pub struct Log {
    pub path: PathBuf,
    _out: StdoutOverride,
    _err: StderrOverride,
}

/// Sends standard output and errors to the log, unless they go to a terminal. None when they stay where they are, or the log
/// can't be made (the app runs without it).
pub fn start() -> Option<Log> {
    if std::io::stdout().is_terminal() {
        return None;
    }
    let dir = landscaper::data_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("landscaper.log");
    keep_previous(&path);
    let file = std::fs::File::create(&path).ok()?;
    let out = StdoutOverride::from_io(file.try_clone().ok()?).ok()?;
    let err = StderrOverride::from_io(file).ok()?;
    Some(Log { path, _out: out, _err: err })
}

/// The last start's log becomes `landscaper.previous.log`, replacing the one before it.
fn keep_previous(log: &Path) {
    if log.is_file() {
        let _ = std::fs::rename(log, log.with_file_name("landscaper.previous.log"));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn two_starts_are_kept() {
        let dir = std::env::temp_dir().join(format!("ls-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("landscaper.log");
        for start in ["first", "second", "third"] {
            super::keep_previous(&log);
            std::fs::write(&log, start).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "third");
        assert_eq!(std::fs::read_to_string(dir.join("landscaper.previous.log")).unwrap(), "second");
        let _ = std::fs::remove_dir_all(dir);
    }
}
