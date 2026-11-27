// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Baked ambience: the particle placements an area lists, written into the tiles that hold them.
//!
//! The area lists them as `"ambience": [{ "kind": "mist", "x": 22, "y": 20, "z": 10.3, "scale": 1, "seed": 7 }]` (area metres;
//! `bearing`, radians anticlockwise from east, turns the kinds that blow or lean along their model's +x). Each kind comes from the
//! particle pack (`packs/particles`, built into the program: its `pack.json` and one ASCII model a kind; [`Pack`]; another
//! with `LANDSCAPER_PARTICLES`). The export copies a kind's nodes
//! into the tile under its point, in the tile's coordinates, as docs/DESIGN-ambience.md's spike 0 found the client needs them:
//!
//! - every baked node of a tile hangs under one dummy named the tile model's name plus `a`, a child of the root; keyed emitters
//!   anywhere else keep their static birth rate;
//! - a value that differs by day and night (a night-only kind's birth rate, a darker night colour) is keyed in the tile's `day`
//!   and `night2day` (the day look) and `night` and `day2night` (the night look), and every animation lists the tile's whole
//!   node tree, as the base game's tiles do;
//! - a kind that `loops` (one a script may clear for a whole tile) has its birth rate 0, keyed on in `animloop01` and off in
//!   `tiledefault`: the `.are`'s `Tile_AnimLoop1` (the export sets it) starts it, and `SetTileAnimationLoops` switches it;
//! - always-on kinds need no keys.
//!
//! The client keeps only the meshes of the scenery beyond the area's edge, so a placement outside the area is skipped, with a
//! warning. So are a kind the pack doesn't have and placements past the area's live particle cap ([`AREA_LIVE_MAX`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail, ensure};
use mg_mdl::{Classification, Controller, Model, Node, NodeKind, ctrl};

/// The most particles one placement of a kind may keep alive (the design's "under 60 a placement").
pub const PLACEMENT_LIVE_MAX: f32 = 60.0;

/// The most particles an area's placements may keep alive together, by day or at night: the design's budget for a view. Placements
/// past it are skipped.
pub const AREA_LIVE_MAX: f32 = 1500.0;

/// Another particle pack than the one built in: `LANDSCAPER_PARTICLES`, a folder holding its `pack.json`.
pub const PACK_ENV: &str = "LANDSCAPER_PARTICLES";

/// One placement of a kind, as the area lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The pack's kind id (lower case).
    pub kind: String,
    /// Area metres; `z` is the height its model's root stands at.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Its size: the kind's positions, particle sizes, speeds and spawn area, times this.
    pub scale: f32,
    /// Radians anticlockwise from east: the kind's +x turns this way.
    pub bearing: f32,
    /// The editor's, for its own drawing; the export doesn't use it (the client's particles are random anyway).
    pub seed: u64,
}

/// The placements in an area header's `ambience` list (none when it has no list).
pub fn placements_of(list: &serde_json::Value) -> Result<Vec<Placement>> {
    let Some(items) = list.as_array() else { return Ok(Vec::new()) };
    items
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let num = |key: &str| p[key].as_f64().map(|v| v as f32);
            let kind = p["kind"].as_str().filter(|s| !s.is_empty()).with_context(|| format!("ambience {}: it has no kind", k + 1))?;
            let (Some(x), Some(y), Some(z)) = (num("x"), num("y"), num("z")) else {
                bail!("ambience {} ({kind}): its x, y and z must be numbers", k + 1);
            };
            Ok(Placement {
                kind: kind.to_ascii_lowercase(),
                x,
                y,
                z,
                scale: num("scale").unwrap_or(1.0),
                bearing: num("bearing").unwrap_or(0.0),
                seed: p["seed"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

/// When a kind shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shows {
    Always,
    Day,
    Night,
}

/// One controller of a node with its day and night values (`name` as the pack spells it: `colorStart`).
#[derive(Clone, Debug, PartialEq)]
struct Keyed {
    name: String,
    day: Vec<f32>,
    night: Vec<f32>,
}

impl Keyed {
    fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

/// A particle kind of the pack.
#[derive(Debug)]
pub struct Kind {
    pub id: String,
    /// Keyed on in `animloop01`, so a script can switch it for a whole tile.
    pub loops: bool,
    /// The particles one placement keeps alive by day and at night: each emitter's birth rate times its particles' life.
    pub live: [f32; 2],
    model: Model,
    /// The day and night values, by node of `model`.
    looks: Vec<(usize, Vec<Keyed>)>,
}

impl Kind {
    /// The textures its nodes draw with (lower case).
    fn textures(&self) -> impl Iterator<Item = String> + '_ {
        self.model.nodes.iter().filter_map(|n| match &n.kind {
            NodeKind::Emitter(e) => e.texture.as_deref().map(str::to_ascii_lowercase),
            NodeKind::Mesh(m) => m.textures[0].as_deref().map(str::to_ascii_lowercase),
            _ => None,
        })
    }
}

/// The particle pack: its kinds, and its own textures (resref, contents), which the area's hak carries for the kinds it uses.
#[derive(Debug)]
pub struct Pack {
    pub kinds: Vec<Kind>,
    textures: Vec<(String, Vec<u8>)>,
    /// Its `pack.json` as it is: the editor's Ambience palette (`/api/particles`).
    pub json: serde_json::Value,
}

/// The particle pack built into the program (`packs/particles`, by build.rs): each file's path in the pack and its contents.
mod built_in {
    include!(concat!(env!("OUT_DIR"), "/particles.rs"));
}

/// Emitter values that are lengths, speeds or an acceleration: a placement's scale multiplies them.
const SCALED: [&str; 12] = [
    "sizestart",
    "sizemid",
    "sizeend",
    "sizestart_y",
    "sizemid_y",
    "sizeend_y",
    "xsize",
    "ysize",
    "velocity",
    "randvel",
    "grav",
    "blurlength",
];

impl Pack {
    /// The pack in folder `dir` (its `pack.json`, `models/` and textures), checked as [`Pack::read`] checks it.
    pub fn load(dir: &Path) -> Result<Pack> {
        Pack::read(&|file| std::fs::read(dir.join(file)).with_context(|| format!("reading {}", dir.join(file).display())))
    }

    /// The pack built into the program (`packs/particles` when it was built).
    pub fn built_in() -> Result<Pack> {
        ensure!(!built_in::FILES.is_empty(), "this Landscaper was built without its particle pack (packs/particles)");
        Pack::read(&|file| {
            built_in::FILES.iter().find(|(f, _)| *f == file).map(|(_, data)| data.to_vec()).with_context(|| format!("no {file} in it"))
        })
    }

    /// The pack whose files `file` gives (by their path in the pack: `pack.json`, `models/lsp_mist.mdl`), checked: each kind's
    /// model has the nodes its looks name, a day-only kind keeps none alive at night (a night-only one none by day), a kind that
    /// loops shows always with one birth rate, every birth rate is 0 or 1 and more, and each kind stays within its budget and
    /// [`PLACEMENT_LIVE_MAX`].
    fn read(file: &dyn Fn(&str) -> Result<Vec<u8>>) -> Result<Pack> {
        let j: serde_json::Value = serde_json::from_slice(&file("pack.json")?).context("reading pack.json")?;
        ensure!(j["format"].as_u64() == Some(1), "pack.json: format {} (this Landscaper reads format 1)", j["format"]);
        let textures = j["textures"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                let name = t["name"].as_str().context("a texture without a name")?.to_ascii_lowercase();
                let data = file(t["file"].as_str().with_context(|| format!("texture {name}: no file"))?)
                    .with_context(|| format!("texture {name}"))?;
                Ok((name, data))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut kinds: Vec<Kind> = Vec::new();
        for k in j["kinds"].as_array().context("pack.json has no kinds")? {
            let kind = kind(file, k)?;
            ensure!(!kinds.iter().any(|o| o.id == kind.id), "two kinds have the id {}", kind.id);
            kinds.push(kind);
        }
        Ok(Pack { kinds, textures, json: j })
    }

    /// A kind by id.
    pub fn kind(&self, id: &str) -> Option<usize> {
        self.kinds.iter().position(|k| k.id.eq_ignore_ascii_case(id))
    }

    /// The pack's own textures the kinds `used` draw with: (resref, contents).
    pub fn textures_for(&self, used: &BTreeSet<usize>) -> Vec<(String, Vec<u8>)> {
        let names: BTreeSet<String> = used.iter().flat_map(|&k| self.kinds[k].textures()).collect();
        self.textures.iter().filter(|(n, _)| names.contains(n)).cloned().collect()
    }
}

/// Kind `k` of the pack whose files `file` gives, checked (see [`Pack::read`]).
fn kind(file: &dyn Fn(&str) -> Result<Vec<u8>>, k: &serde_json::Value) -> Result<Kind> {
    let id = k["id"].as_str().filter(|s| !s.is_empty()).context("a kind without an id")?.to_ascii_lowercase();
    let what = |s: &str| format!("kind {id}: {s}");
    let shows = match k["shows"].as_str() {
        Some("always") => Shows::Always,
        Some("day") => Shows::Day,
        Some("night") => Shows::Night,
        other => bail!(what(&format!("shows {other:?}, not always, day or night"))),
    };
    let loops = k["loops"].as_bool().unwrap_or(false);
    let budget = k["budget"].as_f64().with_context(|| what("no budget"))? as f32;
    let name = k["model"].as_str().with_context(|| what("no model"))?;
    let data = file(&format!("models/{name}.mdl")).with_context(|| what("its model"))?;
    let model = Model::read(&data).map_err(|e| anyhow::anyhow!("{}: {e}", what("its model")))?;
    ensure!(model.nodes.len() > 1, "{}", what("its model has no nodes under its root"));
    let mut looks = Vec::new();
    for (section, emitter) in [("emitters", true), ("meshes", false)] {
        for (node, look) in k[section].as_object().into_iter().flatten() {
            let n = model.node(node).with_context(|| what(&format!("its model has no node {node}")))?;
            let flags = match (&model.nodes[n].kind, emitter) {
                (NodeKind::Emitter(_), true) => ctrl::flags::EMITTER,
                (NodeKind::Mesh(_), false) => ctrl::flags::MESH,
                _ => bail!(what(&format!("{node} is no {}", if emitter { "emitter" } else { "mesh" }))),
            };
            let (Some(day), Some(night)) = (look["day"].as_object(), look["night"].as_object()) else {
                bail!(what(&format!("{node} needs a day and a night look")));
            };
            ensure!(
                day.keys().collect::<BTreeSet<_>>() == night.keys().collect::<BTreeSet<_>>(),
                "{}",
                what(&format!("{node}'s day and night looks set different values"))
            );
            let mut keyed = Vec::new();
            for (c, v) in day {
                let columns = ctrl::columns(flags, &c.to_ascii_lowercase())
                    .filter(|&n| n > 0)
                    .with_context(|| what(&format!("{node}: {c} is no controller of it")))?;
                let values = |v: &serde_json::Value| -> Option<Vec<f32>> {
                    match v {
                        serde_json::Value::Array(a) => a.iter().map(|x| x.as_f64().map(|x| x as f32)).collect(),
                        x => x.as_f64().map(|x| vec![x as f32]),
                    }
                    .filter(|v: &Vec<f32>| v.len() == columns && v.iter().all(|x| x.is_finite()))
                };
                let (Some(d), Some(n)) = (values(v), values(&night[c])) else {
                    bail!(what(&format!("{node}: {c} takes {columns} number(s) by day and at night")));
                };
                keyed.push(Keyed { name: c.clone(), day: d, night: n });
            }
            looks.push((n, keyed));
        }
    }
    ensure!(!loops || shows == Shows::Always, "{}", what("a kind that loops shows always: its loop sets its birth rate"));
    // Live particles: each emitter's birth rate (its look's, else its own) times its particles' life.
    let mut live = [0.0f32; 2];
    for (i, n) in model.nodes.iter().enumerate().filter(|(_, n)| matches!(n.kind, NodeKind::Emitter(_))) {
        let life = n.value("lifeexp").map_or(0.0, |v| v[0]);
        let own = n.value("birthrate").map_or(0.0, |v| v[0]);
        let rate = looks.iter().find(|(m, _)| *m == i).and_then(|(_, l)| l.iter().find(|c| c.is("birthrate")));
        let (day, night) = (rate.map_or(own, |r| r.day[0]), rate.map_or(own, |r| r.night[0]));
        // (The client draws nothing from a birth rate under 1 a second.)
        ensure!(
            [own, day, night].iter().all(|&b| b == 0.0 || b >= 1.0),
            "{}",
            what(&format!("{}: a birth rate under 1 draws nothing (0, or 1 and more)", n.name))
        );
        live[0] += day * life;
        live[1] += night * life;
        if loops {
            ensure!(rate.is_none_or(|r| r.day == r.night), "{}", what("a kind that loops has one birth rate, day and night"));
        }
    }
    match shows {
        Shows::Day => ensure!(live[1] == 0.0, "{}", what("it shows by day, so its birth rates at night are 0")),
        Shows::Night => ensure!(live[0] == 0.0, "{}", what("it shows at night, so its birth rates by day are 0")),
        Shows::Always => {}
    }
    ensure!(budget <= PLACEMENT_LIVE_MAX, "{}", what(&format!("its budget {budget} is over {PLACEMENT_LIVE_MAX} a placement")));
    let most = live[0].max(live[1]);
    ensure!(most <= budget + 1e-3, "{}", what(&format!("it keeps {most} particles alive, over its budget of {budget}")));
    Ok(Kind { id, loops, live, model, looks })
}

/// The particle pack the export uses, read once: the folder [`PACK_ENV`] names, else the one built into the program. An error
/// says why there is none.
pub fn installed() -> std::result::Result<&'static Pack, &'static str> {
    static PACK: OnceLock<std::result::Result<Pack, String>> = OnceLock::new();
    PACK.get_or_init(|| match crate::config::setting_os(PACK_ENV).filter(|d| !d.is_empty()) {
        Some(dir) => {
            let dir = Path::new(&dir);
            Pack::load(dir).map_err(|e| format!("the particle pack in {} ({PACK_ENV}): {e:#}", dir.display()))
        }
        None => Pack::built_in().map_err(|e| format!("{e:#}")),
    })
    .as_ref()
    .map_err(String::as_str)
}

/// A placement the export bakes, in its tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Baked {
    /// Index of its kind in the pack.
    pub kind: usize,
    /// Its point in the tile's coordinates (the tile's middle, at its base height).
    pub at: [f32; 3],
    pub scale: f32,
    pub bearing: f32,
}

/// A placement (or several alike) the export skipped, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    /// Area metres (the first one's, for several).
    pub at: [f32; 2],
    pub count: usize,
    pub why: String,
}

/// The area's placements sorted into its tiles.
#[derive(Debug, Default)]
pub struct Plan {
    /// By tile (column, row): its placements, in the area's order.
    pub tiles: BTreeMap<(usize, usize), Vec<Baked>>,
    pub skipped: Vec<Skipped>,
    /// The particles the baked placements keep alive together, by day and at night.
    pub live: [f32; 2],
    /// How many the area lists.
    pub placements: usize,
}

impl Plan {
    pub fn baked(&self) -> usize {
        self.tiles.values().map(Vec::len).sum()
    }

    /// The kinds it bakes.
    pub fn kinds(&self) -> BTreeSet<usize> {
        self.tiles.values().flatten().map(|b| b.kind).collect()
    }

    /// The live particle count, for the export's report (the editor shows it).
    pub fn report(&self) -> serde_json::Value {
        let round = |v: f32| (v * 10.0).round() / 10.0;
        serde_json::json!({
            "placements": self.placements,
            "baked": self.baked(),
            "tiles": self.tiles.len(),
            "live": { "day": round(self.live[0]), "night": round(self.live[1]) },
            "cap": AREA_LIVE_MAX,
        })
    }

    /// The report's warnings for what was skipped.
    pub fn warnings(&self) -> impl Iterator<Item = serde_json::Value> + '_ {
        self.skipped.iter().map(|s| serde_json::json!({ "kind": "ambience_skipped", "at": s.at, "count": s.count, "why": s.why }))
    }

    /// One line for the export's output.
    pub fn summary(&self) -> String {
        let skipped: usize = self.skipped.iter().map(|s| s.count).sum();
        format!(
            "ambience: {} of {} placements baked into {} tiles, {:.0} particles alive by day and {:.0} at night (at most \
             {AREA_LIVE_MAX:.0}){}",
            self.baked(),
            self.placements,
            self.tiles.len(),
            self.live[0],
            self.live[1],
            if skipped > 0 { format!(", {skipped} skipped") } else { String::new() }
        )
    }
}

/// `list`'s placements sorted into the area's tiles (`tiles`: columns, rows, 10 m each), each at its point less the tile's middle
/// and `base` height. Skipped: a kind `pack` doesn't have (or every one, without a pack), a point outside the area, and, in the
/// list's order, a placement that would take the area past [`AREA_LIVE_MAX`].
pub fn plan(list: &[Placement], pack: std::result::Result<&Pack, &str>, tiles: (usize, usize), base: impl Fn(usize, usize) -> f32) -> Plan {
    let mut plan = Plan { placements: list.len(), ..Plan::default() };
    let pack = match pack {
        Ok(p) => p,
        Err(why) => {
            if let Some(first) = list.first() {
                plan.skipped.push(Skipped { at: [first.x, first.y], count: list.len(), why: why.to_string() });
            }
            return plan;
        }
    };
    let (w, h) = (tiles.0 as f32 * TILE, tiles.1 as f32 * TILE);
    let mut over_cap: Option<Skipped> = None;
    for p in list {
        let skip = |why: String| Skipped { at: [p.x, p.y], count: 1, why };
        let Some(k) = pack.kind(&p.kind) else {
            plan.skipped.push(skip(format!("the particle pack has no kind {}", p.kind)));
            continue;
        };
        if !(0.0..w).contains(&p.x) || !(0.0..h).contains(&p.y) {
            plan.skipped
                .push(skip(format!("{} is outside the area: the scenery beyond its edge keeps no particles (place it inside)", p.kind)));
            continue;
        }
        let live = pack.kinds[k].live;
        if (plan.live[0] + live[0]).max(plan.live[1] + live[1]) > AREA_LIVE_MAX + 1e-3 {
            match &mut over_cap {
                Some(s) => s.count += 1,
                None => {
                    over_cap = Some(skip(format!(
                        "the area's placements would keep more than {AREA_LIVE_MAX:.0} particles alive: these weren't baked (place \
                         fewer)"
                    )));
                }
            }
            continue;
        }
        plan.live = [plan.live[0] + live[0], plan.live[1] + live[1]];
        let (tx, ty) = ((p.x / TILE) as usize, (p.y / TILE) as usize);
        let at = [p.x - (tx as f32 + 0.5) * TILE, p.y - (ty as f32 + 0.5) * TILE, p.z - base(tx, ty)];
        plan.tiles.entry((tx, ty)).or_default().push(Baked { kind: k, at, scale: p.scale, bearing: p.bearing });
    }
    plan.skipped.extend(over_cap);
    plan
}

/// A tile's side (m).
const TILE: f32 = 10.0;

/// The dummy a tile's baked nodes hang under: the tile's name plus `a`, or the client won't play their keys.
pub fn dummy_name(tile: &str) -> String {
    format!("{tile}a")
}

/// The name of node `k` of placement `p`'s kind (`node`, its name in the kind's model) in tile `tile`: unique in the tile, and
/// within the 31 characters a node name has.
fn node_name(tile: &str, p: usize, k: usize, node: &str) -> String {
    let named = format!("{tile}a{p}_{node}");
    if named.len() <= 31 { named } else { format!("{tile}a{p}_{k}") }
}

/// Tile `tile`'s baked nodes, as ASCII model text to go among its root's children: the `<tile>a` dummy and each placement's nodes
/// under it, turned, scaled and moved to its point, their static values the day look (a kind that loops: its birth rate 0).
pub fn tile_nodes(tile: &str, baked: &[Baked], pack: &Pack) -> String {
    if baked.is_empty() {
        return String::new();
    }
    let mut m = Model { name: tile.to_string(), classification: Classification::Tile, animation_scale: 1.0, ..Model::default() };
    m.nodes.push(Node::new(tile, NodeKind::Dummy));
    let mut dummy = Node::new(&dummy_name(tile), NodeKind::Dummy);
    dummy.parent = Some(0);
    m.nodes.push(dummy);
    for (p, b) in baked.iter().enumerate() {
        let kind = &pack.kinds[b.kind];
        let first = m.nodes.len();
        let turn = glam::Quat::from_rotation_z(b.bearing);
        for (k, n) in kind.model.nodes.iter().enumerate().skip(1) {
            let mut c = n.clone();
            c.name = node_name(tile, p, k, &n.name);
            c.children.clear();
            c.part = None;
            let scaled = glam::Vec3::from(n.position) * b.scale;
            match n.parent {
                // Under the kind's root: at the placement's point, turned by its bearing.
                Some(0) | None => {
                    c.parent = Some(1);
                    c.position = (glam::Vec3::from(b.at) + turn * scaled).into();
                    c.orientation = (turn * glam::Quat::from_array(n.orientation)).normalize().to_array();
                }
                Some(q) => {
                    c.parent = Some(first + q - 1);
                    c.position = scaled.into();
                }
            }
            match &mut c.kind {
                NodeKind::Emitter(_) => {
                    for ctl in c.controllers.iter_mut().filter(|ctl| SCALED.contains(&ctl.name.as_str())) {
                        ctl.values.iter_mut().for_each(|v| *v *= b.scale);
                    }
                }
                NodeKind::Mesh(mesh) => mesh.vertices.iter_mut().for_each(|v| *v = v.map(|x| x * b.scale)),
                _ => {}
            }
            for keyed in kind.looks.iter().filter(|(i, _)| *i == k).flat_map(|(_, l)| l) {
                let value = if kind.loops && keyed.is("birthrate") { vec![0.0] } else { keyed.day.clone() };
                set_controller(&mut c, &keyed.name.to_ascii_lowercase(), &value);
            }
            m.nodes.push(c);
        }
    }
    for i in 1..m.nodes.len() {
        if let Some(parent) = m.nodes[i].parent {
            m.nodes[parent].children.push(i);
        }
    }
    let text = mg_mdl::ascii_write::to_ascii(&m, &mg_mdl::ascii_write::Options { normals: true, banner: false });
    // The geometry's nodes after the root's.
    let geometry = text.split_once(&format!("beginmodelgeom {tile}\n")).map_or("", |(_, g)| g);
    let geometry = geometry.split_once(&format!("endmodelgeom {tile}\n")).map_or(geometry, |(g, _)| g);
    geometry.split_once("endnode\n").map_or_else(String::new, |(_, rest)| rest.to_string())
}

fn set_controller(n: &mut Node, name: &str, values: &[f32]) {
    match n.controllers.iter_mut().find(|c| c.name == name) {
        Some(c) => *c = Controller::constant(name, values),
        None => n.controllers.push(Controller::constant(name, values)),
    }
}

/// The tile animations baked ambience may need, in the base game's order: name, length and transition time (as the base game's
/// tiles have them).
const ANIMATIONS: [(&str, f32, f32); 6] = [
    ("animloop01", 0.033_333_3, 0.25),
    ("day", 0.3, 0.25),
    ("night", 0.3, 0.25),
    ("day2night", 0.033_333_3, 0.25),
    ("night2day", 0.033_333_3, 0.25),
    ("tiledefault", 0.0, 0.0),
];

/// Keys by animation, then by node: (controller, values).
type Keys = BTreeMap<&'static str, HashMap<String, Vec<(String, Vec<f32>)>>>;

/// The keys of tile `tile`'s baked placements: what differs by day and night in `day`, `night2day` (the day look), `night` and
/// `day2night` (the night look); a looping kind's birth rate on in `animloop01` and off in `tiledefault`.
fn keys(tile: &str, baked: &[Baked], pack: &Pack) -> Keys {
    let mut keys = Keys::new();
    for (p, b) in baked.iter().enumerate() {
        let kind = &pack.kinds[b.kind];
        for (k, looks) in &kind.looks {
            let node = node_name(tile, p, *k, &kind.model.nodes[*k].name);
            for keyed in looks {
                let mut put = |anim: &'static str, v: &[f32]| {
                    keys.entry(anim).or_default().entry(node.clone()).or_default().push((keyed.name.clone(), v.to_vec()));
                };
                if kind.loops && keyed.is("birthrate") {
                    put("animloop01", &keyed.day);
                    put("tiledefault", &[0.0]);
                } else if keyed.day != keyed.night {
                    put("day", &keyed.day);
                    put("night2day", &keyed.day);
                    put("night", &keyed.night);
                    put("day2night", &keyed.night);
                }
            }
        }
    }
    keys
}

/// The animations of tile `tile` for its baked placements, as ASCII model text to follow its geometry (`model`, its text so far:
/// each animation lists every node of it). None are needed when nothing is keyed.
pub fn tile_animations(tile: &str, model: &str, baked: &[Baked], pack: &Pack) -> String {
    let keys = keys(tile, baked, pack);
    if keys.is_empty() {
        return String::new();
    }
    // Every node of the geometry, with its parent.
    let mut nodes = Vec::new();
    let mut lines = model.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("node ") {
            let mut words = rest.split_whitespace();
            let (Some(kind), Some(name)) = (words.next(), words.next()) else { continue };
            let parent = lines.next().and_then(|l| l.strip_prefix("parent ")).unwrap_or("NULL");
            nodes.push((kind.to_string(), name.to_string(), parent.trim().to_string()));
        }
    }
    let mut s = String::new();
    for (anim, length, transtime) in ANIMATIONS {
        let Some(by_node) = keys.get(anim) else { continue };
        let _ = writeln!(s, "newanim {anim} {tile}\n  length {}\n  transtime {}\n  animroot {tile}", num(length), num(transtime));
        for (kind, name, parent) in &nodes {
            let _ = writeln!(s, "  node {kind} {name}\n    parent {parent}");
            for (c, v) in by_node.get(name).into_iter().flatten() {
                let row = v.iter().map(|x| num(*x)).collect::<Vec<_>>().join(" ");
                let times: &[f32] = if length > 0.0 { &[0.0, length] } else { &[0.0] };
                let _ = writeln!(s, "    {c}key {}", times.len());
                for t in times {
                    let _ = writeln!(s, "      {} {row}", num(*t));
                }
                s.push_str("    endlist\n");
            }
            s.push_str("  endnode\n");
        }
        let _ = writeln!(s, "doneanim {anim} {tile}");
    }
    s
}

fn num(f: f32) -> String {
    mg_mdl::ascii_write::num(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test pack: three kinds in the particle pack's shape (`tests/ambience-pack`).
    fn test_pack() -> Pack {
        Pack::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ambience-pack")).unwrap()
    }

    fn at(kind: &str, x: f32, y: f32, z: f32) -> Placement {
        Placement { kind: kind.into(), x, y, z, scale: 1.0, bearing: 0.0, seed: 0 }
    }

    #[test]
    fn an_areas_list_reads_with_its_defaults_and_a_bad_entry_is_refused() {
        let list = serde_json::json!([
            { "kind": "Mist", "x": 22, "y": 20, "z": 10.3, "scale": 1, "seed": 7 },
            { "kind": "motes", "x": 1.5, "y": 2, "z": 0, "bearing": 1.57 }
        ]);
        let got = placements_of(&list).unwrap();
        assert_eq!(got[0], Placement { kind: "mist".into(), x: 22.0, y: 20.0, z: 10.3, scale: 1.0, bearing: 0.0, seed: 7 });
        assert_eq!((got[1].scale, got[1].bearing, got[1].seed), (1.0, 1.57, 0));
        assert!(placements_of(&serde_json::Value::Null).unwrap().is_empty());
        let err = placements_of(&serde_json::json!([{ "kind": "mist", "x": 1, "y": 2 }])).unwrap_err().to_string();
        assert!(err.contains("ambience 1 (mist): its x, y and z must be numbers"), "{err}");
        assert!(placements_of(&serde_json::json!([{ "x": 1, "y": 2, "z": 0 }])).is_err());
    }

    #[test]
    fn the_test_pack_reads_with_its_live_counts() {
        let pack = test_pack();
        let kind = |id: &str| &pack.kinds[pack.kind(id).unwrap()];
        assert_eq!((kind("mist").loops, kind("mist").live), (true, [18.0, 18.0]));
        assert_eq!((kind("fireflies").loops, kind("fireflies").live), (false, [0.0, 18.0]));
        assert_eq!((kind("motes").loops, kind("motes").live), (false, [30.0, 0.0]));
        let used = BTreeSet::from([pack.kind("fireflies").unwrap()]);
        let tex: Vec<String> = pack.textures_for(&used).into_iter().map(|(n, _)| n).collect();
        assert_eq!(tex, ["lspt_firefly"]);
    }

    /// The particle pack is built into the program: it passes the checks, matches packs/particles file for file, and bakes with no
    /// pack folder to read (what the export uses when `LANDSCAPER_PARTICLES` isn't set).
    #[test]
    fn the_built_in_particle_pack_bakes_with_no_folder() {
        let pack = Pack::built_in().unwrap_or_else(|e| panic!("{e:#}"));
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/particles");
        for (file, data) in built_in::FILES {
            assert_eq!(std::fs::read(dir.join(file)).unwrap(), *data, "{file}");
        }
        assert!(built_in::FILES.iter().any(|(f, _)| *f == "pack.json") && pack.kinds.len() >= 3);
        assert!(pack.kinds.iter().all(|k| k.live[0].max(k.live[1]) <= PLACEMENT_LIVE_MAX));
        if crate::config::setting_os(PACK_ENV).is_none() {
            assert_eq!(installed().map(|p| p.kinds.len()), Ok(pack.kinds.len()));
        }
        let list = [at("mist", 15.0, 15.0, 1.0), at("fireflies", 5.0, 5.0, 1.0), at("motes", 25.0, 5.0, 1.0)];
        let plan = plan(&list, Ok(&pack), (3, 3), |_, _| 0.0);
        assert_eq!((plan.baked(), plan.skipped.len()), (3, 0));
        let flies = &plan.tiles[&(0, 0)];
        let m = compiled(&tile_text("amb_0000", flies, &pack));
        assert_eq!(m.nodes[m.node("amb_0000a0_flies").unwrap()].parent, m.node("amb_0000a"));
        assert!(m.animation("night").is_some());
        let tex: Vec<String> = pack.textures_for(&plan.kinds()).into_iter().map(|(n, _)| n).collect();
        assert!(tex.contains(&"lspt_firefly".to_string()), "{tex:?}");
    }

    /// A pack in a scratch folder: the test pack with `pack.json` changed by `edit`.
    fn edited(name: &str, edit: impl Fn(&mut serde_json::Value)) -> Result<Pack> {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ambience-pack");
        let dir = std::env::temp_dir().join(format!("ls-ambience-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("models")).unwrap();
        for sub in ["models", "textures"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            for e in std::fs::read_dir(from.join(sub)).unwrap().flatten() {
                std::fs::copy(e.path(), dir.join(sub).join(e.file_name())).unwrap();
            }
        }
        let mut j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(from.join("pack.json")).unwrap()).unwrap();
        edit(&mut j);
        std::fs::write(dir.join("pack.json"), j.to_string()).unwrap();
        let pack = Pack::load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        pack
    }

    #[test]
    fn a_pack_breaking_the_rules_is_refused() {
        let err = |name: &str, edit: &dyn Fn(&mut serde_json::Value)| format!("{:#}", edited(name, edit).unwrap_err());
        // Over its budget.
        assert!(err("budget", &|j| j["kinds"][0]["budget"] = 10.into()).contains("over its budget of 10"));
        // Over 60 a placement.
        assert!(err("sixty", &|j| j["kinds"][0]["budget"] = 61.into()).contains("over 60 a placement"));
        // A night-only kind alive by day.
        assert!(
            err("night", &|j| j["kinds"][1]["emitters"]["flies"]["day"]["birthrate"] = 1.into()).contains("its birth rates by day are 0")
        );
        // A looping kind with two birth rates, or one that isn't always on.
        assert!(err("loopday", &|j| j["kinds"][0]["emitters"]["mist"]["night"]["birthrate"] = 1.into()).contains("one birth rate"));
        assert!(err("loopshows", &|j| j["kinds"][2]["loops"] = true.into()).contains("shows always"));
        // A look naming what the model hasn't, or giving a colour one number.
        assert!(err("node", &|j| j["kinds"][0]["emitters"]["fog"] = j["kinds"][0]["emitters"]["mist"].clone()).contains("no node fog"));
        assert!(err("columns", &|j| j["kinds"][0]["emitters"]["mist"]["day"]["colorStart"] = 1.into()).contains("takes 3 number(s)"));
        // A birth rate the client wouldn't draw.
        assert!(err("under1", &|j| j["kinds"][2]["emitters"]["motes"]["day"]["birthrate"] = 0.5.into()).contains("under 1 draws nothing"));
        assert!(edited("fine", |_: &mut serde_json::Value| {}).is_ok());
    }

    #[test]
    fn placements_go_to_the_tile_holding_their_point_in_its_coordinates() {
        let pack = test_pack();
        let list = [at("mist", 22.0, 20.5, 10.3), at("fireflies", 29.99, 0.0, 1.0), at("motes", 0.5, 5.0, 2.0)];
        let plan = plan(&list, Ok(&pack), (3, 3), |tx, ty| (tx * 10 + ty) as f32);
        let mist = &plan.tiles[&(2, 2)][0];
        assert_eq!(mist.kind, pack.kind("mist").unwrap());
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(close(mist.at, [-3.0, -4.5, 10.3 - 22.0]), "{:?}", mist.at);
        assert!(close(plan.tiles[&(2, 0)][0].at, [4.99, -5.0, 1.0 - 20.0]));
        assert!(close(plan.tiles[&(0, 0)][0].at, [-4.5, 0.0, 2.0]));
        assert_eq!((plan.baked(), plan.live, plan.skipped.len()), (3, [48.0, 36.0], 0));
        assert_eq!(plan.report()["live"], serde_json::json!({ "day": 48.0, "night": 36.0 }));
    }

    #[test]
    fn outside_the_area_an_unknown_kind_and_past_the_cap_are_skipped_with_a_warning() {
        let pack = test_pack();
        let mut list = vec![at("mist", -0.5, 5.0, 0.0), at("mist", 5.0, 30.0, 0.0), at("rain", 5.0, 5.0, 0.0)];
        // 1,500 particles is 83 mists (18 each) and a fraction: the 84th and on are skipped.
        list.extend((0..90).map(|i| at("mist", 0.1 + i as f32 * 0.3, 5.0, 0.0)));
        let plan = plan(&list, Ok(&pack), (3, 3), |_, _| 0.0);
        assert_eq!(plan.baked(), 83);
        assert_eq!(plan.live, [83.0 * 18.0; 2]);
        let whys: Vec<(usize, &str)> = plan.skipped.iter().map(|s| (s.count, s.why.as_str())).collect();
        assert_eq!(whys.len(), 4, "{whys:?}");
        assert!(whys[0].1.contains("outside the area") && whys[1].1.contains("outside the area"));
        assert!(whys[2].1.contains("no kind rain"));
        assert_eq!(whys[3].0, 7);
        assert!(whys[3].1.contains("more than 1500 particles"));
        assert_eq!(plan.warnings().count(), 4);
        // No pack: every placement is skipped, in one warning that says why.
        let none = super::plan(&list, Err("no particle pack"), (3, 3), |_, _| 0.0);
        assert_eq!((none.baked(), none.skipped.len(), none.skipped[0].count), (0, 1, list.len()));
    }

    /// The text of a small tile model with `baked`'s nodes and animations, as tile.rs writes it.
    fn tile_text(tile: &str, baked: &[Baked], pack: &Pack) -> String {
        let mut s = format!(
            "newmodel {tile}\nsetsupermodel {tile} NULL\nclassification tile\nsetanimationscale 1\nbeginmodelgeom {tile}\n\
             node dummy {tile}\n  parent NULL\nendnode\nnode light {tile}ml1\n  parent {tile}\n  position 0 0 5\n  radius 14\n  \
             color 0 0 0\nendnode\nnode trimesh {tile}_gnd\n  parent {tile}\n  bitmap grass\n  verts 3\n    -5 -5 0\n    5 -5 0\n    \
             5 5 0\n  tverts 3\n    0 0 0\n    1 0 0\n    1 1 0\n  faces 1\n    0 1 2 1 0 1 2 1\nendnode\n"
        );
        s += &tile_nodes(tile, baked, pack);
        s += &format!("endmodelgeom {tile}\n");
        let anims = tile_animations(tile, &s, baked, pack);
        s + &anims + &format!("donemodel {tile}\n")
    }

    fn baked(pack: &Pack, kind: &str, at: [f32; 3]) -> Baked {
        Baked { kind: pack.kind(kind).unwrap(), at, scale: 1.0, bearing: 0.0 }
    }

    /// The compiled tile, read back.
    fn compiled(text: &str) -> Model {
        let c = mg_mdl::compile::compile(text.as_bytes(), &Default::default()).unwrap();
        Model::read(&c.binary).unwrap()
    }

    fn key(m: &Model, anim: &str, node: &str, ctl: &str) -> Option<Vec<f32>> {
        let a = m.animation(anim)?;
        let n = a.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(node))?;
        n.controllers.iter().find(|c| c.name == ctl).map(|c| c.row(0).to_vec())
    }

    #[test]
    fn a_tile_s_placements_hang_under_its_a_dummy_keyed_day_and_night() {
        let pack = test_pack();
        let tile = "amb_0202";
        let list = [baked(&pack, "fireflies", [1.0, 2.0, 0.5]), baked(&pack, "motes", [-1.0, 0.0, 3.0])];
        let m = compiled(&tile_text(tile, &list, &pack));
        let dummy = m.node("amb_0202a").expect("the <tile>a dummy");
        assert_eq!(m.nodes[dummy].parent, Some(0));
        let flies = m.node("amb_0202a0_flies").unwrap();
        let motes = m.node("amb_0202a1_motes").unwrap();
        assert_eq!((m.nodes[flies].parent, m.nodes[motes].parent), (Some(dummy), Some(dummy)));
        // At the placement's point plus the emitter's own offset in its kind (0.8 m up).
        let p = m.nodes[flies].position;
        assert!((p[0] - 1.0).abs() < 1e-5 && (p[1] - 2.0).abs() < 1e-5 && (p[2] - 1.3).abs() < 1e-5, "{p:?}");
        // Night-only: off as it stands, on at night and at dusk, off by day and at dawn. Day-only: the reverse.
        assert_eq!(m.nodes[flies].value("birthrate"), Some(&[0.0][..]));
        assert_eq!(m.nodes[motes].value("birthrate"), Some(&[3.0][..]));
        for (anim, flies_rate, motes_rate) in [("day", 0.0, 3.0), ("night2day", 0.0, 3.0), ("night", 3.0, 0.0), ("day2night", 3.0, 0.0)] {
            assert_eq!(key(&m, anim, "amb_0202a0_flies", "birthrate"), Some(vec![flies_rate]), "{anim}");
            assert_eq!(key(&m, anim, "amb_0202a1_motes", "birthrate"), Some(vec![motes_rate]), "{anim}");
            // Every animation lists the whole tree.
            assert_eq!(m.animation(anim).unwrap().nodes.len(), m.nodes.len(), "{anim}");
        }
        // Nothing loops here.
        assert!(m.animation("animloop01").is_none() && m.animation("tiledefault").is_none());
        // Both ride with the stock animations' lengths.
        assert!((m.animation("day").unwrap().length - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_looping_kind_is_off_until_animloop01_and_its_night_colour_is_keyed() {
        let pack = test_pack();
        let tile = "amb_0101";
        let m = compiled(&tile_text(tile, &[baked(&pack, "mist", [0.0, 0.0, 0.3])], &pack));
        let mist = m.node("amb_0101a0_mist").unwrap();
        assert_eq!(m.nodes[mist].value("birthrate"), Some(&[0.0][..]));
        assert_eq!(key(&m, "animloop01", "amb_0101a0_mist", "birthrate"), Some(vec![2.0]));
        assert_eq!(key(&m, "tiledefault", "amb_0101a0_mist", "birthrate"), Some(vec![0.0]));
        assert_eq!(key(&m, "night", "amb_0101a0_mist", "birthrate"), None);
        assert_eq!(key(&m, "night", "amb_0101a0_mist", "colorstart"), Some(vec![0.3, 0.34, 0.43]));
        assert_eq!(key(&m, "day", "amb_0101a0_mist", "colorstart"), Some(vec![0.92, 0.94, 0.96]));
        assert_eq!(m.nodes[mist].value("colorstart"), Some(&[0.92, 0.94, 0.96][..]));
    }

    #[test]
    fn an_always_on_kind_with_one_look_needs_no_animations() {
        let pack = test_pack();
        // The motes' look by day as an always-on kind's only look: nothing to key.
        let text = tile_text("amb_0000", &[], &pack);
        assert!(!text.contains("newanim"));
        let mut kinds = test_pack();
        let k = kinds.kind("mist").unwrap();
        kinds.kinds[k].loops = false;
        for (_, looks) in &mut kinds.kinds[k].looks {
            looks.iter_mut().for_each(|l| l.night = l.day.clone());
        }
        let text = tile_text("amb_0000", &[baked(&kinds, "mist", [0.0; 3])], &kinds);
        assert!(text.contains("node dummy amb_0000a") && !text.contains("newanim"), "{text}");
        let m = compiled(&text);
        assert_eq!(m.nodes[m.node("amb_0000a0_mist").unwrap()].value("birthrate"), Some(&[2.0][..]));
    }

    #[test]
    fn a_placement_s_bearing_and_scale_turn_and_size_its_nodes() {
        let pack = test_pack();
        let mut b = baked(&pack, "motes", [2.0, 0.0, 1.0]);
        (b.bearing, b.scale) = (std::f32::consts::FRAC_PI_2, 2.0);
        let m = compiled(&tile_text("amb_0303", &[b], &pack));
        let n = &m.nodes[m.node("amb_0303a0_motes").unwrap()];
        // The motes sit 1 m along the kind's +x and 1.5 m up: turned north and doubled.
        let p = n.position;
        assert!((p[0] - 2.0).abs() < 1e-4 && (p[1] - 2.0).abs() < 1e-4 && (p[2] - 4.0).abs() < 1e-4, "{p:?}");
        // Its +x turned north; its up still up, so flat kinds (`Billboard_to_Local_Z`) stay flat.
        let turn = glam::Quat::from_array(n.orientation);
        assert!((turn * glam::Vec3::X - glam::Vec3::Y).length() < 1e-4, "{turn:?}");
        assert!((turn * glam::Vec3::Z - glam::Vec3::Z).length() < 1e-4, "{turn:?}");
        assert_eq!(n.value("sizestart"), Some(&[0.7][..]));
        assert_eq!(n.value("xsize"), Some(&[500.0][..]));
        // Its life and birth rate (so its live count) stay.
        assert_eq!((n.value("lifeexp"), n.value("birthrate")), (Some(&[10.0][..]), Some(&[3.0][..])));
    }
}
