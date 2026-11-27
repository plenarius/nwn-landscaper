// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Whose workspace a request works in.
//!
//! On your own machine (the default) there is one user, you, and the folders are the plain ones under `~/.local/share/landscaper`.
//!
//! A team server (`landscaper serve --team`) sits behind a sign-in proxy, which says who is asking in headers (`X-Forwarded-User`,
//! `X-Forwarded-Groups`, ...; `LANDSCAPER_AUTH`). Each person then has a workspace of their own,
//! `workspaces/<user>/{areas,canvases,exports,nwn}`, and people in the admin group may look at (never change) anyone's. Nothing
//! here trusts a name the browser sends: only the proxy's headers, and, if `LANDSCAPER_PROXY_SECRET` is set, only requests that
//! carry it (so a request that reaches the program without passing the proxy is refused).
//!
//! A team server answers requests on a fixed set of threads (serve.rs), one request at a time on each, so who is asking (and whose
//! workspace) belongs to the thread: [`begin_request`] clears it before each request, the request sets it once it knows who is asking,
//! and it is read only on that thread. (The heavy jobs, which keep their area in process-wide state, take turns: serve.rs `job`.)
//!
//! User names are folded to lower case ([`clean_user`]): `Alice` and `alice` are one person with one workspace. So the proxy must not
//! let two people have names that differ only in case (for example by letting people pick their own user name at sign-up): the second
//! would share the first's workspace. docs/TEAM-SERVER.md says so for whoever sets one up.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::paint;

static TEAM: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Whose workspace this thread's request uses.
    static USER: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Who is acting on it, and whether they are an admin.
    static ACTING: RefCell<Option<(String, bool)>> = const { RefCell::new(None) };
}

/// Turns team mode on (once, at start).
pub fn enable_team() {
    TEAM.store(true, Ordering::Relaxed);
}

/// Whether this is a team server: every builder in a workspace of their own, behind a sign-in proxy.
pub fn team() -> bool {
    TEAM.load(Ordering::Relaxed)
}

/// Forgets the previous request's identity on this thread: a thread of the pool answers many requests, and one that is
/// refused before it says who is asking must not run as whoever came before.
pub fn begin_request() {
    USER.with(|u| *u.borrow_mut() = None);
    ACTING.with(|a| *a.borrow_mut() = None);
}

/// Who is acting (the signed-in person, not whose workspace an admin
/// is reading) and whether they are an admin. Your own machine: an admin.
pub fn set_acting(user: &str, admin: bool) {
    ACTING.with(|a| *a.borrow_mut() = Some((user.to_string(), admin)));
}

/// Who is acting on this thread's request (`local` on your own machine): what the activity log and the assignments go by.
pub fn acting_user() -> String {
    ACTING.with(|a| a.borrow().as_ref().map_or_else(|| "local".to_string(), |a| a.0.clone()))
}

/// Whether the person acting is an admin (always, on your own machine).
pub fn acting_admin() -> bool {
    !team() || ACTING.with(|a| a.borrow().as_ref().is_some_and(|a| a.1))
}

/// The user whose workspace this request uses (None: your own machine).
pub fn set_user(user: Option<String>) {
    USER.with(|u| *u.borrow_mut() = user);
}

/// Whose workspace this thread's request uses (None: your own machine).
pub fn user() -> Option<String> {
    USER.with(|u| u.borrow().clone())
}

/// Where workspaces of everyone are (team mode).
pub fn workspaces_dir() -> PathBuf {
    paint::data_dir().join("workspaces")
}

/// The current workspace's root. Without a user in team mode, a folder that nothing is ever written to (no request gets that far).
pub fn root() -> PathBuf {
    if !team() {
        return paint::data_dir();
    }
    let user = user();
    // Every route sets the user before it touches a workspace; reaching here without one is a bug (a background thread,
    // or a route that skipped `set_user`). Release builds still land in the empty `_nobody` folder rather than stop.
    debug_assert!(user.is_some(), "a team server's workspace was used outside a request");
    workspaces_dir().join(user.unwrap_or_else(|| "_nobody".into()))
}

/// How a path on the server is shown to whoever asked. On a team server, relative to their workspace
/// (`exports/.check/cave_nwn/cave.mod`), or only its file name when it is elsewhere: an answer never tells a builder where the
/// server keeps its data and everyone's workspaces. The server's log keeps the whole path. On your own machine, the whole path.
pub fn shown(path: &Path) -> String {
    if !team() {
        return path.display().to_string();
    }
    shown_from(path, user().map(|u| workspaces_dir().join(u)).as_deref())
}

/// [`shown`] on a team server, for the workspace `root` (None: no one's).
fn shown_from(path: &Path, root: Option<&Path>) -> String {
    match root.and_then(|r| path.strip_prefix(r).ok()) {
        Some(inside) => inside.display().to_string(),
        None => path.file_name().map_or_default(|f| f.to_string_lossy().into_owned()),
    }
}

/// `text` (an error message for an answer) with the server's own paths taken out, as [`shown`] does for a path: the current
/// workspace's (its files read as `exports/cave.json`), then the data folder's, the module checkout's and the game's, which read
/// as `<data>`, `<module>` and `<game>`. On your own machine, `text` as it is.
pub fn scrub(text: &str) -> String {
    if !team() {
        return text.to_string();
    }
    // (Longest first: the workspace is inside the data folder.)
    let mut places: Vec<(String, &str)> = Vec::new();
    if let Some(u) = user() {
        let root = workspaces_dir().join(u).display().to_string();
        places.push((format!("{root}/"), ""));
        places.push((root, "your workspace"));
    }
    places.push((paint::data_dir().display().to_string(), "<data>"));
    places.push((crate::config::module_repo().display().to_string(), "<module>"));
    if let Some(game) = crate::config::game_root() {
        places.push((game.display().to_string(), "<game>"));
    }
    scrubbed(text, places)
}

/// `text` with each of `places` (a path, in the order given) replaced by what it reads as.
fn scrubbed(text: &str, places: Vec<(String, &str)>) -> String {
    // (Never "/" alone: it is in every path, and in plenty of messages.)
    places.into_iter().filter(|(p, _)| p.len() > 1).fold(text.to_string(), |t, (p, by)| t.replace(&p, by))
}

/// Exports and imports (`<name>.json`, `<name>.import/`, ...).
pub fn exports_dir() -> PathBuf {
    root().join("exports")
}

/// The editor's working copies (`<name>.json`).
pub fn areas_dir() -> PathBuf {
    root().join("areas")
}

/// The editor's canvases, a folder each (`<name>/`, see `crate::canvas`).
pub fn canvases_dir() -> PathBuf {
    root().join("canvases")
}

/// The areas deleted in the editor, each set aside in a folder of its own (`<name>-<Unix seconds>/`) with its working copy and
/// import as they were (`areas/<name>.json`, `exports/<name>.import/`), for an admin to bring back.
pub fn deleted_dir() -> PathBuf {
    root().join("deleted")
}

/// The NWN user folder exports go into (hak/ and modules/). Your own
/// machine: the game's (`--user`, else as the game finds it). A team server: this workspace's own, so one
/// person's haks and modules never mix with another's or the game's.
pub fn nwn_user_dir() -> Option<PathBuf> {
    if team() {
        let d = root().join("nwn");
        let _ = std::fs::create_dir_all(d.join("hak"));
        let _ = std::fs::create_dir_all(d.join("modules"));
        return Some(d);
    }
    crate::models::game_folders().1
}

/// A user name safe to use as a folder name: lower case letters, digits and `. _ -`, at most 64, not starting with a dot. Upper case
/// is folded to lower (`Alice` is `alice`, see the module comment); anything else is refused.
pub fn clean_user(name: &str) -> Option<String> {
    let n = name.trim().to_ascii_lowercase();
    let ok = !n.is_empty()
        && n.len() <= 64
        && !n.starts_with('.')
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    ok.then_some(n)
}

/// A name from a request that is safe as one piece of a path: 1 to 64 ASCII letters, digits, `_` or `-`. No dot, so no `..`;
/// no `/`, `\` or NUL, so it can't leave the folder it is joined to. The routes check their own stricter rules first
/// (an area name, a resref, a file name); this is the floor for anything that builds a path from a request.
pub fn plain_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Who is asking, as the proxy says.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Identity {
    /// Their sign-in name, lower case: also their workspace's folder.
    pub user: String,
    /// The name the editor greets them by.
    pub name: String,
    /// Their groups, as the proxy lists them.
    pub groups: Vec<String>,
    /// Whether they are an admin.
    pub admin: bool,
}

/// Why a request was refused: (HTTP status, what to say).
pub type Denied = (u16, &'static str);

/// The identity behind a request: `header(name)` reads a request header. Your own machine: you, an admin. A team server: from the
/// sign-in proxy's headers (config::AuthConfig), and only for people it allows.
pub fn identify(cfg: &crate::config::AuthConfig, header: impl Fn(&str) -> Option<String>) -> Result<Identity, Denied> {
    if !team() {
        return Ok(Identity { user: "local".into(), name: "You".into(), groups: vec![], admin: true });
    }
    identify_with(cfg, header)
}

/// Whether a team server's request came through the sign-in proxy: it carries the proxy secret, or none is set (`--insecure`,
/// where nothing but the proxy can reach the server). Only then are the proxy's headers (who is asking, `X-Forwarded-For`)
/// the proxy's and not the sender's own.
pub fn from_proxy(cfg: &crate::config::AuthConfig, header: impl Fn(&str) -> Option<String>) -> bool {
    cfg.proxy_secret.as_ref().is_none_or(|secret| header("X-Landscaper-Proxy").is_some_and(|sent| same_secret(&sent, secret)))
}

/// [`identify`] with the headers and groups spelled out.
pub fn identify_with(cfg: &crate::config::AuthConfig, header: impl Fn(&str) -> Option<String>) -> Result<Identity, Denied> {
    if !from_proxy(cfg, &header) {
        return Err((403, "this request did not come through the sign-in proxy"));
    }
    let Some(user) = header(&cfg.user).and_then(|u| clean_user(&u)) else {
        return Err((401, "sign in first (no user from the sign-in proxy)"));
    };
    let groups: Vec<String> =
        header(&cfg.groups).unwrap_or_default().split(cfg.separator).map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect();
    let admin = cfg.admin_groups.iter().any(|a| groups.contains(a)) || cfg.admin_users.contains(&user);
    let allowed = admin || cfg.allowed_groups.iter().any(|a| a == "*" || groups.contains(a));
    if !allowed {
        return Err((403, "you are not in a group allowed to use this (ask an admin to add you)"));
    }
    let name = header(&cfg.name).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| user.clone());
    Ok(Identity { user, name, groups, admin })
}

/// Whether `sent` is `secret`, in time that doesn't depend on where they differ (an early `!=` would let someone who can
/// time the answers find the secret one byte at a time). Only the length shows, and [`proxy_secret_problem`] keeps that
/// at 16 or more.
pub fn same_secret(sent: &str, secret: &str) -> bool {
    let (a, b) = (sent.as_bytes(), secret.as_bytes());
    let diff = a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y));
    std::hint::black_box(diff) == 0 && a.len() == b.len()
}

/// The shortest `LANDSCAPER_PROXY_SECRET` a team server takes.
pub const MIN_SECRET_LEN: usize = 16;

/// Why a team server must not start with the proxy secret `secret` (`LANDSCAPER_PROXY_SECRET`, `None` when unset or
/// empty), or None when it may. `insecure` is `--insecure` or `LANDSCAPER_INSECURE=1`: the only way to run without a
/// secret, for a server that nothing but the proxy can reach. A secret that is set must be at least [`MIN_SECRET_LEN`]
/// characters and not the example's `change-me`, with or without `insecure`.
pub fn proxy_secret_problem(secret: Option<&str>, insecure: bool) -> Option<String> {
    match secret.map(str::trim).filter(|s| !s.is_empty()) {
        None if insecure => None,
        None => Some(
            "LANDSCAPER_PROXY_SECRET is not set, so anything that reaches the server can say who it is. Set it to a long random \
             string the proxy sends in X-Landscaper-Proxy (e.g. `openssl rand -hex 24`), or, only where nothing but the proxy can \
             reach the server, start it with --insecure (or LANDSCAPER_INSECURE=1)"
                .to_string(),
        ),
        Some(s) if s.eq_ignore_ascii_case("change-me") => Some(
            "LANDSCAPER_PROXY_SECRET is still the example's `change-me`: set a long random string (e.g. `openssl rand -hex 24`)".into(),
        ),
        Some(s) if s.chars().count() < MIN_SECRET_LEN => Some(format!(
            "LANDSCAPER_PROXY_SECRET is {} characters: use at least {MIN_SECRET_LEN} (e.g. `openssl rand -hex 24`)",
            s.chars().count()
        )),
        Some(_) => None,
    }
}

/// Workspaces that exist: (user, areas saved, last change in Unix seconds).
pub fn list_users() -> Vec<(String, usize, u64)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(workspaces_dir()).into_iter().flatten().flatten() {
        let Some(user) = e.file_name().to_str().and_then(clean_user) else { continue };
        let mut areas = 0;
        let mut last = 0;
        for d in ["areas", "exports"] {
            for f in std::fs::read_dir(e.path().join(d)).into_iter().flatten().flatten() {
                let p = f.path();
                if p.extension().is_some_and(|x| x == "json") && !p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.contains('.')) {
                    if d == "areas" {
                        areas += 1;
                    }
                    let t = f
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs());
                    last = last.max(t);
                }
            }
        }
        out.push((user, areas, last));
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.2));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_builder_is_shown_paths_in_their_workspace_never_the_servers() {
        let root = Path::new("/var/lib/landscaper/workspaces/bob");
        let shown = |p: &str| shown_from(Path::new(p), Some(root));
        assert_eq!(shown("/var/lib/landscaper/workspaces/bob/exports/.check/cav_nwn/cav.mod"), "exports/.check/cav_nwn/cav.mod");
        assert_eq!(shown("/var/lib/landscaper/workspaces/carol/uploads/chess.mod"), "chess.mod");
        assert_eq!(shown_from(Path::new("/var/lib/x/y.mod"), None), "y.mod");
        let places = vec![
            ("/var/lib/landscaper/workspaces/bob/".to_string(), ""),
            ("/var/lib/landscaper/workspaces/bob".to_string(), "your workspace"),
            ("/var/lib/landscaper".to_string(), "<data>"),
            ("/".to_string(), "never"),
        ];
        let msg = "copying /var/lib/landscaper/workspaces/bob/exports/cav.mod: in /var/lib/landscaper/workspaces/bob, not \
                   /var/lib/landscaper/textures/a.tga";
        assert_eq!(
            scrubbed(msg, places),
            "copying exports/cav.mod: in your workspace, not <data>/textures/a.tga",
            "the workspace first, then the data folder; a lone / is left alone"
        );
    }

    #[test]
    fn user_names_are_folder_safe() {
        assert_eq!(clean_user("Alice"), Some("alice".into()));
        assert_eq!(clean_user("a.b_c-9"), Some("a.b_c-9".into()));
        for bad in ["", "..", ".hidden", "a/b", "a b", "../x", "é", &"x".repeat(65)] {
            assert_eq!(clean_user(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_team_server_needs_a_real_secret_or_says_it_runs_without_one() {
        let long = "0123456789abcdef";
        assert_eq!(proxy_secret_problem(Some(long), false), None);
        assert_eq!(proxy_secret_problem(Some(long), true), None);
        assert_eq!(proxy_secret_problem(None, true), None, "--insecure: no secret is allowed");
        assert!(proxy_secret_problem(None, false).is_some_and(|p| p.contains("--insecure")));
        assert!(proxy_secret_problem(Some("   "), false).is_some(), "blank is unset");
        for bad in ["change-me", "CHANGE-ME", " change-me ", "s3", &"x".repeat(MIN_SECRET_LEN - 1)] {
            assert!(proxy_secret_problem(Some(bad), false).is_some(), "{bad:?}");
            assert!(proxy_secret_problem(Some(bad), true).is_some(), "{bad:?} with --insecure");
        }
    }

    #[test]
    fn the_secret_compare_needs_every_byte_and_the_length() {
        assert!(same_secret("0123456789abcdef", "0123456789abcdef"));
        for other in ["0123456789abcdeX", "X123456789abcdef", "0123456789abcde", "0123456789abcdef0", ""] {
            assert!(!same_secret(other, "0123456789abcdef"), "{other:?}");
        }
    }

    #[test]
    fn plain_names_never_leave_their_folder() {
        for ok in ["bay1", "My_Area-2", "a", &"x".repeat(64)] {
            assert!(plain_name(ok), "{ok:?}");
        }
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "/abs", "C:", "a\0b", "a.b", "a b", "é", &"x".repeat(65)] {
            assert!(!plain_name(bad), "{bad:?}");
        }
    }
}

#[cfg(test)]
mod identify_tests {
    use super::*;
    use crate::config::AuthConfig;

    fn cfg(pairs: &[(&str, &str)]) -> AuthConfig {
        AuthConfig::from(&|k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()))
    }

    fn headers<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.to_string())
    }

    #[test]
    fn authentiks_headers_and_separator_work() {
        let me = identify_with(
            &cfg(&[("LANDSCAPER_AUTH", "authentik"), ("LANDSCAPER_ALLOWED_GROUPS", "Builders")]),
            headers(&[("X-authentik-username", "Dorg"), ("X-authentik-groups", "Builders|Other"), ("X-authentik-name", "Dorg A")]),
        )
        .unwrap();
        assert_eq!((me.user.as_str(), me.name.as_str(), me.admin, me.groups.len()), ("dorg", "Dorg A", false, 2));
    }

    #[test]
    fn another_proxies_headers_and_separator_work() {
        let c = cfg(&[("LANDSCAPER_AUTH", "authelia"), ("LANDSCAPER_ADMIN_GROUPS", "Administrator")]);
        let me =
            identify_with(&c, headers(&[("Remote-User", "mia"), ("Remote-Groups", "Administrator,dev"), ("Remote-Name", "Mia")])).unwrap();
        assert_eq!((me.user.as_str(), me.admin), ("mia", true));
        // the other proxy's headers mean nothing here
        assert_eq!(identify_with(&c, headers(&[("X-authentik-username", "mia")])).unwrap_err().0, 401);
    }

    #[test]
    fn with_no_allowed_groups_nobody_but_an_admin_gets_in() {
        let c = cfg(&[("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ADMIN_USERS", "alice")]);
        assert_eq!(identify_with(&c, headers(&[("X-Forwarded-User", "bob"), ("X-Forwarded-Groups", "Builders")])).unwrap_err().0, 403);
        assert!(identify_with(&c, headers(&[("X-Forwarded-User", "alice")])).unwrap().admin);
    }

    #[test]
    fn a_team_without_groups_lets_everyone_in_and_names_its_admins() {
        let c = cfg(&[("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_ADMIN_USERS", "alice")]);
        let alice = identify_with(&c, headers(&[("X-Forwarded-User", "alice")])).unwrap();
        let bob = identify_with(&c, headers(&[("X-Forwarded-User", "bob")])).unwrap();
        assert_eq!((alice.admin, bob.admin), (true, false));
    }

    #[test]
    fn admin_groups_none_or_empty_means_a_groups_header_makes_nobody_an_admin() {
        let forged = [("X-Forwarded-User", "bob"), ("X-Forwarded-Groups", "Administrator")];
        // An admin group: a proxy that passes the header through makes bob an admin (why the proxy must strip it).
        let c = cfg(&[("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_ADMIN_GROUPS", "Administrator")]);
        assert!(identify_with(&c, headers(&forged)).unwrap().admin);
        let unset = cfg(&[("LANDSCAPER_AUTH", "headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_ADMIN_USERS", "alice")]);
        assert!(!identify_with(&unset, headers(&forged)).unwrap().admin, "unset is no admin group");
        for off in ["none", "NONE", "", "  "] {
            let c = cfg(&[
                ("LANDSCAPER_AUTH", "headers"),
                ("LANDSCAPER_ALLOWED_GROUPS", "*"),
                ("LANDSCAPER_ADMIN_GROUPS", off),
                ("LANDSCAPER_ADMIN_USERS", "alice"),
            ]);
            assert!(c.admin_groups.is_empty(), "{off:?}");
            assert!(!identify_with(&c, headers(&forged)).unwrap().admin, "{off:?}");
            assert!(identify_with(&c, headers(&[("X-Forwarded-User", "alice")])).unwrap().admin, "{off:?}");
        }
        let c = cfg(&[("LANDSCAPER_ADMIN_GROUPS", " Leads , Admins ")]);
        assert_eq!(c.admin_groups, ["Leads", "Admins"]);
    }

    #[test]
    fn someone_outside_the_allowed_groups_is_refused_and_the_proxy_secret_is_required() {
        let c = cfg(&[("LANDSCAPER_AUTH", "authentik"), ("LANDSCAPER_ALLOWED_GROUPS", "Builders"), ("LANDSCAPER_PROXY_SECRET", "s3")]);
        let good = [("X-authentik-username", "x"), ("X-authentik-groups", "Strangers"), ("X-Landscaper-Proxy", "s3")];
        assert_eq!(identify_with(&c, headers(&good)).unwrap_err().0, 403);
        assert_eq!(identify_with(&c, headers(&[("X-authentik-username", "x"), ("X-authentik-groups", "Builders")])).unwrap_err().0, 403);
        assert!(
            identify_with(&c, headers(&[("X-authentik-username", "x"), ("X-authentik-groups", "Builders"), ("X-Landscaper-Proxy", "s3")]))
                .is_ok()
        );
    }

    #[test]
    fn a_secret_with_a_trailing_space_or_crlf_still_matches_what_the_proxy_sends() {
        for raw in ["0123456789abcdef0123 ", "0123456789abcdef0123\r", "\t0123456789abcdef0123\r\n"] {
            let c = cfg(&[("LANDSCAPER_ALLOWED_GROUPS", "*"), ("LANDSCAPER_PROXY_SECRET", raw)]);
            assert_eq!(c.proxy_secret.as_deref(), Some("0123456789abcdef0123"), "{raw:?}");
            assert!(proxy_secret_problem(c.proxy_secret.as_deref(), false).is_none());
            let sent = [("X-Forwarded-User", "bob"), ("X-Landscaper-Proxy", "0123456789abcdef0123")];
            assert!(identify_with(&c, headers(&sent)).is_ok(), "{raw:?}");
            assert!(from_proxy(&c, headers(&sent)));
            assert!(!from_proxy(&c, headers(&[("X-Forwarded-User", "bob")])));
        }
    }
}
