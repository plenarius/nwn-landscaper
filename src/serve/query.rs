// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A request's query string, and the checks on the names it carries. Every route reads its parameters through [`Query`], so
//! they are all decoded the same way: the first `key=value` pair with that key wins, `%XX` and `+` are decoded, and a missing
//! key is `None` (or an empty string, for the routes that treat the two alike).

use anyhow::Result;

/// A request's query string (what follows the `?`, undecoded).
#[derive(Clone, Copy, Debug)]
pub(super) struct Query<'q>(&'q str);

impl<'q> Query<'q> {
    /// The query string `s`, without its `?`.
    pub(super) fn new(s: &'q str) -> Query<'q> {
        Query(s)
    }

    /// The query string as it came.
    pub(super) fn as_str(&self) -> &'q str {
        self.0
    }

    /// The first value of `key`, as sent (not decoded).
    pub(super) fn raw(&self, key: &str) -> Option<&'q str> {
        self.0.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
    }

    /// The first value of `key`, decoded.
    pub(super) fn get(&self, key: &str) -> Option<String> {
        self.raw(key).map(percent_decode)
    }

    /// [`Query::get`], or an empty string when `key` isn't given.
    pub(super) fn text(&self, key: &str) -> String {
        self.get(key).unwrap_or_default()
    }

    /// [`Query::text`] in lower case (ASCII).
    pub(super) fn lower(&self, key: &str) -> String {
        self.text(key).to_ascii_lowercase()
    }

    /// [`Query::get`], or `None` when it is empty too.
    pub(super) fn nonempty(&self, key: &str) -> Option<String> {
        self.get(key).filter(|v| !v.is_empty())
    }

    /// [`Query::get`] parsed as a `T`, or `None` when it is missing or doesn't parse.
    pub(super) fn number<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// [`Query::get`], or the error `<key> missing`.
    pub(super) fn required(&self, key: &str) -> Result<String> {
        self.get(key).ok_or_else(|| anyhow::anyhow!("{key} missing"))
    }

    /// Whether the first value of `key` is `1` (decoded).
    pub(super) fn one(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v == "1")
    }

    /// Whether any pair is exactly `key=1`, as sent (a switch the editor adds to a URL, such as `any=1`).
    pub(super) fn flag(&self, key: &str) -> bool {
        self.0.split('&').any(|kv| kv.strip_prefix(key).is_some_and(|rest| rest == "=1"))
    }
}

/// Decodes a query value (%XX escapes, two hex digits, and '+' for space). A `%` not followed by two hex digits is kept as it
/// is (`u8::from_str_radix` alone would also take a sign: `%+1`).
pub(super) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut k = 0;
    while k < b.len() {
        match b[k] {
            b'%' if k + 2 < b.len() => match b[k + 1..k + 3]
                .iter()
                .all(u8::is_ascii_hexdigit)
                .then(|| std::str::from_utf8(&b[k + 1..k + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
                .flatten()
            {
                Some(v) => {
                    out.push(v);
                    k += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        k += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An area's name, as the editor makes them: 1-9 lower case letters, digits or `_`. It prefixes every resref of the area.
pub(super) fn area_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 9 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// An area's name from a request, refused unless [`area_name_ok`]: every route that makes file names from it checks it
/// first, before it touches the disk.
pub(super) fn checked_area_name(name: &str) -> Result<String> {
    anyhow::ensure!(area_name_ok(name), "bad area name {name:?} (1-9 lower case letters, digits or _)");
    Ok(name.to_string())
}

/// A resref-like name: 1-16 ASCII letters, digits or `_` (a module, an imported area, a model or texture).
pub(super) fn short_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A model's or texture's name as the game's files have them: 1-16 ASCII letters, digits, `_` or `-` (haks have textures such as
/// `des_cactus-01`, which the game loads). Names the Landscaper makes keep to [`short_name_ok`].
pub(super) fn resource_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resource_name_may_have_a_hyphen_a_short_name_may_not() {
        for ok in ["des_cactus-01", "grs_bush-03", "plc_rock", "a"] {
            assert!(resource_name_ok(ok), "{ok}");
        }
        for bad in ["", "../x", "a.b", "a b", "seventeen_chars_x", "x%2f"] {
            assert!(!resource_name_ok(bad), "{bad}");
        }
        assert!(!short_name_ok("des_cactus-01"));
    }

    #[test]
    fn an_escape_is_two_hex_digits_and_nothing_else() {
        assert_eq!(percent_decode("a%2Fb%2fc+d%41"), "a/b/c dA");
        // A sign or a non-hex digit isn't an escape: kept as sent.
        for kept in ["%+1", "%-1", "% 1", "%g1", "%1", "%"] {
            assert_eq!(percent_decode(kept), kept.replace('+', " "), "{kept}");
        }
    }

    #[test]
    fn the_first_pair_with_a_key_wins_and_is_decoded() {
        let q = Query::new("name=a%2Fb&name=c&q=x+y&named=z&empty=&touch=1&dry=%31");
        assert_eq!(q.get("name").as_deref(), Some("a/b"));
        assert_eq!(q.get("q").as_deref(), Some("x y"));
        assert_eq!(q.raw("name"), Some("a%2Fb"));
        assert_eq!(q.get("na"), None, "a key is matched whole");
        assert_eq!(q.text("missing"), "");
        assert_eq!(q.nonempty("empty"), None);
        assert!(q.flag("touch") && !q.flag("dry"), "a flag is compared as sent");
        assert!(q.one("dry"), "one() decodes");
        assert_eq!(format!("{:#}", q.required("tiles").unwrap_err()), "tiles missing");
    }
}
