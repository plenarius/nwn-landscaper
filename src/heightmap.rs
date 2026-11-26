// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Painted terrain from the editor: a JSON header and a raw height grid.
//!
//! ```json
//! { "name": "riverbend", "tiles": 8, "spacing": 0.625,
//!   "width": 129, "height": 129, "data": "riverbend.f32" }
//! ```
//! `data` (relative to the JSON) holds `width * height` little-endian f32
//! heights in metres, row by row from the south edge (y = 0) to the north,
//! each row west (x = 0) to east. The grid covers the whole area
//! (`tiles * 10` m a side) including both edges.
//!
//! Painted textures, optional: `"materials": "riverbend.materials"` holds one byte a sample in the same order, an index into
//! `"palette": ["grassMedium", "dirt", ...]` (texture library ids).
//!
//! Water, optional: `"water": [{ "i": 40, "j": 12, "level": 6.5 }, ...]`, bodies of water each filled from a sample (i, j) to a level:
//! every sample joined to it (4-connected) through samples lower than the level is under water, unless it is under less than
//! [`WET_DEPTH`]. The editor floods them the same way.
//!
//! Chasms, optional: `"chasm": [[first, count], ...]`, runs of samples (indices in the order above) that drop away into a bottomless
//! chasm: each is lowered to [`CHASM_DEPTH`] under its rim (the nearest sample that is not in a chasm, see [`fit_chasm`]), nothing
//! walks there, and it is drawn fading to black. Water never fills a chasm.

use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};

/// A cave entrance (see generator/entrance.rs): the middle of its mouth on the floor, the direction into the slope (radians from east,
/// anticlockwise), its width and depth (m) and its floor's height.
#[derive(Clone, Debug, PartialEq)]
pub struct Entrance {
    /// Its id in the area (1 and up): its door's tag is `<area>_ent<id>` (`generator::entrance::door_tag`). Given
    /// once, when the editor makes the entrance, and never reused in the area (see [`entrances_of`]), so taking an entrance
    /// out never renames the others' doors (transitions and scripts refer to them by tag).
    pub id: u32,
    /// The middle of its mouth, east (area metres).
    pub x: f32,
    /// The middle of its mouth, north (area metres).
    pub y: f32,
    /// The direction into the slope, radians anticlockwise from east.
    pub dir: f32,
    /// How wide its mouth is (m).
    pub width: f32,
    /// How far it goes into the slope (m).
    pub depth: f32,
    /// Its floor's height (m).
    pub floor: f32,
    /// The texture (library id) of its rock; unset: rockCliff.
    pub rock: Option<String>,
    /// The tag of the door it leads to (another entrance's, in the same module); unset: linked in the toolset, if at all.
    pub link: Option<String>,
}

/// The most an entrance's id may be (its door's tag, `<area>_ent<id>`, stays within the game's 32 characters).
pub const MAX_ENTRANCE_ID: u64 = 99_999;

/// The entrances in a header's `entrances` list (one without its numbers is left out, as before). Each keeps its `id`; one
/// without (an area saved or exported before entrances had ids) gets its place in the list, counting from 1, which is the
/// number its door's tag always had, so no existing tag changes. Ids must be 1 to [`MAX_ENTRANCE_ID`] and different.
pub fn entrances_of(list: &serde_json::Value) -> Result<Vec<Entrance>> {
    let mut out: Vec<Entrance> = Vec::new();
    for e in list.as_array().into_iter().flatten() {
        let f = |k: &str| e[k].as_f64().map(|v| v as f32);
        let (Some(x), Some(y), Some(dir), Some(width), Some(depth), Some(floor)) =
            (f("x"), f("y"), f("dir"), f("width"), f("depth"), f("floor"))
        else {
            continue;
        };
        let id = match &e["id"] {
            serde_json::Value::Null => out.len() as u64 + 1,
            v => v
                .as_u64()
                .filter(|id| (1..=MAX_ENTRANCE_ID).contains(id))
                .with_context(|| format!("the entrance at ({x}, {y}): its id is {v}, not a whole number from 1 to {MAX_ENTRANCE_ID}"))?,
        };
        if let Some(twin) = out.iter().find(|o| u64::from(o.id) == id) {
            bail!("two entrances have the id {id} (at ({}, {}) and ({x}, {y})): each needs its own", twin.x, twin.y);
        }
        out.push(Entrance {
            id: id as u32,
            x,
            y,
            dir,
            width,
            depth,
            floor,
            rock: e["rock"].as_str().map(str::to_string),
            link: e["link"].as_str().filter(|l| !l.is_empty()).map(str::to_string),
        });
    }
    Ok(out)
}

/// The area's next entrance id after an export of `entrances` (the request's, ids as [`entrances_of`] gives them), given the
/// last export's header `before` (its `entranceNextId`, else one past its entrances' ids): one past the highest id the area has
/// ever had, so an id is never given twice. An entrance the request names by an `id` of its own that the last export didn't
/// have, and that is below the last export's counter, is refused: that id belonged to an entrance taken out since, and its door's
/// tag may still be in transitions and scripts.
pub fn next_entrance_id(entrances: &serde_json::Value, before: Option<&serde_json::Value>) -> Result<u64> {
    let now = entrances_of(entrances)?;
    let then = before.map(|b| entrances_of(&b["entrances"])).transpose()?.unwrap_or_default();
    let counter =
        before.and_then(|b| b["entranceNextId"].as_u64()).unwrap_or(0).max(then.iter().map(|e| u64::from(e.id) + 1).max().unwrap_or(1));
    let given = entrances.as_array().into_iter().flatten().filter(|e| !e["id"].is_null()).filter_map(|e| e["id"].as_u64());
    for id in given {
        if id < counter && !then.iter().any(|e| u64::from(e.id) == id) {
            bail!("entrance id {id} was an entrance's that has been taken out: a new entrance needs a new id ({counter} or more)");
        }
    }
    Ok(counter.max(now.iter().map(|e| u64::from(e.id) + 1).max().unwrap_or(1)))
}

/// A placeable baked into the terrain (see objects.rs): its model, where its origin stands (area metres), its bearing (radians,
/// anticlockwise from east, as the game's) and its scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    /// The model's resref.
    pub model: String,
    /// Where its origin stands, east (area metres).
    pub x: f32,
    /// Where its origin stands, north (area metres).
    pub y: f32,
    /// Its origin's height (m).
    pub z: f32,
    /// Its facing: radians anticlockwise from east, as the game's.
    pub bearing: f32,
    /// Its size, 1 as the model is.
    pub scale: f32,
    /// Whether its walkmesh blocks walking (decoration like flowers or pebbles doesn't; it still casts shadows).
    pub solid: bool,
    /// Baked into the terrain (scenery), else written as a placeable to use and script (object mode).
    pub baked: bool,
    /// placeables.2da's row (object mode's appearance).
    pub appearance: Option<u32>,
    /// The editor's number for it: its tag in object mode (`<area>_p<id>`).
    pub id: Option<u32>,
}

/// An area as the editor saved it: its ground, paint, water and what stands on it. The generator reads the loaded one ([`current`]).
pub struct Heightmap {
    /// Lower-case, 1-9 characters [a-z0-9_]: prefixes every resref.
    pub name: String,
    /// Tiles across (west to east).
    pub tiles: usize,
    /// Tiles up (south to north).
    pub tiles_y: usize,
    /// Metres between samples (0.625: 16 a tile).
    pub spacing: f32,
    /// Samples across: 16 a tile, plus one.
    pub width: usize,
    /// Samples up: 16 a tile, plus one.
    pub height: usize,
    /// Each sample's height (m), rows from the south.
    pub data: Vec<f32>,
    /// Each sample's index into `palette`; empty when unpainted.
    pub materials: Vec<u8>,
    /// The texture ids `materials` indexes.
    pub palette: Vec<String>,
    /// Each sample's water level (NaN: dry), from the water bodies.
    pub water: Vec<f32>,
    /// Cave entrances, from the editor.
    pub entrances: Vec<Entrance>,
    /// Placeables baked in, from the editor.
    pub objects: Vec<Object>,
    /// Particle placements baked into the tiles (see `ambience`).
    pub ambience: Vec<crate::ambience::Placement>,
    /// A cave or an interior: no sky; rock tops at `roof` and above are void (drawn black).
    pub underground: bool,
    /// An interior (rooms): flagged interior only, not underground or natural.
    pub interior: bool,
    /// The height (m) from which rock tops are void in a cave or an interior; None: none are.
    pub roof: Option<f32>,
    /// Each sample's ceiling, when the roof follows the floor (`clearance`): empty, the flat `roof`.
    pub ceiling: Vec<f32>,
    /// Each sample's rim height when the area has a chasm ([`fit_chasm`]): a chasm sample's is its nearest open sample's, an open
    /// one's its own. Empty: no chasm.
    pub rim: Vec<f32>,
    /// Baked shadows' strength (0..1), if the editor set it.
    pub shadow: Option<f32>,
    /// The editor's light and fog (see `atmosphere`), if set.
    pub atmosphere: Option<serde_json::Value>,
    /// Creatures, doors, triggers, waypoints, sounds... the editor placed (see `things`).
    pub things: Vec<crate::things::Thing>,
    /// The area's own settings (see `settings`), if set.
    pub settings: Option<serde_json::Value>,
    /// Edits to an imported area's original objects: `{ removed: ["List:3"], moved: { "List:4": { x, y, bearing } } }`.
    pub originals: Option<serde_json::Value>,
    /// Bumped on every load, so caches keyed on the terrain notice.
    pub serial: u64,
    /// Each sample's normal ([`Heightmap::sample_normal`]), made on first use.
    pub(crate) normals: std::sync::OnceLock<Vec<[f32; 3]>>,
}

static CURRENT: RwLock<Option<Arc<Heightmap>>> = RwLock::new(None);

/// How many samples each way steep ground's normal is taken over ([`Heightmap::normal`]): 1.25 m at 0.625 m a sample, about
/// as far as a river's bank climbs.
const WIDE: usize = 2;
/// The tilts (as the up part of the wide normal) between which steep ground's normal turns from its own slope to the wide one:
/// 15 and 30 degrees. Gentler ground keeps the light on its every bump.
const GENTLE_NZ: f32 = 0.966;
const STEEP_NZ: f32 = 0.866;

/// The area loaded last (by [`load`]), which the generator builds from; None before any.
pub fn current() -> Option<Arc<Heightmap>> {
    CURRENT.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// The loaded terrain's height at area coordinates, if one is loaded (cheap: no Arc clone; called for every sample).
pub fn sample(x: f32, y: f32) -> Option<f32> {
    CURRENT.read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map(|h| h.at(x, y))
}

/// Whether area point (x, y) is under more than `shallow` metres of water
/// (cheap when there is none: called for every walkmesh sample).
pub fn deep_water(x: f32, y: f32, shallow: f32) -> bool {
    CURRENT
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .is_some_and(|h| h.water_at(x, y).is_some_and(|l| l - h.at(x, y) > shallow))
}

/// Reads the terrain `json_path` (see the module comment) and makes it the loaded one ([`current`]).
pub fn load(json_path: &Path) -> Result<Arc<Heightmap>> {
    let hm = read(json_path)?;
    *CURRENT.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hm.clone());
    Ok(hm)
}

/// Reads the terrain `json_path`, checked, without loading it.
fn read(json_path: &Path) -> Result<Arc<Heightmap>> {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let text = crate::atomic::read_to_string(json_path).with_context(|| format!("reading {}", json_path.display()))?;
    let j: serde_json::Value = serde_json::from_str(&text)?;
    let name = j["name"].as_str().context("name")?.to_lowercase();
    if name.is_empty() || name.len() > 9 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("name must be 1-9 characters of a-z, 0-9, _ (it prefixes every resref, 16 characters at most)");
    }
    // `tiles` across (west to east), `tilesY` up (south to north; as many as across when not given).
    let tiles = j["tiles"].as_u64().context("tiles")? as usize;
    let tiles_y = j["tilesY"].as_u64().map_or(tiles, |v| v as usize);
    // (3 at least: a side of 2 tiles can't be given a tileset the module builder accepts: "no tile fits cell (0, 0)".)
    if !(3..=32).contains(&tiles) || !(3..=32).contains(&tiles_y) {
        bail!("an area is 3 to 32 tiles each way, not {tiles} by {tiles_y}");
    }
    let spacing = j["spacing"].as_f64().context("spacing")? as f32;
    anyhow::ensure!(spacing.is_finite() && spacing >= 0.1, "spacing {spacing}: a sample every 0.1 m at least");
    let (width, height) = (j["width"].as_u64().context("width")? as usize, j["height"].as_u64().context("height")? as usize);
    let samples = |t: usize| (t as f32 * 10.0 / spacing).round() as usize + 1;
    if width != samples(tiles) || height != samples(tiles_y) {
        bail!("{tiles} by {tiles_y} tiles at {spacing} m need a {} x {} grid, not {width} x {height}", samples(tiles), samples(tiles_y));
    }
    let data_path = json_path.parent().unwrap_or(Path::new(".")).join(j["data"].as_str().context("data")?);
    let bytes = crate::atomic::read(&data_path).with_context(|| format!("reading {}", data_path.display()))?;
    if bytes.len() != width * height * 4 {
        bail!("{}: {} bytes, expected {}", data_path.display(), bytes.len(), width * height * 4);
    }
    let mut data: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if let Some(k) = data.iter().position(|h| !h.is_finite()) {
        bail!("the height at sample ({}, {}) is {}: heights must be numbers", k % width, k / width, data[k]);
    }
    // An enclosed area whose roof follows the floor: the rock is cut down to `clearance` over the nearest open ground.
    let enclosed = matches!(j["kind"].as_str(), Some("underground" | "interior"));
    let ceiling = match (enclosed, j["roof"].as_f64(), j["clearance"].as_f64()) {
        (true, Some(roof), Some(clearance)) if clearance > 0.0 => fit_ceiling(&mut data, width, roof as f32, clearance as f32),
        _ => Vec::new(),
    };
    let (mut materials, mut palette) = (Vec::new(), Vec::new());
    if let Some(file) = j["materials"].as_str() {
        let path = json_path.parent().unwrap_or(Path::new(".")).join(file);
        materials = crate::atomic::read(&path).with_context(|| format!("reading {}", path.display()))?;
        if materials.len() != width * height {
            bail!("{}: {} bytes, expected {}", path.display(), materials.len(), width * height);
        }
        palette =
            j["palette"].as_array().context("materials without a palette")?.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();
        if let Some(bad) = materials.iter().find(|&&m| m as usize >= palette.len()) {
            bail!("material {bad} is past the palette's {} textures", palette.len());
        }
    }
    // A chasm: its samples dropped under their rims (after the ceiling: a chasm is open ground).
    let chasm = chasm_of(&j["chasm"], width * height)?;
    let rim = if chasm.iter().any(|&c| c) { fit_chasm(&mut data, width, &chasm) } else { Vec::new() };
    let water = match j["water"].as_array().filter(|b| !b.is_empty()) {
        Some(bodies) => {
            let bodies: Vec<Body> = bodies
                .iter()
                .map(|b| {
                    let bounds = b["bounds"].as_array().filter(|a| a.len() == 4).and_then(|a| {
                        let v: Vec<usize> = a.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect();
                        (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
                    });
                    Some(Body { i: b["i"].as_u64()? as usize, j: b["j"].as_u64()? as usize, level: b["level"].as_f64()?, bounds })
                })
                .collect::<Option<_>>()
                .context("water: each body needs i, j and level")?;
            // (water never runs into a chasm: its samples are walls to it)
            let walled: Vec<f32> = data.iter().zip(&chasm).map(|(&h, &c)| if c { f32::INFINITY } else { h }).collect();
            dry_shallows(flood(&walled, width, &bodies), &data)
        }
        None => Vec::new(),
    };
    let hm = Arc::new(Heightmap {
        name,
        tiles,
        tiles_y,
        spacing,
        width,
        height,
        data,
        materials,
        palette,
        atmosphere: j.get("atmosphere").filter(|a| a.is_object()).cloned(),
        things: j["things"]
            .as_array()
            .map(|l| l.iter().map(|t| crate::things::Thing::parse(t).map_err(anyhow::Error::msg)).collect::<Result<Vec<_>>>())
            .transpose()?
            .unwrap_or_default(),
        settings: j.get("settings").filter(|a| a.is_object()).cloned(),
        originals: j.get("originals").filter(|a| a.is_object()).cloned(),
        water,
        shadow: j["shadow"].as_f64().map(|v| v as f32),
        entrances: entrances_of(&j["entrances"])?,
        underground: matches!(j["kind"].as_str(), Some("underground" | "interior")),
        interior: j["kind"].as_str() == Some("interior"),
        roof: j["roof"].as_f64().map(|v| v as f32),
        ceiling,
        rim,
        objects: j["objects"].as_array().map_or_default(|list| {
            list.iter()
                .filter_map(|o| {
                    let f = |k: &str| o[k].as_f64().map(|v| v as f32);
                    Some(Object {
                        model: o["model"].as_str()?.to_ascii_lowercase(),
                        x: f("x")?,
                        y: f("y")?,
                        z: f("z")?,
                        bearing: f("bearing").unwrap_or(0.0),
                        scale: f("scale").unwrap_or(1.0),
                        solid: o["solid"].as_bool().unwrap_or(true),
                        baked: o["baked"].as_bool().unwrap_or(true),
                        appearance: o["appearance"].as_u64().map(|v| v as u32),
                        id: o["id"].as_u64().map(|v| v as u32),
                    })
                })
                .collect()
        }),
        ambience: crate::ambience::placements_of(&j["ambience"])?,
        serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        normals: Default::default(),
    });
    in_range(&hm)?;
    Ok(hm)
}

/// How far outside the area (metres) a placed object, thing or entrance may stand: the editor keeps them inside (it clamps a
/// move to the area), so this is only room for rounding.
pub const MARGIN: f32 = 10.0;

/// The sizes an object may be scaled to: the editor's own range (`SCALE_RANGE` in landscaper-web's `transform.js`, and the Size
/// box's `min` and `max`).
pub const SCALE: std::ops::RangeInclusive<f32> = 0.1..=5.0;

/// Whether what was placed on `hm` is where an area can hold it: positions within the area (plus [`MARGIN`]), every number a
/// number, objects within [`SCALE`], an entrance no wider or deeper than the area. The request body says all of it, and an
/// object scaled by 1e5 or placed a million metres away has the export's footprint and shadow grids loop over millions of cells
/// (a 3 by 3 area's check was killed out of memory at 3 GB).
fn in_range(hm: &Heightmap) -> Result<()> {
    let (w, h) = (hm.tiles as f32 * 10.0, hm.tiles_y as f32 * 10.0);
    let inside = |x: f32, y: f32| (-MARGIN..=w + MARGIN).contains(&x) && (-MARGIN..=h + MARGIN).contains(&y);
    let at = |x: f32, y: f32| format!("({x}, {y})");
    for o in &hm.objects {
        let what = || format!("object {} at {}", o.model, at(o.x, o.y));
        anyhow::ensure!(inside(o.x, o.y), "{}: outside the {w} by {h} m area", what());
        anyhow::ensure!(o.z.is_finite() && o.bearing.is_finite(), "{}: its height and bearing must be numbers", what());
        anyhow::ensure!(SCALE.contains(&o.scale), "{}: its size is {}, not {} to {}", what(), o.scale, SCALE.start(), SCALE.end());
    }
    for t in &hm.things {
        anyhow::ensure!(inside(t.x, t.y), "thing {} at {}: outside the {w} by {h} m area", t.id, at(t.x, t.y));
        anyhow::ensure!(t.bearing.is_finite(), "thing {}: its bearing must be a number", t.id);
        if let Some(p) = t.outline.iter().find(|p| !inside(p[0], p[1])) {
            bail!("thing {}: its outline point {} is outside the {w} by {h} m area", t.id, at(p[0], p[1]));
        }
    }
    for e in &hm.entrances {
        anyhow::ensure!(inside(e.x, e.y), "the entrance at {}: outside the {w} by {h} m area", at(e.x, e.y));
        anyhow::ensure!(
            e.dir.is_finite() && e.floor.is_finite() && e.width > 0.0 && e.width <= w.max(h) && e.depth > 0.0 && e.depth <= w.max(h),
            "the entrance at {}: its direction and floor must be numbers, its width and depth more than 0 and at most the area's side",
            at(e.x, e.y)
        );
    }
    for a in &hm.ambience {
        let what = || format!("ambience {} at {}", a.kind, at(a.x, a.y));
        anyhow::ensure!(inside(a.x, a.y), "{}: outside the {w} by {h} m area", what());
        anyhow::ensure!(a.z.is_finite() && a.bearing.is_finite(), "{}: its height and bearing must be numbers", what());
        anyhow::ensure!(SCALE.contains(&a.scale), "{}: its size is {}, not {} to {}", what(), a.scale, SCALE.start(), SCALE.end());
    }
    for (k, v) in [("roof", hm.roof), ("shadow", hm.shadow)] {
        anyhow::ensure!(v.is_none_or(f32::is_finite), "the {k} must be a number");
    }
    Ok(())
}

/// A body of water: filled from sample (i, j) to `level`, within `bounds` (samples i0, j0, i1, j1, inclusive) if given: an imported
/// area's water stays where its tiles drew it.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    /// The sample it fills from: column.
    pub i: usize,
    /// The sample it fills from: row.
    pub j: usize,
    /// The water's height (m).
    pub level: f64,
    /// The samples it may fill (i0, j0, i1, j1, inclusive); None: as far as it flows.
    pub bounds: Option<[usize; 4]>,
}

/// Each sample's water level (NaN: dry): each body floods from its sample through 4-connected samples lower than its level (within its
/// bounds); where bodies meet, the higher level.
pub fn flood(heights: &[f32], n: usize, bodies: &[Body]) -> Vec<f32> {
    // n samples a row; as many rows as the heights hold.
    let rows = heights.len() / n.max(1);
    let mut level = vec![f32::NAN; heights.len()];
    let mut seen = vec![false; heights.len()];
    for b in bodies {
        let (i, j, l) = (b.i, b.j, b.level);
        if i >= n || j >= rows {
            continue;
        }
        let [i0, j0, i1, j1] = b.bounds.unwrap_or([0, 0, n - 1, rows - 1]);
        seen.iter_mut().for_each(|s| *s = false);
        let mut stack = vec![j * n + i];
        while let Some(k) = stack.pop() {
            let (x, y) = (k % n, k / n);
            if seen[k] || f64::from(heights[k]) >= l || x < i0 || x > i1 || y < j0 || y > j1 {
                continue;
            }
            seen[k] = true;
            if level[k].is_nan() || level[k] < l as f32 {
                level[k] = l as f32;
            }
            if x > 0 {
                stack.push(k - 1);
            }
            if x + 1 < n {
                stack.push(k + 1);
            }
            if y > 0 {
                stack.push(k - n);
            }
            if y + 1 < rows {
                stack.push(k + n);
            }
        }
    }
    level
}

/// Water shallower than this (m) is dry ground: the game's water shader draws it almost clear, so it reads as dry until a footstep
/// ripples it. The editor has the same (area.js).
pub const WET_DEPTH: f32 = 0.15;

/// `level` (from [`flood`]) with the samples under less than [`WET_DEPTH`] of water dry. They still joined the water when it flooded,
/// so a shallow ford doesn't split a river in two.
pub fn dry_shallows(mut level: Vec<f32>, heights: &[f32]) -> Vec<f32> {
    for (l, h) in level.iter_mut().zip(heights) {
        if *l - h < WET_DEPTH {
            *l = f32::NAN;
        }
    }
    level
}

/// A rock sample is void from this far below its ceiling up.
pub const VOID_BELOW: f32 = 0.3;

/// Makes the roof follow the floor: every sample's ceiling is the height of the nearest open (not rock) sample plus
/// `clearance`, and the rock samples are cut down to it. Returns the ceilings.
pub fn fit_ceiling(data: &mut [f32], width: usize, roof: f32, clearance: f32) -> Vec<f32> {
    let height = data.len() / width;
    let open: Vec<bool> = data.iter().map(|&h| h < roof - VOID_BELOW).collect();
    let mut from = vec![usize::MAX; data.len()];
    let mut queue = std::collections::VecDeque::new();
    for k in 0..data.len() {
        if open[k] {
            from[k] = k;
            queue.push_back(k);
        }
    }
    while let Some(k) = queue.pop_front() {
        let (i, j) = ((k % width) as i64, (k / width) as i64);
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= width as i64 || b >= height as i64 {
                continue;
            }
            let n = b as usize * width + a as usize;
            if from[n] == usize::MAX {
                from[n] = from[k];
                queue.push_back(n);
            }
        }
    }
    // No open ground at all: the flat roof stays.
    let ceiling: Vec<f32> = (0..data.len()).map(|k| if from[k] == usize::MAX { roof } else { data[from[k]] + clearance }).collect();
    for k in 0..data.len() {
        if !open[k] {
            data[k] = ceiling[k].min(data[k]);
        }
    }
    ceiling
}

/// How far a chasm drops under its rim (m). Deep enough that its walls fade to black well before the bottom and that the game's
/// fog (outdoor areas start it at 20 to 40 m from the camera) takes the rest; the floor is black anyway, so no camera angle shows
/// one. The editor's `CHASM_DEPTH` is the same.
pub const CHASM_DEPTH: f32 = 20.0;

/// Ground more than this under its rim (m) is in a chasm: nothing walks there.
pub const CHASM_BELOW: f32 = 0.3;

/// The most chasm runs a header may list: one a sample, on the largest area (32 by 32 tiles).
const MAX_CHASM_RUNS: usize = 513 * 513;

/// The chasm samples of a header's `chasm` list (runs of `[first, count]` over `samples` samples; null or missing: none).
pub fn chasm_of(list: &serde_json::Value, samples: usize) -> Result<Vec<bool>> {
    let mut mask = vec![false; samples];
    if list.is_null() {
        return Ok(mask);
    }
    let runs = list.as_array().context("chasm: a list of [first, count] runs")?;
    anyhow::ensure!(runs.len() <= MAX_CHASM_RUNS, "chasm: {} runs, at most {MAX_CHASM_RUNS}", runs.len());
    for r in runs {
        let (Some(first), Some(count)) = (r.get(0).and_then(serde_json::Value::as_u64), r.get(1).and_then(serde_json::Value::as_u64))
        else {
            bail!("chasm: each run is [first, count], not {r}");
        };
        let (first, count) = (first as usize, count as usize);
        anyhow::ensure!(
            first.checked_add(count).is_some_and(|end| end <= samples),
            "chasm: the run [{first}, {count}] goes past the area's {samples} samples"
        );
        mask[first..first + count].iter_mut().for_each(|c| *c = true);
    }
    Ok(mask)
}

/// Drops the chasm (`chasm`, one flag a sample, rows `width` wide): every chasm sample's rim is the height of the nearest sample
/// that is not in it (breadth first, east, west, north, south in turn, as [`fit_ceiling`] and the editor's `chasmFit` go), and
/// it is lowered to [`CHASM_DEPTH`] under that rim. Returns each sample's rim (an open sample's is its own height; with no open
/// sample at all, every sample's own).
pub fn fit_chasm(data: &mut [f32], width: usize, chasm: &[bool]) -> Vec<f32> {
    let height = data.len() / width;
    let mut from = vec![usize::MAX; data.len()];
    let mut queue = std::collections::VecDeque::new();
    for k in 0..data.len() {
        if !chasm[k] {
            from[k] = k;
            queue.push_back(k);
        }
    }
    while let Some(k) = queue.pop_front() {
        let (i, j) = ((k % width) as i64, (k / width) as i64);
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= width as i64 || b >= height as i64 {
                continue;
            }
            let n = b as usize * width + a as usize;
            if from[n] == usize::MAX {
                from[n] = from[k];
                queue.push_back(n);
            }
        }
    }
    let rim: Vec<f32> = (0..data.len()).map(|k| if from[k] == usize::MAX { data[k] } else { data[from[k]] }).collect();
    for k in 0..data.len() {
        if chasm[k] {
            data[k] = rim[k] - CHASM_DEPTH;
        }
    }
    rim
}

impl Heightmap {
    /// How far (x, y) is under its chasm's rim (m, bilinear): 0 on open ground, [`CHASM_DEPTH`] on a chasm's floor; 0 with no chasm.
    pub fn chasm_depth(&self, x: f32, y: f32) -> f32 {
        if self.rim.is_empty() {
            return 0.0;
        }
        let fx = (x / self.spacing).clamp(0.0, (self.width - 1) as f32);
        let fy = (y / self.spacing).clamp(0.0, (self.height - 1) as f32);
        let (ix, iy) = ((fx.floor() as usize).min(self.width - 2), (fy.floor() as usize).min(self.height - 2));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let d = |i: usize, j: usize| self.rim[j * self.width + i] - self.data[j * self.width + i];
        let a = d(ix, iy) + (d(ix + 1, iy) - d(ix, iy)) * tx;
        let b = d(ix, iy + 1) + (d(ix + 1, iy + 1) - d(ix, iy + 1)) * tx;
        (a + (b - a) * ty).max(0.0)
    }

    /// Whether (x, y) is in a chasm: more than [`CHASM_BELOW`] under its rim.
    pub fn in_chasm(&self, x: f32, y: f32) -> bool {
        self.chasm_depth(x, y) > CHASM_BELOW
    }

    /// The ceiling at area coordinates: where the rock tops out (bilinear), the flat roof if it does not follow the floor.
    pub fn roof_at(&self, x: f32, y: f32) -> Option<f32> {
        if self.ceiling.is_empty() {
            return self.roof;
        }
        let fx = (x / self.spacing).clamp(0.0, (self.width - 1) as f32);
        let fy = (y / self.spacing).clamp(0.0, (self.height - 1) as f32);
        let (ix, iy) = ((fx.floor() as usize).min(self.width - 2), (fy.floor() as usize).min(self.height - 2));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let h = |i: usize, j: usize| self.ceiling[j * self.width + i];
        let a = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * tx;
        let b = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * tx;
        Some(a + (b - a) * ty)
    }

    /// The water level over sample (i, j), if it is under water (clamped).
    pub fn water_level(&self, i: i64, j: i64) -> Option<f32> {
        let (i, j) = (i.clamp(0, self.width as i64 - 1) as usize, j.clamp(0, self.height as i64 - 1) as usize);
        self.water.get(j * self.width + i).copied().filter(|l| !l.is_nan())
    }

    /// Water at area point (x, y): the nearest sample's level, if wet.
    pub fn water_at(&self, x: f32, y: f32) -> Option<f32> {
        self.water_level((x / self.spacing).round() as i64, (y / self.spacing).round() as i64)
    }

    /// The painted texture (palette index) of sample (i, j), clamped.
    pub fn material(&self, i: i64, j: i64) -> u8 {
        let (i, j) = (i.clamp(0, self.width as i64 - 1) as usize, j.clamp(0, self.height as i64 - 1) as usize);
        self.materials.get(j * self.width + i).copied().unwrap_or(0)
    }

    /// Whether the area has paint: an unpainted one takes the default textures.
    pub fn painted(&self) -> bool {
        !self.materials.is_empty()
    }

    /// Bilinear height at area coordinates (clamped to the grid).
    pub fn at(&self, x: f32, y: f32) -> f32 {
        let fx = (x / self.spacing).clamp(0.0, (self.width - 1) as f32);
        let fy = (y / self.spacing).clamp(0.0, (self.height - 1) as f32);
        let (ix, iy) = ((fx.floor() as usize).min(self.width - 2), (fy.floor() as usize).min(self.height - 2));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let h = |i: usize, j: usize| self.data[j * self.width + i];
        let a = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * tx;
        let b = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * tx;
        a + (b - a) * ty
    }

    /// The ground's unit normal at area coordinates (x east, y north, z up), smooth across the whole area: each sample's
    /// ([`Self::sample_normal`]), blended bilinearly between samples. The editor's view gives gentle ground the same normals
    /// (js/view.js, `updateNormals`), so slopes are textured alike in both ([`crate::triplanar`]); on steep ground they differ.
    pub fn normal(&self, x: f32, y: f32) -> [f32; 3] {
        let fx = (x / self.spacing).clamp(0.0, (self.width - 1) as f32);
        let fy = (y / self.spacing).clamp(0.0, (self.height - 1) as f32);
        let (ix, iy) = ((fx.floor() as usize).min(self.width - 2), (fy.floor() as usize).min(self.height - 2));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let mut n = [0.0; 3];
        for (i, j, w) in
            [(ix, iy, (1.0 - tx) * (1.0 - ty)), (ix + 1, iy, tx * (1.0 - ty)), (ix, iy + 1, (1.0 - tx) * ty), (ix + 1, iy + 1, tx * ty)]
        {
            let s = self.sample_normal(i, j);
            for k in 0..3 {
                n[k] += s[k] * w;
            }
        }
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        n.map(|c| c / len)
    }

    /// Sample (i, j)'s unit normal. On gentle ground, its slope between its neighbours. Where a steep face runs across the samples'
    /// grid at a slant (a river's bank), that slope swings from one sample to the next with how the face crosses them, and the
    /// game's sun drew it as dark blotches along the face. Steep ground takes the slope over [`WIDE`] samples each way instead,
    /// blended in between [`GENTLE_NZ`] and [`STEEP_NZ`] of that wide slope's tilt. Made once for every sample, on first use.
    fn sample_normal(&self, i: usize, j: usize) -> [f32; 3] {
        let normals = self.normals.get_or_init(|| {
            (0..self.height)
                .flat_map(|j| (0..self.width).map(move |i| (i, j)))
                .map(|(i, j)| {
                    let (near, wide) = (self.slope_normal(i, j, 0), self.slope_normal(i, j, WIDE));
                    let t = ((wide[2] - GENTLE_NZ) / (STEEP_NZ - GENTLE_NZ)).clamp(0.0, 1.0);
                    let t = t * t * (3.0 - 2.0 * t);
                    let n: [f32; 3] = std::array::from_fn(|k| near[k] + (wide[k] - near[k]) * t);
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    n.map(|c| c / len)
                })
                .collect()
        });
        normals[j * self.width + i]
    }

    /// Sample (i, j)'s unit normal from its rise per metre east and north between its neighbours, of the heights averaged over
    /// `r` samples each way (0: the heights themselves). Near the area's edge the average takes fewer, as many on each side, so a
    /// plane's normal stays the plane's.
    fn slope_normal(&self, i: usize, j: usize, r: usize) -> [f32; 3] {
        let mean = |i: usize, j: usize| {
            let (ri, rj) = (r.min(i).min(self.width - 1 - i), r.min(j).min(self.height - 1 - j));
            let mut sum = 0.0;
            for y in j - rj..=j + rj {
                sum += self.data[y * self.width + i - ri..=y * self.width + i + ri].iter().sum::<f32>();
            }
            sum / ((2 * ri + 1) * (2 * rj + 1)) as f32
        };
        let (w, e, s, n) = (i.saturating_sub(1), (i + 1).min(self.width - 1), j.saturating_sub(1), (j + 1).min(self.height - 1));
        let east = (mean(e, j) - mean(w, j)) / ((e - w) as f32 * self.spacing);
        let north = (mean(i, n) - mean(i, s)) / ((n - s) as f32 * self.spacing);
        let len = (east * east + north * north + 1.0).sqrt();
        [-east / len, -north / len, 1.0 / len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Body {
        /// A body flooding from sample (`i`, `j`) up to `level`, with no bounds.
        fn new(i: usize, j: usize, level: f64) -> Body {
            Body { i, j, level, bounds: None }
        }
    }

    /// A 3 by 3 tile area in a scratch folder: its header `extra` merged into a plain one, its heights `heights` (all 1 m
    /// when None). Returns the header's path.
    fn area(tag: &str, extra: serde_json::Value, heights: Option<Vec<f32>>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ls-heightmap-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let n = 3 * 16 + 1;
        let heights = heights.unwrap_or_else(|| vec![1.0; n * n]);
        std::fs::write(dir.join("t.f32"), heights.iter().flat_map(|h| h.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let mut header = serde_json::json!({ "name": "t", "tiles": 3, "spacing": 0.625, "width": n, "height": n, "data": "t.f32" });
        for (k, v) in extra.as_object().unwrap() {
            header[k] = v.clone();
        }
        let path = dir.join("t.json");
        std::fs::write(&path, header.to_string()).unwrap();
        path
    }

    #[test]
    fn objects_things_entrances_and_ambience_out_of_range_are_refused() {
        let object = |o: serde_json::Value| {
            let mut base = serde_json::json!({ "model": "plc_a01", "x": 15, "y": 15, "z": 1, "bearing": 0, "scale": 1 });
            for (k, v) in o.as_object().unwrap() {
                base[k] = v.clone();
            }
            serde_json::json!({ "objects": [base] })
        };
        let thing = |t: serde_json::Value| serde_json::json!({ "things": [t] });
        let entrance = |e: serde_json::Value| serde_json::json!({ "entrances": [e] });
        let ambience = |a: serde_json::Value| {
            let mut base = serde_json::json!({ "kind": "mist", "x": 15, "y": 15, "z": 1, "scale": 1, "seed": 7 });
            for (k, v) in a.as_object().unwrap() {
                base[k] = v.clone();
            }
            serde_json::json!({ "ambience": [base] })
        };
        let fine = [
            object(serde_json::json!({})),
            object(serde_json::json!({ "scale": 0.1, "x": 0, "y": 30 })),
            object(serde_json::json!({ "scale": 5, "x": -5, "y": 35 })),
            thing(serde_json::json!({ "id": 1, "kind": "trigger", "x": 1, "y": 2, "outline": [[1, 2], [29, 2], [29, 29]] })),
            entrance(serde_json::json!({ "x": 15, "y": 0, "dir": 1.57, "width": 6, "depth": 8, "floor": 0 })),
            ambience(serde_json::json!({})),
            ambience(serde_json::json!({ "x": -5, "bearing": 1.57, "scale": 0.5 })),
        ];
        for (k, extra) in fine.into_iter().enumerate() {
            read(&area(&format!("fine{k}"), extra.clone(), None)).unwrap_or_else(|e| panic!("{extra}: {e:#}"));
        }
        let refused = [
            (object(serde_json::json!({ "scale": 1e5 })), "its size is 100000"),
            (object(serde_json::json!({ "scale": 0.01 })), "its size is 0.01"),
            (object(serde_json::json!({ "x": 1e9 })), "outside the 30 by 30 m area"),
            (object(serde_json::json!({ "y": -11 })), "outside the 30 by 30 m area"),
            (object(serde_json::json!({ "z": 1e39 })), "must be numbers"),
            (thing(serde_json::json!({ "id": 2, "kind": "creature", "x": 1e9, "y": 2 })), "thing 2"),
            (
                thing(serde_json::json!({ "id": 3, "kind": "trigger", "x": 1, "y": 2, "outline": [[1, 2], [5e8, 2], [5, 5]] })),
                "outline point",
            ),
            (
                entrance(serde_json::json!({ "x": 15, "y": 0, "dir": 1.57, "width": 1e9, "depth": 8, "floor": 0 })),
                "at most the area's side",
            ),
            (entrance(serde_json::json!({ "x": -1e9, "y": 0, "dir": 1.57, "width": 6, "depth": 8, "floor": 0 })), "outside"),
            (ambience(serde_json::json!({ "x": 1e9 })), "ambience mist at (1000000000, 15): outside"),
            (ambience(serde_json::json!({ "scale": 1e5 })), "its size is 100000"),
            (ambience(serde_json::json!({ "z": 1e39 })), "must be numbers"),
            (ambience(serde_json::json!({ "z": "high" })), "x, y and z must be numbers"),
        ];
        for (k, (extra, why)) in refused.into_iter().enumerate() {
            let e = format!("{:#}", read(&area(&format!("bad{k}"), extra.clone(), None)).err().unwrap_or_else(|| panic!("{extra}: taken")));
            assert!(e.contains(why), "{extra}: {e}");
        }
    }

    #[test]
    fn a_height_that_isnt_a_number_is_refused() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut h = vec![1.0; 49 * 49];
            h[49 + 2] = bad;
            let e = format!("{:#}", read(&area("nan", serde_json::json!({}), Some(h))).err().expect("refused"));
            assert!(e.contains("sample (2, 1)"), "{bad}: {e}");
        }
    }

    #[test]
    fn the_roof_follows_the_floor() {
        // A row: a floor at 5, a floor at 0, rock (roof 11) between and beyond.
        let mut d = vec![11.0, 5.0, 11.0, 11.0, 0.0, 11.0];
        let c = fit_ceiling(&mut d, 6, 11.0, 4.0);
        assert_eq!(c[1], 9.0);
        assert_eq!(c[4], 4.0);
        // Rock beside each floor is cut to that floor's ceiling; the floors are untouched.
        assert_eq!((d[0], d[1], d[4], d[5]), (9.0, 5.0, 0.0, 4.0));
        assert!(d[1] < c[1] - VOID_BELOW && d[0] >= c[0] - VOID_BELOW);
    }

    #[test]
    fn a_chasm_drops_under_its_nearest_rim() {
        // A row: ground at 5, two chasm samples, ground at 12.
        let mut d = vec![5.0, 9.0, 9.0, 12.0];
        let rim = fit_chasm(&mut d, 4, &[false, true, true, false]);
        assert_eq!(rim, vec![5.0, 5.0, 12.0, 12.0]);
        assert_eq!(d, vec![5.0, 5.0 - CHASM_DEPTH, 12.0 - CHASM_DEPTH, 12.0]);
        // All of it chasm: no rim, each sample drops from its own height.
        let mut d = vec![3.0, 4.0];
        assert_eq!(fit_chasm(&mut d, 2, &[true, true]), vec![3.0, 4.0]);
        assert_eq!(d, vec![3.0 - CHASM_DEPTH, 4.0 - CHASM_DEPTH]);
    }

    #[test]
    fn chasm_runs_are_read_and_checked() {
        let mask = chasm_of(&serde_json::json!([[1, 2], [6, 1]]), 8).unwrap();
        assert_eq!(mask, vec![false, true, true, false, false, false, true, false]);
        assert_eq!(chasm_of(&serde_json::Value::Null, 3).unwrap(), vec![false; 3]);
        for bad in [serde_json::json!([[7, 2]]), serde_json::json!([[1]]), serde_json::json!({ "a": 1 }), serde_json::json!([[-1, 2]])] {
            assert!(chasm_of(&bad, 8).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_loaded_chasm_is_deep_unwalkable_and_dry() {
        // A 3 by 3 tile area at 1 m, a lake to 3 m over all of it, and a chasm across row 20 (the whole row).
        let n = 3 * 16 + 1;
        let path = area("chasm", serde_json::json!({ "chasm": [[20 * n, n]], "water": [{ "i": 2, "j": 2, "level": 3.0 }] }), None);
        let hm = read(&path).unwrap();
        let (y, x) = (20.0 * 0.625, 10.0);
        assert_eq!(hm.at(x, y), 1.0 - CHASM_DEPTH);
        assert_eq!(hm.chasm_depth(x, y), CHASM_DEPTH);
        assert!(hm.in_chasm(x, y) && !hm.in_chasm(x, 5.0));
        // Halfway down its wall, between the rim's row and the chasm's.
        assert!((hm.chasm_depth(x, 19.5 * 0.625) - CHASM_DEPTH / 2.0).abs() < 1e-4);
        // The lake stops at the chasm: it never fills it, so it stays south of it.
        assert!(hm.water_at(x, y).is_none() && hm.water_at(x, 5.0).is_some() && hm.water_at(x, 25.0).is_none());
        // No chasm: no rims, nothing in one.
        let plain = read(&area("plain", serde_json::json!({}), None)).unwrap();
        assert!(plain.rim.is_empty() && !plain.in_chasm(x, y));
    }

    #[test]
    fn bilinear_at_grid_points_and_between() {
        let hm = Heightmap {
            name: "t".into(),
            tiles: 2,
            tiles_y: 2,
            spacing: 10.0,
            width: 3,
            height: 3,
            data: vec![0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0],
            materials: vec![],
            palette: vec![],
            atmosphere: None,
            things: vec![],
            settings: None,
            originals: None,
            water: vec![],
            shadow: None,
            entrances: vec![],
            objects: vec![],
            ambience: vec![],
            underground: false,
            interior: false,
            roof: None,
            ceiling: vec![],
            rim: vec![],
            serial: 0,
            normals: Default::default(),
        };
        assert_eq!(hm.at(10.0, 0.0), 1.0);
        assert_eq!(hm.at(5.0, 0.0), 0.5);
        assert_eq!(hm.at(0.0, 15.0), 2.0); // halfway between 0 (row 1) and 4 (row 2)
        assert_eq!(hm.at(100.0, 100.0), 6.0); // clamped
    }

    #[test]
    fn a_plane_s_normal_is_the_same_everywhere_on_it_edges_and_all() {
        // Rising 1 m a metre east and 2 m a metre north (samples 1 m apart): the normal (-1, -2, 1), made unit.
        let data = (0..4).flat_map(|j| (0..4).map(move |i| i as f32 + 2.0 * j as f32)).collect();
        let hm = Heightmap { spacing: 1.0, width: 4, height: 4, data, ..plain() };
        let want = [-1.0, -2.0, 1.0].map(|c: f32| c / 6.0f32.sqrt());
        for (x, y) in [(0.0, 0.0), (1.5, 2.25), (3.0, 3.0), (2.0, 0.5)] {
            let n = hm.normal(x, y);
            assert!(n.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "({x}, {y}): {n:?}");
        }
    }

    #[test]
    fn a_bank_slanting_across_the_samples_is_lit_evenly_and_gentle_ground_keeps_its_own_slope() {
        // Gentle ground (3 degrees, rising east) with a river's bank across it at a slant: 1.6 m in one sample, along y = 0.4 x + 8.
        let (n, sp) = (49, 0.625);
        let data = (0..n)
            .flat_map(|j| (0..n).map(move |i| (i as f32 * sp, j as f32 * sp)))
            .map(|(x, y)| 0.05 * x + if y > 0.4 * x + 8.0 { 1.6 } else { 0.0 })
            .collect();
        let hm = Heightmap { spacing: sp, width: n, height: n, data, ..plain() };
        // Each sample's own slope, blended as `normal` blends: the normals before steep ground took the wide one.
        let near = |x: f32, y: f32| {
            let (fx, fy) = (x / sp, y / sp);
            let (i, j, tx, ty) = (fx as usize, fy as usize, fx.fract(), fy.fract());
            let mut v = [0.0; 3];
            for (i, j, w) in
                [(i, j, (1.0 - tx) * (1.0 - ty)), (i + 1, j, tx * (1.0 - ty)), (i, j + 1, (1.0 - tx) * ty), (i + 1, j + 1, tx * ty)]
            {
                let s = hm.slope_normal(i, j, 0);
                for k in 0..3 {
                    v[k] += s[k] * w;
                }
            }
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            v.map(|c| c / len)
        };
        // The sun's light (from the south, 37 degrees up) along the face, every 0.1 m: how far it swings.
        let sun = [0.0, -0.8, 0.6];
        let swing = |f: &dyn Fn(f32, f32) -> [f32; 3]| {
            let light: Vec<f32> = (50..200)
                .map(|k| k as f32 / 10.0)
                .map(|x| f(x, 0.4 * x + 8.0))
                .map(|v| v[0] * sun[0] + v[1] * sun[1] + v[2] * sun[2])
                .collect();
            light.iter().copied().fold(f32::MIN, f32::max) - light.iter().copied().fold(f32::MAX, f32::min)
        };
        let (before, now) = (swing(&near), swing(&|x, y| hm.normal(x, y)));
        assert!(before > 0.15 && now < 0.03, "light along the bank swings {before:.3} on each sample's slope, {now:.3} now");
        // 4 m from the bank, the ground's own slope, as before.
        let len = (0.05f32 * 0.05 + 1.0).sqrt();
        for x in [8.0, 12.0, 16.0] {
            let v = hm.normal(x, 0.4 * x + 4.0);
            assert!(v.iter().zip([-0.05 / len, 0.0, 1.0 / len]).all(|(a, b)| (a - b).abs() < 1e-6), "({x}): {v:?}");
        }
    }

    /// A flat 3 x 3 terrain of nothing else, for tests to change.
    fn plain() -> Heightmap {
        Heightmap {
            name: "t".into(),
            tiles: 1,
            tiles_y: 1,
            spacing: 5.0,
            width: 3,
            height: 3,
            data: vec![0.0; 9],
            materials: vec![],
            palette: vec![],
            atmosphere: None,
            things: vec![],
            settings: None,
            originals: None,
            water: vec![],
            shadow: None,
            entrances: vec![],
            objects: vec![],
            ambience: vec![],
            underground: false,
            interior: false,
            roof: None,
            ceiling: vec![],
            rim: vec![],
            serial: 0,
            normals: Default::default(),
        }
    }

    #[test]
    fn water_fills_the_hollow_it_starts_in() {
        // Two hollows (heights 0) parted by a ridge (5); the left one filled to 2.
        let h = [0.0, 5.0, 0.0, 0.0, 5.0, 0.0, 0.0, 5.0, 0.0];
        let w = flood(&h, 3, &[Body::new(0, 0, 2.0)]);
        assert_eq!(w.iter().filter(|l| !l.is_nan()).count(), 3);
        assert!(w[2].is_nan() && w[0] == 2.0);
        // Higher than the ridge: both.
        assert_eq!(flood(&h, 3, &[Body::new(0, 0, 6.0)]).iter().filter(|l| !l.is_nan()).count(), 9);
        // Bounded to the first column: only it, however high.
        let b = Body { bounds: Some([0, 0, 0, 2]), ..Body::new(0, 0, 6.0) };
        assert_eq!(flood(&h, 3, &[b]).iter().filter(|l| !l.is_nan()).count(), 3);
    }

    #[test]
    fn water_shallower_than_wet_depth_is_dry_but_still_carries_the_flood() {
        // A ford (1.95) between two pools (0) with water to 2: it joins them, and is dry itself.
        let h = [0.0, 1.95, 0.0];
        let w = dry_shallows(flood(&h, 3, &[Body::new(0, 0, 2.0)]), &h);
        assert!(w[0] == 2.0 && w[1].is_nan() && w[2] == 2.0);
        // A loaded area: a lake to 1.5 m over ground at 1 m, half of it a shelf under a few centimetres of water.
        let n = 3 * 16 + 1;
        let heights = (0..n * n).map(|k| if k % n < 24 { 1.0 } else { 1.5 - WET_DEPTH / 2.0 }).collect();
        let hm = read(&area("shallows", serde_json::json!({ "water": [{ "i": 2, "j": 2, "level": 1.5 }] }), Some(heights))).unwrap();
        assert!(hm.water_at(5.0, 10.0).is_some() && hm.water_at(25.0, 10.0).is_none());
    }

    /// An entrance as the editor sends one, with `id` if given.
    fn entrance(x: f32, id: Option<u64>) -> serde_json::Value {
        let mut e = serde_json::json!({ "x": x, "y": 0, "dir": 1.57, "width": 6, "depth": 8, "floor": 0 });
        if let Some(id) = id {
            e["id"] = id.into();
        }
        e
    }

    fn ids(list: &serde_json::Value) -> Vec<u32> {
        entrances_of(list).unwrap().iter().map(|e| e.id).collect()
    }

    #[test]
    fn an_area_from_before_entrance_ids_keeps_its_door_tags() {
        // Saved or exported before ids: each gets its place in the list, the number its door's tag always had
        // (`door_tag(k)` was `<area>_ent<k + 1>` over the entrances read, an unreadable one left out).
        let old = serde_json::json!([entrance(10.0, None), { "x": 1, "y": 2 }, entrance(20.0, None), entrance(30.0, None)]);
        assert_eq!(ids(&old), [1, 2, 3]);
        // Its next id starts after the highest.
        assert_eq!(next_entrance_id(&old, None).unwrap(), 4);
        assert_eq!(next_entrance_id(&serde_json::Value::Null, Some(&serde_json::json!({ "entrances": old }))).unwrap(), 4);
    }

    #[test]
    fn taking_out_a_middle_entrance_leaves_the_others_tags() {
        let all = serde_json::json!([entrance(10.0, Some(1)), entrance(20.0, Some(2)), entrance(30.0, Some(3))]);
        let without_2 = serde_json::json!([entrance(10.0, Some(1)), entrance(30.0, Some(3))]);
        assert_eq!(ids(&without_2), [1, 3], "entrance 3 keeps its id, so its door stays <area>_ent3");
        let before = serde_json::json!({ "entrances": all, "entranceNextId": 4 });
        assert_eq!(next_entrance_id(&without_2, Some(&before)).unwrap(), 4, "the counter doesn't go down");
    }

    #[test]
    fn an_entrance_id_is_never_given_twice() {
        // The last export had 1 and 3 (2 taken out earlier) and counted to 4.
        let before = serde_json::json!({ "entrances": [entrance(10.0, Some(1)), entrance(30.0, Some(3))], "entranceNextId": 4 });
        let with = |id| serde_json::json!([entrance(10.0, Some(1)), entrance(30.0, Some(3)), entrance(40.0, Some(id))]);
        let e = next_entrance_id(&with(2), Some(&before)).unwrap_err().to_string();
        assert!(e.contains("entrance id 2 was an entrance's that has been taken out") && e.contains("4 or more"), "{e}");
        assert_eq!(next_entrance_id(&with(4), Some(&before)).unwrap(), 5);
        // 4 taken out again: the counter, kept in the header, still remembers it.
        let after = serde_json::json!({ "entrances": [entrance(10.0, Some(1)), entrance(30.0, Some(3))], "entranceNextId": 5 });
        assert!(next_entrance_id(&with(4), Some(&after)).is_err());
        assert_eq!(next_entrance_id(&with(7), Some(&after)).unwrap(), 8, "a gap is fine");
        // Two with one id, and ids that can't be one.
        assert!(
            entrances_of(&serde_json::json!([entrance(1.0, Some(5)), entrance(2.0, Some(5))]))
                .unwrap_err()
                .to_string()
                .contains("two entrances")
        );
        for bad in [serde_json::json!(0), serde_json::json!(-1), serde_json::json!(1.5), serde_json::json!("2"), serde_json::json!(100_000)]
        {
            let mut e = entrance(1.0, None);
            e["id"] = bad.clone();
            assert!(entrances_of(&serde_json::json!([e])).is_err(), "{bad}");
        }
    }
}
