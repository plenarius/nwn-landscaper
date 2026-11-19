// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The small JSON files a team server keeps its records in beside the workspaces (`assignments.json`, `markers.json`): each
//! is read whole, changed and written back whole ([`crate::atomic::write`]).

use std::path::Path;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

/// A records file: the default (no rows) when there is none yet, an error when it is there and can't be read or doesn't
/// parse. Read as empty, a damaged file would be replaced on the next change by one with only that change's row. The error
/// names the file without its folder, since builders see it.
pub fn read<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    let name = path.file_name().map_or_default(|n| n.to_string_lossy().into_owned());
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(e).with_context(|| format!("{name} can't be read")),
    };
    serde_json::from_slice(&data)
        .with_context(|| format!("{name} doesn't parse: fix or restore it (nothing that needs it works until then)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_has_no_rows_and_a_damaged_one_is_an_error() {
        let dir = std::env::temp_dir().join(format!("landscaper-records-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("assignments.json");
        let _ = std::fs::remove_file(&p);
        assert_eq!(read::<Vec<u32>>(&p).unwrap(), Vec::<u32>::new());
        std::fs::write(&p, "[1, 2,]").unwrap();
        let e = format!("{:#}", read::<Vec<u32>>(&p).unwrap_err());
        assert!(e.starts_with("assignments.json doesn't parse") && !e.contains(&*dir.to_string_lossy()), "{e}");
        std::fs::write(&p, "[1, 2]").unwrap();
        assert_eq!(read::<Vec<u32>>(&p).unwrap(), vec![1, 2]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
