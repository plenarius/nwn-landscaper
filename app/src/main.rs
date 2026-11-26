// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! NWN Landscaper as an app (docs/DESIGN-desktop-app.md): the editor in a window of its own, with the engine inside it. The
//! engine serves on 127.0.0.1 (port 8766, else the next free one) on a thread, as `landscaper serve` would; the window shows the
//! app's own page (`ui/`) until the engine answers, or asks for the game's folders when it can't find them.
//!
//! - The editor opens at `/?app`, so it knows it is in the app, and the engine answers the app's own route, `POST /api/app/open`
//!   ([`app_open`]): the data folder, the log, or this page's game folders again.
//! - Only one copy runs: a second start brings the first one's window forward.
//! - What the engine prints goes to `landscaper.log` in the data folder ([`log`]).
//! - Links to anything but the engine open in the default browser; downloads go to the Downloads folder ([`place`]).
//! - On Linux, WebKitGTK's renderer is switched for NVIDIA's driver ([`webview`]).

// A window program on Windows: no console beside it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod log;
mod place;
mod webview;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// Where the app is, for its page (`status`).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
enum Phase {
    /// The game's folders: needed first, or asked for again from the editor (`back` is then the editor's address).
    Setup { root: Option<String>, user: Option<String>, problem: Option<String>, back: Option<String> },
    /// The engine is starting (on a first start it builds the texture library from the game's files).
    Starting,
    /// The engine answers here.
    Ready { url: String },
    /// The engine stopped: why, and where the log is.
    Failed { message: String },
}

struct App {
    phase: Mutex<Phase>,
    port: u16,
    log: Option<PathBuf>,
    handle: tauri::AppHandle,
    /// The app's own page, where the window starts.
    home: OnceLock<tauri::Url>,
    /// Whether the engine has been started (it reads the game's folders once).
    started: AtomicBool,
}

type Shared = Arc<App>;

fn set(app: &App, phase: Phase) {
    *app.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = phase;
}

/// The page asks where the app is.
#[tauri::command]
fn status(app: tauri::State<'_, Shared>) -> serde_json::Value {
    let phase = app.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let mut v = serde_json::to_value(phase).unwrap_or_default();
    v["log"] = app.log.as_ref().map(|p| p.display().to_string()).into();
    v
}

/// A folder picker; None when the person cancels.
#[tauri::command]
async fn pick_folder(handle: tauri::AppHandle, title: String) -> Option<String> {
    handle.dialog().file().set_title(title).blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.display().to_string())
}

/// The folders the person chose: checked, kept in the settings file, then the engine starts. An error says what is wrong.
#[tauri::command]
fn use_folders(app: tauri::State<'_, Shared>, root: String, user: Option<String>) -> Result<(), String> {
    let root = PathBuf::from(root.trim());
    if let Some(why) = landscaper::why_not_game(&root) {
        return Err(format!("That isn't the game's folder: {why}."));
    }
    let user = user.map(|u| PathBuf::from(u.trim())).filter(|u| !u.as_os_str().is_empty());
    if let Some(u) = &user
        && !u.is_dir()
    {
        return Err(format!("{} isn't a folder.", u.display()));
    }
    landscaper::save_game_folders(&root, user.as_deref()).map_err(|e| format!("{e:#}"))?;
    println!("game folders saved in {}", landscaper::data_dir().join("settings.json").display());
    if app.started.load(Ordering::SeqCst) {
        // The engine has read the old ones: start again with these.
        app.handle.restart();
    }
    start_engine(app.inner().clone());
    Ok(())
}

/// The app's route (`POST /api/app/open`, the editor's Help menu): opens the data folder or the log with the system, or shows
/// the game-folder page again. An error is what the editor shows.
fn app_open(app: &Shared, what: &str) -> Result<(), String> {
    let opener = app.handle.opener();
    match what {
        "data" => opener.open_path(landscaper::data_dir().display().to_string(), None::<&str>).map_err(|e| e.to_string()),
        "log" => match &app.log {
            Some(log) => opener.open_path(log.display().to_string(), None::<&str>).map_err(|e| e.to_string()),
            None => Err("The Landscaper was started from a terminal: what it prints is there, not in a log file.".into()),
        },
        "folders" => {
            let (root, user) = landscaper::found_game();
            let shown = |p: Option<PathBuf>| p.map(|p| p.display().to_string());
            let back = Some(place::engine_url(app.port));
            set(app, Phase::Setup { root: shown(root), user: shown(user), problem: None, back });
            let (Some(w), Some(home)) = (app.handle.get_webview_window("main"), app.home.get()) else {
                return Err("the app's window isn't there".into());
            };
            w.navigate(home.clone()).map_err(|e| e.to_string())
        }
        _ => Err(format!("can't open {what}")),
    }
}

/// The engine on its thread, and a second thread that says when it answers.
fn start_engine(app: Shared) {
    set(&app, Phase::Starting);
    app.started.store(true, Ordering::SeqCst);
    let port = app.port;
    let serving = app.clone();
    std::thread::spawn(move || {
        // (`landscaper serve` finds the editor beside the program, or a checkout beside the working folder, when it isn't bundled.)
        let web = webview::bundled_web();
        let asked = serving.clone();
        let open: landscaper::AppOpen = Box::new(move |what| app_open(&asked, what));
        if let Err(e) = landscaper::serve_local(port, web.as_deref(), open) {
            eprintln!("error: {e:#}");
            set(&serving, Phase::Failed { message: format!("{e:#}") });
        }
    });
    std::thread::spawn(move || {
        loop {
            if matches!(*app.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner), Phase::Failed { .. }) {
                return;
            }
            if landscaper::is_landscaper(port) {
                set(&app, Phase::Ready { url: place::engine_url(port) });
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

/// The app's start, once the single-instance check has passed (a second start never gets here): the log, the port, the game's
/// folders, then the engine or the page that asks for them.
fn begin(handle: tauri::AppHandle) -> Shared {
    let log = log::start();
    println!("NWN Landscaper {} (the app)", env!("CARGO_PKG_VERSION"));
    let port = place::free_port();
    if port != place::PORTS.start {
        println!("port {} is taken by another program, so the editor takes {port}: its settings start empty there", place::PORTS.start);
    }
    let (root, user) = landscaper::found_game();
    let app: Shared = Arc::new(App {
        phase: Mutex::new(Phase::Starting),
        port,
        log: log.as_ref().map(|l| l.path.clone()),
        handle,
        home: OnceLock::new(),
        started: AtomicBool::new(false),
    });
    // The log is written until the program ends.
    std::mem::forget(log);
    match &root {
        Some(_) => start_engine(app.clone()),
        None => set(
            &app,
            Phase::Setup {
                root: None,
                user: user.as_ref().map(|u| u.display().to_string()),
                problem: std::env::var_os("NWN_ROOT")
                    .map(PathBuf::from)
                    .and_then(|r| landscaper::why_not_game(&r).map(|why| format!("NWN_ROOT is {}: {why}", r.display()))),
                back: None,
            },
        ),
    }
    app
}

fn main() {
    webview::switch();
    tauri::Builder::default()
        // First: a second start only brings this window forward, and goes no further.
        .plugin(tauri_plugin_single_instance::init(|handle, _, _| {
            if let Some(w) = handle.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![status, pick_folder, use_folders])
        .setup(move |tauri_app| {
            let app = begin(tauri_app.handle().clone());
            let port = app.port;
            let home = app.clone();
            tauri_app.manage(app);
            let handle = tauri_app.handle().clone();
            let links = handle.clone();
            let popups = handle.clone();
            let downloads = handle.path().download_dir().ok();
            let (width, height) = place::window_size(tauri_app.primary_monitor().ok().flatten().map(|m| {
                let s = m.size().to_logical::<f64>(m.scale_factor());
                (s.width, s.height)
            }));
            let window = WebviewWindowBuilder::new(tauri_app, "main", WebviewUrl::App("index.html".into()))
                .title("NWN Landscaper")
                .inner_size(width, height)
                .min_inner_size(960.0, 600.0)
                .center()
                .on_navigation(move |url| {
                    if place::stays_inside(url, port) {
                        return true;
                    }
                    let _ = links.opener().open_url(url.as_str(), None::<&str>);
                    false
                })
                .on_new_window(move |url, _| {
                    let _ = popups.opener().open_url(url.as_str(), None::<&str>);
                    tauri::webview::NewWindowResponse::Deny
                })
                .on_download(move |_, event| {
                    match event {
                        tauri::webview::DownloadEvent::Requested { url, destination } => {
                            let Some(dir) = &downloads else { return false };
                            *destination = place::download_path(dir, destination, &url);
                            println!("download: {url} -> {}", destination.display());
                        }
                        tauri::webview::DownloadEvent::Finished { url, path, success } => {
                            let to = path.map_or_else(String::new, |p| p.display().to_string());
                            println!("download {}: {url} {to}", if success { "done" } else { "failed" });
                        }
                        _ => {}
                    }
                    true
                })
                .build()?;
            if let Ok(url) = window.url() {
                let _ = home.home.set(url);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("can't start the app");
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_is_the_engines_version() {
        let engine = include_str!("../../Cargo.toml");
        let version = engine.lines().find_map(|l| l.strip_prefix("version = \"")).and_then(|v| v.strip_suffix('"'));
        assert_eq!(version, Some(env!("CARGO_PKG_VERSION")), "app/Cargo.toml's version follows the engine's");
    }
}
