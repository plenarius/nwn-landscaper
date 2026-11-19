// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Choosing a tile's PathNode from its walkmesh.
//!
//! The game server's tile-to-tile planner does not read walkmeshes: it uses a built-in layout per PathNode letter and orientation
//! (measured on a real game install into src/pathnodes.json). Each layout splits the tile into nodes, each reaching some of 20 points
//! on the tile's edges (1, 2.5, 5, 7.5 and 9 m along each side). A tile must get a layout whose nodes only join edge points its
//! walkmesh joins, or creatures are sent along routes they cannot walk.

use std::collections::HashMap;

/// Tile coordinates 0..10, x east, y north.
pub type Pt = (f32, f32);

/// A path node layout the generator may give a tile: its letter and orientation, and where its nodes and exits are.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The path node letter the `.set` gives a tile with this layout.
    pub letter: char,
    /// Its orientation, in quarter turns.
    pub orientation: u8,
    /// Each node's exit points.
    pub nodes: Vec<Vec<Pt>>,
    /// Each node's position (same order as `nodes`).
    pub positions: Vec<Pt>,
}

/// One letter and orientation's table in src/pathnodes.json, as measured on the game server, in the canonical frame: its
/// nodes, its exit points on the tile's edges, and the node that owns each exit.
pub struct Measured {
    /// The path node letter.
    pub letter: char,
    /// The orientation, in quarter turns.
    pub orientation: u8,
    /// Each region's node point.
    pub nodes: Vec<Pt>,
    /// The exit points on the tile's edges.
    pub exits: Vec<Pt>,
    /// The node each exit belongs to.
    pub exit_node: Vec<usize>,
}

/// Every table of src/pathnodes.json, in the order of their keys (`<letter><orientation>`); read once.
pub fn measured() -> &'static [Measured] {
    #[derive(serde::Deserialize)]
    struct Entry {
        nodes: Vec<[f64; 2]>,
        exits: Vec<[f64; 2]>,
        exit_node: Vec<usize>,
    }
    static TABLES: std::sync::OnceLock<Vec<Measured>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let json: std::collections::BTreeMap<String, Entry> =
            serde_json::from_str(include_str!("pathnodes.json")).expect("pathnodes.json holds the measured tables");
        let pts = |v: Vec<[f64; 2]>| v.into_iter().map(|[x, y]| (x as f32, y as f32)).collect();
        json.into_iter()
            .map(|(key, e)| Measured {
                letter: key.chars().next().expect("a table's key starts with its letter"),
                orientation: key[1..].parse().expect("a table's key ends with its orientation"),
                nodes: pts(e.nodes),
                exits: pts(e.exits),
                exit_node: e.exit_node,
            })
            .collect()
    })
}

/// Each letter and orientation's layout (the nodes that own an exit), sorted by letter and orientation; made once.
pub fn layouts() -> &'static [Layout] {
    static LAYOUTS: std::sync::OnceLock<Vec<Layout>> = std::sync::OnceLock::new();
    LAYOUTS.get_or_init(|| {
        let mut out: Vec<Layout> = measured()
            .iter()
            .map(|m| {
                let mut nodes = vec![Vec::new(); m.nodes.len()];
                for (&p, &o) in m.exits.iter().zip(&m.exit_node) {
                    nodes[o].push(p);
                }
                let (mut kept, mut positions) = (Vec::new(), Vec::new());
                for (n, &p) in nodes.into_iter().zip(&m.nodes) {
                    if !n.is_empty() {
                        kept.push(n);
                        positions.push(p);
                    }
                }
                Layout { letter: m.letter, orientation: m.orientation, nodes: kept, positions }
            })
            .collect();
        out.sort_by_key(|l| (l.letter, l.orientation));
        out
    })
}

/// Every edge point any layout uses.
pub fn edge_points(layouts: &[Layout]) -> Vec<Pt> {
    let mut pts: Vec<Pt> = layouts.iter().flat_map(|l| l.nodes.iter().flatten().copied()).collect();
    pts.sort_by(|a, b| a.partial_cmp(b).expect("layout points are measured numbers, never NaN"));
    pts.dedup();
    pts
}

fn key(p: Pt) -> (i32, i32) {
    ((p.0 * 100.0).round() as i32, (p.1 * 100.0).round() as i32)
}

/// Which walkable region of a tile each edge point lies in. `verts` are in
/// tile coordinates (0..10); `faces` carry whether they are walkable.
pub struct Regions {
    at: HashMap<(i32, i32), Option<usize>>,
    /// Each walkable face's region (None for non-walkable faces).
    face_region: Vec<Option<usize>>,
    verts: Vec<[f32; 3]>,
    faces: Vec<([u32; 3], bool)>,
}

impl Regions {
    /// The regions of a tile's walkmesh, and which one each of `points` is in; a point near an edge snaps up to 1.25 m in, to the
    /// ground the engine would cross to.
    pub fn new(verts: &[[f32; 3]], faces: &[([u32; 3], bool)], points: &[Pt]) -> Regions {
        Regions::with_snap(verts, faces, points, &[0.3, 0.75, 1.25])
    }

    /// As `new`, for our own generated tiles: an exit only counts where the ground right at the edge (0.3 m in) is walkable, since the
    /// engine has to cross the edge there; it still snaps along the edge.
    pub fn strict(verts: &[[f32; 3]], faces: &[([u32; 3], bool)], points: &[Pt]) -> Regions {
        Regions::with_snap(verts, faces, points, &[0.3])
    }

    fn with_snap(verts: &[[f32; 3]], faces: &[([u32; 3], bool)], points: &[Pt], depths: &[f32]) -> Regions {
        // Faces meet where they share an edge (by position, so duplicated vertices still join).
        let pos = |i: u32| {
            let v = verts[i as usize];
            ((v[0] * 1000.0).round() as i64, (v[1] * 1000.0).round() as i64, (v[2] * 1000.0).round() as i64)
        };
        let mut parent: Vec<usize> = (0..faces.len()).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        let mut edges: HashMap<_, usize> = HashMap::new();
        for (fi, (f, walk)) in faces.iter().enumerate() {
            if !walk {
                continue;
            }
            for k in 0..3 {
                let (a, b) = (pos(f[k]), pos(f[(k + 1) % 3]));
                let e = if a < b { (a, b) } else { (b, a) };
                if let Some(&other) = edges.get(&e) {
                    let (ra, rb) = (find(&mut parent, fi), find(&mut parent, other));
                    parent[ra] = rb;
                } else {
                    edges.insert(e, fi);
                }
            }
        }
        let mut at = HashMap::new();
        for &p in points {
            // Inside the edge (at `depths`); the engine snaps an exit to the nearest
            // walkable ground, so look a little along and into the tile.
            let horizontal = p.1 <= 0.0 || p.1 >= 10.0;
            let inward = if p.0 <= 0.0 || p.1 <= 0.0 { 1.0 } else { -1.0 };
            let mut region = None;
            'search: for &depth in depths {
                for along in [0.0f32, 0.5, -0.5, 1.0, -1.0] {
                    let q = if horizontal { (p.0 + along, p.1 + inward * depth) } else { (p.0 + inward * depth, p.1 + along) };
                    for (fi, (f, walk)) in faces.iter().enumerate() {
                        if *walk && contains(verts, f, q) {
                            region = Some(find(&mut parent, fi));
                            break 'search;
                        }
                    }
                }
            }
            at.insert(key(p), region);
        }
        let face_region = (0..faces.len()).map(|i| faces[i].1.then(|| find(&mut parent, i))).collect();
        Regions { at, face_region, verts: verts.to_vec(), faces: faces.to_vec() }
    }

    /// Walkable faces in regions that touch the tile border but belong to no
    /// node of `l`: crossings there go nowhere the engine knows about.
    pub fn unowned_border_faces(&self, l: &Layout) -> Vec<usize> {
        let owned: Vec<Option<usize>> = l.nodes.iter().map(|n| self.region(n[0])).collect();
        let on_border = |v: [f32; 3]| v[0] <= 1e-3 || v[0] >= 10.0 - 1e-3 || v[1] <= 1e-3 || v[1] >= 10.0 - 1e-3;
        let touching: std::collections::HashSet<usize> = self
            .faces
            .iter()
            .enumerate()
            .filter(|(i, (f, _))| self.face_region[*i].is_some() && f.iter().any(|&v| on_border(self.verts[v as usize])))
            .filter_map(|(i, _)| self.face_region[i])
            .collect();
        (0..self.faces.len())
            .filter(|&i| matches!(self.face_region[i], Some(r) if touching.contains(&r) && !owned.contains(&Some(r))))
            .collect()
    }

    /// The region of the walkable face under a point, as the engine's FindTriangle finds it (walkable faces only).
    pub fn region_at(&self, p: Pt) -> Option<usize> {
        self.faces.iter().enumerate().find(|(_, (f, w))| *w && contains(&self.verts, f, p)).and_then(|(i, _)| self.face_region[i])
    }

    /// The region point `p` (one of those given to [`Regions::new`]) is in; None when on no walkable ground.
    pub fn region(&self, p: Pt) -> Option<usize> {
        self.at.get(&key(p)).copied().flatten()
    }

    /// Whether the layout fits: every node joins only points of one region, and no two nodes share a region (the engine maps each node
    /// to its own region of the walkmesh, and crashes in FindClosestRegion otherwise).
    pub fn allows(&self, l: &Layout) -> bool {
        let mut seen = Vec::new();
        l.nodes.iter().zip(&l.positions).all(|(n, &pos)| {
            let r = self.region(n[0]);
            let ok = r.is_some() && n.iter().all(|&p| self.region(p) == r) && !seen.contains(&r);
            seen.push(r);
            // FindClosestRegion finds a node by the walkable triangle under its
            // position; a node off its own region sends creatures astray.
            let placed = l.nodes.len() < 2 || std::env::var_os("LS_IGNORE_POS").is_some() || self.region_at(pos) == r;
            ok && placed
        })
    }
}

fn contains(verts: &[[f32; 3]], f: &[u32; 3], p: Pt) -> bool {
    let [a, b, c] = f.map(|i| verts[i as usize]);
    let s = |u: [f32; 3], v: [f32; 3]| (v[0] - u[0]) * (p.1 - u[1]) - (v[1] - u[1]) * (p.0 - u[0]);
    let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// The edge a point lies on: 0 south, 1 east, 2 north, 3 west.
fn side(p: Pt) -> Option<usize> {
    if p.1 <= 0.0 {
        Some(0)
    } else if p.0 >= 10.0 {
        Some(1)
    } else if p.1 >= 10.0 {
        Some(2)
    } else if p.0 <= 0.0 {
        Some(3)
    } else {
        None
    }
}

/// The same point seen from the neighbour across its edge.
fn across(p: Pt) -> Pt {
    match side(p) {
        Some(0) => (p.0, 10.0),
        Some(1) => (0.0, p.1),
        Some(2) => (p.0, 0.0),
        _ => (10.0, p.1),
    }
}

/// Chooses a layout for each tile of a `width` × `height` grid (row-major from the south-west): only layouts the tile's walkmesh
/// allows, and among those the ones whose exits meet their neighbours' (an exit leads on only if the neighbour's layout has the same
/// point). Returns indices into `layouts`, or None for a tile no layout fits. The crossing points GenerateBestIntraTileExit tries when
/// a route leaves through exit `p` of `layout` (FINDINGS / sim::intra exit_table): five points along the edge, spaced by how many
/// exits that edge has and where `p` sits among them. Points off the edge are dropped.
pub fn crossing_candidates(layout: &Layout, p: Pt) -> Vec<Pt> {
    let horizontal = p.1 <= 0.0 || p.1 >= 10.0;
    let along = |q: Pt| if horizontal { q.0 } else { q.1 };
    let fixed = |q: Pt| if horizontal { q.1 } else { q.0 };
    let on_edge: Vec<Pt> = layout.nodes.iter().flatten().copied().filter(|&q| (fixed(q) - fixed(p)).abs() <= 1e-4).collect();
    let n = on_edge.len();
    let k = on_edge.iter().filter(|&&q| along(q) < along(p)).count();
    let (inner, outer) = match (n, k) {
        (2, _) => (1.0, 2.0),
        (3, 1) => (1.2, 2.4),
        (3, _) => (0.4, 0.8),
        _ => (2.0, 4.0),
    };
    let c = along(p);
    [c - outer, c - inner, c, c + inner, c + outer]
        .into_iter()
        .filter(|v| (0.05..=9.95).contains(v))
        .map(|v| if horizontal { (v, p.1) } else { (p.0, v) })
        .collect()
}

/// A point on a tile edge moved `d` metres into the tile.
fn inset(p: Pt, d: f32) -> Pt {
    let x = if p.0 <= 0.0 {
        d
    } else if p.0 >= 10.0 {
        10.0 - d
    } else {
        p.0
    };
    let y = if p.1 <= 0.0 {
        d
    } else if p.1 >= 10.0 {
        10.0 - d
    } else {
        p.1
    };
    (x, y)
}

/// Whether leaving tile `from` (layout `lf`) through exit `p` into `to` (layout `lt`) can pick a crossing that fails: a candidate
/// walkable on the leaving side but outside the exit's region there, or outside the entry exit's region on the entering side.
fn crossing_breaks(lf: &Layout, rf: &Regions, rt: &Regions, p: Pt) -> bool {
    let (own_from, own_to) = (rf.region(p), rt.region(across(p)));
    crossing_candidates(lf, p).into_iter().any(|c| {
        let here = rf.region_at(inset(c, 0.3));
        here.is_some() && (here != own_from || rt.region_at(inset(across(c), 0.3)) != own_to)
    })
}

/// A layout for each tile (row-major, `width` by `height`) whose exits fall on its own walkable regions and agree with its neighbours'
/// across each edge; None where none fits.
pub fn choose(layouts: &[Layout], regions: &[Regions], width: usize, height: usize) -> Vec<Option<usize>> {
    let clock = std::time::Instant::now();
    let lap = |what: &str| {
        if std::env::var_os("LS_TIMING").is_some() {
            eprintln!("    choose {what} {:.2}s", clock.elapsed().as_secs_f32());
        }
    };
    let allowed: Vec<Vec<usize>> = regions.iter().map(|r| (0..layouts.len()).filter(|&i| r.allows(&layouts[i])).collect()).collect();
    let exits = |i: usize| layouts[i].nodes.iter().flatten().copied().collect::<Vec<Pt>>();
    let neighbour = |t: usize, s: usize| -> Option<usize> {
        let (x, y) = (t % width, t / width);
        match s {
            0 if y > 0 => Some(t - width),
            1 if x + 1 < width => Some(t + 1),
            2 if y + 1 < height => Some(t + width),
            3 if x > 0 => Some(t - 1),
            _ => None,
        }
    };
    if let Some(t) = std::env::var("LS_DEBUG_TILE").ok().and_then(|v| v.parse::<usize>().ok()) {
        let pts = edge_points(layouts);
        let regs: Vec<String> = pts
            .iter()
            .map(|&p| {
                format!(
                    "{}{}:{:?}",
                    ["S", "E", "N", "W"][side(p).expect("a layout's nodes are on the tile's edges")],
                    if p.1 <= 0.0 || p.1 >= 10.0 { p.0 } else { p.1 },
                    regions[t].region(p)
                )
            })
            .collect();
        eprintln!("tile {t}: edge regions {}", regs.join(" "));
        eprintln!(
            "tile {t}: {} layouts allowed: {}",
            allowed[t].len(),
            allowed[t].iter().map(|&i| format!("{}{}", layouts[i].letter, layouts[i].orientation)).collect::<Vec<_>>().join(" ")
        );
    }
    lap("allowed");
    // Start from the layout with the most exits, fewest nodes.
    let mut pick: Vec<Option<usize>> =
        allowed.iter().map(|c| c.iter().copied().max_by_key(|&i| (exits(i).len(), usize::MAX - layouts[i].nodes.len()))).collect();
    for _pass in 0..6 {
        let mut changed = false;
        for t in 0..pick.len() {
            let score = |i: usize, pick: &[Option<usize>]| -> i64 {
                let mut s = 0i64;
                for p in exits(i) {
                    let sd = side(p).expect("a layout's nodes are on the tile's edges");
                    match neighbour(t, sd) {
                        // A matched exit is what lets routes cross the edge.
                        Some(n) => {
                            if let Some(j) = pick[n]
                                && exits(j).iter().any(|&q| key(q) == key(across(p)))
                            {
                                // A crossing that can be picked badly (either way) gets denied mid-route, which can lose the whole
                                // request: worse than having no exit there.
                                let bad = crossing_breaks(&layouts[i], &regions[t], &regions[n], p)
                                    || crossing_breaks(&layouts[j], &regions[n], &regions[t], across(p));
                                s += if bad && std::env::var_os("LS_NO_SPREAD").is_none() { 2 } else { 10 };
                            }
                        }
                        None => s += 1, // the area's border: harmless
                    }
                }
                s * 100 - layouts[i].nodes.len() as i64
            };
            let best = allowed[t].iter().copied().max_by_key(|&i| score(i, &pick));
            if best != pick[t] && best.map(|b| score(b, &pick)) > pick[t].map(|p| score(p, &pick)) {
                pick[t] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    lap("passes");
    if std::env::var_os("LS_NO_UNBREAK").is_none() {
        pick = remove_bad_crossings(layouts, regions, &allowed, pick, &neighbour);
    }
    lap("unbreak");
    if std::env::var_os("LS_DEBUG_BREAKS").is_some() {
        for t in 0..pick.len() {
            let Some(i) = pick[t] else { continue };
            for p in exits(i) {
                let Some(n) = neighbour(t, side(p).expect("a layout's nodes are on the tile's edges")) else { continue };
                let Some(j) = pick[n] else { continue };
                if exits(j).iter().any(|&q| key(q) == key(across(p))) && crossing_breaks(&layouts[i], &regions[t], &regions[n], p) {
                    let ok_alternatives = allowed[t].len();
                    eprintln!(
                        "  BREAK tile {t} ({}{}) exit {:?} -> tile {n} ({}{}); {} layouts allowed here",
                        layouts[i].letter, layouts[i].orientation, p, layouts[j].letter, layouts[j].orientation, ok_alternatives
                    );
                }
            }
        }
    }
    pick
}

/// Walkable faces on a tile edge whose crossing the engine can't use: the regions either side are not joined through any exit of the
/// chosen layouts (e.g. a pass between the fixed exit positions). Returns (tile, face) pairs; making them non-walkable keeps the
/// walkmesh honest about what can be pathed.
pub fn unrepresented_crossings(
    layouts: &[Layout],
    regions: &[Regions],
    pick: &[Option<usize>],
    width: usize,
    height: usize,
) -> Vec<(usize, usize)> {
    let neighbour = |t: usize, s: usize| -> Option<usize> {
        let (x, y) = (t % width, t / width);
        match s {
            0 if y > 0 => Some(t - width),
            1 if x + 1 < width => Some(t + 1),
            2 if y + 1 < height => Some(t + width),
            3 if x > 0 => Some(t - 1),
            _ => None,
        }
    };
    let graph = region_graph(layouts, regions, pick, &neighbour);
    let mut out = Vec::new();
    for (t, r) in regions.iter().enumerate() {
        for (fi, (f, walk)) in r.faces.iter().enumerate() {
            if !walk {
                continue;
            }
            for k in 0..3 {
                let (p, q) = (r.verts[f[k] as usize], r.verts[f[(k + 1) % 3] as usize]);
                let edge_side = |a: f32, b: f32| (a - b).abs() < 1e-3 && (a <= 1e-3 || a >= 10.0 - 1e-3);
                if !(edge_side(p[0], q[0]) || edge_side(p[1], q[1])) {
                    continue;
                }
                let m: Pt = ((p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0);
                let m = (
                    if m.0 < 1e-3 {
                        0.0
                    } else if m.0 > 10.0 - 1e-3 {
                        10.0
                    } else {
                        m.0
                    },
                    if m.1 < 1e-3 {
                        0.0
                    } else if m.1 > 10.0 - 1e-3 {
                        10.0
                    } else {
                        m.1
                    },
                );
                let Some(sd) = side(m) else { continue };
                let Some(n) = neighbour(t, sd) else { continue };
                let (Some(rt), Some(rn)) = (r.face_region[fi], regions[n].region_at(inset(across(m), 0.05))) else { continue };
                let joined = match (graph.get(&(t, rt)), graph.get(&(n, rn))) {
                    (Some(a), Some(b)) => a == b,
                    _ => false,
                };
                if !joined {
                    out.push((t, fi));
                }
            }
        }
    }
    out
}

/// The tile-to-tile graph the engine would see: (tile, region) joined
/// wherever both tiles' layouts have the exit point, as union-find roots.
fn region_graph(
    layouts: &[Layout],
    regions: &[Regions],
    pick: &[Option<usize>],
    neighbour: &dyn Fn(usize, usize) -> Option<usize>,
) -> HashMap<(usize, usize), (usize, usize)> {
    let mut parent: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    fn find(p: &mut HashMap<(usize, usize), (usize, usize)>, k: (usize, usize)) -> (usize, usize) {
        let mut r = k;
        while let Some(&q) = p.get(&r) {
            if q == r {
                break;
            }
            r = q;
        }
        p.insert(k, r);
        r
    }
    for (t, i) in pick.iter().enumerate() {
        let Some(i) = i else { continue };
        for node in &layouts[*i].nodes {
            let Some(r) = regions[t].region(node[0]) else { continue };
            parent.entry((t, r)).or_insert((t, r));
            for &p in node {
                let Some(n) = neighbour(t, side(p).expect("a layout's nodes are on the tile's edges")) else { continue };
                let Some(j) = pick[n] else { continue };
                if !layouts[j].nodes.iter().flatten().any(|&q| key(q) == key(across(p))) {
                    continue;
                }
                let Some(rn) = regions[n].region(across(p)) else { continue };
                parent.entry((n, rn)).or_insert((n, rn));
                let (a, b) = (find(&mut parent, (t, r)), find(&mut parent, (n, rn)));
                parent.insert(a, b);
            }
        }
    }
    let keys: Vec<_> = parent.keys().copied().collect();
    keys.into_iter().map(|k| (k, find(&mut parent, k))).collect()
}

/// The matched crossings out of tile `t` that can be picked badly.
fn bad_from(
    layouts: &[Layout],
    pick: &[Option<usize>],
    neighbour: &dyn Fn(usize, usize) -> Option<usize>,
    t: usize,
    breaks: &dyn Fn(usize, usize, usize, Pt) -> bool,
) -> usize {
    let Some(i) = pick[t] else { return 0 };
    let mut bad = 0;
    for p in layouts[i].nodes.iter().flatten().copied() {
        let Some(n) = neighbour(t, side(p).expect("a layout's nodes are on the tile's edges")) else { continue };
        let Some(j) = pick[n] else { continue };
        if layouts[j].nodes.iter().flatten().any(|&q| key(q) == key(across(p))) && breaks(t, i, n, p) {
            bad += 1;
        }
    }
    bad
}

/// Greedily swaps layouts to remove crossings that can be picked badly, accepting a swap only if every pair of regions connected
/// through exits in the starting choice stays connected.
fn remove_bad_crossings(
    layouts: &[Layout],
    regions: &[Regions],
    allowed: &[Vec<usize>],
    mut pick: Vec<Option<usize>>,
    neighbour: &dyn Fn(usize, usize) -> Option<usize>,
) -> Vec<Option<usize>> {
    let reference = region_graph(layouts, regions, &pick, neighbour);
    let keeps = |pick: &[Option<usize>]| -> bool {
        let now = region_graph(layouts, regions, pick, neighbour);
        // Every reference component must still be one component.
        let mut seen: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        for (k, root) in &reference {
            let Some(&r_now) = now.get(k) else { return false };
            match seen.get(root) {
                Some(&r) if r != r_now => return false,
                None => {
                    seen.insert(*root, r_now);
                }
                _ => {}
            }
        }
        true
    };
    // Whether a crossing can break depends only on the two tiles' walkmeshes and the leaving layout, so it is worked out once; a swap
    // at `t` only changes the crossings out of `t` and its neighbours.
    let cache = std::cell::RefCell::new(HashMap::new());
    let breaks = |t: usize, i: usize, n: usize, p: Pt| -> bool {
        *cache.borrow_mut().entry((t, i, key(p))).or_insert_with(|| crossing_breaks(&layouts[i], &regions[t], &regions[n], p))
    };
    let around = |t: usize| -> Vec<usize> { std::iter::once(t).chain((0..4).filter_map(|s| neighbour(t, s))).collect() };
    let local =
        |pick: &[Option<usize>], t: usize| -> usize { around(t).into_iter().map(|u| bad_from(layouts, pick, neighbour, u, &breaks)).sum() };
    let exits_of = |i: Option<usize>| -> usize { i.map_or(0, |i| layouts[i].nodes.iter().map(Vec::len).sum()) };
    let mut bad: usize = (0..pick.len()).map(|t| bad_from(layouts, &pick, neighbour, t, &breaks)).sum();
    for _round in 0..8 {
        let mut improved = false;
        for t in 0..pick.len() {
            for &i in &allowed[t] {
                if Some(i) == pick[t] {
                    continue;
                }
                let before = local(&pick, t);
                let old = pick[t];
                pick[t] = Some(i);
                let b = bad - before + local(&pick, t);
                let more_exits = exits_of(Some(i)) > exits_of(old);
                if (b < bad || (b == bad && more_exits)) && keeps(&pick) {
                    bad = b;
                    improved = true;
                } else {
                    pick[t] = old;
                }
            }
        }
        if !improved || bad == 0 {
            break;
        }
    }
    pick
}
