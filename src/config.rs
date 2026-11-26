// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where things are and what a project calls them: environment variables with defaults that work on a fresh machine, so another
//! team can run the Landscaper without touching the code. (Per-feature settings, such as Gitea's, are read where they are used;
//! docs/CONFIGURATION.md lists every setting.)
//!
//! - `LANDSCAPER_MODULE_REPO`: a checkout of the module's source as nwn_gff JSON, laid out as `LANDSCAPER_LAYOUT` says (by
//!   default nasher's: everything in `src/`): where blueprints, scripts, conversations and the areas that can be imported come
//!   from. Default: `./nwn-module`.
//! - `LANDSCAPER_WEB` (or `--web`): the editor's files. Default: `web/` beside the program, `./nwn-landscaper-web`,
//!   `./landscaper-web`, `../nwn-landscaper-web`, `../landscaper-web`; with none of those, the server stops at start and
//!   names the setting.
//! - `LANDSCAPER_LAYOUT` (and the `LANDSCAPER_*_DIR` settings): where those files are in the checkout; see [`Layout`].
//! - `LANDSCAPER_AREA_TEMPLATE`: a JSON file of what a new area starts with (scripts, PvP, weather, music...). Default: a plain one.
//! - `LANDSCAPER_TLK`: the module's custom talk table (a file name in the game's `tlk/` or `hak/` folder of the user folder, or a
//!   path). Default: the only `.tlk` there, if there is exactly one.
//! - `LANDSCAPER_AUTH` (and the `LANDSCAPER_*_HEADER` settings): which headers say who is signed in on a team server, which
//!   must set it; see [`AuthConfig`].
//! - `LANDSCAPER_PROJECT`: what the module is called in messages (default "the module").
//! - `LANDSCAPER_DEV_MODULE`, `LANDSCAPER_DEV_JOIN`: for a team with a dev server behind its review service: the file `dev-pack`
//!   writes (default `Landscaper_dev`, so `Landscaper_dev.mod`) and how the editor tells a builder to join (default: nothing).
//! - `LANDSCAPER_JOBS` (or `--jobs N`): how many threads the heavy jobs work on (an import, an export, a check). Default: every
//!   core for an export, half the machine's cores (at least 1) for an import or a check; see [`threads`], [`export_threads`].
//! - `LANDSCAPER_REVIEW_URL`, `LANDSCAPER_REVIEW_KEY`: the review service a team server gets the module from and publishes
//!   through (docs/REVIEW-SERVICE.md); see [`review_service`].
//! - `LANDSCAPER_HAKS` (or `--haks`): the haks the editor's placeables, models, blueprints and textures come from, in the
//!   module's order, or `module` for the module checkout's own list; see [`haks`]. Default: on a team server the module's hak
//!   list from the review service, else none (the game's own).
//! - `LANDSCAPER_PARTICLES`: another particle pack's folder for an area's ambience. Default: the one built into the program
//!   (`packs/particles`); see [`crate::ambience::installed`].
//!
//! The command line's `--root`, `--user`, `--tlk`, `--haks` and `--jobs` win over their variables: [`set_given`].
//!
//! Every setting can also be in the settings file, `settings.json` in the data folder, for a program started without a shell's
//! environment (the desktop app, a double-click from the Dock): the environment wins over it. See [`setting`].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

/// What the command line said about the game and the threads, over the environment (`--root`, `--user`, `--tlk`, `--jobs`).
#[derive(Clone, Debug, Default)]
pub struct Given {
    /// `--root`: the game's folder.
    pub root: Option<PathBuf>,
    /// `--user`: the game's user folder, where exports go.
    pub user: Option<PathBuf>,
    /// `--tlk`: the module's talk table.
    pub tlk: Option<String>,
    /// `--haks`: the haks the editor reads the game with.
    pub haks: Option<String>,
    /// `--jobs`: how many threads the heavy jobs work on.
    pub jobs: Option<usize>,
}

static GIVEN: OnceLock<Given> = OnceLock::new();

/// Records the command line's choices, once, at start (before anything opens the game).
pub fn set_given(given: Given) {
    let _ = GIVEN.set(given);
}

fn given() -> &'static Given {
    GIVEN.get_or_init(Given::default)
}

/// The game install `--root` named, else the settings file's `NWN_ROOT` when the environment has none (taken as `--root` is);
/// None leaves it to `NWN_ROOT` and the usual places.
pub fn game_root() -> Option<PathBuf> {
    given().root.clone().or_else(|| file_folder(&["NWN_ROOT"]))
}

/// The user folder `--user` named, else the settings file's `NWN_HOME` (or `NWN_USER_DIRECTORY`) when the environment has
/// neither; None leaves it to `NWN_HOME` and the game's own rule.
pub fn game_user() -> Option<PathBuf> {
    given().user.clone().or_else(|| file_folder(&["NWN_HOME", "NWN_USER_DIRECTORY"]))
}

/// The settings file's value of the first of `names`, when none of them is in the environment. (The game's own detection reads
/// the environment's.)
fn file_folder(names: &[&str]) -> Option<PathBuf> {
    if names.iter().any(|n| std::env::var_os(n).is_some_and(|v| !v.is_empty())) {
        return None;
    }
    names.iter().find_map(|n| file_settings().get(*n)).map(PathBuf::from)
}

/// The custom talk table: `--tlk`, else `LANDSCAPER_TLK`; None finds the only one in the user folder.
pub fn tlk() -> Option<String> {
    given().tlk.clone().or_else(|| var("LANDSCAPER_TLK"))
}

/// Which haks the editor reads the game with (its placeables, models, blueprints and textures), as `--haks` or
/// `LANDSCAPER_HAKS` give it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Haks {
    /// Not given: the review service's module haks on a team server, else none.
    Unset,
    /// These, first highest (as a module lists them).
    List(Vec<String>),
    /// The module checkout's own list (`Mod_HakList` of its `module.ifo.json`).
    Module,
}

/// The setting that names the editor's haks.
pub const HAKS: &str = "LANDSCAPER_HAKS";

/// `--haks`, else `LANDSCAPER_HAKS` ([`haks_from`]).
pub fn haks() -> Haks {
    haks_from(given().haks.clone().or_else(|| var(HAKS)).as_deref())
}

/// A hak setting's value: hak names separated by commas or spaces (`.hak` may be added), or `module`. Blank is unset.
pub fn haks_from(v: Option<&str>) -> Haks {
    let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) else { return Haks::Unset };
    if v.eq_ignore_ascii_case("module") {
        return Haks::Module;
    }
    let mut out: Vec<String> = Vec::new();
    for name in v.split([',', ' ', '\t', '\n']).map(str::trim).filter(|n| !n.is_empty()) {
        let name = if name.to_ascii_lowercase().ends_with(".hak") { &name[..name.len() - 4] } else { name };
        if !out.iter().any(|h| h.eq_ignore_ascii_case(name)) {
            out.push(name.to_string());
        }
    }
    Haks::List(out)
}

/// The review service's address and key: `LANDSCAPER_REVIEW_URL` and `LANDSCAPER_REVIEW_KEY`. None unless both are set.
pub fn review_service() -> Option<(String, String)> {
    review_service_from(&setting)
}

/// The setting with the review service's address.
pub const REVIEW_URL: &str = "LANDSCAPER_REVIEW_URL";
/// The setting with the review service's key, sent with every call to it.
pub const REVIEW_KEY: &str = "LANDSCAPER_REVIEW_KEY";

fn review_service_from(get: &dyn Fn(&str) -> Option<String>) -> Option<(String, String)> {
    let set = |k: &str| get(k).filter(|v| !v.trim().is_empty());
    let url = set(REVIEW_URL)?;
    let key = set(REVIEW_KEY)?;
    Some((url.trim().trim_end_matches('/').to_string(), key))
}

/// The setting with the key a trusted service sends to the read-only `/api/service/*` routes (docs/REVIEW-SERVICE.md,
/// "Fetching exports").
pub const SERVICE_KEY: &str = "LANDSCAPER_SERVICE_KEY";
/// The shortest [`SERVICE_KEY`] a team server takes.
pub const MIN_SERVICE_KEY_LEN: usize = 24;

/// [`SERVICE_KEY`], if set: the service routes answer 404 without it.
pub fn service_key() -> Option<String> {
    var(SERVICE_KEY).map(|k| k.trim().to_string())
}

fn var(name: &str) -> Option<String> {
    setting(name).filter(|v| !v.trim().is_empty())
}

/// The settings file: `settings.json` in the data folder ([`crate::paint::data_dir`]). One JSON object of settings, named as in
/// the environment (`NWN_ROOT`, `NWN_HOME`, `LANDSCAPER_*`), each a string:
/// `{ "NWN_ROOT": "D:\\Games\\Neverwinter Nights", "LANDSCAPER_HAKS": "my_plc,my_tex" }`. The desktop app writes the game's
/// folders there; anyone may add the rest. The debugging switches (`LS_*`) stay in the environment.
pub fn settings_file() -> PathBuf {
    crate::paint::data_dir().join("settings.json")
}

/// A setting: the environment's value when it is set and not blank, else the settings file's, else the environment's blank one
/// (so a blank variable means what it meant before there was a file).
pub fn setting(name: &str) -> Option<String> {
    setting_from(name, std::env::var(name).ok(), file_settings())
}

/// [`setting`] for a value that may not be UTF-8 (a path).
pub fn setting_os(name: &str) -> Option<OsString> {
    let env = std::env::var_os(name);
    if env.as_ref().is_some_and(|v| v.to_str().is_none_or(|s| !s.trim().is_empty())) {
        return env;
    }
    file_settings().get(name).map(OsString::from).or(env)
}

fn setting_from(name: &str, env: Option<String>, file: &BTreeMap<String, String>) -> Option<String> {
    if env.as_deref().is_some_and(|v| !v.trim().is_empty()) {
        return env;
    }
    file.get(name).cloned().or(env)
}

/// The settings file's settings, read once. A file that isn't there is none; one that can't be read or isn't a JSON object is
/// said once and ignored, and so is each entry that isn't a setting.
fn file_settings() -> &'static BTreeMap<String, String> {
    static FILE: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    FILE.get_or_init(|| {
        let path = settings_file();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
            Err(e) => {
                eprintln!("warning: {} can't be read ({e}), so its settings are ignored", path.display());
                return BTreeMap::new();
            }
        };
        let (settings, problems) = settings_from(&text);
        for p in problems {
            eprintln!("warning: {}: {p}", path.display());
        }
        settings
    })
}

/// A settings file's settings, and what in it was ignored and why. Only `NWN_ROOT`, `NWN_HOME`, `NWN_USER_DIRECTORY` and
/// `LANDSCAPER_*` are settings; a number or true/false is taken as written, null leaves the setting unset.
fn settings_from(text: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let mut settings = BTreeMap::new();
    let mut problems = Vec::new();
    let object = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(o)) => o,
        Ok(_) => return (settings, vec!["it isn't a JSON object of settings, so it is ignored".into()]),
        Err(e) => return (settings, vec![format!("it isn't JSON ({e}), so it is ignored")]),
    };
    for (name, value) in object {
        if !(matches!(name.as_str(), "NWN_ROOT" | "NWN_HOME" | "NWN_USER_DIRECTORY") || name.starts_with("LANDSCAPER_")) {
            problems.push(format!("{name} is no setting (NWN_ROOT, NWN_HOME or LANDSCAPER_*), so it is ignored"));
            continue;
        }
        let value = match value {
            serde_json::Value::String(v) => v,
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => continue,
            _ => {
                problems.push(format!("{name} isn't a string, so it is ignored"));
                continue;
            }
        };
        settings.insert(name, value);
    }
    (settings, problems)
}

/// The module's source checkout.
pub fn module_repo() -> PathBuf {
    if let Some(p) = var("LANDSCAPER_MODULE_REPO") {
        return PathBuf::from(p);
    }
    PathBuf::from("nwn-module")
}

/// The editor's files, when `--web` and `LANDSCAPER_WEB` aren't given: the first of `web/` beside the program,
/// `./nwn-landscaper-web`, `./landscaper-web`, `../nwn-landscaper-web` and `../landscaper-web` that has an `index.html` (a
/// clone of the editor's repository is `nwn-landscaper-web`; a checkout made before it was renamed is `landscaper-web`). None
/// of them is an error that names the settings.
pub fn web_dir() -> anyhow::Result<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("web")));
    let near = ["nwn-landscaper-web", "landscaper-web", "../nwn-landscaper-web", "../landscaper-web"].map(|p| Some(PathBuf::from(p)));
    std::iter::once(beside).chain(near).flatten().find(|p| p.join("index.html").is_file()).ok_or_else(|| {
        anyhow::anyhow!(
            "the editor's files aren't in web/ beside the program, ./nwn-landscaper-web, ./landscaper-web, \
             ../nwn-landscaper-web or ../landscaper-web: set LANDSCAPER_WEB (or pass --web) to the folder of the \
             nwn-landscaper-web repository"
        )
    })
}

/// What the module is called in messages.
pub fn project() -> String {
    var("LANDSCAPER_PROJECT").unwrap_or_else(|| "the module".to_string())
}

/// The module `dev-pack` writes for the dev server, without `.mod` (`LANDSCAPER_DEV_MODULE`, default `Landscaper_dev`).
pub fn dev_module() -> String {
    dev_module_from(var("LANDSCAPER_DEV_MODULE"))
}

/// Letters, digits and `_ . -` only (the name reaches paths and command lines); anything else is ignored.
fn dev_module_from(v: Option<String>) -> String {
    v.map(|s| s.trim().to_string())
        .filter(|s| {
            s.len() <= 64 && !s.starts_with('.') && !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        })
        .unwrap_or_else(|| "Landscaper_dev".to_string())
}

/// How the editor tells a builder to join the dev server, such as "dev.example.org port 5121" (`LANDSCAPER_DEV_JOIN`; none when unset).
pub fn dev_join() -> Option<String> {
    var("LANDSCAPER_DEV_JOIN")
}

/// The limit on the heavy jobs' threads, if one was set: `--jobs`, else `LANDSCAPER_JOBS`. Read once: a bad value is reported a
/// single time and ignored.
fn jobs() -> Option<usize> {
    static JOBS: OnceLock<Option<usize>> = OnceLock::new();
    *JOBS.get_or_init(|| {
        given().jobs.or_else(|| {
            jobs_from(setting("LANDSCAPER_JOBS").as_deref()).unwrap_or_else(|v| {
                eprintln!("LANDSCAPER_JOBS={v:?} is not a whole number of 1 or more; ignoring it");
                None
            })
        })
    })
}

/// `LANDSCAPER_JOBS`'s value `v` as a thread limit: a whole number of 1 or more as it is, none when unset or empty, and the
/// (trimmed) value back as an error otherwise.
fn jobs_from(v: Option<&str>) -> Result<Option<usize>, String> {
    match v.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n > 0 => Ok(Some(n)),
            _ => Err(v.to_string()),
        },
    }
}

/// The machine's cores, at least 1.
fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

/// How many threads an import and a check (its routes) work on: `--jobs`, else `LANDSCAPER_JOBS`, else half the machine's
/// cores, at least 1, so a job on a shared server leaves the rest to everything else. What they make is the same whatever it is.
pub fn threads() -> usize {
    jobs().unwrap_or_else(|| (cores() / 2).max(1))
}

/// How many threads an export works on (its routes, baked textures and shadow maps): `--jobs`, else `LANDSCAPER_JOBS`, else
/// every core. A team server started with a limit keeps it for exports too. What an export makes is the same whatever it is.
pub fn export_threads() -> usize {
    jobs().unwrap_or_else(cores)
}

/// The domain of the commit author address for saved versions (`user@<domain>`).
pub fn noreply_domain() -> String {
    var("LANDSCAPER_NOREPLY_DOMAIN").unwrap_or_else(|| "users.noreply.invalid".to_string())
}

/// What a new area starts with: the file `LANDSCAPER_AREA_TEMPLATE` names, else a plain template.
pub fn area_template() -> serde_json::Value {
    let text = var("LANDSCAPER_AREA_TEMPLATE").and_then(|p| std::fs::read_to_string(p).ok());
    template_from(text.as_deref())
}

/// The template in `text`, or the plain one when there is none or it isn't JSON.
pub fn template_from(text: Option<&str>) -> serde_json::Value {
    let plain = || serde_json::from_str(include_str!("area_template.json")).expect("area_template.json");
    text.and_then(|t| serde_json::from_str(t).ok()).unwrap_or_else(plain)
}

/// Where a module's source files are in its checkout (relative to `LANDSCAPER_MODULE_REPO`).
///
/// `LANDSCAPER_LAYOUT` picks a preset: `nasher` (the default: nasher's own, everything in `src/`) or `folders` (a folder per kind:
/// `areas/<group>/`, `core/<ext>/` for blueprints and conversations, `core/nss/` for scripts, `module.ifo.json` at the root; new
/// areas go in the `core` group). Unset or empty is `nasher`; another value is refused at a team server's start
/// ([`layout_problem`]) and read as `nasher` elsewhere. Any of
/// `LANDSCAPER_AREAS_DIR`, `LANDSCAPER_BLUEPRINTS_DIR` (with `{ext}` for the file type: `utc`, `uti`, `dlg`...),
/// `LANDSCAPER_SCRIPTS_DIR`, `LANDSCAPER_IFO` and `LANDSCAPER_AREA_FOLDER` (the group a new area goes in) overrides one.
/// Areas and scripts are found anywhere below their folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The areas' folder (`LANDSCAPER_AREAS_DIR`).
    pub areas: String,
    /// The blueprints' and conversations' folder, `{ext}` standing for the file type (`LANDSCAPER_BLUEPRINTS_DIR`).
    pub blueprints: String,
    /// The scripts' folder (`LANDSCAPER_SCRIPTS_DIR`).
    pub scripts: String,
    /// The path of `module.ifo.json` (`LANDSCAPER_IFO`).
    pub ifo: String,
    /// The group a new area goes in, below `areas` (`LANDSCAPER_AREA_FOLDER`; empty: in no group).
    pub area_folder: String,
}

impl Layout {
    /// The layout the settings in `get` describe (the environment, normally; a test's own map otherwise).
    pub fn from(get: &dyn Fn(&str) -> Option<String>) -> Layout {
        // Set but empty is a setting of its own here: new areas in no group.
        let area_folder = get("LANDSCAPER_AREA_FOLDER");
        let get = |k: &str| get(k).map(|v| v.trim().trim_matches('/').to_string()).filter(|v| !v.is_empty());
        let (areas, blueprints, scripts, ifo, folder) = match get("LANDSCAPER_LAYOUT").unwrap_or_default().to_ascii_lowercase().as_str() {
            "folders" => ("areas", "core/{ext}", "core/nss", "module.ifo.json", "core"),
            _ => ("src", "src", "src", "src/module.ifo.json", ""),
        };
        Layout {
            areas: get("LANDSCAPER_AREAS_DIR").unwrap_or_else(|| areas.into()),
            blueprints: get("LANDSCAPER_BLUEPRINTS_DIR").unwrap_or_else(|| blueprints.into()),
            scripts: get("LANDSCAPER_SCRIPTS_DIR").unwrap_or_else(|| scripts.into()),
            ifo: get("LANDSCAPER_IFO").unwrap_or_else(|| ifo.into()),
            area_folder: area_folder.map_or_else(|| folder.into(), |f| f.trim().trim_matches('/').to_string()),
        }
    }

    /// The layout the environment describes.
    pub fn from_env() -> Layout {
        Layout::from(&setting)
    }

    /// The folder of a type's blueprints, relative to the module.
    pub fn blueprint_rel(&self, ext: &str) -> String {
        self.blueprints.replace("{ext}", ext)
    }

    /// The folder of a type's blueprints in the module checkout.
    pub fn blueprint_dir(&self, ext: &str) -> PathBuf {
        module_repo().join(self.blueprint_rel(ext))
    }

    /// The areas' folder in the module checkout.
    pub fn areas_dir(&self) -> PathBuf {
        module_repo().join(&self.areas)
    }

    /// The scripts' folder in the module checkout.
    pub fn scripts_dir(&self) -> PathBuf {
        module_repo().join(&self.scripts)
    }

    /// `module.ifo.json` in the module checkout.
    pub fn ifo_path(&self) -> PathBuf {
        module_repo().join(&self.ifo)
    }
}

/// The values `LANDSCAPER_LAYOUT` takes.
pub const LAYOUT_PRESETS: [&str; 2] = ["nasher", "folders"];

/// What is wrong with `LANDSCAPER_LAYOUT` in `get`, if anything: a value that is no preset (a typo would otherwise read the module
/// from the wrong folders, and a publish would write the area's files there).
pub fn layout_problem(get: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let v = get("LANDSCAPER_LAYOUT").map(|v| v.trim().to_ascii_lowercase()).filter(|v| !v.is_empty())?;
    (!LAYOUT_PRESETS.contains(&v.as_str()))
        .then(|| format!("LANDSCAPER_LAYOUT is {v:?}: it is one of {} (unset is nasher)", LAYOUT_PRESETS.join(", ")))
}

/// Joins path pieces with `/`, skipping empty ones (a package's paths).
pub fn rel(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join("/")
}

/// The headers a team server's sign-in proxy sends, and who may do what.
///
/// `LANDSCAPER_AUTH` picks a preset: `authentik` (`X-authentik-username`, `-groups` separated by `|`, `-name`), `oauth2-proxy`
/// (`X-Forwarded-User`, `X-Forwarded-Groups`, `X-Forwarded-Preferred-Username`), `authelia` (`Remote-User`, `Remote-Groups`,
/// `Remote-Name`) or `headers` (the oauth2-proxy names: whatever forward-auth setup sends them). Any of `LANDSCAPER_USER_HEADER`,
/// `LANDSCAPER_GROUPS_HEADER`, `LANDSCAPER_NAME_HEADER` and `LANDSCAPER_GROUPS_SEPARATOR` overrides one. A team server must set
/// `LANDSCAPER_AUTH` ([`team_problems`]); unset (your own machine) reads the `headers` names.
///
/// Who may do what: `LANDSCAPER_ADMIN_GROUPS` (default: no group; `none` or empty says so) and `LANDSCAPER_ADMIN_USERS` are
/// admins; `LANDSCAPER_ALLOWED_GROUPS` (`*` lets everyone the proxy signs in) may use it, and a team server must set it. A team
/// without groups uses `LANDSCAPER_ALLOWED_GROUPS=*` and `LANDSCAPER_ADMIN_USERS=alice,bob`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthConfig {
    /// The header that names who signed in.
    pub user: String,
    /// The header that lists their groups.
    pub groups: String,
    /// The header with their display name.
    pub name: String,
    /// What separates the groups in `groups`.
    pub separator: char,
    /// Members of these groups are admins (`LANDSCAPER_ADMIN_GROUPS`).
    pub admin_groups: Vec<String>,
    /// These users (lower case) are admins (`LANDSCAPER_ADMIN_USERS`).
    pub admin_users: Vec<String>,
    /// Members of these groups may use the server; `*` lets everyone the proxy signs in (`LANDSCAPER_ALLOWED_GROUPS`).
    pub allowed_groups: Vec<String>,
    /// `LANDSCAPER_PROXY_SECRET`: what the proxy sends in `X-Landscaper-Proxy`, so nobody can reach the server around it.
    pub proxy_secret: Option<String>,
}

fn list(value: Option<String>, default: &str) -> Vec<String> {
    value.unwrap_or_else(|| default.to_string()).split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// `LANDSCAPER_ADMIN_GROUPS`: unset, `none` or empty is no group at all (admins only by `LANDSCAPER_ADMIN_USERS`), so a groups
/// header can't make anyone an admin.
fn admin_groups(value: Option<String>) -> Vec<String> {
    match value {
        Some(v) if v.trim().eq_ignore_ascii_case("none") => Vec::new(),
        v => list(v, ""),
    }
}

/// The values `LANDSCAPER_AUTH` takes.
const AUTH_PRESETS: [&str; 4] = ["authentik", "oauth2-proxy", "authelia", "headers"];

impl AuthConfig {
    /// From the settings in `get` (the environment, normally).
    pub fn from(get: &dyn Fn(&str) -> Option<String>) -> AuthConfig {
        let get_raw = get;
        let get = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let (user, groups, name, sep) = match get("LANDSCAPER_AUTH").unwrap_or_default().to_ascii_lowercase().as_str() {
            "oauth2-proxy" | "headers" => ("X-Forwarded-User", "X-Forwarded-Groups", "X-Forwarded-Preferred-Username", ','),
            "authentik" => ("X-authentik-username", "X-authentik-groups", "X-authentik-name", '|'),
            "authelia" => ("Remote-User", "Remote-Groups", "Remote-Name", ','),
            _ => ("X-Forwarded-User", "X-Forwarded-Groups", "X-Forwarded-Preferred-Username", ','),
        };
        AuthConfig {
            user: get("LANDSCAPER_USER_HEADER").unwrap_or_else(|| user.into()),
            groups: get("LANDSCAPER_GROUPS_HEADER").unwrap_or_else(|| groups.into()),
            name: get("LANDSCAPER_NAME_HEADER").unwrap_or_else(|| name.into()),
            separator: get("LANDSCAPER_GROUPS_SEPARATOR")
                .and_then(|s| if s.eq_ignore_ascii_case("space") { Some(' ') } else { s.chars().next() })
                .unwrap_or(sep),
            admin_groups: admin_groups(get_raw("LANDSCAPER_ADMIN_GROUPS")),
            admin_users: list(get("LANDSCAPER_ADMIN_USERS"), "").into_iter().map(|u| u.to_ascii_lowercase()).collect(),
            allowed_groups: list(get("LANDSCAPER_ALLOWED_GROUPS"), ""),
            // Trimmed as `proxy_secret_problem` checks it: an env file with a trailing space or a CRLF line ending would
            // otherwise start fine and refuse every request.
            proxy_secret: get("LANDSCAPER_PROXY_SECRET").map(|s| s.trim().to_string()),
        }
    }

    /// The sign-in settings the environment describes.
    pub fn from_env() -> AuthConfig {
        AuthConfig::from(&setting)
    }
}

/// What is wrong with the server's settings in `get` (the environment, normally), said so it can be fixed: a setting that is
/// an unresolved `${NAME}` (an env file copied from a deployment tool that fills those in: compose turns it into an empty
/// string), and a service address with no key or token for it (the review service, Gitea). Both fail quietly otherwise: the server
/// starts and looks healthy but its area list is empty or its saves are refused.
pub fn setting_problems(get: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let set = |k: &str| get(k).filter(|v| !v.trim().is_empty());
    let mut out = Vec::new();
    for (url, secret) in [(REVIEW_URL, REVIEW_KEY), ("LANDSCAPER_GITEA_URL", "LANDSCAPER_GITEA_TOKEN")] {
        if set(url).is_some() && set(secret).is_none() {
            out.push(format!("{url} is set but {secret} is empty (an env file with `{secret}=${{{secret}}}` left unresolved gives that)"));
        }
    }
    for k in [REVIEW_URL, REVIEW_KEY, "LANDSCAPER_GITEA_URL", "LANDSCAPER_GITEA_TOKEN", "LANDSCAPER_PROXY_SECRET", SERVICE_KEY] {
        if get(k).is_some_and(|v| v.trim_start().starts_with("${")) {
            out.push(format!("{k} is an unresolved ${{...}} placeholder, not a value"));
        }
    }
    if let Some(k) = set(SERVICE_KEY).map(|k| k.trim().to_string())
        && !k.starts_with("${")
        && k.chars().count() < MIN_SERVICE_KEY_LEN
    {
        out.push(format!(
            "{SERVICE_KEY} is {} characters: use at least {MIN_SERVICE_KEY_LEN} (e.g. `openssl rand -hex 24`)",
            k.chars().count()
        ));
    }
    out.extend(layout_problem(get));
    out
}

/// What a team server must have set that a machine of one doesn't: which sign-in proxy it is behind (`LANDSCAPER_AUTH`) and who
/// may use it (`LANDSCAPER_ALLOWED_GROUPS`). Neither has a default, so no project's choice is taken for another's.
pub fn team_problems(get: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let set = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let mut out = Vec::new();
    match set("LANDSCAPER_AUTH") {
        None => out.push(format!(
            "LANDSCAPER_AUTH is not set: a team server needs to know which sign-in proxy it is behind ({})",
            AUTH_PRESETS.join(", ")
        )),
        Some(a) if !AUTH_PRESETS.contains(&a.to_ascii_lowercase().as_str()) => {
            out.push(format!("LANDSCAPER_AUTH={a:?} is none of {}", AUTH_PRESETS.join(", ")));
        }
        Some(_) => {}
    }
    if set("LANDSCAPER_ALLOWED_GROUPS").is_none() {
        out.push(
            "LANDSCAPER_ALLOWED_GROUPS is not set: name the groups that may use the server (comma separated), or `*` for everyone \
             the proxy signs in"
                .into(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Haks, haks_from};

    #[test]
    fn the_hak_setting_is_names_in_order_or_module() {
        let list = |v: &[&str]| Haks::List(v.iter().map(|s| (*s).to_string()).collect());
        assert_eq!(haks_from(None), Haks::Unset);
        assert_eq!(haks_from(Some("  ")), Haks::Unset);
        assert_eq!(haks_from(Some("module")), Haks::Module);
        assert_eq!(haks_from(Some(" Module ")), Haks::Module);
        // The module's order, first highest; `.hak` dropped, a name given twice kept once (the first), commas or spaces between.
        assert_eq!(haks_from(Some("my_plc, my_tex.hak,my_plc  extra.HAK")), list(&["my_plc", "my_tex", "extra"]));
        assert_eq!(haks_from(Some("a\nb\tc")), list(&["a", "b", "c"]));
        assert_eq!(haks_from(Some(",, ,")), list(&[]));
    }

    #[test]
    fn the_jobs_limit_is_a_whole_number_of_one_or_more() {
        assert_eq!(jobs_from(None), Ok(None));
        assert_eq!(jobs_from(Some("")), Ok(None));
        assert_eq!(jobs_from(Some("  ")), Ok(None));
        assert_eq!(jobs_from(Some("2")), Ok(Some(2)));
        assert_eq!(jobs_from(Some(" 12 ")), Ok(Some(12)));
        assert_eq!(jobs_from(Some("0")), Err("0".to_string()));
        assert_eq!(jobs_from(Some("lots")), Err("lots".to_string()));
        assert_eq!(jobs_from(Some("-3")), Err("-3".to_string()));
    }

    #[test]
    fn the_review_service_needs_both_settings() {
        let env = |pairs: &'static [(&str, &str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        let r = |pairs: &'static [(&str, &str)]| review_service_from(&env(pairs));
        assert_eq!(r(&[]), None);
        let want = Some(("http://r".to_string(), "k".to_string()));
        assert_eq!(r(&[("LANDSCAPER_REVIEW_URL", "http://r/"), ("LANDSCAPER_REVIEW_KEY", "k")]), want);
        assert_eq!(r(&[("LANDSCAPER_REVIEW_URL", "http://r")]), None);
        assert_eq!(r(&[("LANDSCAPER_REVIEW_URL", "http://r"), ("LANDSCAPER_REVIEW_KEY", " ")]), None);
        let p = setting_problems(&env(&[("LANDSCAPER_REVIEW_URL", "http://r")]));
        assert!(p.len() == 1 && p[0].contains("LANDSCAPER_REVIEW_URL is set but LANDSCAPER_REVIEW_KEY"), "{p:?}");
        assert!(setting_problems(&env(&[("LANDSCAPER_REVIEW_URL", "http://r"), ("LANDSCAPER_REVIEW_KEY", "k")])).is_empty());
    }

    #[test]
    fn a_service_address_without_its_key_and_an_unresolved_placeholder_are_problems() {
        let env = |pairs: &'static [(&str, &str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert!(setting_problems(&env(&[])).is_empty(), "nothing set up is fine (a machine of one)");
        assert!(
            setting_problems(&env(&[
                ("LANDSCAPER_REVIEW_URL", "http://x"),
                ("LANDSCAPER_REVIEW_KEY", "k"),
                ("LANDSCAPER_GITEA_URL", "https://g"),
                ("LANDSCAPER_GITEA_TOKEN", "t")
            ]))
            .is_empty()
        );
        let p = setting_problems(&env(&[
            ("LANDSCAPER_REVIEW_URL", "http://x"),
            ("LANDSCAPER_REVIEW_KEY", ""),
            ("LANDSCAPER_GITEA_URL", "https://g"),
        ]));
        assert_eq!(p.len(), 2, "{p:?}");
        let p = setting_problems(&env(&[("LANDSCAPER_REVIEW_URL", "http://x"), ("LANDSCAPER_REVIEW_KEY", "${LANDSCAPER_REVIEW_KEY}")]));
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("placeholder"));
        // The service key: short is refused, a placeholder said once, 24 characters or more taken.
        let p = setting_problems(&env(&[(SERVICE_KEY, "short")]));
        assert!(p.len() == 1 && p[0].contains("at least 24"), "{p:?}");
        let p = setting_problems(&env(&[(SERVICE_KEY, "${LANDSCAPER_SERVICE_KEY}")]));
        assert!(p.len() == 1 && p[0].contains("placeholder"), "{p:?}");
        assert!(setting_problems(&env(&[(SERVICE_KEY, "kkkkkkkkkkkkkkkkkkkkkkkk")])).is_empty());
    }

    use super::*;

    #[test]
    fn the_plain_template_has_no_project_scripts() {
        let t = template_from(None);
        assert_eq!(t["scripts"]["OnEnter"], "");
        assert_eq!(t["pvp"], 3);
    }

    #[test]
    fn a_projects_own_template_replaces_it_and_a_broken_one_falls_back() {
        let t = template_from(Some(r#"{ "scripts": { "OnEnter": "my_enter" }, "pvp": 0 }"#));
        assert_eq!((t["scripts"]["OnEnter"].as_str(), t["pvp"].as_i64()), (Some("my_enter"), Some(0)));
        assert_eq!(template_from(Some("not json"))["scripts"]["OnEnter"], "");
    }

    #[test]
    fn the_dev_module_is_named_by_the_project_or_plain() {
        assert_eq!(dev_module_from(None), "Landscaper_dev");
        assert_eq!(dev_module_from(Some(" Mymodule_dev ".into())), "Mymodule_dev");
        for bad in ["", "  ", "../x", "a b", ".hidden", "x/y", "semi;colon"] {
            assert_eq!(dev_module_from(Some(bad.into())), "Landscaper_dev", "{bad:?}");
        }
    }

    fn layout(pairs: &[(&str, &str)]) -> Layout {
        Layout::from(&|k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn with_no_layout_set_the_module_is_read_as_nasher_lays_it_out() {
        for unset in [&[][..], &[("LANDSCAPER_LAYOUT", "")], &[("LANDSCAPER_LAYOUT", " Nasher ")]] {
            let d = layout(unset);
            assert_eq!(
                (d.areas.as_str(), d.blueprint_rel("utc").as_str(), d.scripts.as_str(), d.ifo.as_str(), d.area_folder.as_str()),
                ("src", "src", "src", "src/module.ifo.json", ""),
                "{unset:?}"
            );
        }
        // A setting changes one place.
        let o = layout(&[("LANDSCAPER_BLUEPRINTS_DIR", "/content/{ext}/")]);
        assert_eq!((o.blueprint_rel("uti").as_str(), o.areas.as_str()), ("content/uti", "src"));
        assert_eq!(rel(&["areas", "", "x.are.json"]), "areas/x.are.json");
    }

    #[test]
    fn the_folders_layout_is_the_one_that_was_the_default() {
        // LANDSCAPER_LAYOUT=folders: every path as before nasher became the default.
        let f = layout(&[("LANDSCAPER_LAYOUT", "folders")]);
        assert_eq!(
            (f.areas.as_str(), f.blueprint_rel("utc").as_str(), f.blueprint_rel("dlg").as_str(), f.scripts.as_str(), f.ifo.as_str()),
            ("areas", "core/utc", "core/dlg", "core/nss", "module.ifo.json")
        );
        assert_eq!(f.area_folder, "core");
        assert_eq!(layout(&[("LANDSCAPER_LAYOUT", "FOLDERS")]), f);
    }

    #[test]
    fn a_layout_that_is_no_preset_is_refused_at_a_team_servers_start() {
        let problem = |v: &str| layout_problem(&|k| (k == "LANDSCAPER_LAYOUT").then(|| v.to_string()));
        for ok in ["", " ", "nasher", "folders", "Folders"] {
            assert_eq!(problem(ok), None, "{ok:?}");
        }
        let p = problem("mymodule").unwrap();
        assert!(p.contains("\"mymodule\"") && p.contains("nasher, folders"), "{p}");
        assert!(setting_problems(&|k| (k == "LANDSCAPER_LAYOUT").then(|| "nashr".to_string())).iter().any(|p| p.contains("nashr")));
    }

    fn auth(pairs: &[(&str, &str)]) -> AuthConfig {
        AuthConfig::from(&|k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn the_default_is_the_plain_headers_with_no_groups_and_a_preset_or_a_setting_changes_them() {
        let a = auth(&[]);
        assert_eq!((a.user.as_str(), a.separator), ("X-Forwarded-User", ','));
        assert!(a.admin_groups.is_empty() && a.allowed_groups.is_empty());
        let k = auth(&[("LANDSCAPER_AUTH", "Authentik")]);
        assert_eq!((k.user.as_str(), k.groups.as_str(), k.separator), ("X-authentik-username", "X-authentik-groups", '|'));
        let o = auth(&[("LANDSCAPER_AUTH", "oauth2-proxy")]);
        assert_eq!((o.user.as_str(), o.groups.as_str(), o.separator), ("X-Forwarded-User", "X-Forwarded-Groups", ','));
        let r = auth(&[("LANDSCAPER_AUTH", "authelia"), ("LANDSCAPER_USER_HEADER", "X-Me"), ("LANDSCAPER_GROUPS_SEPARATOR", "space")]);
        assert_eq!((r.user.as_str(), r.groups.as_str(), r.separator), ("X-Me", "Remote-Groups", ' '));
        assert_eq!(auth(&[("LANDSCAPER_ADMIN_USERS", " Alice, bob ")]).admin_users, vec!["alice", "bob"]);
    }

    #[test]
    fn a_team_server_must_say_which_proxy_and_who_may_use_it() {
        let env = |pairs: &'static [(&str, &str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        let p = team_problems(&env(&[]));
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].contains("LANDSCAPER_AUTH") && p[1].contains("LANDSCAPER_ALLOWED_GROUPS"), "{p:?}");
        assert_eq!(team_problems(&env(&[("LANDSCAPER_AUTH", " "), ("LANDSCAPER_ALLOWED_GROUPS", "*")])).len(), 1);
        let p = team_problems(&env(&[("LANDSCAPER_AUTH", "kerberos"), ("LANDSCAPER_ALLOWED_GROUPS", "*")]));
        assert!(p.len() == 1 && p[0].contains("kerberos"), "{p:?}");
        // The forge's own settings.
        assert!(team_problems(&env(&[("LANDSCAPER_AUTH", "authentik"), ("LANDSCAPER_ALLOWED_GROUPS", "Builders")])).is_empty());
        assert!(team_problems(&env(&[("LANDSCAPER_AUTH", "Headers"), ("LANDSCAPER_ALLOWED_GROUPS", "*")])).is_empty());
    }

    #[test]
    fn the_settings_file_holds_settings_and_says_what_it_ignores() {
        let (s, problems) = settings_from(
            r#"{ "NWN_ROOT": "D:\\Games\\Neverwinter Nights", "LANDSCAPER_JOBS": 4, "LANDSCAPER_TEAM": true,
                 "LANDSCAPER_TLK": null, "XDG_DATA_HOME": "/x", "LS_FLAT": "1", "LANDSCAPER_HAKS": ["a"] }"#,
        );
        assert_eq!(s.get("NWN_ROOT").map(String::as_str), Some(r"D:\Games\Neverwinter Nights"));
        assert_eq!((s["LANDSCAPER_JOBS"].as_str(), s["LANDSCAPER_TEAM"].as_str()), ("4", "true"));
        // Null leaves it unset; the data folder and the debugging switches stay in the environment; a list is no value.
        assert!(["LANDSCAPER_TLK", "XDG_DATA_HOME", "LS_FLAT", "LANDSCAPER_HAKS"].iter().all(|k| !s.contains_key(*k)));
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("XDG_DATA_HOME is no setting")));
        assert!(problems.iter().any(|p| p.starts_with("LANDSCAPER_HAKS isn't a string")));
        for broken in ["[1, 2]", "not json", ""] {
            let (s, problems) = settings_from(broken);
            assert!(s.is_empty() && problems.len() == 1, "{broken}: {problems:?}");
        }
    }

    #[test]
    fn the_environment_wins_over_the_settings_file_and_a_blank_variable_does_not() {
        let file: BTreeMap<String, String> = [("LANDSCAPER_HAKS".to_string(), "from_file".to_string())].into();
        let get = |env: Option<&str>| setting_from("LANDSCAPER_HAKS", env.map(String::from), &file);
        assert_eq!(get(Some("from_env")).as_deref(), Some("from_env"));
        assert_eq!(get(None).as_deref(), Some("from_file"));
        assert_eq!(get(Some("  ")).as_deref(), Some("from_file"));
        // Without the file, everything is as before it: a blank variable is still there, for the settings that look at that.
        let none = BTreeMap::new();
        assert_eq!(setting_from("LANDSCAPER_TEAM", Some(String::new()), &none).as_deref(), Some(""));
        assert_eq!(setting_from("LANDSCAPER_TEAM", None, &none), None);
    }
}
