// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The module's areas, from the review service instead of a copy on this machine.
//!
//! A team server holds no copy of the module. When `LANDSCAPER_REVIEW_URL` and `LANDSCAPER_REVIEW_KEY` are set it lists areas
//! (`/api/v1/areas/index`: names, tags, sizes, no layouts) and fetches one area's ARE and GIT when a builder with an assignment
//! imports it (`/api/v1/areas/<resref>?user=`). The service logs and limits those fetches. docs/REVIEW-SERVICE.md describes every
//! call this file and publish.rs make.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;

/// The review service's address and key, if this server is set up to use it.
pub fn config() -> Option<(String, String)> {
    crate::config::review_service()
}

/// Whether areas come from the review service (a team server that has it set up).
pub fn active() -> bool {
    crate::workspace::team() && config().is_some()
}

/// How long the area list, the references and the hak list are kept.
const KEEP: Duration = Duration::from_secs(600);
/// How long a failed fetch of one is kept before the next ask tries again: a service that is down costs one call every 30
/// seconds, not one per request.
const KEEP_FAILURE: Duration = Duration::from_secs(30);

/// The timeout of the calls whose answers are kept: every save of a new area and every import waits for the area list.
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// The timeout of the other reads (an area, a blueprint, a script).
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// A GET of the service's JSON; `None` when it answers 404 (no such area, blueprint or script).
fn get(path: &str, timeout: Duration) -> Result<Option<Value>> {
    let (url, key) = config().context("the review service isn't set up (LANDSCAPER_REVIEW_URL, LANDSCAPER_REVIEW_KEY)")?;
    let r = ureq::get(&format!("{url}{path}")).set("Authorization", &format!("Bearer {key}")).timeout(timeout).call();
    match r {
        Ok(r) => Ok(Some(r.into_json()?)),
        Err(ureq::Error::Status(429, _)) => anyhow::bail!("too many area downloads today: try again tomorrow or ask an admin"),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => anyhow::bail!("the module service didn't answer: {}", crate::remote::why(&e)),
    }
}

/// A GET whose answer must be there: 404 is an error too.
fn get_some(path: &str, timeout: Duration) -> Result<Value> {
    get(path, timeout)?.with_context(|| format!("the module service has no {}", path.trim_start_matches("/api/v1/")))
}

/// One of the service's answers, kept for [`KEEP`]. The lock is never held across the call: one asker fetches, the others
/// take the answer they had (or, with none yet, wait for that one call), so a slow service costs every asker one call's
/// time at most, not one call each in turn.
struct Kept<T> {
    state: Mutex<KeptState<T>>,
    fetched: Condvar,
    keep: Duration,
    keep_failure: Duration,
}

struct KeptState<T> {
    /// The answer and when it was fetched; `None` for one read from disk ([`Kept::seed`]), which is never fresh.
    value: Option<(Option<Instant>, T)>,
    failed: Option<(Instant, String)>,
    fetching: bool,
}

impl<T: Clone> Kept<T> {
    const fn new() -> Self {
        Kept::keeping(KEEP, KEEP_FAILURE)
    }

    const fn keeping(keep: Duration, keep_failure: Duration) -> Self {
        Kept { state: Mutex::new(KeptState { value: None, failed: None, fetching: false }), fetched: Condvar::new(), keep, keep_failure }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, KeptState<T>> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Starts from `v` (an answer kept from before a restart) when nothing has been fetched yet. It is never fresh: the next
    /// ask fetches, and `v` is what a failed fetch falls back on.
    fn seed(&self, v: T) {
        let mut g = self.lock();
        if g.value.is_none() {
            g.value = Some((None, v));
        }
    }

    /// The kept answer while it is fresh; otherwise `fetch`'s (one asker at a time). When the fetch fails, the last answer
    /// there was, however old; with none, the error. A failure is kept [`KEEP_FAILURE`] before the next fetch.
    fn get(&self, fetch: impl FnOnce() -> Result<T>) -> Result<T> {
        let mut g = self.lock();
        loop {
            if let Some((Some(t), v)) = &g.value
                && t.elapsed() < self.keep
            {
                return Ok(v.clone());
            }
            let recent_failure = g.failed.as_ref().filter(|(t, _)| t.elapsed() < self.keep_failure).map(|(_, e)| e.clone());
            if let Some(e) = recent_failure {
                return g.value.as_ref().map(|(_, v)| v.clone()).ok_or_else(|| anyhow::anyhow!("{e}"));
            }
            if !g.fetching {
                break;
            }
            // Someone is fetching: an old answer will do; with none, wait for theirs.
            if let Some((_, v)) = &g.value {
                return Ok(v.clone());
            }
            g = self.fetched.wait(g).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        g.fetching = true;
        drop(g);
        // (A panic in `fetch` must not leave `fetching` set, or every later asker would wait for ever.)
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(fetch))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the module service's answer couldn't be read")));
        let mut g = self.lock();
        g.fetching = false;
        let out = match got {
            Ok(v) => {
                g.value = Some((Some(Instant::now()), v.clone()));
                g.failed = None;
                Ok(v)
            }
            Err(e) => {
                let e = format!("{e:#}");
                g.failed = Some((Instant::now(), e.clone()));
                g.value.as_ref().map(|(_, v)| v.clone()).ok_or_else(|| anyhow::anyhow!("{e}"))
            }
        };
        drop(g);
        self.fetched.notify_all();
        out
    }
}

/// The module's area list, as [`sources`] gives it.
static AREAS: Kept<Vec<Value>> = Kept::new();

/// Where the last area list the service sent is kept, so a restart while the service is down doesn't stop builders making,
/// importing and exporting areas: `review-areas.json` in the server's data folder, beside `assignments.json`.
fn areas_file() -> std::path::PathBuf {
    crate::paint::data_dir().join("review-areas.json")
}

/// Reads the area list kept by the last run ([`areas_file`]), for [`sources`] to fall back on until the service answers. A
/// team server calls it at start. A missing file is no list; one that doesn't read is only a warning, since the service's
/// next answer replaces it.
pub fn read_kept_areas() {
    let file = areas_file();
    match std::fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice::<Vec<Value>>(&bytes) {
            Ok(list) => AREAS.seed(list),
            Err(e) => eprintln!("{}: not read ({e}); waiting for the review service's area list", file.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("{}: not read ({e}); waiting for the review service's area list", file.display()),
    }
}

/// The areas, as `import::sources` entries (`"source": "ops:<resref>"`). Kept ten minutes, and on disk ([`areas_file`]);
/// while the service doesn't answer, the last list there was, from before a restart too. An error when there has never been
/// one: a new area's name can't be checked without it.
pub fn sources() -> Result<Vec<Value>> {
    AREAS
        .get(|| {
            let v = get_some("/api/v1/areas/index", LIST_TIMEOUT).inspect_err(|e| eprintln!("couldn't list the module's areas: {e:#}"))?;
            let list: Vec<Value> = v["areas"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| {
                    let mut e = a.clone();
                    e["source"] = format!("ops:{}", a["area"].as_str().unwrap_or_default()).into();
                    e["from"] = "module repository".into();
                    e
                })
                .collect();
            // (A list that can't be written is still used: only a restart while the service is down would miss it.)
            let file = areas_file();
            let bytes = serde_json::to_vec(&list)?;
            if std::fs::read(&file).ok().as_deref() != Some(bytes.as_slice())
                && let Err(e) = crate::atomic::write(&file, &bytes)
            {
                eprintln!("couldn't keep the module's area list: {e:#}");
            }
            Ok(list)
        })
        .context("the module's area list isn't available")
}

/// One area as the review service gives it: its ARE and GIT as GFF JSON text, the module's haks and talk table.
pub struct Fetched {
    /// The area's ARE, as the module keeps it (GFF JSON).
    pub are: String,
    /// Its GIT, as the module keeps it (None: it has none).
    pub git: Option<String>,
    /// The module's haks, in order.
    pub haks: Vec<String>,
    /// The module's custom talk table.
    pub tlk: Option<String>,
}

/// Downloads `resref` for `user` (who has been checked against the assignments).
pub fn fetch(user: &str, resref: &str) -> Result<Fetched> {
    anyhow::ensure!(resref.len() <= 16 && resref.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "not an area resref");
    let v = get(&format!("/api/v1/areas/{}?user={}", resref.to_ascii_lowercase(), user), READ_TIMEOUT)?
        .with_context(|| format!("the module has no area {resref}"))?;
    Ok(Fetched {
        are: v["are"].as_str().context("the module service sent no area")?.to_string(),
        git: v["git"].as_str().map(str::to_string),
        haks: v["haks"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect(),
        tlk: v["tlk"].as_str().filter(|t| !t.is_empty()).map(str::to_string),
    })
}

/// A blueprint of the skeleton module as GFF JSON text (`ext`: utc, uti...), or None if it has none.
pub fn blueprint(ext: &str, resref: &str) -> Result<Option<String>> {
    anyhow::ensure!(
        ext.chars().all(|c| c.is_ascii_lowercase()) && resref.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "bad blueprint name"
    );
    Ok(get(&format!("/api/v1/blueprints/{ext}/{}", resref.to_ascii_lowercase()), READ_TIMEOUT)?
        .and_then(|v| v["json"].as_str().map(str::to_string)))
}

/// The skeleton's blueprints of a kind matching `q` (name or resref): `[{resref, name}]`.
pub fn blueprint_list(ext: &str, q: &str, limit: usize) -> Result<Vec<Value>> {
    anyhow::ensure!(ext.chars().all(|c| c.is_ascii_lowercase()), "bad blueprint kind");
    let q: String = q.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '_' || *c == '-').take(40).collect();
    let v = get_some(&format!("/api/v1/blueprints/{ext}?q={}&limit={limit}", q.replace(' ', "%20")), READ_TIMEOUT)?;
    Ok(v["blueprints"].as_array().cloned().unwrap_or_default())
}

/// A script of the skeleton module (its source), or None if it has none.
pub fn script(resref: &str) -> Result<Option<String>> {
    anyhow::ensure!(resref.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !resref.is_empty(), "bad script name");
    Ok(get(&format!("/api/v1/scripts/{}", resref.to_ascii_lowercase()), READ_TIMEOUT)?
        .and_then(|v| v["source"].as_str().map(str::to_string)))
}

/// The skeleton's scripts whose name has `q`: `[{resref, folder}]`.
pub fn script_list(q: &str, limit: usize) -> Result<Vec<Value>> {
    let q: String = q.chars().filter(|c| c.is_alphanumeric() || *c == '_').take(40).collect();
    let v = get_some(&format!("/api/v1/scripts?q={q}&limit={limit}"), READ_TIMEOUT)?;
    Ok(v["scripts"].as_array().cloned().unwrap_or_default())
}

/// A call to the review service whose error text the caller shows (a `detail` the service sent: someone holds the dev
/// server, a job is running, a name it refused).
pub fn call(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let (url, key) = config().context("the review service isn't set up (LANDSCAPER_REVIEW_URL, LANDSCAPER_REVIEW_KEY)")?;
    let req =
        ureq::request(method, &format!("{url}{path}")).set("Authorization", &format!("Bearer {key}")).timeout(Duration::from_secs(60));
    let r = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match r {
        Ok(r) => Ok(r.into_json()?),
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_json::<Value>().ok().map(|v| match &v["detail"] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            });
            anyhow::bail!("{}", detail.filter(|d| !d.is_empty()).unwrap_or_else(|| format!("the service said {code}")))
        }
        Err(e) => anyhow::bail!("the service didn't answer: {}", crate::remote::why(&e)),
    }
}

/// What the module has that an area can refer to (scripts, dialogs, waypoint and door tags): kept ten minutes. None on a
/// machine of your own, and when the review service doesn't answer and never did (the checks that need it are skipped).
pub fn refs() -> Option<crate::validate::Refs> {
    static KEPT: Kept<crate::validate::Refs> = Kept::new();
    config()?;
    KEPT.get(|| Ok(crate::validate::Refs::from_json(&get_some("/api/v1/areas/refs", LIST_TIMEOUT)?))).ok()
}

/// The most bytes a file from the review service may hold ([`fetch_bytes`]): a creature's scene and textures are a few MB.
const FILE_MAX: u64 = 64 << 20;

/// A file from the review service as bytes (a creature preview's scene and textures, an item's icon), which must be of the
/// content type `want` (`image/png`, say): None when the service has no such file. The editor's origin serves it under
/// `want`, never under a type the service chose (a page sent as `text/html` would open as the editor's own).
pub fn fetch_bytes(path: &str, want: &str) -> Result<Option<Vec<u8>>> {
    let (url, key) = config().context("the review service isn't set up (LANDSCAPER_REVIEW_URL, LANDSCAPER_REVIEW_KEY)")?;
    let r = ureq::get(&format!("{url}{path}")).set("Authorization", &format!("Bearer {key}")).timeout(Duration::from_secs(60)).call();
    match r {
        Ok(r) => {
            let ctype = r.content_type().to_string();
            file_of(&ctype, want, r.into_reader(), FILE_MAX).map(Some)
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => anyhow::bail!("the service didn't answer: {}", crate::remote::why(&e)),
    }
}

/// `body`, sent as `ctype`, if that is `want` (its parameters aside) and it holds at most `max` bytes. A longer file is an
/// error, not cut short: the editor's browser keeps what it gets for a year.
fn file_of(ctype: &str, want: &str, body: impl std::io::Read, max: u64) -> Result<Vec<u8>> {
    let essence = ctype.split(';').next().unwrap_or_default().trim();
    anyhow::ensure!(essence.eq_ignore_ascii_case(want), "the review service sent {essence:?} where {want} was expected");
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(body, max + 1), &mut out).context("reading the review service's file")?;
    anyhow::ensure!(out.len() as u64 <= max, "the review service's file is over {} MB", max >> 20);
    Ok(out)
}

/// The module's hak list, in the module's own order: what its areas' tilesets, models and textures are found in. Kept ten
/// minutes. Empty when the review service isn't there, or doesn't answer and never did.
pub fn module_haks() -> Vec<String> {
    static KEPT: Kept<Vec<String>> = Kept::new();
    if config().is_none() {
        return Vec::new();
    }
    KEPT.get(|| {
        let v = get_some("/api/v1/areas/module", LIST_TIMEOUT)?;
        Ok(v["haks"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect())
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn a_failed_fetch_gives_the_last_answer_and_is_kept_before_the_next_try() {
        // Nothing is fresh (kept no time), and a failure is kept an hour.
        let kept = Kept::<u32>::keeping(Duration::ZERO, Duration::from_secs(3600));
        assert_eq!(kept.get(|| Ok(7)).unwrap(), 7);
        let calls = AtomicUsize::new(0);
        let failing = || {
            calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("down")
        };
        assert_eq!(kept.get(failing).unwrap(), 7, "the last answer");
        assert_eq!(kept.get(failing).unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the failure was kept");
        // With no answer ever, the error.
        let none = Kept::<u32>::keeping(Duration::ZERO, Duration::from_secs(3600));
        assert!(none.get(|| anyhow::bail!("down")).is_err());
        assert!(none.get(|| Ok(1)).is_err(), "kept: not tried again yet");
    }

    #[test]
    fn askers_at_once_share_one_fetch() {
        let kept = Kept::<u32>::keeping(Duration::from_secs(3600), Duration::ZERO);
        let calls = AtomicUsize::new(0);
        let started = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    let v = kept.get(|| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(300));
                        Ok(5)
                    });
                    assert_eq!(v.unwrap(), 5);
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() < Duration::from_millis(1500), "{:?}: in turn, not at once", started.elapsed());
    }

    #[test]
    fn a_file_from_the_service_must_be_the_type_asked_for_and_whole() {
        assert_eq!(file_of("image/png", "image/png", &b"png"[..], 10).unwrap(), b"png");
        assert_eq!(file_of("Application/JSON; charset=utf-8", "application/json", &b"{}"[..], 10).unwrap(), b"{}");
        // A page where a picture was expected, and a type no header can hold: refused, never passed on or a panic.
        for ctype in ["text/html", "image/svg+xml", "", "image/png\r\nX-Evil: 1"] {
            let e = file_of(ctype, "image/png", &b"<script>"[..], 10).unwrap_err();
            assert!(e.to_string().contains("where image/png was expected"), "{ctype:?}: {e}");
        }
        // Over the limit: an error, not the first `max` bytes kept for a year.
        assert!(file_of("image/png", "image/png", &[0u8; 11][..], 10).unwrap_err().to_string().contains("over"));
        assert_eq!(file_of("image/png", "image/png", &[0u8; 10][..], 10).unwrap().len(), 10);
    }
}
