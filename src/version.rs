// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The engine's version as it tells it (`--version`, `/api/me`, the start line): Cargo.toml's, which is the last release's,
//! with `+dev` after it when changes wait for the next release: a changelog entry in changelog.d/ (counted when it is built,
//! build.rs), or a CHANGELOG.md that starts with `## Unreleased`. A release's tag has neither, so a build of master between
//! releases doesn't claim to be the last one.

use std::path::Path;
use std::sync::OnceLock;

/// This build's version: `0.6.1`, or `0.6.1+dev` for a build after 0.6.1 with changes not yet released.
pub fn text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| of(env!("CARGO_PKG_VERSION"), include_str!("../CHANGELOG.md"), env!("LANDSCAPER_CHANGES_WAITING") != "0"))
}

/// `version`, with `+dev` when changes are `waiting` in changelog.d/ or `changelog`'s first `## ` heading is `Unreleased`.
fn of(version: &str, changelog: &str, waiting: bool) -> String {
    let top = changelog.lines().find_map(|l| l.strip_prefix("## "));
    if waiting || top.is_some_and(|t| t.trim().eq_ignore_ascii_case("unreleased")) { format!("{version}+dev") } else { version.to_string() }
}

/// The changelog entries waiting in a changelog.d/ folder, by name: its `.md` files, not its dot files (build.rs counts the
/// engine's own the same way). None when there is no such folder.
pub fn fragments(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).map_or_else(|_| Vec::new(), |d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect());
    names.retain(|n| !n.starts_with('.') && Path::new(n).extension().is_some_and(|x| x == "md"));
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::{fragments, of};

    #[test]
    fn a_release_is_its_version_and_a_build_after_it_says_dev() {
        let released = "# Changelog\n\nIntro.\n\n## 0.6.1\n\n- A fix.\n\n## 0.6.0\n";
        assert_eq!(of("0.6.1", released, false), "0.6.1");
        let after = "# Changelog\n\n## Unreleased\n\n- Something new.\n\n## 0.6.1\n\n- A fix.\n";
        assert_eq!(of("0.6.1", after, false), "0.6.1+dev");
        assert_eq!(of("0.6.1", "# Changelog\n", false), "0.6.1");
        // (a `###` heading inside a release isn't a version's)
        assert_eq!(of("0.6.1", "## 0.6.1\n\n### Unreleased\n", false), "0.6.1");
        // An entry waiting in changelog.d/ after a release, whose changelog starts at its version.
        assert_eq!(of("0.6.1", released, true), "0.6.1+dev");
    }

    #[test]
    fn the_waiting_entries_are_the_md_files_but_not_the_dot_files() {
        let dir = std::env::temp_dir().join(format!("landscaper-fragments-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(fragments(&dir).is_empty(), "no folder");
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["b.md", "a.md", ".markdownlint.jsonc", ".hidden.md", "notes.txt"] {
            std::fs::write(dir.join(f), "- x\n").unwrap();
        }
        assert_eq!(fragments(&dir), ["a.md", "b.md"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
