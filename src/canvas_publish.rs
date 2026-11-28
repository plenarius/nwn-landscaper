// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Publishing a canvas (docs/DESIGN-canvas-cut.md, step 7): which pieces changed since the last publish and why, and whether a
//! piece's export still matches the cut its neighbours were exported with. The routes are serve/canvases.rs.
//!
//! Each piece's area files are summed in parts ([`Print`]), so a change can be told apart: its own ground, its edge (the halo,
//! its neighbours' ground and objects), what stands in it, its crossings (one sum per neighbour) and its settings. The sums of
//! what was last published are kept in the canvas's folder ([`PUBLISHED`]); the sums of the canvas as it is now come from
//! cutting it again, and those of the last export from its files.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::canvas::{Canvas, TILE_SAMPLES};
use crate::cut::{self, Piece};

/// The record of the last publish of each piece, in the canvas's folder.
pub const PUBLISHED: &str = "published.json";

/// A piece's area files summed in parts (hex SHA-256s).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Print {
    /// Its own samples (heights, paint, blend, chasm), its palette and its water.
    pub ground: String,
    /// The halo's samples and the objects standing on it.
    pub edge: String,
    /// The objects, things (not crossings), entrances, particles and lights in it.
    pub objects: String,
    /// Its crossings' triggers and waypoints, by the neighbour they lead to or come from.
    pub crossings: BTreeMap<String, String>,
    /// The rest of the header: settings, atmosphere, shadow, kind.
    pub settings: String,
}

/// Header keys that name files or are counted by the export, not the landscape.
const NOT_SETTINGS: [&str; 22] = [
    "name",
    "module",
    "tagPrefix",
    "tiles",
    "tilesY",
    "spacing",
    "width",
    "height",
    "halo",
    "data",
    "materials",
    "blend",
    "entranceNextId",
    "palette",
    "water",
    "chasm",
    "objects",
    "things",
    "entrances",
    "ambience",
    "lights",
    "imported",
];

/// `v` with every object's keys sorted, as text: a header read back from its file sums the same as the one written.
fn canonical(v: &Value) -> String {
    fn sorted(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let s: BTreeMap<&String, Value> = m.iter().map(|(k, v)| (k, sorted(v))).collect();
                Value::Object(s.into_iter().map(|(k, v)| (k.clone(), v)).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
            v => v.clone(),
        }
    }
    sorted(v).to_string()
}

fn hex(h: Sha256) -> String {
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The neighbour a crossing's thing leads to or comes from: `To <piece>` on a trigger, `From <piece>` on a waypoint (as
/// `cut::area` names them).
fn crossing_neighbour(t: &Value) -> Option<String> {
    let name = t["name"].as_str()?;
    name.strip_prefix("To ").or_else(|| name.strip_prefix("From ")).map(str::to_string)
}

/// The [`Print`] of an area: its `header` as the engine reads it and its grids (heights 4 bytes a sample, materials one, blend
/// two; empty: none), all counted with the halo.
pub fn print(header: &Value, heights: &[u8], materials: &[u8], blend: &[u8]) -> Result<Print> {
    let u = |k: &str| header[k].as_u64().map(|v| v as usize);
    let tiles = u("tiles").context("tiles")?;
    let (nx, ny) = (tiles * TILE_SAMPLES + 1, u("tilesY").unwrap_or(tiles) * TILE_SAMPLES + 1);
    let halo = u("halo").unwrap_or(0);
    let (fw, fh) = (nx + 2 * halo, ny + 2 * halo);
    let n = fw * fh;
    anyhow::ensure!(heights.len() == n * 4, "{} bytes of heights, {} expected", heights.len(), n * 4);
    let materials = if materials.len() == n { materials } else { &[] };
    // (All weights 0 is no layer, as the export writes it.)
    let blend = if blend.len() == n * 2 && blend.chunks(2).any(|p| p[1] > 0) { blend } else { &[] };
    let chasm = crate::heightmap::chasm_of(&header["chasm"], n)?;
    let (mut ground, mut edge) = (Sha256::new(), Sha256::new());
    ground.update(format!("{nx}x{ny}"));
    ground.update(canonical(&header["palette"]));
    // A water body only in the halo is a neighbour's: its edge.
    for w in header["water"].as_array().into_iter().flatten() {
        let b: Vec<usize> = w["bounds"].as_array().into_iter().flatten().filter_map(|v| v.as_u64().map(|v| v as usize)).collect();
        let inside = b.len() != 4 || (b[0] < halo + nx && b[2] >= halo && b[1] < halo + ny && b[3] >= halo);
        (if inside { &mut ground } else { &mut edge }).update(canonical(w));
    }
    for k in 0..n {
        let (a, b) = (k % fw, k / fw);
        let own = (halo..halo + nx).contains(&a) && (halo..halo + ny).contains(&b);
        let h = if own { &mut ground } else { &mut edge };
        h.update(&heights[k * 4..k * 4 + 4]);
        if !materials.is_empty() {
            h.update([materials[k]]);
        }
        if !blend.is_empty() {
            h.update(&blend[k * 2..k * 2 + 2]);
        }
        h.update([u8::from(chasm[k])]);
    }
    // Objects past the piece's own rectangle stand on its halo (`cut::area`).
    let (w_m, h_m) = ((nx - 1) as f64 * 0.625, (ny - 1) as f64 * 0.625);
    let mut objects = Sha256::new();
    for o in header["objects"].as_array().into_iter().flatten() {
        let (x, y) = (o["x"].as_f64().unwrap_or(0.0), o["y"].as_f64().unwrap_or(0.0));
        let on_halo = x < 0.0 || y < 0.0 || x > w_m || y > h_m;
        (if on_halo { &mut edge } else { &mut objects }).update(canonical(o));
    }
    let mut by_neighbour: BTreeMap<String, Sha256> = BTreeMap::new();
    for t in header["things"].as_array().into_iter().flatten() {
        let crossing = t["id"].as_u64().is_some_and(|id| id >= u64::from(cut::CROSSING_IDS));
        match crossing.then(|| crossing_neighbour(t)).flatten() {
            Some(nb) => by_neighbour.entry(nb).or_default().update(canonical(t)),
            None => objects.update(canonical(t)),
        }
    }
    for k in ["entrances", "ambience", "lights"] {
        objects.update(canonical(&header[k]));
    }
    let rest: serde_json::Map<String, Value> = header
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| !NOT_SETTINGS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut settings = Sha256::new();
    settings.update(canonical(&Value::Object(rest)));
    Ok(Print {
        ground: hex(ground),
        edge: hex(edge),
        objects: hex(objects),
        crossings: by_neighbour.into_iter().map(|(k, h)| (k, hex(h))).collect(),
        settings: hex(settings),
    })
}

/// The [`Print`] of `name`'s last export in `dir` (the exports folder), when it was exported into module `module`; `None` when it
/// wasn't, or its files are gone.
pub fn exported_print(dir: &std::path::Path, name: &str, module: &str) -> Option<Print> {
    let header: Value = serde_json::from_slice(&crate::atomic::read(&dir.join(format!("{name}.json"))).ok()?).ok()?;
    if header["module"].as_str() != Some(module) {
        return None;
    }
    let file = |k: &str| header[k].as_str().map_or_else(|| Some(Vec::new()), |f| crate::atomic::read(&dir.join(f)).ok());
    print(&header, &file("data")?, &file("materials")?, &file("blend")?).ok()
}

/// The canvas cut now: its pieces and each one's [`Print`] (or why it can't be cut), as `serve/canvases.rs` exports them.
pub struct Now {
    pub pieces: Vec<Piece>,
    pub prints: BTreeMap<String, Result<Print, String>>,
}

/// Canvas `c` cut, each piece summed. Holds the canvas's lock while it reads.
pub fn now(c: &Canvas) -> Result<Now> {
    let (header, ground) = {
        let lock = crate::canvas::lock(&c.dir);
        let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let header = c.header()?;
        let ground = cut::Ground::load(c, &header)?;
        (header, ground)
    };
    let pieces = cut::pieces(&header);
    let (crossings, _) = cut::crossings(&c.name, &header, &ground, &pieces);
    let prints = pieces
        .iter()
        .map(|p| {
            let made = cut::area(&c.name, &header, &ground, p, &crossings, cut::HALO)
                .and_then(|a| print(&a.header, &a.heights, &a.materials, &a.blend))
                .map_err(|e| format!("{e:#}"));
            (p.name.clone(), made)
        })
        .collect();
    Ok(Now { pieces, prints })
}

/// What was last published of one piece.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sent {
    pub id: u64,
    /// Its rectangle on the canvas then, tiles: x, y, w, h.
    pub rect: [usize; 4],
    /// When (Unix seconds).
    pub at: u64,
    /// The export id sent (docs/REVIEW-SERVICE.md).
    #[serde(default)]
    pub export: String,
    pub print: Print,
}

/// The record of a canvas's publishes, a piece by name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Published {
    #[serde(default)]
    pub pieces: BTreeMap<String, Sent>,
}

impl Published {
    /// Canvas `c`'s record (none yet: empty). One that can't be read is an error, not "never published": every piece would tick.
    pub fn read(c: &Canvas) -> Result<Published> {
        match crate::atomic::read(&c.dir.join(PUBLISHED)) {
            Ok(d) => serde_json::from_slice(&d).with_context(|| format!("canvas {}'s {PUBLISHED} doesn't parse", c.name)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Published::default()),
            Err(e) => Err(e).with_context(|| format!("reading canvas {}'s {PUBLISHED}", c.name)),
        }
    }

    /// Adds `sent` (replacing those pieces' last) and writes the record, under the canvas's lock.
    pub fn record(c: &Canvas, sent: Vec<(String, Sent)>) -> Result<()> {
        let lock = crate::canvas::lock(&c.dir);
        let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut p = Published::read(c)?;
        p.pieces.extend(sent);
        crate::atomic::write(&c.dir.join(PUBLISHED), serde_json::to_string_pretty(&p)?.as_bytes())
    }
}

/// Pieces whose halo reaches `p`'s: their ground or objects show in its edge scenery (5 tiles).
fn near(p: &Piece, q: &Piece) -> bool {
    let reach = cut::HALO / TILE_SAMPLES;
    p.name != q.name && q.x < p.x + p.w + reach && p.x < q.x + q.w + reach && q.y < p.y + p.h + reach && p.y < q.y + q.h + reach
}

fn why(kind: &str, text: String) -> Value {
    json!({ "kind": kind, "text": text })
}

/// Which pieces changed since the last publish and why, from the canvas cut now (`now`), each piece's last export (`exported`,
/// `None`: not exported into the canvas's module) and the record of the last publish:
///
/// `{ pieces: [{ name, id, exported, stale, published, changed, why: [{ kind, text, from? }] }], removed: [{ name, id,
/// renamed? }], ticked }`. `kind` is `new`, `ground`, `objects`, `edge` (with `from`, the neighbours whose change shows in it),
/// `crossings` (with `from`), `settings`, `moved` (its corner moved: numbers in scripts no longer point at the same place) or
/// `refused` (the cut refuses it). `stale`: the last export isn't the canvas now (export it again before publishing). `ticked`
/// are the changed pieces, which Publish sends when not told which.
pub fn changes(now: &Now, exported: &BTreeMap<String, Option<Print>>, published: &Published) -> Value {
    let rect = |p: &Piece| [p.x, p.y, p.w, p.h];
    // (Pieces whose own ground or objects changed, for naming where an edge's change comes from.)
    let own_changed: BTreeSet<&str> = now
        .pieces
        .iter()
        .filter(|p| match (now.prints.get(&p.name), published.pieces.get(&p.name)) {
            (Some(Ok(n)), Some(s)) => n.ground != s.print.ground || n.objects != s.print.objects || rect(p) != s.rect,
            _ => true,
        })
        .map(|p| p.name.as_str())
        .collect();
    let mut rows = Vec::new();
    let mut ticked = Vec::new();
    for p in &now.pieces {
        let mut reasons = Vec::new();
        let print = now.prints.get(&p.name);
        let exp = exported.get(&p.name).cloned().flatten();
        let stale = match print {
            Some(Ok(n)) => exp.as_ref() != Some(n),
            _ => true,
        };
        match (print, published.pieces.get(&p.name)) {
            (Some(Err(e)), _) => reasons.push(why("refused", e.clone())),
            (None, _) => {}
            (Some(Ok(_)), None) => reasons.push(why("new", "never published".into())),
            (Some(Ok(n)), Some(s)) => {
                let o = &s.print;
                if [p.x, p.y] != [s.rect[0], s.rect[1]] {
                    reasons.push(why(
                        "moved",
                        format!(
                            "its corner moved from tile {},{} to {},{}: a location written as numbers in a script now points elsewhere",
                            s.rect[0], s.rect[1], p.x, p.y
                        ),
                    ));
                }
                if n.ground != o.ground {
                    reasons.push(why("ground", "its ground changed".into()));
                }
                if n.objects != o.objects {
                    reasons.push(why("objects", "what stands in it changed".into()));
                }
                if n.edge != o.edge {
                    let from: Vec<&str> = now
                        .pieces
                        .iter()
                        .filter(|q| near(p, q) && own_changed.contains(q.name.as_str()))
                        .map(|q| q.name.as_str())
                        .collect();
                    let text = if from.is_empty() {
                        "the ground at its edge changed".to_string()
                    } else {
                        format!("its edge shows {}'s change", from.join(", "))
                    };
                    reasons.push(json!({ "kind": "edge", "text": text, "from": from }));
                }
                if n.crossings != o.crossings {
                    let from: Vec<&String> = n
                        .crossings
                        .keys()
                        .chain(o.crossings.keys())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .filter(|k| n.crossings.get(*k) != o.crossings.get(*k))
                        .collect();
                    let names: Vec<&str> = from.iter().map(|s| s.as_str()).collect();
                    reasons.push(
                        json!({ "kind": "crossings", "text": format!("its crossings with {} changed", names.join(", ")), "from": from }),
                    );
                }
                if n.settings != o.settings {
                    reasons.push(why("settings", "its settings changed".into()));
                }
            }
        }
        let changed = !reasons.is_empty();
        if changed {
            ticked.push(p.name.clone());
        }
        let sent = published.pieces.get(&p.name);
        rows.push(json!({
            "name": p.name, "id": p.id, "exported": exp.is_some(), "stale": stale,
            "published": sent.map(|s| s.at), "changed": changed, "why": reasons,
        }));
    }
    let removed: Vec<Value> = published
        .pieces
        .iter()
        .filter(|(n, _)| !now.pieces.iter().any(|p| &p.name == *n))
        .map(|(n, s)| {
            let mut r = json!({ "name": n, "id": s.id });
            if let Some(p) = now.pieces.iter().find(|p| p.id == s.id) {
                r["renamed"] = p.name.clone().into();
            }
            r
        })
        .collect();
    json!({ "pieces": rows, "removed": removed, "ticked": ticked })
}

/// The pieces left out of `chosen` whose crossings with a chosen piece aren't in their last export as the cut makes them now
/// (or that were never exported into the canvas's module): exporting only `chosen` would leave a trigger leading to a waypoint
/// that isn't there, or one that leads to a waypoint no longer there. Sorted by name.
pub fn export_first(now: &Now, chosen: &[String], exported: &BTreeMap<String, Option<Print>>) -> Vec<String> {
    let mut out = Vec::new();
    for p in now.pieces.iter().filter(|p| !chosen.contains(&p.name)) {
        let Some(Ok(n)) = now.prints.get(&p.name) else { continue };
        let exp = exported.get(&p.name).cloned().flatten();
        let stale = chosen.iter().any(|c| {
            let fresh = n.crossings.get(c);
            match &exp {
                Some(e) => fresh != e.crossings.get(c),
                None => fresh.is_some(),
            }
        });
        if stale {
            out.push(p.name.clone());
        }
    }
    out
}

/// The pieces left out of `chosen` that share a crossing with one of them and whose last publish (none: never published) had
/// different crossings with it: sending `chosen` alone leaves a crossing leading to a waypoint the live area lacks.
pub fn send_too(now: &Now, chosen: &[String], published: &Published) -> Vec<String> {
    let mut out = Vec::new();
    for p in now.pieces.iter().filter(|p| !chosen.contains(&p.name)) {
        let Some(Ok(n)) = now.prints.get(&p.name) else { continue };
        let sent = published.pieces.get(&p.name).map(|s| &s.print.crossings);
        if chosen.iter().any(|c| n.crossings.contains_key(c) && sent.and_then(|s| s.get(c)) != n.crossings.get(c)) {
            out.push(p.name.clone());
        }
    }
    out
}

/// A canvas cut now, its pieces' last exports and the record of its publishes: what publishing, a package and a trial choose
/// their pieces from.
pub struct State {
    pub now: Now,
    pub exported: BTreeMap<String, Option<Print>>,
    pub published: Published,
}

impl State {
    /// Every piece's name.
    pub fn all(&self) -> Vec<String> {
        self.now.pieces.iter().map(|p| p.name.clone()).collect()
    }

    /// The pieces to send: `only`, or with none named `default`. Each must be on the canvas and exported into its module; one
    /// whose export is older than the canvas stops a real send (`strict`, since its crossings may not match the others'), else
    /// is a warning. The names and the warnings (`{ kind, why, pieces? | piece? }`).
    pub fn ready(&self, only: &[String], default: Vec<String>, strict: bool) -> Result<(Vec<String>, Vec<Value>)> {
        let names = if only.is_empty() { default } else { only.to_vec() };
        let mut stale = Vec::new();
        for n in &names {
            anyhow::ensure!(self.now.pieces.iter().any(|p| &p.name == n), "the canvas has no piece {n}");
            let Some(exp) = self.exported.get(n).cloned().flatten() else {
                anyhow::bail!("{n} hasn't been exported with the canvas: export it first");
            };
            if self.now.prints.get(n).and_then(|r| r.as_ref().ok()) != Some(&exp) {
                stale.push(n.clone());
            }
        }
        let mut warnings = Vec::new();
        if !stale.is_empty() {
            let why = format!("{} changed since {} last export: export again first", stale.join(", "), its(&stale));
            anyhow::ensure!(!strict, "{why}");
            warnings.push(json!({ "kind": "stale", "pieces": stale, "why": why }));
        }
        Ok((names, warnings))
    }

    /// What a publish sends ([`State::ready`], by default the pieces changed since the last publish) and its warnings: the
    /// neighbours whose live crossings with them differ (`send_too`), pieces no longer on the canvas (`removed`), and sent
    /// pieces whose corner moved (`moved`).
    pub fn to_publish(&self, only: &[String], dry: bool) -> Result<(Vec<String>, Vec<Value>)> {
        let changed = changes(&self.now, &self.exported, &self.published);
        let ticked: Vec<String> = serde_json::from_value(changed["ticked"].clone()).unwrap_or_default();
        anyhow::ensure!(
            !only.is_empty() || !ticked.is_empty(),
            "no piece changed since the last publish: name the pieces to send them anyway"
        );
        let (names, mut warnings) = self.ready(only, ticked, !dry)?;
        let also = send_too(&self.now, &names, &self.published);
        if !also.is_empty() {
            warnings.push(json!({ "kind": "send_too", "pieces": also,
                "why": format!("{} crossings with these pieces aren't live yet: send {} too, or they lead nowhere", its(&also), them(&also)) }));
        }
        for r in changed["removed"].as_array().into_iter().flatten() {
            let n = r["name"].as_str().unwrap_or_default();
            let why = match r["renamed"].as_str() {
                Some(to) => {
                    format!("{n} is now called {to}: to the module that is a new area, and {n}'s stays until a reviewer removes it")
                }
                None => format!("{n} is no longer on the canvas: its area stays in the module until a reviewer removes it"),
            };
            warnings.push(json!({ "kind": "removed", "piece": n, "why": why }));
        }
        for row in changed["pieces"].as_array().into_iter().flatten().filter(|r| names.iter().any(|n| r["name"] == n.as_str())) {
            for w in row["why"].as_array().into_iter().flatten().filter(|w| w["kind"] == "moved") {
                let n = row["name"].as_str().unwrap_or_default();
                warnings.push(json!({ "kind": "moved", "piece": n, "why": format!("{n}: {}", w["text"].as_str().unwrap_or_default()) }));
            }
        }
        Ok((names, warnings))
    }

    /// What a trial on dev sends (every piece unless told, refused when one's export is older than the canvas), and a warning
    /// naming the pieces left out that the sent ones cross into: those crossings lead to dev's live copy.
    pub fn to_try(&self, only: &[String]) -> Result<(Vec<String>, Vec<Value>)> {
        let (names, mut warnings) = self.ready(only, self.all(), true)?;
        let left: Vec<String> = self
            .now
            .pieces
            .iter()
            .filter(|p| !names.contains(&p.name))
            .filter(|p| {
                self.now
                    .prints
                    .get(&p.name)
                    .and_then(|r| r.as_ref().ok())
                    .is_some_and(|pr| names.iter().any(|n| pr.crossings.contains_key(n)))
            })
            .map(|p| p.name.clone())
            .collect();
        if !left.is_empty() {
            warnings.push(json!({ "kind": "left_out", "pieces": left,
                "why": format!("crossings into {} lead to dev's live copy, which may not have their waypoints", left.join(", ")) }));
        }
        Ok((names, warnings))
    }
}

fn its(names: &[String]) -> &'static str {
    if names.len() == 1 { "its" } else { "their" }
}

fn them(names: &[String]) -> &'static str {
    if names.len() == 1 { "it" } else { "them" }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3 by 3 tile area with a 1-tile halo (16 samples): heights `f(a, b)` over the whole grid, unpainted.
    fn area(f: impl Fn(usize, usize) -> f32, things: Value, objects: Value) -> (Value, Vec<u8>) {
        let fw = 49 + 32;
        let heights = (0..fw * fw).flat_map(|k| f(k % fw, k / fw).to_le_bytes()).collect();
        let h = json!({ "name": "rivl01", "tiles": 3, "tilesY": 3, "spacing": 0.625, "width": 49, "height": 49, "halo": 16,
                        "module": "rivl", "tagPrefix": "rivl", "things": things, "objects": objects });
        (h, heights)
    }

    fn crossing(id: u64, to: &str, x: f64) -> Value {
        json!({ "id": id, "kind": "trigger", "x": x, "y": 1.0, "tag": "rivl_x1_1e", "name": format!("To {to}") })
    }

    #[test]
    fn a_print_tells_the_ground_from_the_edge_objects_and_crossings() {
        let base = || area(|_, _| 1.0, json!([crossing(1_000_000_000, "rivl02", 29.0)]), json!([{ "model": "tree", "x": 5.0, "y": 5.0 }]));
        let (h, hs) = base();
        let p = print(&h, &hs, &[], &[]).unwrap();
        // The same files read back (keys in any order) sum the same; the export's own keys don't count.
        let mut again: Value = serde_json::from_str(&serde_json::to_string_pretty(&h).unwrap()).unwrap();
        again["data"] = "rivl01.f32".into();
        again["entranceNextId"] = 4.into();
        assert_eq!(print(&again, &hs, &[], &[]).unwrap(), p);

        // A raised sample in the middle is its ground; on the halo, its edge.
        let (h2, hs2) = area(|a, b| if (a, b) == (40, 40) { 3.0 } else { 1.0 }, h["things"].clone(), h["objects"].clone());
        let q = print(&h2, &hs2, &[], &[]).unwrap();
        assert!(q.ground != p.ground && q.edge == p.edge && q.objects == p.objects);
        let (h3, hs3) = area(|a, b| if (a, b) == (2, 2) { 3.0 } else { 1.0 }, h["things"].clone(), h["objects"].clone());
        let q = print(&h3, &hs3, &[], &[]).unwrap();
        assert!(q.ground == p.ground && q.edge != p.edge);

        // An object on the halo is edge scenery; one inside is the piece's own.
        let mut h4 = h.clone();
        h4["objects"] = json!([{ "model": "tree", "x": -3.0, "y": 5.0 }]);
        let q = print(&h4, &hs, &[], &[]).unwrap();
        assert!(q.edge != p.edge && q.objects != p.objects && q.ground == p.ground);

        // A crossing is summed by its neighbour, apart from the piece's own things.
        let mut h5 = h.clone();
        h5["things"] = json!([crossing(1_000_000_000, "rivl02", 28.0)]);
        let q = print(&h5, &hs, &[], &[]).unwrap();
        assert_eq!(q.crossings.keys().collect::<Vec<_>>(), ["rivl02"]);
        assert!(q.crossings != p.crossings && q.objects == p.objects);
        // A pond only in the halo is the neighbour's, one reaching in is the piece's own.
        let mut h7 = h.clone();
        h7["water"] = json!([{ "i": 2, "j": 2, "level": 1.5, "bounds": [0, 0, 10, 10] }]);
        let q = print(&h7, &hs, &[], &[]).unwrap();
        assert!(q.edge != p.edge && q.ground == p.ground);
        h7["water"] = json!([{ "i": 2, "j": 2, "level": 1.5, "bounds": [0, 0, 20, 20] }]);
        let q = print(&h7, &hs, &[], &[]).unwrap();
        assert!(q.edge == p.edge && q.ground != p.ground);
        let mut h6 = h.clone();
        h6["settings"] = json!({ "name": "Riverford" });
        assert!(print(&h6, &hs, &[], &[]).unwrap().settings != p.settings);
    }

    fn piece(id: u64, x: usize) -> Piece {
        Piece { id, name: format!("rivl{id:02}"), x, y: 0, w: 8, h: 8, json: Value::Null }
    }

    fn p(tag: &str) -> Print {
        Print { ground: tag.into(), edge: tag.into(), objects: tag.into(), crossings: BTreeMap::new(), settings: tag.into() }
    }

    fn sent(id: u64, x: usize, print: Print) -> Sent {
        Sent { id, rect: [x, 0, 8, 8], at: 1, export: String::new(), print }
    }

    /// Three pieces in a row, all published as `a`.
    fn three() -> (Vec<Piece>, Published) {
        let pieces = vec![piece(1, 0), piece(2, 8), piece(3, 16)];
        let published = Published { pieces: (1..=3).map(|i| (format!("rivl{i:02}"), sent(i, (i as usize - 1) * 8, p("a")))).collect() };
        (pieces, published)
    }

    fn reasons(v: &Value, name: &str) -> Vec<String> {
        let row = v["pieces"].as_array().unwrap().iter().find(|r| r["name"] == name).unwrap();
        row["why"].as_array().unwrap().iter().map(|w| w["kind"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn a_neighbours_ground_ticks_the_pieces_whose_edge_shows_it() {
        let (pieces, published) = three();
        let mut prints: BTreeMap<String, Result<Print, String>> = pieces.iter().map(|q| (q.name.clone(), Ok(p("a")))).collect();
        // rivl02's ground changed; rivl01 and rivl03 carry it on their halo.
        prints.insert("rivl02".into(), Ok(Print { ground: "b".into(), ..p("a") }));
        for n in ["rivl01", "rivl03"] {
            prints.insert(n.into(), Ok(Print { edge: "b".into(), ..p("a") }));
        }
        let now = Now { pieces, prints };
        let exported: BTreeMap<String, Option<Print>> = now.prints.iter().map(|(k, v)| (k.clone(), v.clone().ok())).collect();
        let v = changes(&now, &exported, &published);
        assert_eq!(v["ticked"], json!(["rivl01", "rivl02", "rivl03"]));
        assert_eq!(reasons(&v, "rivl02"), ["ground"]);
        assert_eq!(reasons(&v, "rivl01"), ["edge"]);
        let row = &v["pieces"][0];
        assert_eq!(row["why"][0]["from"], json!(["rivl02"]));
        assert_eq!(row["why"][0]["text"], "its edge shows rivl02's change");
        assert_eq!(row["stale"], false);
        assert_eq!(row["published"], 1);
    }

    #[test]
    fn nothing_changed_ticks_nothing_and_a_new_moved_or_unexported_piece_says_so() {
        let (mut pieces, mut published) = three();
        let prints: BTreeMap<String, Result<Print, String>> = pieces.iter().map(|q| (q.name.clone(), Ok(p("a")))).collect();
        let exported: BTreeMap<String, Option<Print>> = prints.iter().map(|(k, v)| (k.clone(), v.clone().ok())).collect();
        let now = Now { pieces: pieces.clone(), prints: prints.clone() };
        assert_eq!(changes(&now, &exported, &published)["ticked"], json!([]));

        // rivl03 renamed rivl09 (same id), rivl01's corner moved, rivl02 never exported since its change.
        pieces[2].name = "rivl09".into();
        pieces[0].x = 1;
        let mut prints = prints;
        let p3 = prints.remove("rivl03").unwrap();
        prints.insert("rivl09".into(), p3);
        prints.insert("rivl02".into(), Ok(Print { objects: "b".into(), ..p("a") }));
        published.pieces.get_mut("rivl02").unwrap().print = p("a");
        let mut exported = exported;
        exported.remove("rivl03");
        let now = Now { pieces, prints };
        let v = changes(&now, &exported, &published);
        assert_eq!(reasons(&v, "rivl01"), ["moved"]);
        assert_eq!(reasons(&v, "rivl02"), ["objects"]);
        assert_eq!(reasons(&v, "rivl09"), ["new"]);
        assert_eq!(v["pieces"][1]["stale"], true, "rivl02's export is older than its change");
        assert_eq!(v["pieces"][2]["exported"], false);
        assert_eq!(v["removed"], json!([{ "name": "rivl03", "id": 3, "renamed": "rivl09" }]));
    }

    fn crossed(with: &[(&str, &str)]) -> Print {
        Print { crossings: with.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect(), ..p("a") }
    }

    #[test]
    fn exporting_one_piece_names_the_neighbours_to_export_first() {
        let (pieces, published) = three();
        // The cut moved between rivl01 and rivl02: their crossings with each other are new; rivl03's with rivl02 are as they were.
        let prints: BTreeMap<String, Result<Print, String>> = [
            ("rivl01", crossed(&[("rivl02", "new")])),
            ("rivl02", crossed(&[("rivl01", "new"), ("rivl03", "old")])),
            ("rivl03", crossed(&[("rivl02", "old")])),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), Ok(v)))
        .collect();
        let mut exported: BTreeMap<String, Option<Print>> = [
            ("rivl01", Some(crossed(&[("rivl02", "old")]))),
            ("rivl02", Some(crossed(&[("rivl01", "old"), ("rivl03", "old")]))),
            ("rivl03", Some(crossed(&[("rivl02", "old")]))),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let now = Now { pieces, prints };
        let only = |n: &str| vec![n.to_string()];
        assert_eq!(export_first(&now, &only("rivl02"), &exported), ["rivl01"]);
        assert_eq!(export_first(&now, &only("rivl03"), &exported), Vec::<String>::new());
        assert_eq!(export_first(&now, &["rivl01".into(), "rivl02".into()], &exported), Vec::<String>::new());
        // A neighbour never exported into the canvas's module must be, once it has a crossing with the piece.
        exported.insert("rivl03".into(), None);
        assert_eq!(export_first(&now, &only("rivl02"), &exported), ["rivl01", "rivl03"]);

        // Publishing rivl02 alone: rivl01's live crossings with it are the old ones, so it goes too; rivl03's are as published.
        let mut published = published;
        published.pieces.get_mut("rivl01").unwrap().print = crossed(&[("rivl02", "old")]);
        published.pieces.get_mut("rivl03").unwrap().print = crossed(&[("rivl02", "old")]);
        assert_eq!(send_too(&now, &only("rivl02"), &published), ["rivl01"]);
        assert_eq!(send_too(&now, &["rivl01".into(), "rivl02".into()], &published), Vec::<String>::new());
    }

    /// Three pieces published as `a` with their crossings; rivl02's ground changed since, and rivl03 was never exported.
    fn state() -> State {
        let (pieces, mut published) = three();
        let c = |with: &[(&str, &str)]| crossed(with);
        let mut prints: BTreeMap<String, Result<Print, String>> = BTreeMap::new();
        prints.insert("rivl01".into(), Ok(c(&[("rivl02", "k")])));
        prints.insert("rivl02".into(), Ok(Print { ground: "b".into(), ..c(&[("rivl01", "k"), ("rivl03", "k")]) }));
        prints.insert("rivl03".into(), Ok(c(&[("rivl02", "k")])));
        published.pieces.get_mut("rivl01").unwrap().print = c(&[("rivl02", "k")]);
        published.pieces.get_mut("rivl02").unwrap().print = c(&[("rivl01", "k"), ("rivl03", "k")]);
        published.pieces.get_mut("rivl03").unwrap().print = c(&[("rivl02", "old")]);
        let mut exported: BTreeMap<String, Option<Print>> = prints.iter().map(|(k, v)| (k.clone(), v.clone().ok())).collect();
        exported.insert("rivl03".into(), None);
        State { now: Now { pieces, prints }, exported, published }
    }

    #[test]
    fn a_publish_sends_the_changed_pieces_and_names_the_neighbours_whose_crossings_arent_live() {
        let s = state();
        // By default the changed ones: rivl02, and rivl03 whose crossings changed.
        let e = s.to_publish(&[], true).unwrap_err().to_string();
        assert!(e.contains("rivl03 hasn't been exported"), "{e}");
        let (names, warnings) = s.to_publish(&["rivl02".into()], true).unwrap();
        assert_eq!(names, ["rivl02"]);
        // rivl03's live crossings with rivl02 are the old ones.
        assert_eq!(
            warnings,
            [json!({ "kind": "send_too", "pieces": ["rivl03"],
            "why": "its crossings with these pieces aren't live yet: send it too, or they lead nowhere" })]
        );
        assert!(s.to_publish(&["rivl09".into()], true).unwrap_err().to_string().contains("no piece rivl09"));

        // An export older than the canvas: a check warns, a publish refuses.
        let mut stale = state();
        stale.exported.insert("rivl02".into(), Some(crossed(&[("rivl01", "k"), ("rivl03", "k")])));
        let (_, w) = stale.to_publish(&["rivl02".into()], true).unwrap();
        assert_eq!(w[0]["kind"], "stale");
        let e = stale.to_publish(&["rivl02".into()], false).unwrap_err().to_string();
        assert_eq!(e, "rivl02 changed since its last export: export again first");

        // Nothing changed: nothing to send unless named.
        let mut same = state();
        for (n, p) in &same.now.prints {
            same.published.pieces.get_mut(n).unwrap().print = p.clone().unwrap();
        }
        assert!(same.to_publish(&[], true).unwrap_err().to_string().contains("no piece changed"));
        // A piece gone from the canvas is listed for the reviewer.
        same.published.pieces.insert("rivl07".into(), sent(7, 24, p("a")));
        let (names, w) = same.to_publish(&["rivl01".into()], true).unwrap();
        assert_eq!((names.len(), w[0]["kind"].as_str()), (1, Some("removed")));
    }

    #[test]
    fn a_trial_takes_every_piece_and_says_which_crossings_lead_to_dev_s_live_copy() {
        let mut s = state();
        assert!(s.to_try(&[]).unwrap_err().to_string().contains("rivl03 hasn't been exported"));
        s.exported.insert("rivl03".into(), s.now.prints["rivl03"].clone().ok());
        let (names, w) = s.to_try(&[]).unwrap();
        assert_eq!((names, w), (vec!["rivl01".to_string(), "rivl02".into(), "rivl03".into()], vec![]));
        let (_, w) = s.to_try(&["rivl01".into()]).unwrap();
        assert_eq!(w[0]["pieces"], json!(["rivl02"]));
    }

    #[test]
    fn the_record_reads_back_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("landscaper-published-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("rivl")).unwrap();
        let c = Canvas::at(&dir, "rivl");
        assert_eq!(Published::read(&c).unwrap(), Published::default());
        Published::record(&c, vec![("rivl01".into(), sent(1, 0, crossed(&[("rivl02", "x")])))]).unwrap();
        Published::record(&c, vec![("rivl02".into(), sent(2, 8, p("b")))]).unwrap();
        let back = Published::read(&c).unwrap();
        assert_eq!(back.pieces.keys().collect::<Vec<_>>(), ["rivl01", "rivl02"]);
        assert_eq!(back.pieces["rivl01"].print.crossings["rivl02"], "x");
        std::fs::write(dir.join("rivl").join(PUBLISHED), "{").unwrap();
        assert!(Published::read(&c).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
