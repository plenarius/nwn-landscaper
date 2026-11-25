// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The engine's version as it tells it (`--version`, `/api/me`, the start line): Cargo.toml's, which is the last release's,
//! with `+dev` after it when this CHANGELOG.md starts with `## Unreleased`. A release's tag starts with its own heading, so a
//! build of master between releases doesn't claim to be the last one.

use std::sync::OnceLock;

/// This build's version: `0.6.1`, or `0.6.1+dev` for a build after 0.6.1 with changes not yet released.
pub fn text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| of(env!("CARGO_PKG_VERSION"), include_str!("../CHANGELOG.md")))
}

/// `version`, with `+dev` when `changelog`'s first `## ` heading is `Unreleased`.
fn of(version: &str, changelog: &str) -> String {
    let top = changelog.lines().find_map(|l| l.strip_prefix("## "));
    if top.is_some_and(|t| t.trim().eq_ignore_ascii_case("unreleased")) { format!("{version}+dev") } else { version.to_string() }
}

#[cfg(test)]
mod tests {
    use super::of;

    #[test]
    fn a_release_is_its_version_and_a_build_after_it_says_dev() {
        let released = "# Changelog\n\nIntro.\n\n## 0.6.1\n\n- A fix.\n\n## 0.6.0\n";
        assert_eq!(of("0.6.1", released), "0.6.1");
        let after = "# Changelog\n\n## Unreleased\n\n- Something new.\n\n## 0.6.1\n\n- A fix.\n";
        assert_eq!(of("0.6.1", after), "0.6.1+dev");
        assert_eq!(of("0.6.1", "# Changelog\n"), "0.6.1");
        // (a `###` heading inside a release isn't a version's)
        assert_eq!(of("0.6.1", "## 0.6.1\n\n### Unreleased\n"), "0.6.1");
    }
}
