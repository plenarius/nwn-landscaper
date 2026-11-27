// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Edge scenery from the halo (`docs/DESIGN-canvas-cut.md`, "Edge scenery", variants G and T): each boundary tile's model reaches
//! out over the halo beside it, so the game draws the real ground beyond the area's edge, lit and culled with the tile. The edge
//! models the client repeats 5 deep stay empty on an area with a halo.
//!
//! - **Ground:** a vertex every 2.5 m, no walkmesh. A 2.5 m cell a shore runs through, or whose ground the coarse grid misses by
//!   more than [`MISS`] (a road's groove, a ditch, a bank), is drawn at the samples' 0.625 m. Cells meet without cracks: an edge takes
//!   the finer side's vertices, and an edge on the area's border takes every vertex of the tile's own outline.
//! - **Its texture:** one [`HALO_BAKE`] pixel picture a halo tile, the paint and the baked shade in it.
//! - **Water:** the halo's wet samples as rectangles at their level.
//! - **Trees:** the halo's baked objects, each drawn by the boundary tile nearest it. A tree [`CARD_FROM`] or more past the edge is
//!   cards (`cards`), the rest their models.
//! - **The fade:** everything here is drawn by `ls_fd_fs`, which mixes towards the fog colour from [`crate::shared::FADE`] metres past the area's
//!   edge, so the scenery's end meets the background instead of stopping.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;

use super::cards::{self, Atlas, CARD_FROM};
use super::{Grid, Painter, TILE, WATER_METRES, WATER_TEXTURE, height, mdl, num, shadow_strength, tiles_x, tiles_y, tileset_name};
use crate::{heightmap, objects, paint, shade};

/// A coarse cell's side (m).
const COARSE: f32 = 2.5;
/// Fine cells a coarse cell's side: the samples' 0.625 m.
const FINE: usize = 4;
/// The picture of a halo tile (pixels a side): its paint and shade.
pub(super) const HALO_BAKE: u32 = 128;
/// A coarse cell whose ground the 2.5 m grid misses by more than this (m) is drawn at the samples' spacing.
const MISS: f32 = 0.25;

/// What the edge scenery holds, counted (triangles).
#[derive(Default)]
pub(super) struct Counts {
    pub(super) ground: AtomicUsize,
    pub(super) water: AtomicUsize,
    pub(super) trees: AtomicUsize,
    pub(super) cards: AtomicUsize,
    pub(super) card_trees: AtomicUsize,
    pub(super) model_trees: AtomicUsize,
}

/// The edge scenery of the loaded area, worked out once for every boundary tile.
pub(super) struct Overhang {
    hm: Arc<heightmap::Heightmap>,
    /// Halo tiles beyond each side.
    k: i64,
    /// The coarse cells drawn fine (2.5 m cells, counted from the area's south-west corner).
    fine: HashSet<(i64, i64)>,
    /// The halo's baked objects (indices into the scene's) by the boundary tile that draws them.
    trees: HashMap<(usize, usize), Vec<usize>>,
    /// The fade material of each texture the halo's tree models use.
    materials: BTreeMap<String, String>,
    /// The atlases of the models drawn as cards, and each one's number.
    atlases: BTreeMap<String, (Arc<Atlas>, usize)>,
    pub(super) counts: Counts,
}

/// The name of halo tile (i, j) (tiles from the area's south-west corner; negative beyond its west or south), `k` tiles of halo:
/// its picture's and its material's.
fn halo_name(i: i64, j: i64, k: i64) -> String {
    format!("{}_h{:02}{:02}", tileset_name(), i + k, j + k)
}

/// A fade material: our shaders, `bitmap`, the area's size for the fade, the fragment shader `fs` (`ls_fd_fs`, or `ls_fdt_fs` for
/// the top of a card).
fn fade_mtr(bitmap: &str, fs: &str) -> String {
    let (w, h) = (tiles_x() as f32 * TILE, tiles_y() as f32 * TILE);
    format!(
        "// NWN Landscaper: edge scenery, fading to the fog colour past the area's edge\ncustomshadervs ls_lm_vs\ncustomshaderfs {fs}\n\
         texture0 {bitmap}\nparameter float lsAreaW {}\nparameter float lsAreaH {}\n",
        num(w),
        num(h)
    )
}

impl Overhang {
    /// The loaded area's edge scenery; None without a halo. `dump`: each card atlas written there as a picture.
    pub(super) fn plan(dump: Option<&std::path::Path>) -> Result<Option<Overhang>> {
        let Some(hm) = heightmap::current() else { return Ok(None) };
        let Some(halo) = hm.halo.as_ref() else { return Ok(None) };
        let per_tile = (TILE / hm.spacing).round() as usize;
        let k = (halo.n / per_tile) as i64;
        if k == 0 {
            return Ok(None);
        }
        let (nx, ny) = (tiles_x() as i64, tiles_y() as i64);
        let per = (TILE / COARSE) as i64;
        let inside = |ci: i64, cj: i64| (0..nx * per).contains(&ci) && (0..ny * per).contains(&cj);
        let mut fine = HashSet::new();
        for cj in -k * per..(ny + k) * per {
            for ci in -k * per..(nx + k) * per {
                if !inside(ci, cj) && Self::refine(&hm, ci, cj) {
                    fine.insert((ci, cj));
                }
            }
        }
        let (mut trees, mut materials, mut atlases): (HashMap<_, Vec<usize>>, BTreeMap<_, _>, BTreeMap<String, (Arc<Atlas>, usize)>) =
            Default::default();
        if let Some(scene) = objects::scene() {
            for (i, o) in scene.objects.iter().enumerate().filter(|(_, o)| o.halo && o.baked) {
                let Some(placed) = hm.objects.get(i) else { continue };
                let owner = |v: f32, n: i64| ((v / TILE).floor() as i64).clamp(0, n - 1) as usize;
                trees.entry((owner(placed.x, nx), owner(placed.y, ny))).or_default().push(i);
                if past(&hm, placed.x, placed.y) >= CARD_FROM && cards::is_tree(&placed.model) {
                    if !atlases.contains_key(&placed.model) {
                        match cards::atlas(&placed.model) {
                            Ok(a) => {
                                if let Some(dir) = dump {
                                    cards::dump(dir, &placed.model, &a)?;
                                }
                                let n = atlases.len();
                                atlases.insert(placed.model.clone(), (Arc::new(a), n));
                            }
                            Err(e) => eprintln!("  {}: drawn as its model past the edge, no cards ({e:#})", placed.model),
                        }
                    }
                    if atlases.contains_key(&placed.model) {
                        continue;
                    }
                }
                for (texture, _, _, _) in &o.groups {
                    let n = materials.len();
                    materials.entry(texture.to_ascii_lowercase()).or_insert_with(|| format!("{}_f{n}", tileset_name()));
                }
            }
        }
        Ok(Some(Overhang { hm, k, fine, trees, materials, atlases, counts: Counts::default() }))
    }

    /// Whether coarse cell (ci, cj) is drawn fine: a shore runs through it (wet and dry samples in it), or the coarse grid misses its
    /// ground by more than [`MISS`].
    fn refine(hm: &heightmap::Heightmap, ci: i64, cj: i64) -> bool {
        let f = FINE as i64;
        let (i0, j0) = (ci * f, cj * f);
        let corner = |a: i64, b: i64| hm.sample_height(i0 + a * f, j0 + b * f);
        let (h00, h10, h01, h11) = (corner(0, 0), corner(1, 0), corner(0, 1), corner(1, 1));
        let mut wet = (false, false);
        for b in 0..=f {
            for a in 0..=f {
                if hm.surface_level(i0 + a, j0 + b).is_some() {
                    wet.0 = true;
                } else {
                    wet.1 = true;
                }
                let (u, v) = (a as f32 / f as f32, b as f32 / f as f32);
                let coarse = (h00 * (1.0 - u) + h10 * u) * (1.0 - v) + (h01 * (1.0 - u) + h11 * u) * v;
                if (hm.sample_height(i0 + a, j0 + b) - coarse).abs() > MISS {
                    return true;
                }
            }
        }
        wet.0 && wet.1
    }

    /// The halo tiles boundary tile (tx, ty) draws: the row of halo tiles beyond each of its sides on the border, and at a corner
    /// of the area the block beyond it.
    fn owned(&self, tx: usize, ty: usize) -> Vec<(i64, i64)> {
        let (nx, ny, k) = (tiles_x() as i64, tiles_y() as i64, self.k);
        let (tx, ty) = (tx as i64, ty as i64);
        let (west, east, south, north) = (tx == 0, tx == nx - 1, ty == 0, ty == ny - 1);
        let mut out = Vec::new();
        let xs = |lo: bool, hi: bool| -> Vec<i64> {
            let mut v = Vec::new();
            if lo {
                v.extend(-k..0);
            }
            if hi {
                v.extend(nx..nx + k);
            }
            v
        };
        let ys = |lo: bool, hi: bool| -> Vec<i64> {
            let mut v = Vec::new();
            if lo {
                v.extend(-k..0);
            }
            if hi {
                v.extend(ny..ny + k);
            }
            v
        };
        for i in xs(west, east) {
            out.push((i, ty));
        }
        for j in ys(south, north) {
            out.push((tx, j));
        }
        for i in xs(west, east) {
            for j in ys(south, north) {
                out.push((i, j));
            }
        }
        out
    }

    /// Every halo tile, with its picture as a DDS and its material.
    pub(super) fn textures(&self, painter: Option<&Painter>) -> Vec<(String, Vec<u8>, String)> {
        let (nx, ny, k) = (tiles_x() as i64, tiles_y() as i64, self.k);
        let tiles: Vec<(i64, i64)> = (-k..ny + k)
            .flat_map(|j| (-k..nx + k).map(move |i| (i, j)))
            .filter(|&(i, j)| !((0..nx).contains(&i) && (0..ny).contains(&j)))
            .collect();
        let strength = shadow_strength();
        let sun = !self.hm.underground;
        let hm = self.hm.clone();
        super::in_parallel(&tiles, |(i, j)| {
            let (ox, oy) = (i as f32 * TILE, j as f32 * TILE);
            // The shade on a coarser grid (it changes slowly), raised to the colour space the shader lights textures in.
            const SHADE: usize = 33;
            let ground = |x: f32, y: f32| hm.at(x, y);
            let shade: Vec<f32> = (0..SHADE * SHADE)
                .map(|n| {
                    let (x, y) = (ox + (n % SHADE) as f32 * TILE / (SHADE - 1) as f32, oy + (n / SHADE) as f32 * TILE / (SHADE - 1) as f32);
                    if strength > 0.0 { shade::shade_lit(x, y, strength, sun, &ground).powf(1.0 / 2.2) } else { 1.0 }
                })
                .collect();
            let shade_at = |u: f32, v: f32| {
                let (fx, fy) = (u * (SHADE - 1) as f32, v * (SHADE - 1) as f32);
                let (a, b) = ((fx.floor() as usize).min(SHADE - 2), (fy.floor() as usize).min(SHADE - 2));
                let (tx, ty) = (fx - a as f32, fy - b as f32);
                let s = |a: usize, b: usize| shade[b * SHADE + a];
                (s(a, b) * (1.0 - tx) + s(a + 1, b) * tx) * (1.0 - ty) + (s(a, b + 1) * (1.0 - tx) + s(a + 1, b + 1) * tx) * ty
            };
            let colour = painter.map(|p| p.bake_square(ox, oy, HALO_BAKE));
            let picture = paint::picture(HALO_BAKE, HALO_BAKE, |px, py| {
                let (u, v) = ((px as f32 + 0.5) / HALO_BAKE as f32, (py as f32 + 0.5) / HALO_BAKE as f32);
                let c = colour.as_ref().map_or([0.30, 0.38, 0.18], |c| {
                    let p = c.pixel(px, py);
                    [f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0]
                });
                // (A chasm in the halo darkens towards its floor, as its walls do in the area.)
                let dark = 1.0 - (hm.chasm_depth(ox + u * TILE, oy + v * TILE) / heightmap::CHASM_DEPTH * 2.0).clamp(0.0, 1.0);
                let s = shade_at(u, v) * dark;
                c.map(|c| c * s)
            });
            let name = halo_name(i, j, k);
            let mtr = fade_mtr(&name, "ls_fd_fs");
            (name, paint::dds(&picture), mtr)
        })
    }

    /// The fade materials of the halo's tree textures and cards, and the card atlases: (resref, MTR) and (resref, DDS).
    pub(super) fn materials(&self) -> (Vec<(String, String)>, Vec<(String, Vec<u8>)>) {
        let mut mtrs: Vec<(String, String)> = self.materials.iter().map(|(t, m)| (m.clone(), fade_mtr(t, "ls_fd_fs"))).collect();
        let mut atlases = Vec::new();
        for (model, (a, n)) in &self.atlases {
            let atlas = cards::atlas_name(model);
            mtrs.push((self.card_mtr(*n, false), fade_mtr(&atlas, "ls_fd_fs")));
            mtrs.push((self.card_mtr(*n, true), fade_mtr(&atlas, "ls_fdt_fs")));
            atlases.push((atlas, cards::dds(&a.image)));
        }
        (mtrs, atlases)
    }

    fn card_mtr(&self, n: usize, top: bool) -> String {
        format!("{}_{}{n}", tileset_name(), if top { 't' } else { 'c' })
    }

    /// The edge scenery's nodes for boundary tile (tx, ty) (empty for any other), in its model `name` whose (0, 0, 0) is area point
    /// `origin`. `g`: the tile's ground mesh as drawn (tile-centred), whose vertices on the area's border the scenery meets.
    pub(super) fn nodes(&self, name: &str, tx: usize, ty: usize, g: &Grid, origin: [f32; 3]) -> String {
        let owned = self.owned(tx, ty);
        let mut s = String::new();
        if owned.is_empty() && !self.trees.contains_key(&(tx, ty)) {
            return s;
        }
        let [ox, oy, base] = origin;
        let (w, h) = (tiles_x() as f32 * TILE, tiles_y() as f32 * TILE);
        // The tile's vertices on the area's border, in area metres, by side (west, east, south, north), sorted along it.
        let mut outline: [Vec<[f32; 3]>; 4] = Default::default();
        for v in &g.verts {
            let p = [v[0] + ox, v[1] + oy, v[2] + base];
            for (side, on) in
                [p[0].abs() < 1e-3, (p[0] - w).abs() < 1e-3, p[1].abs() < 1e-3, (p[1] - h).abs() < 1e-3].into_iter().enumerate()
            {
                if on {
                    outline[side].push(p);
                }
            }
        }
        for (side, l) in outline.iter_mut().enumerate() {
            let along = if side < 2 { 1 } else { 0 };
            l.sort_by(|a, b| a[along].total_cmp(&b[along]));
            l.dedup_by(|a, b| (a[along] - b[along]).abs() < 1e-4);
        }
        let mesh = Mesh { fine: &self.fine, outline: &outline, size: [w, h] };
        for (n, &(i, j)) in owned.iter().enumerate() {
            let (verts, faces) = mesh.tile(i, j);
            if faces.is_empty() {
                continue;
            }
            self.counts.ground.fetch_add(faces.len(), Ordering::Relaxed);
            let local: Vec<[f32; 3]> = verts.iter().map(|p| [p[0] - ox, p[1] - oy, p[2] - base]).collect();
            let normals: Vec<[f32; 3]> = verts.iter().map(|p| self.hm.normal(p[0], p[1])).collect();
            let (x0, y0) = (i as f32 * TILE, j as f32 * TILE);
            let uvs: Vec<[f32; 2]> =
                verts.iter().map(|p| [((p[0] - x0) / TILE).clamp(0.002, 0.998), ((p[1] - y0) / TILE).clamp(0.002, 0.998)]).collect();
            let tex = halo_name(i, j, self.k);
            mdl::write_node(
                &mut s,
                &format!("{name}_h{n}"),
                name,
                &tex,
                &format!("  materialname {tex}\n"),
                &local,
                Some(&normals),
                &uvs,
                None,
                None,
                &faces,
                &faces,
            );
            if let Some(water) = self.water(i, j) {
                self.counts.water.fetch_add(water.len() / 3, Ordering::Relaxed);
                let verts: Vec<[f32; 3]> = water.iter().map(|p| [p[0] - ox, p[1] - oy, p[2] - base]).collect();
                let uvs: Vec<[f32; 2]> = water.iter().map(|p| [p[0] / WATER_METRES, p[1] / WATER_METRES]).collect();
                mdl::trimesh_listed(&mut s, &format!("{name}_hw{n}"), name, WATER_TEXTURE, "  transparencyhint 1\n", &verts, &uvs);
            }
        }
        if let Some(list) = self.trees.get(&(tx, ty)) {
            self.tree_nodes(&mut s, name, list, origin);
        }
        s
    }

    /// Halo tile (i, j)'s water: each row's run of wet sample cells at one level as a rectangle (two triangles, three vertices each,
    /// area metres); None when dry.
    fn water(&self, i: i64, j: i64) -> Option<Vec<[f32; 3]>> {
        let hm = &self.hm;
        let per = (TILE / hm.spacing).round() as i64;
        let sp = hm.spacing;
        // (A cell is wet when any of its corners is, at the highest of their levels, as the area's tiles draw water.)
        let level = |a: i64, b: i64| {
            let l =
                [(0, 0), (1, 0), (0, 1), (1, 1)].iter().filter_map(|(da, db)| hm.surface_level(a + da, b + db)).fold(f32::NAN, f32::max);
            (!l.is_nan()).then_some(l)
        };
        // Runs per row, then runs that repeat on the next row joined into one rectangle.
        let mut open: HashMap<(i64, i64, u32), i64> = HashMap::new();
        let mut rects: Vec<(i64, i64, i64, i64, f32)> = Vec::new();
        for b in j * per..=(j + 1) * per {
            let mut runs: Vec<(i64, i64, u32)> = Vec::new();
            if b < (j + 1) * per {
                let mut a = i * per;
                while a < (i + 1) * per {
                    if let Some(l) = level(a, b) {
                        let start = a;
                        while a < (i + 1) * per && level(a, b).is_some_and(|m| m.to_bits() == l.to_bits()) {
                            a += 1;
                        }
                        runs.push((start, a, l.to_bits()));
                    } else {
                        a += 1;
                    }
                }
            }
            let mut next = HashMap::new();
            for r in &runs {
                next.insert(*r, open.remove(r).unwrap_or(b));
            }
            for ((a0, a1, l), b0) in open.drain() {
                rects.push((a0, b0, a1, b, f32::from_bits(l)));
            }
            open = next;
        }
        if rects.is_empty() {
            return None;
        }
        rects.sort_by_key(|r| (r.1, r.0));
        let mut out = Vec::new();
        for (a0, b0, a1, b1, l) in rects {
            let (x0, y0, x1, y1) = (a0 as f32 * sp, b0 as f32 * sp, a1 as f32 * sp, b1 as f32 * sp);
            out.extend([[x0, y0, l], [x1, y0, l], [x1, y1, l], [x0, y0, l], [x1, y1, l], [x0, y1, l]]);
        }
        Some(out)
    }

    /// The halo's trees that boundary tile `origin` draws: cards past [`CARD_FROM`], else their models' triangles.
    fn tree_nodes(&self, s: &mut String, name: &str, list: &[usize], origin: [f32; 3]) {
        let Some(scene) = objects::scene() else { return };
        let [ox, oy, base] = origin;
        // Card triangles by model: the sides and the tops, each one node.
        let mut sides: BTreeMap<usize, (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>)> = BTreeMap::new();
        let mut tops: BTreeMap<usize, (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>)> = BTreeMap::new();
        let mut node = 0;
        for &i in list {
            let (Some(o), Some(p)) = (scene.objects.get(i), self.hm.objects.get(i)) else { continue };
            if let Some((a, n)) = self.atlases.get(&p.model).filter(|_| past(&self.hm, p.x, p.y) >= CARD_FROM) {
                self.counts.card_trees.fetch_add(1, Ordering::Relaxed);
                let (sn, cs) = p.bearing.sin_cos();
                let turn =
                    |v: [f32; 3], scale: f32| [cs * v[0] * scale - sn * v[1] * scale, sn * v[0] * scale + cs * v[1] * scale, v[2] * scale];
                let all = cards::cards(a, true);
                for (k, (v, nrm, uv)) in all.into_iter().enumerate() {
                    let q = turn(v, p.scale);
                    let at = [p.x + q[0] - ox, p.y + q[1] - oy, p.z + q[2] - base];
                    let nm = turn(nrm, 1.0);
                    let into = if k < 24 { sides.entry(*n).or_default() } else { tops.entry(*n).or_default() };
                    into.0.push(at);
                    into.1.push(nm);
                    into.2.push(uv);
                }
                continue;
            }
            self.counts.model_trees.fetch_add(1, Ordering::Relaxed);
            for (k, (texture, first, count, hint)) in o.groups.iter().enumerate() {
                let (mut verts, mut uvs) = (Vec::new(), Vec::new());
                for t in *first..first + count {
                    for c in 0..3 {
                        let v = o.positions[3 * t + c];
                        verts.push([v[0] - ox, v[1] - oy, v[2] - base]);
                        uvs.push(o.uvs[3 * t + c]);
                    }
                }
                if verts.is_empty() {
                    continue;
                }
                self.counts.trees.fetch_add(verts.len() / 3, Ordering::Relaxed);
                let mut props = if *hint != 0 { format!("  transparencyhint {hint}\n") } else { String::new() };
                if let Some(c) = o.glow.get(k).filter(|c| c.iter().any(|&x| x > 0.0)) {
                    let _ = writeln!(props, "  selfillumcolor {} {} {}", num(c[0]), num(c[1]), num(c[2]));
                }
                if let Some(m) = self.materials.get(&texture.to_ascii_lowercase()) {
                    let _ = writeln!(props, "  materialname {m}");
                }
                mdl::trimesh_listed(s, &format!("{name}_ht{node}"), name, texture, &props, &verts, &uvs);
                node += 1;
            }
        }
        for (top, map) in [(false, sides), (true, tops)] {
            for (n, (verts, normals, uvs)) in map {
                let Some(model) = self.atlases.iter().find(|(_, (_, m))| *m == n).map(|(model, _)| model) else { continue };
                self.counts.cards.fetch_add(verts.len() / 3, Ordering::Relaxed);
                let faces: Vec<[u32; 3]> = (0..verts.len() as u32 / 3).map(|t| [3 * t, 3 * t + 1, 3 * t + 2]).collect();
                let props = format!("  transparencyhint 1\n  materialname {}\n", self.card_mtr(n, top));
                let node = format!("{name}_{}{n}", if top { "bt" } else { "bc" });
                mdl::write_node(
                    s,
                    &node,
                    name,
                    &cards::atlas_name(model),
                    &props,
                    &verts,
                    Some(&normals),
                    &uvs,
                    None,
                    None,
                    &faces,
                    &faces,
                );
            }
        }
    }
}

/// How far area point (x, y) is past the area's edge (m): 0 inside, else to the nearest point of the area, as the fade measures.
fn past(hm: &heightmap::Heightmap, x: f32, y: f32) -> f32 {
    let (w, h) = (hm.tiles as f32 * TILE, hm.tiles_y as f32 * TILE);
    let (dx, dy) = ((-x).max(x - w).max(0.0), (-y).max(y - h).max(0.0));
    (dx * dx + dy * dy).sqrt()
}

/// The halo's ground, meshed: what a halo tile's cells need to know about their neighbours.
struct Mesh<'a> {
    fine: &'a HashSet<(i64, i64)>,
    /// The boundary tile's vertices on the area's border: west, east, south, north (area metres, sorted along the side).
    outline: &'a [Vec<[f32; 3]>; 4],
    size: [f32; 2],
}

impl Mesh<'_> {
    /// Halo tile (i, j)'s ground: vertices (area metres) and triangles, counter-clockwise from above.
    fn tile(&self, i: i64, j: i64) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let per = (TILE / COARSE) as i64;
        let mut verts: Vec<[f32; 3]> = Vec::new();
        let mut index: HashMap<(i64, i64), u32> = HashMap::new();
        let mut faces = Vec::new();
        for cj in j * per..(j + 1) * per {
            for ci in i * per..(i + 1) * per {
                let (x0, y0) = (ci as f32 * COARSE, cj as f32 * COARSE);
                if self.fine.contains(&(ci, cj)) {
                    let f = COARSE / FINE as f32;
                    for b in 0..FINE {
                        for a in 0..FINE {
                            let (fx, fy) = (x0 + a as f32 * f, y0 + b as f32 * f);
                            self.cell([fx, fy], [fx + f, fy + f], false, &mut verts, &mut index, &mut faces);
                        }
                    }
                } else {
                    self.cell([x0, y0], [x0 + COARSE, y0 + COARSE], true, &mut verts, &mut index, &mut faces);
                }
            }
        }
        (verts, faces)
    }

    /// Whether area point (x, y) is inside the area (not on its border).
    fn inside(&self, x: f32, y: f32) -> bool {
        x > 1e-3 && y > 1e-3 && x < self.size[0] - 1e-3 && y < self.size[1] - 1e-3
    }

    /// Which side of the area's border (x, y) lies on, if any: 0 west, 1 east, 2 south, 3 north.
    fn border(&self, x: f32, y: f32) -> Option<usize> {
        let (w, h) = (self.size[0], self.size[1]);
        let on_x = (-1e-3..=w + 1e-3).contains(&x);
        let on_y = (-1e-3..=h + 1e-3).contains(&y);
        if x.abs() < 1e-3 && on_y {
            Some(0)
        } else if (x - w).abs() < 1e-3 && on_y {
            Some(1)
        } else if y.abs() < 1e-3 && on_x {
            Some(2)
        } else if (y - h).abs() < 1e-3 && on_x {
            Some(3)
        } else {
            None
        }
    }

    /// The point (x, y) on the ground: on the area's border, on the tile's outline there (linear between its vertices, so the two
    /// meshes share the line), else the ground's height.
    fn point(&self, x: f32, y: f32) -> [f32; 3] {
        if let Some(side) = self.border(x, y) {
            let l = &self.outline[side];
            let (along, t) = if side < 2 { (1, y) } else { (0, x) };
            if let Some(k) = l.iter().position(|p| p[along] >= t - 1e-4)
                && !l.is_empty()
            {
                let b = l[k];
                if (b[along] - t).abs() < 1e-4 || k == 0 {
                    return [x, y, b[2]];
                }
                let a = l[k - 1];
                let u = (t - a[along]) / (b[along] - a[along]);
                return [x, y, a[2] + (b[2] - a[2]) * u];
            }
        }
        [x, y, height(x, y)]
    }

    /// The points strictly between `a` and `b` (an edge of a cell, along x or y) that the cell on its other side has: the tile's
    /// outline on the area's border, the fine spacing beside a fine cell (for a coarse edge), else none.
    fn between(&self, a: [f32; 2], b: [f32; 2], coarse: bool, out_dir: [f32; 2]) -> Vec<[f32; 2]> {
        let along = if (a[0] - b[0]).abs() < 1e-4 { 1 } else { 0 };
        let (lo, hi) = (a[along].min(b[along]), a[along].max(b[along]));
        let mid = [(a[0] + b[0]) / 2.0 + out_dir[0] * 0.01, (a[1] + b[1]) / 2.0 + out_dir[1] * 0.01];
        let mut pts: Vec<[f32; 2]> = Vec::new();
        if self.inside(mid[0], mid[1]) {
            if let Some(side) = self.border((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0) {
                pts = self.outline[side].iter().filter(|p| p[along] > lo + 1e-4 && p[along] < hi - 1e-4).map(|p| [p[0], p[1]]).collect();
            }
        } else if coarse {
            let neighbour = ((mid[0] / COARSE).floor() as i64, (mid[1] / COARSE).floor() as i64);
            if self.fine.contains(&neighbour) {
                let f = COARSE / FINE as f32;
                pts = (1..FINE).map(|k| if along == 0 { [lo + k as f32 * f, a[1]] } else { [a[0], lo + k as f32 * f] }).collect();
            }
        }
        // From a to b.
        if a[along] > b[along] {
            pts.reverse();
        }
        pts
    }

    /// One cell from `lo` to `hi` (area metres): two triangles, or a fan from its middle when its sides carry more points.
    fn cell(
        &self,
        lo: [f32; 2],
        hi: [f32; 2],
        coarse: bool,
        verts: &mut Vec<[f32; 3]>,
        index: &mut HashMap<(i64, i64), u32>,
        faces: &mut Vec<[u32; 3]>,
    ) {
        let corners = [[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]];
        let outward = [[0.0, -1.0], [1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]];
        let mut ring: Vec<[f32; 2]> = Vec::new();
        for e in 0..4 {
            ring.push(corners[e]);
            ring.extend(self.between(corners[e], corners[(e + 1) % 4], coarse, outward[e]));
        }
        let mut id = |p: [f32; 2]| -> u32 {
            let key = ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64);
            *index.entry(key).or_insert_with(|| {
                verts.push(self.point(p[0], p[1]));
                (verts.len() - 1) as u32
            })
        };
        let ids: Vec<u32> = ring.iter().map(|&p| id(p)).collect();
        if ids.len() == 4 {
            faces.push([ids[0], ids[1], ids[2]]);
            faces.push([ids[0], ids[2], ids[3]]);
        } else {
            let c = id([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]);
            for k in 0..ids.len() {
                faces.push([c, ids[k], ids[(k + 1) % ids.len()]]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The halo tile west of a 4 by 4 area's south-west tile, meshed against an outline of 0.5 m steps with one more vertex (a
    /// split face's) at 3.3 m, one coarse cell drawn fine.
    fn west_tile() -> (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<[f32; 3]>) {
        let mut west: Vec<[f32; 3]> = (0..=20).map(|k| [0.0, k as f32 * 0.5, height(0.0, k as f32 * 0.5)]).collect();
        west.push([0.0, 3.3, height(0.0, 3.3) + 0.01]);
        west.sort_by(|a, b| a[1].total_cmp(&b[1]));
        let outline = [west.clone(), Vec::new(), Vec::new(), Vec::new()];
        let fine: HashSet<(i64, i64)> = [(-1, 1), (-3, 2)].into_iter().collect();
        let mesh = Mesh { fine: &fine, outline: &outline, size: [40.0, 40.0] };
        let (verts, faces) = mesh.tile(-1, 0);
        (verts, faces, west)
    }

    #[test]
    fn the_halo_s_ground_covers_its_tile_once_without_cracks() {
        let (verts, faces, _) = west_tile();
        let mut area = 0.0;
        for f in &faces {
            let [a, b, c] = f.map(|v| verts[v as usize]);
            let doubled = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
            assert!(doubled > 0.0, "every face counter-clockwise from above");
            area += doubled / 2.0;
        }
        assert!((area - TILE * TILE).abs() < 1e-3, "{area}");
        // No vertex lies inside another face's edge: a T there would open a crack.
        for f in &faces {
            for e in 0..3 {
                let (a, b) = (verts[f[e] as usize], verts[f[(e + 1) % 3] as usize]);
                for p in &verts {
                    let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                    let dot = (p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1]);
                    let len2 = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
                    assert!(!(cross.abs() < 1e-5 && dot > 1e-4 && dot < len2 - 1e-4), "{p:?} inside the edge {a:?}-{b:?}");
                }
            }
        }
    }

    #[test]
    fn the_halo_s_ground_takes_every_vertex_of_the_tile_s_outline() {
        let (verts, _, west) = west_tile();
        for o in &west {
            assert!(verts.iter().any(|v| (v[0] - o[0]).abs() < 1e-4 && (v[1] - o[1]).abs() < 1e-4 && (v[2] - o[2]).abs() < 1e-5), "{o:?}");
        }
        // And nothing else on the border but points on the outline's line (a fine cell's corner between two of its vertices).
        for v in verts.iter().filter(|v| v[0].abs() < 1e-4) {
            let k = west.iter().position(|o| o[1] >= v[1] - 1e-4).unwrap();
            let z = if (west[k][1] - v[1]).abs() < 1e-4 {
                west[k][2]
            } else {
                let (a, b) = (west[k - 1], west[k]);
                a[2] + (b[2] - a[2]) * (v[1] - a[1]) / (b[1] - a[1])
            };
            assert!((v[2] - z).abs() < 1e-5, "{v:?} off the outline");
        }
    }

    #[test]
    fn coarse_cells_are_two_triangles_and_fine_ones_draw_the_samples() {
        let (_, faces, _) = west_tile();
        // 16 coarse cells: 13 plain (2 each, some fanned beside the fine cells and the outline) and 2 fine (16 cells each).
        assert!(faces.len() > 16 * 2 + 2 * 16 * 2 - 26 && faces.len() < 200, "{}", faces.len());
    }

    #[test]
    fn a_halo_tile_s_name_fits_a_resref() {
        assert!(halo_name(-5, 41, 5).len() <= 16);
    }
}
