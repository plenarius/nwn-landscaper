// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An area's own settings: what the toolset's Area Properties holds. Name, tag, the four area scripts,
//! PvP and rest, weather, music and ambient sound, load screen, spot and listen modifiers, sky box,
//! comments and local variables. (Light and fog are `atmosphere`; the kind of area is `kind`.)
//!
//! ```json
//! { "name": "Elm Wood", "tag": "elmwood",
//!   "scripts": { "OnEnter": "lay_area_onenter", "OnExit": "...", "OnHeartbeat": "", "OnUserDefined": "..." },
//!   "pvp": 3, "noRest": false,
//!   "weather": { "rain": 0, "snow": 0, "lightning": 0, "wind": 0 },
//!   "music": { "day": 37, "night": 10, "battle": 70, "delay": 90000 },
//!   "ambient": { "day": 62, "night": 62, "dayVol": 127, "nightVol": 127, "env": 0 },
//!   "loadScreen": 0, "spot": 0, "listen": 0, "skyBox": 0, "comments": "",
//!   "vars": [{ "name": "x", "type": "int", "value": 1 }] }
//! ```
//! Whatever is missing keeps what the area has (a new area's own defaults, an imported one's original).
//! The ARE gets the area's fields, the GIT's `AreaProperties` the music and ambient sound.

use anyhow::{Context, Result};
use mg_core::{Gender, Language, LocString};
use mg_gff::{Struct, Value};
use serde_json::Value as Json;

/// The template a new area starts with.
pub fn template() -> Json {
    crate::config::area_template()
}

/// `text` as an English localized string, as the toolset writes a name typed into it.
pub fn loc(text: &str) -> Value {
    Value::LocString(LocString::from_text(Language::ENGLISH, Gender::Male, text.as_bytes().to_vec()))
}

fn int(j: &Json, path: &[&str]) -> Option<i64> {
    let mut v = j;
    for p in path {
        v = v.get(*p)?;
    }
    v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))
}

fn text<'a>(j: &'a Json, path: &[&str]) -> Option<&'a str> {
    let mut v = j;
    for p in path {
        v = v.get(*p)?;
    }
    v.as_str()
}

/// The choices behind the numbers: the game's music, ambient sound, reverb, load screen and sky box tables
/// (rows with a name), as `{ music: [{ row, name }], ... }`.
pub fn lists(game: &mg_rules::GameData) -> Json {
    let one = |table: &str, name_cols: &[&str], strref_col: Option<&str>| -> Vec<Json> {
        let Ok(t) = game.table(table) else { return Vec::new() };
        (0..t.len())
            .filter_map(|row| {
                let from_ref = strref_col
                    .and_then(|c| t.get(row, c))
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .and_then(|v| game.string(mg_core::StrRef(v)))
                    .filter(|s| !s.trim().is_empty());
                let name = from_ref.or_else(|| {
                    name_cols
                        .iter()
                        .filter_map(|c| t.get(row, c))
                        .map(str::trim)
                        .find(|v| !v.is_empty() && *v != "****")
                        .map(str::to_string)
                })?;
                Some(serde_json::json!({ "row": row, "name": name }))
            })
            .collect()
    };
    serde_json::json!({
        "music": one("ambientmusic", &["Resource"], Some("Description")),
        "ambient": one("ambientsound", &["Resource"], Some("Description")),
        "env": one("soundeax", &["Label"], Some("Description")),
        "loadScreen": one("loadscreens", &["Label", "ScriptingName"], Some("StrRef")),
        "skyBox": one("skyboxes", &["LABEL"], None),
        "traps": one("traps", &["Label"], Some("Name")),
    })
}

/// Local variables (`[{name, type: int|float|string, value}]`) as a GFF `VarTable`.
pub fn var_table(vars: &[Json]) -> Option<Value> {
    let list: Vec<Struct> = vars
        .iter()
        .filter_map(|v| {
            let name = v["name"].as_str().filter(|n| !n.is_empty() && n.len() <= 32)?;
            let mut s = Struct::new(0);
            s.set("Name", Value::String(name.as_bytes().to_vec()));
            match v["type"].as_str().unwrap_or("int") {
                "float" => {
                    s.set("Type", Value::Dword(2));
                    s.set("Value", Value::Float(v["value"].as_f64()? as f32));
                }
                "string" => {
                    s.set("Type", Value::Dword(3));
                    s.set("Value", Value::String(v["value"].as_str().unwrap_or("").as_bytes().to_vec()));
                }
                _ => {
                    s.set("Type", Value::Dword(1));
                    s.set("Value", Value::Int(v["value"].as_i64()? as i32));
                }
            }
            Some(s)
        })
        .collect();
    (!list.is_empty()).then_some(Value::List(list))
}

/// Writes `s` into the area's ARE and GIT root.
pub fn apply(are: &mut Struct, git: &mut Struct, s: &Json) -> Result<()> {
    if let Some(n) = text(s, &["name"]).map(str::trim).filter(|n| !n.is_empty()) {
        are.set("Name", loc(n));
    }
    if let Some(t) = text(s, &["tag"]).map(str::trim).filter(|t| !t.is_empty()) {
        anyhow::ensure!(t.len() <= 32 && !t.contains(char::is_whitespace), "the area's tag is up to 32 characters, no spaces");
        are.set("Tag", Value::String(t.as_bytes().to_vec()));
    }
    for (key, label) in [("OnEnter", "OnEnter"), ("OnExit", "OnExit"), ("OnHeartbeat", "OnHeartbeat"), ("OnUserDefined", "OnUserDefined")] {
        if let Some(r) = text(s, &["scripts", key]) {
            // (Empty: no script. The game's resrefs are lower case; an imported area's `Lay_Enter` is the same script.)
            let r = r.to_ascii_lowercase();
            anyhow::ensure!(
                r.is_empty() || crate::newbp::resref_ok(&r),
                "{key}: '{r}' isn't a script name (up to 16 letters, digits or _)"
            );
            are.set(label, Value::resref(mg_core::ResRef::from_str(&r).with_context(|| format!("{key} {r}"))?));
        }
    }
    let clamp = |v: i64, lo: i64, hi: i64| v.clamp(lo, hi);
    if let Some(v) = int(s, &["pvp"]) {
        are.set("PlayerVsPlayer", Value::Byte(clamp(v, 0, 3) as u8));
    }
    if let Some(v) = s.get("noRest").and_then(Json::as_bool) {
        are.set("NoRest", Value::Byte(u8::from(v)));
    }
    for (key, label, hi) in
        [("rain", "ChanceRain", 100), ("snow", "ChanceSnow", 100), ("lightning", "ChanceLightning", 100), ("wind", "WindPower", 2)]
    {
        if let Some(v) = int(s, &["weather", key]) {
            are.set(label, Value::Int(clamp(v, 0, hi) as i32));
        }
    }
    if let Some(v) = int(s, &["loadScreen"]) {
        are.set("LoadScreenID", Value::Word(clamp(v, 0, 65535) as u16));
    }
    for (key, label) in [("spot", "ModSpotCheck"), ("listen", "ModListenCheck")] {
        if let Some(v) = int(s, &[key]) {
            are.set(label, Value::Int(clamp(v, -127, 127) as i32));
        }
    }
    if let Some(v) = int(s, &["skyBox"]) {
        are.set("SkyBox", Value::Byte(clamp(v, 0, 255) as u8));
    }
    if let Some(c) = text(s, &["comments"]) {
        are.set("Comments", Value::String(c.as_bytes().to_vec()));
    }
    if let Some(vars) = s.get("vars").and_then(Json::as_array) {
        match var_table(vars) {
            Some(t) => are.set("VarTable", t),
            None => {
                are.remove("VarTable");
            }
        }
    }
    // Music and ambient sound live in the GIT's AreaProperties.
    let mut props = git.child("AreaProperties").cloned().unwrap_or_else(|| Struct::new(100));
    for (path, label, hi) in [
        (["music", "day"], "MusicDay", 1000),
        (["music", "night"], "MusicNight", 1000),
        (["music", "battle"], "MusicBattle", 1000),
        (["music", "delay"], "MusicDelay", 1_000_000),
        (["ambient", "day"], "AmbientSndDay", 1000),
        (["ambient", "night"], "AmbientSndNight", 1000),
        (["ambient", "dayVol"], "AmbientSndDayVol", 127),
        (["ambient", "nightVol"], "AmbientSndNitVol", 127),
        (["ambient", "env"], "EnvAudio", 1000),
    ] {
        if let Some(v) = int(s, &path) {
            props.set(label, Value::Int(clamp(v, 0, hi) as i32));
        }
    }
    git.set("AreaProperties", Value::Struct(props));
    Ok(())
}

/// An area's settings as `apply` takes them (for an imported area, so the editor shows what it has).
/// `name_text` is the area's name read as text (a talk table name is not rewritten unless changed).
pub fn read(are: &Struct, git: &Struct, name_text: Option<String>) -> Json {
    let i = |l: &str| are.integer(l).unwrap_or(0);
    let s = |l: &str| are.string(l).map_or_default(|b| String::from_utf8_lossy(b).into_owned());
    let r = |l: &str| are.resref(l).map_or_default(|r| r.to_string());
    let p = |l: &str| git.child("AreaProperties").and_then(|p| p.integer(l)).unwrap_or(0);
    let vars: Vec<Json> = are
        .list("VarTable")
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| {
            let name = String::from_utf8_lossy(v.string("Name")?).into_owned();
            let (ty, value) = match v.integer("Type")? {
                1 => ("int", serde_json::json!(v.integer("Value")?)),
                2 => ("float", serde_json::json!(v.float("Value")?)),
                3 => ("string", serde_json::json!(String::from_utf8_lossy(v.string("Value")?))),
                _ => return None,
            };
            Some(serde_json::json!({ "name": name, "type": ty, "value": value }))
        })
        .collect();
    // Variables of kinds the editor can't hold (objects, locations) stay as they are: say nothing about them.
    let all_vars = are.list("VarTable").is_none_or(|l| l.iter().all(|v| matches!(v.integer("Type"), Some(1..=3))));
    let mut out = serde_json::json!({
        "tag": s("Tag"),
        "nameText": name_text,
        "scripts": { "OnEnter": r("OnEnter"), "OnExit": r("OnExit"), "OnHeartbeat": r("OnHeartbeat"), "OnUserDefined": r("OnUserDefined") },
        "pvp": i("PlayerVsPlayer"), "noRest": i("NoRest") != 0,
        "weather": { "rain": i("ChanceRain"), "snow": i("ChanceSnow"), "lightning": i("ChanceLightning"), "wind": i("WindPower") },
        "music": { "day": p("MusicDay"), "night": p("MusicNight"), "battle": p("MusicBattle"), "delay": p("MusicDelay") },
        "ambient": {
            "day": p("AmbientSndDay"), "night": p("AmbientSndNight"), "dayVol": p("AmbientSndDayVol"), "nightVol": p("AmbientSndNitVol"),
            "env": p("EnvAudio"),
        },
        "loadScreen": i("LoadScreenID"), "spot": i("ModSpotCheck"), "listen": i("ModListenCheck"), "skyBox": i("SkyBox"),
        "comments": s("Comments"),
    });
    if all_vars {
        out["vars"] = Json::Array(vars);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_go_into_the_area_and_come_back() {
        let (mut are, mut git) = (Struct::new(0), Struct::new(0));
        let s = serde_json::json!({
            "name": "Elm Wood", "tag": "elmwood",
            "scripts": { "OnEnter": "area_enter", "OnExit": "area_exit", "OnHeartbeat": "", "OnUserDefined": "area_user" },
            "pvp": 3, "noRest": true, "weather": { "rain": 40, "snow": 0, "lightning": 5, "wind": 2 },
            "music": { "day": 37, "night": 10, "battle": 70, "delay": 90000 },
            "ambient": { "day": 62, "night": 63, "dayVol": 127, "nightVol": 90, "env": 8 },
            "loadScreen": 63, "spot": -4, "listen": 5, "skyBox": 11, "comments": "hi",
            "vars": [{ "name": "ZONE", "type": "int", "value": 1 }, { "name": "S", "type": "string", "value": "a" }],
        });
        apply(&mut are, &mut git, &s).unwrap();
        assert_eq!(are.string("Tag"), Some(&b"elmwood"[..]));
        assert_eq!(
            (are.integer("PlayerVsPlayer"), are.integer("NoRest"), are.integer("ChanceRain"), are.integer("WindPower")),
            (Some(3), Some(1), Some(40), Some(2))
        );
        assert_eq!((are.integer("ModSpotCheck"), are.integer("LoadScreenID"), are.integer("SkyBox")), (Some(-4), Some(63), Some(11)));
        let back = read(&are, &git, None);
        for k in
            ["tag", "scripts", "pvp", "noRest", "weather", "music", "ambient", "loadScreen", "spot", "listen", "skyBox", "comments", "vars"]
        {
            assert_eq!(back[k], s[k], "{k}");
        }
    }

    #[test]
    fn what_is_missing_keeps_what_the_area_has() {
        let (mut are, mut git) = (Struct::new(0), Struct::new(0));
        are.set("ChanceRain", Value::Int(77));
        are.set("OnEnter", Value::resref(mg_core::ResRef::from_str("keep").unwrap()));
        apply(&mut are, &mut git, &serde_json::json!({ "pvp": 1 })).unwrap();
        assert_eq!((are.integer("ChanceRain"), are.integer("PlayerVsPlayer")), (Some(77), Some(1)));
        assert_eq!(are.resref("OnEnter").map(|r| r.to_string()), Some("keep".into()));
    }

    #[test]
    fn bad_scripts_and_tags_are_refused() {
        let (mut are, mut git) = (Struct::new(0), Struct::new(0));
        assert!(apply(&mut are, &mut git, &serde_json::json!({ "scripts": { "OnEnter": "this_is_far_too_long_for_a_resref" } })).is_err());
        assert!(apply(&mut are, &mut git, &serde_json::json!({ "tag": "has space" })).is_err());
    }

    #[test]
    fn a_project_template_file_is_a_template() {
        // A whole file, as a project's LANDSCAPER_AREA_TEMPLATE would hold it.
        const FILE: &str = r#"{
  "_note": "What a new area of this project starts with.",
  "scripts": { "OnEnter": "my_area_enter", "OnExit": "my_area_exit", "OnHeartbeat": "", "OnUserDefined": "my_area_user" },
  "pvp": 3,
  "noRest": false,
  "weather": { "rain": 0, "snow": 0, "lightning": 0, "wind": 0 },
  "music": { "day": 0, "night": 0, "battle": 0, "delay": 0 },
  "ambient": { "day": 0, "night": 0, "dayVol": 127, "nightVol": 127, "env": 0 },
  "loadScreen": 0, "spot": 0, "listen": 0, "skyBox": 0, "comments": "", "vars": []
}"#;
        let t = crate::config::template_from(Some(FILE));
        assert_eq!(t["scripts"]["OnEnter"], "my_area_enter");
        assert_eq!(t["pvp"], 3);
        let (mut are, mut git) = (Struct::new(0), Struct::new(0));
        apply(&mut are, &mut git, &t).unwrap();
        assert_eq!(are.resref("OnEnter").map(|r| r.to_string()), Some("my_area_enter".into()));
    }
}
