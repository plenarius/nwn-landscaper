// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The webview's side: the editor's files in a bundle, and on Linux the switch for WebKitGTK's renderer.

use std::path::PathBuf;

/// The editor's files where a bundle puts them: beside the program (Windows), in `Contents/Resources/web` (macOS) or
/// `/usr/lib/NWN Landscaper/web` (the `.deb` and the AppImage). None outside a bundle: the engine then looks as `serve` does.
pub fn bundled_web() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    ["web", "../Resources/web", "../lib/NWN Landscaper/web"].iter().map(|p| dir.join(p)).find(|p| p.join("index.html").is_file())
}

/// The variable set once the switch has run, so it runs once.
#[cfg(target_os = "linux")]
const SWITCHED: &str = "LANDSCAPER_WEBVIEW_SWITCHED";

/// On Linux, starts the app again with WebKitGTK's DMA-BUF renderer off when NVIDIA's own driver is loaded (under Wayland it
/// leaves the window blank), and with compositing off too when `LANDSCAPER_WEBVIEW_SAFE=1` asks for it. Variables the person set
/// stay as they are. (Starting again because the variables must be there before WebKitGTK starts, and this crate sets none of
/// its own: changing the environment of a running program is unsafe.)
pub fn switch() {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        if std::env::var_os(SWITCHED).is_some() {
            return;
        }
        let nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists();
        let safe = landscaper_safe();
        let set = variables(nvidia, safe, &|k| std::env::var_os(k).is_some());
        if set.is_empty() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).envs(set).env(SWITCHED, "1").exec();
        eprintln!("warning: couldn't start again with the webview's switch ({err}); starting without it");
    }
}

#[cfg(target_os = "linux")]
fn landscaper_safe() -> bool {
    std::env::var("LANDSCAPER_WEBVIEW_SAFE").is_ok_and(|v| v.trim() == "1")
}

/// The variables to set: DMA-BUF off for NVIDIA or the safe switch, compositing off for the safe switch, none already set.
#[cfg_attr(all(not(target_os = "linux"), not(test)), expect(dead_code, reason = "Linux only; its test runs everywhere"))]
fn variables(nvidia: bool, safe: bool, is_set: &dyn Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::new();
    if nvidia || safe {
        v.push(("WEBKIT_DISABLE_DMABUF_RENDERER", "1"));
    }
    if safe {
        v.push(("WEBKIT_DISABLE_COMPOSITING_MODE", "1"));
    }
    v.retain(|(k, _)| !is_set(k));
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn nvidia_turns_off_dmabuf_and_the_safe_switch_compositing_too() {
        let none = |_: &str| false;
        assert!(super::variables(false, false, &none).is_empty());
        assert_eq!(super::variables(true, false, &none), [("WEBKIT_DISABLE_DMABUF_RENDERER", "1")]);
        assert_eq!(super::variables(false, true, &none).len(), 2);
        // What the person set stays theirs.
        assert!(super::variables(true, false, &|k| k == "WEBKIT_DISABLE_DMABUF_RENDERER").is_empty());
    }
}
