// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tauri's build step. The app's commands need a permission, which only the app's own page has (capabilities/default.json):
//! the editor, served by the engine, can't call them.

fn main() {
    let commands = tauri_build::AppManifest::new().commands(&["status", "pick_folder", "use_folders"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands)).expect("tauri's build step");
}
