// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where things go: the engine's port, which addresses stay in the window, where a download is saved, how big the window is.

use std::path::{Path, PathBuf};

use tauri::Url;

/// The app's ports, first choice first: 8766 is its own (a double-click's engine takes 8765), so the app and a browser session
/// keep separate storage. The editor's settings belong to the port, so any other one starts them empty.
pub const PORTS: std::ops::Range<u16> = 8766..8786;

/// The first of [`PORTS`] nothing listens on (8766 when all are taken: the engine then says it can't listen).
pub fn free_port() -> u16 {
    PORTS.into_iter().find(|&p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok()).unwrap_or(PORTS.start)
}

/// The editor's address in the app: `?app` tells the editor it is inside the app (its Help menu, no "start the engine" link).
pub fn engine_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/?app")
}

/// The app's own page, as each webview serves it: `http://tauri.localhost` on Windows (WebView2 takes no custom schemes),
/// `tauri://localhost` elsewhere.
pub fn home() -> Url {
    let base = if cfg!(windows) { "http://tauri.localhost/index.html" } else { "tauri://localhost/index.html" };
    Url::parse(base).expect("a fixed address")
}

/// Whether the window may go to `url`: the engine's editor, or the app's own page. Anything else opens in the default browser.
pub fn stays_inside(url: &Url, port: u16) -> bool {
    match url.scheme() {
        // The app's own page: tauri://localhost on macOS and Linux, http://tauri.localhost on Windows.
        "tauri" => true,
        "http" => {
            let host = url.host_str().unwrap_or("");
            (host == "127.0.0.1" && url.port() == Some(port)) || host == "tauri.localhost"
        }
        "about" | "blob" | "data" => true,
        _ => false,
    }
}

/// Where a download goes: the Downloads folder, under the name the page gave it (the webview's suggestion), else the address's
/// last part, numbered when a file of that name is there already.
pub fn download_path(dir: &Path, suggested: &Path, url: &Url) -> PathBuf {
    let name = suggested
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .or_else(|| url.path_segments().and_then(|mut s| s.next_back()).filter(|s| !s.is_empty()).map(str::to_string))
        .unwrap_or_else(|| "download".to_string());
    // (A name is never a path: the page can't write outside the folder.)
    let name = Path::new(&name).file_name().and_then(|n| n.to_str()).unwrap_or("download").to_string();
    let first = dir.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (2..1000).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// The window's size: 1440 by 900, or nine tenths of a smaller screen (logical pixels).
pub fn window_size(screen: Option<(f64, f64)>) -> (f64, f64) {
    let Some((w, h)) = screen else { return (1440.0, 900.0) };
    (1440.0_f64.min((w * 0.9).round()).max(960.0), 900.0_f64.min((h * 0.9).round()).max(600.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_engine_and_the_apps_page_stay_in_the_window() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(stays_inside(&url("http://127.0.0.1:8766/js/app.js"), 8766));
        assert!(stays_inside(&url(&engine_url(8766)), 8766));
        assert!(stays_inside(&home(), 8766));
        assert_eq!(engine_url(8767), "http://127.0.0.1:8767/?app");
        assert!(stays_inside(&url("tauri://localhost/index.html"), 8766));
        assert!(stays_inside(&url("http://tauri.localhost/index.html"), 8766));
        assert!(!stays_inside(&url("http://127.0.0.1:8765/"), 8766), "another engine's port");
        assert!(!stays_inside(&url("https://github.com/plenarius/nwn-landscaper/releases"), 8766));
        assert!(!stays_inside(&url("http://localhost:8766/"), 8766), "the engine's Host check wants 127.0.0.1");
        assert!(!stays_inside(&url("mailto:someone@example.com"), 8766));
    }

    #[test]
    fn a_download_takes_its_own_name_in_the_downloads_folder_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("ls-downloads-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = Url::parse("http://127.0.0.1:8766/api/package-download?name=lake").unwrap();
        let p = download_path(&dir, Path::new("/tmp/elsewhere/lake.zip"), &url);
        assert_eq!(p, dir.join("lake.zip"));
        std::fs::write(&p, "x").unwrap();
        assert_eq!(download_path(&dir, Path::new("lake.zip"), &url), dir.join("lake (2).zip"));
        assert_eq!(download_path(&dir, Path::new(""), &url), dir.join("package-download"));
        assert_eq!(download_path(&dir, Path::new("../../evil"), &url), dir.join("evil"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_window_fits_the_screen() {
        assert_eq!(window_size(None), (1440.0, 900.0));
        assert_eq!(window_size(Some((2560.0, 1440.0))), (1440.0, 900.0));
        assert_eq!(window_size(Some((1280.0, 800.0))), (1152.0, 720.0));
        assert_eq!(window_size(Some((800.0, 600.0))), (960.0, 600.0));
    }
}
