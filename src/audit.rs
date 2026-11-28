// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A record of who did what with the module's areas, and limits on how fast a builder may search them.
//!
//! One line of JSON per event, appended to `audit/<year>-<month>.jsonl` (UTC) beside the workspaces: when, who (the sign-in name,
//! whether an admin), from which address, what, which area, whether it was allowed, and a few words of detail. Admins read it back
//! (`GET /api/audit`). Some events also go to Discord (`LANDSCAPER_DISCORD_WEBHOOK`): a builder importing an existing area, being
//! refused one, or hitting the search limit. Writing the log or sending a notice never fails the request.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::paint;

thread_local! {
    /// The address of the request this thread is answering.
    static IP: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Notes the address of the request this thread answers, for the log lines it writes.
pub fn set_address(ip: String) {
    IP.with(|i| *i.borrow_mut() = ip);
}

pub fn address() -> String {
    IP.with(|i| i.borrow().clone())
}

/// The client's address: the proxy appends it as the last entry of `X-Forwarded-For`; without a proxy, the connection's own.
pub fn client_address(forwarded_for: Option<&str>, remote: &str) -> String {
    forwarded_for.and_then(|f| f.split(',').next_back()).map(str::trim).filter(|a| !a.is_empty()).unwrap_or(remote).to_string()
}

fn dir() -> PathBuf {
    paint::data_dir().join("audit")
}

/// (year, month, day, hour, minute, second) in UTC for Unix seconds.
fn civil(t: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (t / 86_400) as i64;
    let rem = t % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

fn stamp(t: u64) -> String {
    let (y, mo, d, h, mi, s) = civil(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn file_for(t: u64) -> PathBuf {
    let (y, mo, ..) = civil(t);
    month_file(y, mo)
}

fn month_file(y: i64, mo: u32) -> PathBuf {
    dir().join(format!("{y:04}-{mo:02}.jsonl"))
}

/// The calendar month of `t` (UTC) and the one before it, newest first: the log files [`read`] looks in.
fn this_month_and_last(t: u64) -> [(i64, u32); 2] {
    let (y, mo, ..) = civil(t);
    let last = if mo == 1 { (y - 1, 12) } else { (y, mo - 1) };
    [(y, mo), last]
}

static WRITE: Mutex<()> = Mutex::new(());

/// Records an event by the person acting now: `action`, the area it is about (if any), whether it was allowed, and some detail.
pub fn log(action: &str, area: Option<&str>, ok: bool, detail: &str) {
    if !crate::workspace::team() {
        return;
    }
    let t = crate::assignments::now();
    let line = json!({
        "t": t, "at": stamp(t), "user": crate::workspace::acting_user(), "admin": crate::workspace::acting_admin(),
        "ip": address(), "action": action, "area": area, "ok": ok, "detail": detail,
    });
    write_line(t, &line);
}

/// An event from someone who couldn't be identified or isn't allowed in.
pub fn log_refused(user: Option<&str>, code: u16, why: &str) {
    if !crate::workspace::team() {
        return;
    }
    // One a minute from an address is enough to know it happened.
    static LAST: Mutex<Option<Throttle>> = Mutex::new(None);
    if !LAST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(Throttle::default)
        .allow(&address(), Instant::now())
    {
        return;
    }
    let t = crate::assignments::now();
    write_line(
        t,
        &json!({
            "t": t, "at": stamp(t), "user": user, "admin": false, "ip": address(), "action": "refused", "area": null, "ok": false,
            "detail": format!("{code} {why}"),
        }),
    );
}

/// At most one line a minute per address, for at most [`Throttle::MAX`] addresses a minute: the addresses seen in the last
/// minute are all it keeps, so many senders can't grow it (or the log) without limit.
#[derive(Default)]
struct Throttle {
    last: HashMap<String, Instant>,
}

impl Throttle {
    const MAX: usize = 1024;
    const EVERY: Duration = Duration::from_secs(60);

    /// Whether a refusal from `address` at `now` is written.
    fn allow(&mut self, address: &str, now: Instant) -> bool {
        if self.last.get(address).is_some_and(|t| now.duration_since(*t) < Self::EVERY) {
            return false;
        }
        if self.last.len() >= Self::MAX {
            self.last.retain(|_, t| now.duration_since(*t) < Self::EVERY);
            if self.last.len() >= Self::MAX {
                return false;
            }
        }
        self.last.insert(address.to_string(), now);
        true
    }
}

fn write_line(t: u64, line: &Value) {
    let _g = WRITE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = std::fs::create_dir_all(dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file_for(t)) {
        let _ = writeln!(f, "{line}");
    }
}

/// The newest events first (this calendar month and the one before), at most `limit`, those matching `user` and `action` if
/// given.
pub fn read(limit: usize, user: Option<&str>, action: Option<&str>) -> Vec<Value> {
    let now = crate::assignments::now();
    let mut out = Vec::new();
    for f in this_month_and_last(now).map(|(y, mo)| month_file(y, mo)) {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for line in text.lines().rev() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            if user.is_some_and(|u| v["user"].as_str() != Some(u)) || action.is_some_and(|a| v["action"].as_str() != Some(a)) {
                continue;
            }
            out.push(v);
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

// --- Rate limits ---------------------------------------------------------------------

static SEARCHES: Mutex<Option<HashMap<String, VecDeque<Instant>>>> = Mutex::new(None);

fn limit(var: &str, default: usize) -> usize {
    crate::config::setting(var).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Whether `user` may search now: at most `LANDSCAPER_SEARCH_PER_MIN` (30) in a minute and `LANDSCAPER_SEARCH_PER_DAY` (300) in a day.
/// Counts this one when it is allowed. (In memory: a restart starts the count again.)
pub fn search_allowed(user: &str) -> bool {
    let (per_min, per_day) = (limit("LANDSCAPER_SEARCH_PER_MIN", 30), limit("LANDSCAPER_SEARCH_PER_DAY", 300));
    let now = Instant::now();
    let mut g = SEARCHES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let all = g.get_or_insert_with(HashMap::new);
    // (Everyone who hasn't searched for a day goes, so the map holds only the last day's searchers.)
    let day = Duration::from_secs(86_400);
    all.retain(|u, q| u == user || q.back().is_some_and(|t| now.duration_since(*t) <= day));
    let q = all.entry(user.to_string()).or_default();
    while q.front().is_some_and(|t| now.duration_since(*t) > day) {
        q.pop_front();
    }
    let last_minute = q.iter().rev().take_while(|t| now.duration_since(**t) <= Duration::from_secs(60)).count();
    if q.len() >= per_day || last_minute >= per_min {
        return false;
    }
    q.push_back(now);
    true
}

// --- Discord -------------------------------------------------------------------------

/// The most notices waiting to be sent; more are dropped (the activity log has them all).
const QUEUED: usize = 64;

/// Posts `text` to the Discord webhook, if one is set. Never blocks or fails the request: one thread sends the notices in turn,
/// and a notice that finds [`QUEUED`] already waiting is dropped.
fn notify(text: String) {
    use std::sync::mpsc::{SyncSender, sync_channel};
    static QUEUE: std::sync::OnceLock<Option<SyncSender<String>>> = std::sync::OnceLock::new();
    if !crate::workspace::team() {
        return;
    }
    let queue = QUEUE.get_or_init(|| {
        let url = crate::config::setting("LANDSCAPER_DISCORD_WEBHOOK").filter(|u| !u.trim().is_empty())?;
        let (send, notices) = sync_channel::<String>(QUEUED);
        std::thread::spawn(move || {
            for text in notices {
                let body = json!({ "content": text.chars().take(1900).collect::<String>(), "allowed_mentions": { "parse": [] } });
                if let Err(e) = ureq::post(&url).timeout(Duration::from_secs(10)).send_json(body) {
                    // (Not `{e}`: its text is the URL, and a webhook's URL is its token.)
                    eprintln!("couldn't post to Discord: {}", crate::remote::why(&e));
                }
            }
        });
        Some(send)
    });
    if let Some(q) = queue {
        let _ = q.try_send(text);
    }
}

/// Sends a notice (see [`notify`]) unless one with the same `key` went in the last ten minutes: a person importing, being
/// refused or hitting the search limit again and again is one message. Keys are a kind and a person (`import:<user>`), never
/// what the request named, which its sender can vary at will.
pub fn notify_throttled(key: &str, text: String) {
    static SENT: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
    if !Throttled::allow(&mut SENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner), key, Instant::now()) {
        return;
    }
    notify(text);
}

/// The notices sent in the last ten minutes, by key.
struct Throttled;

impl Throttled {
    const EVERY: Duration = Duration::from_secs(600);

    /// Whether a notice `key` may go at `now` (and notes it if so). Keys older than ten minutes are dropped as it goes, so the
    /// map holds only the last ten minutes' keys.
    fn allow(sent: &mut Option<HashMap<String, Instant>>, key: &str, now: Instant) -> bool {
        let m = sent.get_or_insert_with(HashMap::new);
        if m.get(key).is_some_and(|t| now.duration_since(*t) < Self::EVERY) {
            return false;
        }
        m.retain(|_, t| now.duration_since(*t) < Self::EVERY);
        m.insert(key.to_string(), now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_log_writes_once_a_minute_an_address_and_keeps_only_the_last_minutes_addresses() {
        let mut t = Throttle::default();
        let start = Instant::now();
        assert!(t.allow("10.0.0.1", start));
        assert!(!t.allow("10.0.0.1", start + Duration::from_secs(59)));
        assert!(t.allow("10.0.0.1", start + Duration::from_secs(61)));
        // Many addresses: at most MAX a minute are written and kept, then the old ones make room.
        let full = (0..Throttle::MAX * 3).filter(|k| t.allow(&format!("a{k}"), start + Duration::from_secs(61))).count();
        assert_eq!(full, Throttle::MAX - 1, "10.0.0.1 already holds one place");
        assert!(t.last.len() <= Throttle::MAX);
        assert!(t.allow("late", start + Duration::from_secs(200)), "a minute later the old addresses are gone");
        assert_eq!(t.last.len(), 1);
    }

    #[test]
    fn dates_are_utc_civil_dates() {
        assert_eq!(stamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(stamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(stamp(1_792_906_684), "2026-10-25T05:38:04Z");
    }

    #[test]
    fn the_log_is_read_from_this_calendar_month_and_the_one_before() {
        // 2027-03-01T12:00:00Z: 31 days back is January, which skipped February's log.
        assert_eq!(this_month_and_last(1_803_902_400), [(2027, 3), (2027, 2)]);
        // 2027-01-15T00:00:00Z: last month is the year before's December.
        assert_eq!(this_month_and_last(1_799_971_200), [(2027, 1), (2026, 12)]);
        // 2026-10-31T00:00:00Z: a 31-day month and a 30-day one before it.
        assert_eq!(this_month_and_last(1_793_404_800), [(2026, 10), (2026, 9)]);
    }

    #[test]
    fn the_client_is_the_last_forwarded_address() {
        assert_eq!(client_address(Some("198.51.100.4, 10.0.0.2"), "172.18.0.2"), "10.0.0.2");
        assert_eq!(client_address(Some("203.0.113.9"), "172.18.0.2"), "203.0.113.9");
        assert_eq!(client_address(None, "172.18.0.2"), "172.18.0.2");
        assert_eq!(client_address(Some(" "), "172.18.0.2"), "172.18.0.2");
    }

    #[test]
    fn a_notice_goes_once_in_ten_minutes_a_key_and_old_keys_are_dropped() {
        let (mut sent, start) = (None, Instant::now());
        assert!(Throttled::allow(&mut sent, "denied:bob", start));
        assert!(!Throttled::allow(&mut sent, "denied:bob", start + Duration::from_secs(599)));
        assert!(Throttled::allow(&mut sent, "import:bob", start + Duration::from_secs(1)));
        assert!(Throttled::allow(&mut sent, "denied:bob", start + Duration::from_secs(601)));
        assert!(Throttled::allow(&mut sent, "limit:carol", start + Duration::from_secs(1300)));
        assert_eq!(sent.as_ref().map(HashMap::len), Some(1), "only the last ten minutes' keys are kept");
    }
}
