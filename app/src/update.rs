// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's updates (docs/DESIGN-desktop-app.md, question 6). At start the app reads `latest.json` on the newest published
//! release (`plugins.updater` in tauri.conf.json); when it names a newer version, the editor's version menu offers it
//! (`GET /api/app/update`). On the person's yes (`POST /api/app/update`) the app downloads it, checks its signature against the
//! public key built into the app, installs it and starts again: the NSIS installer on Windows (which starts the app itself), the
//! `.app` on macOS, the AppImage on Linux. A `.deb`, or a program outside an installer, can't replace itself: the menu says to
//! download the new one instead.

use std::sync::{Arc, Mutex, PoisonError};

#[cfg(target_os = "linux")]
use tauri::Manager;
use tauri::utils::config::BundleType;
use tauri_plugin_updater::{Update, UpdaterExt};

/// What the editor's version menu shows.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum State {
    /// The check hasn't answered yet.
    Checking,
    /// Nothing newer, or the check failed (the log says why): the menu offers nothing.
    None,
    /// A newer version, with its release notes; `error` is why the last try to install it failed.
    Available { version: String, notes: Option<String>, how: How, error: Option<String> },
    /// Downloading it (`done` bytes of `total`), then installing; the app starts again after.
    Installing { version: String, done: u64, total: Option<u64> },
}

/// How this copy gets a new version.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    /// The app installs it itself: the AppImage (run as one), the Windows installer's, or the macOS app.
    Install,
    /// A `.deb`: dpkg needs an administrator, so the person downloads the new `.deb` and installs it as the first.
    Deb,
    /// A program built or unpacked outside an installer: from the release page.
    Download,
}

pub struct Updates {
    state: Mutex<State>,
    /// The update the menu offers, as the updater found it (its address and signature).
    offered: Mutex<Option<Update>>,
}

impl Updates {
    pub fn new() -> Self {
        Self { state: Mutex::new(State::Checking), offered: Mutex::new(None) }
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set(&self, state: State) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }
}

/// How this copy gets a new version ([`How`]), from the installer the bundler marked it with.
fn how(handle: &tauri::AppHandle) -> How {
    // (An AppImage replaces itself only when run as one: unpacked, it has no file to replace.)
    #[cfg(target_os = "linux")]
    let appimage = handle.env().appimage.is_some();
    #[cfg(not(target_os = "linux"))]
    let appimage = {
        let _ = handle;
        false
    };
    match tauri::utils::platform::bundle_type() {
        Some(BundleType::AppImage) if appimage => How::Install,
        Some(BundleType::Nsis | BundleType::Msi | BundleType::App) => How::Install,
        Some(BundleType::Deb) => How::Deb,
        _ => How::Download,
    }
}

/// Reads `latest.json` (on Tauri's async runtime) and keeps what it offers. A failure is logged, and the menu offers nothing.
pub fn check(handle: tauri::AppHandle, updates: Arc<Updates>) {
    tauri::async_runtime::spawn(async move {
        let found = match handle.updater() {
            Ok(updater) => updater.check().await,
            Err(e) => Err(e),
        };
        match found {
            Ok(Some(update)) => {
                println!("update: {} is out (this is {})", update.version, update.current_version);
                updates.set(State::Available {
                    version: update.version.clone(),
                    notes: update.body.clone().filter(|n| !n.trim().is_empty()),
                    how: how(&handle),
                    error: None,
                });
                *updates.offered.lock().unwrap_or_else(PoisonError::into_inner) = Some(update);
            }
            Ok(None) => {
                println!("update: this is the newest version");
                updates.set(State::None);
            }
            Err(e) => {
                println!("update: couldn't check for a newer version: {e}");
                updates.set(State::None);
            }
        }
    });
}

/// Downloads the offered update, checks its signature, installs it and starts the app again. Err when there is nothing this copy
/// can install, or an install is under way; a failure later goes back to offering it, with why.
pub fn install(handle: &tauri::AppHandle, updates: &Arc<Updates>) -> Result<(), String> {
    let update = {
        let mut state = updates.state.lock().unwrap_or_else(PoisonError::into_inner);
        let State::Available { version, how, .. } = &*state else {
            return Err(match &*state {
                State::Installing { version, .. } => format!("{version} is being installed already."),
                _ => "There is no update to install.".to_string(),
            });
        };
        if *how != How::Install {
            return Err(format!("This copy of the app can't update itself: download {version} from its release page."));
        }
        let update = updates.offered.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let Some(update) = update else { return Err("There is no update to install.".to_string()) };
        *state = State::Installing { version: version.clone(), done: 0, total: None };
        update
    };
    let handle = handle.clone();
    let updates = updates.clone();
    tauri::async_runtime::spawn(async move {
        println!("update: downloading {} from {}", update.version, update.download_url);
        let mut done = 0u64;
        let progress = |chunk: usize, total: Option<u64>| {
            done += chunk as u64;
            updates.set(State::Installing { version: update.version.clone(), done, total });
        };
        // (On Windows the installer takes over here: the app ends, and the installer starts the new one.)
        // (The signature is checked once the download is complete, before anything is installed.)
        match update.download_and_install(progress, || println!("update: downloaded, checking its signature")).await {
            Ok(()) => {
                println!("update: signature good, {} installed, starting it", update.version);
                handle.restart();
            }
            Err(e) => {
                println!("update: couldn't install {}: {e}", update.version);
                updates.set(State::Available {
                    version: update.version.clone(),
                    notes: update.body.clone().filter(|n| !n.trim().is_empty()),
                    how: How::Install,
                    error: Some(format!("{e}. Nothing was installed: this version keeps running.")),
                });
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{How, State};

    #[test]
    fn the_menu_reads_the_state_by_its_name() {
        let v = |s: State| serde_json::to_value(s).unwrap();
        assert_eq!(v(State::Checking), serde_json::json!({ "state": "checking" }));
        let offered = v(State::Available { version: "0.8.0".into(), notes: None, how: How::Deb, error: None });
        assert_eq!(offered["state"], "available");
        assert_eq!(offered["version"], "0.8.0");
        assert_eq!(offered["how"], "deb");
        assert_eq!(v(State::Installing { version: "0.8.0".into(), done: 5, total: Some(10) })["done"], 5);
    }
}
