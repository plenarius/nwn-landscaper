// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The area's light and fog from the editor's "Sky & fog" panel, written into the .are:
//!
//! ```json
//! { "time": "cycle" | "day" | "night", "fogClip": 150,
//!   "sun":  { "ambient": "#50504f", "diffuse": "#d6d6d6", "fog": "#bbbbbb", "fogAmount": 7, "shadows": true },
//!   "moon": { ... },
//!   "sky": "dusk", "skyRow": 3 }
//! ```
//! `sky` and `skyRow` are the area's sky (an installed Landscaper sky, else a row of the module's `skyboxes.2da`), which the
//! export writes as `SkyBox` (crate::skies).
//! Colours are `#rrggbb`; the .are stores them as 0x00BBGGRR.

use anyhow::{Context, Result};
use mg_gff::{Struct, Value};
use serde_json::Value as Json;

fn bgr(hex: &str) -> Result<u32> {
    let rgb = u32::from_str_radix(hex.trim_start_matches('#'), 16).with_context(|| format!("colour '{hex}'"))?;
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    Ok((b << 16) | (g << 8) | r)
}

fn sky(are: &mut Struct, prefix: &str, s: &Json) -> Result<()> {
    let colour = |k: &str| s[k].as_str().with_context(|| format!("{prefix} {k}")).and_then(bgr);
    are.set(&format!("{prefix}AmbientColor"), Value::Dword(colour("ambient")?));
    are.set(&format!("{prefix}DiffuseColor"), Value::Dword(colour("diffuse")?));
    are.set(&format!("{prefix}FogColor"), Value::Dword(colour("fog")?));
    are.set(&format!("{prefix}FogAmount"), Value::Byte(s["fogAmount"].as_u64().unwrap_or(0).min(255) as u8));
    are.set(&format!("{prefix}Shadows"), Value::Byte(u8::from(s["shadows"].as_bool().unwrap_or(false))));
    Ok(())
}

/// Writes `a` into the area `are`.
pub fn apply(are: &mut Struct, a: &Json) -> Result<()> {
    sky(are, "Sun", &a["sun"])?;
    sky(are, "Moon", &a["moon"])?;
    let time = a["time"].as_str().unwrap_or("cycle");
    are.set("DayNightCycle", Value::Byte(u8::from(time == "cycle")));
    are.set("IsNight", Value::Byte(u8::from(time == "night")));
    are.set("FogClipDist", Value::Float(a["fogClip"].as_f64().unwrap_or(150.0) as f32));
    Ok(())
}

fn hex(bgr: i64) -> String {
    let v = bgr as u32;
    format!("#{:02x}{:02x}{:02x}", v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF)
}

/// An area's light and fog as the editor keeps them (`apply`'s inverse), for importing an area.
pub fn read(are: &Struct) -> Json {
    let sky = |prefix: &str| {
        let int = |k: &str| are.integer(&format!("{prefix}{k}")).unwrap_or(0);
        serde_json::json!({
            "ambient": hex(int("AmbientColor")), "diffuse": hex(int("DiffuseColor")), "fog": hex(int("FogColor")),
            "fogAmount": int("FogAmount"), "shadows": int("Shadows") != 0,
        })
    };
    let time = if are.integer("DayNightCycle").unwrap_or(1) != 0 {
        "cycle"
    } else if are.integer("IsNight").unwrap_or(0) != 0 {
        "night"
    } else {
        "day"
    };
    let mut a = serde_json::json!({
        "preset": "imported", "time": time, "fogClip": are.float("FogClipDist").unwrap_or(150.0),
        "sun": sky("Sun"), "moon": sky("Moon"),
    });
    // (Its sky as the Sky & fog panel keeps it: the game's row, which `skies::name_rows` names when it is a Landscaper sky's.)
    if let Some(row) = are.integer("SkyBox").and_then(|r| u8::try_from(r).ok()) {
        a["skyRow"] = row.into();
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_stored_blue_first() {
        assert_eq!(bgr("#112233").unwrap(), 0x332211);
        let mut are = Struct::new(0);
        let s = serde_json::json!({ "ambient": "#ff0000", "diffuse": "#00ff00", "fog": "#0000ff", "fogAmount": 9, "shadows": true });
        apply(&mut are, &serde_json::json!({ "time": "night", "fogClip": 80, "sun": s, "moon": s })).unwrap();
        assert_eq!(are.integer("SunAmbientColor"), Some(0x0000FF));
        assert_eq!(are.integer("MoonFogColor"), Some(0xFF0000));
        assert_eq!((are.integer("IsNight"), are.integer("DayNightCycle"), are.integer("SunFogAmount")), (Some(1), Some(0), Some(9)));
        assert_eq!(are.float("FogClipDist"), Some(80.0));
        // And back.
        let back = read(&are);
        assert_eq!(
            (back["sun"]["ambient"].as_str(), back["moon"]["fog"].as_str(), back["time"].as_str()),
            (Some("#ff0000"), Some("#0000ff"), Some("night"))
        );
        assert!(back.get("skyRow").is_none());
        are.set("SkyBox", Value::Byte(4));
        assert_eq!(read(&are)["skyRow"], 4);
    }
}
