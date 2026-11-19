// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Which builder may work on which existing (production) area.
//!
//! Builders have no access to the module's areas unless an admin assigned one to them (`docs/TEAM-SERVER.md`). An assignment is one
//! row: who, which area (its resref), who granted it and when, when it ends (if it does), and when it was revoked. It is kept in
//! `assignments.json` beside the workspaces, replaced atomically on every change (a handful of rows: a database would be more than it
//! needs). A file that is there but doesn't parse is an error everywhere (and stops a team server at start): read as empty, the next
//! grant would write it back with one row.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paint;

/// A grant of one production area (or all of them) to one builder: what lets them import, export and publish it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Assignment {
    /// The builder it is granted to (their sign-in name, lower case).
    pub user: String,
    /// The production area's resref, lower case; `*` is every area (a builder trusted with the whole module).
    pub area: String,
    /// The admin who granted it.
    pub granted_by: String,
    /// When it was granted (Unix seconds).
    pub granted_at: u64,
    /// When it ends (Unix seconds); None: when revoked.
    #[serde(default)]
    pub expires_at: Option<u64>,
    /// When it was revoked (Unix seconds); kept, so the history stays.
    #[serde(default)]
    pub revoked_at: Option<u64>,
    /// The admin's note: what the work is for.
    #[serde(default)]
    pub note: Option<String>,
}

/// The wildcard area: an assignment of it covers every area.
pub const ALL: &str = "*";

/// Whether an assignment of `held` covers `area`.
pub fn covers(held: &str, area: &str) -> bool {
    held == ALL || held.eq_ignore_ascii_case(area)
}

impl Assignment {
    /// Whether it still lets `user` work on the area at `now` (Unix seconds).
    pub fn active_at(&self, now: u64) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|e| now < e)
    }
}

/// Serialises changes (the file is read, changed and written back whole).
static LOCK: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    paint::data_dir().join("assignments.json")
}

/// The time now in Unix seconds, as the records keep it (0 on a clock before 1970).
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn read() -> Result<Vec<Assignment>> {
    crate::records::read(&path())
}

/// Whether `assignments.json` reads (a team server checks it at start).
pub fn check() -> Result<()> {
    read().map(drop).with_context(|| path().display().to_string())
}

fn write(list: &[Assignment]) -> Result<()> {
    crate::atomic::write(&path(), &serde_json::to_vec_pretty(list)?)
}

/// Every assignment ever made (revoked and ended ones included), oldest first.
pub fn all() -> Result<Vec<Assignment>> {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    read()
}

/// `user`'s assignments that still count.
pub fn active_for(user: &str) -> Result<Vec<Assignment>> {
    let t = now();
    Ok(all()?.into_iter().filter(|a| a.user == user && a.active_at(t)).collect())
}

/// Whether `user` may work on production area `area` now.
pub fn allows(user: &str, area: &str) -> Result<bool> {
    let t = now();
    Ok(all()?.iter().any(|a| a.user == user && covers(&a.area, area) && a.active_at(t)))
}

/// The most days a grant may run: 100 years. More is refused rather than wrapped round: `days` times 86,400 seconds
/// overflowed, a 500 in a debug build and in a release build a grant meant to run for centuries that ended within a day.
pub const MAX_DAYS: u64 = 36_525;

/// When a grant made at `t` (Unix seconds) for `days` days ends (None, or 0 days: it doesn't).
fn expiry(t: u64, days: Option<u64>) -> Result<Option<u64>> {
    let Some(d) = days.filter(|d| *d > 0) else { return Ok(None) };
    anyhow::ensure!(d <= MAX_DAYS, "a grant runs 1 to {MAX_DAYS} days, or with no end: not {d}");
    d.checked_mul(86_400).and_then(|s| t.checked_add(s)).map(Some).context("the grant's end is past what the server can count")
}

/// Gives `user` the area (for `days` days if given, at most [`MAX_DAYS`]). Granting one that is
/// already active renews its end instead of adding another row.
pub fn grant(user: &str, area: &str, by: &str, days: Option<u64>, note: Option<String>) -> Result<Assignment> {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut list = read()?;
    let t = now();
    let area = area.to_ascii_lowercase();
    let expires_at = expiry(t, days)?;
    if let Some(a) = list.iter_mut().find(|a| a.user == user && a.area == area && a.active_at(t)) {
        a.expires_at = expires_at;
        a.note = note.or(a.note.take());
        let out = a.clone();
        write(&list)?;
        return Ok(out);
    }
    let a = Assignment { user: user.to_string(), area, granted_by: by.to_string(), granted_at: t, expires_at, revoked_at: None, note };
    list.push(a.clone());
    write(&list)?;
    Ok(a)
}

/// Ends `user`'s active assignments of `area`; how many it ended.
pub fn revoke(user: &str, area: &str) -> Result<usize> {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut list = read()?;
    let t = now();
    let mut n = 0;
    for a in list.iter_mut().filter(|a| a.user == user && a.area.eq_ignore_ascii_case(area) && a.revoked_at.is_none()) {
        a.revoked_at = Some(t);
        n += 1;
    }
    write(&list)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(expires: Option<u64>, revoked: Option<u64>) -> Assignment {
        Assignment {
            user: "a".into(),
            area: "x".into(),
            granted_by: "j".into(),
            granted_at: 100,
            expires_at: expires,
            revoked_at: revoked,
            note: None,
        }
    }

    #[test]
    fn a_grants_days_are_bounded_and_never_wrap_round() {
        let t = 1_795_023_409;
        assert_eq!(expiry(t, None).unwrap(), None);
        assert_eq!(expiry(t, Some(0)).unwrap(), None);
        assert_eq!(expiry(t, Some(2)).unwrap(), Some(t + 2 * 86_400));
        assert_eq!(expiry(t, Some(MAX_DAYS)).unwrap(), Some(t + MAX_DAYS * 86_400));
        // The review's value: it overflowed (a 500, or in a release build an end 17 hours away).
        for d in [MAX_DAYS + 1, 213_503_982_334_602, u64::MAX] {
            assert!(expiry(t, Some(d)).is_err(), "{d}");
        }
    }

    #[test]
    fn a_wildcard_covers_every_area_and_a_name_only_its_own() {
        assert!(covers("*", "anything") && covers("bay1", "BAY1"));
        assert!(!covers("bay1", "bay2"));
    }

    #[test]
    fn an_assignment_counts_until_it_ends_or_is_revoked() {
        assert!(row(None, None).active_at(1_000_000));
        assert!(row(Some(500), None).active_at(499));
        assert!(!row(Some(500), None).active_at(500));
        assert!(!row(None, Some(200)).active_at(300));
        assert!(!row(Some(500), Some(200)).active_at(300));
    }
}
