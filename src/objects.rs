// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Placeables baked into the terrain: the editor's objects, their models' triangles copied into the tiles (clipped at tile edges, each
//! tile its share, textures referenced by name), their walkmesh footprints made ground nothing can walk on (so the path-node planner
//! and the simulator see them), and their triangles casting shadows (occluders.rs).

use std::sync::{Arc, RwLock};

use anyhow::Result;

use crate::heightmap::Object;
use crate::models::{self, Flat};

/// An object placed: its model's flattened triangles and footprint, in area metres.
pub struct Placed {
    /// Three positions a triangle.
    pub positions: Vec<[f32; 3]>,
    /// Each position's texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Each position's normal, turned as it stands; empty when its model gives none ([`Flat::normals`]).
    pub normals: Vec<[f32; 3]>,
    /// (texture, first triangle, count, transparency hint).
    pub groups: Vec<(String, usize, usize, u32)>,
    /// Each group's self-illumination colour; empty: none glows.
    pub glow: Vec<[f32; 3]>,
    /// The triangles it covers on the ground (x, y): where it blocks walking or casts its patch.
    pub footprint: Vec<[[f32; 2]; 3]>,
    /// Its bounding box's low corner (area metres).
    pub min: [f32; 3],
    /// Its bounding box's high corner (area metres).
    pub max: [f32; 3],
    /// Baked into the tiles (else an object: footprint only).
    pub baked: bool,
    /// Standing on the halo, beyond the area's edge (the neighbour's): drawn in the edge scenery and casting shadows, never in a
    /// tile or blocking.
    pub halo: bool,
    /// Its model and placement, for the shadow cache.
    pub what: String,
    /// A digest of its model's triangles, texture coordinates and textures ([`model_digest`]), for the shadow cache: the same
    /// name may be another model after a hak changes.
    pub model: [u8; 20],
}

/// A digest of what of model `f` casts a shadow: its triangles, their texture coordinates, and each group's texture and
/// transparency.
pub fn model_digest(f: &Flat) -> [u8; 20] {
    use sha1::Digest;
    let mut h = sha1::Sha1::new();
    for p in f.positions.iter().flatten() {
        h.update(p.to_le_bytes());
    }
    for u in f.uvs.iter().flatten() {
        h.update(u.to_le_bytes());
    }
    for ((texture, first, count), hint) in f.groups.iter().zip(&f.transparency) {
        h.update(texture.as_bytes());
        h.update([0]);
        for v in [*first as u64, *count as u64, u64::from(*hint)] {
            h.update(v.to_le_bytes());
        }
    }
    h.finalize().into()
}

fn place(o: &Object, f: &Flat, model: [u8; 20]) -> Placed {
    let (s, c) = o.bearing.sin_cos();
    let at = |p: [f32; 3]| {
        let (x, y, z) = (p[0] * o.scale, p[1] * o.scale, p[2] * o.scale);
        [o.x + c * x - s * y, o.y + s * x + c * y, o.z + z]
    };
    let positions: Vec<[f32; 3]> = f.positions.iter().map(|p| at(*p)).collect();
    // (Scaled alike every way, so only turned.)
    let normals = if f.normals.len() == f.positions.len() {
        f.normals.iter().map(|n| [c * n[0] - s * n[1], s * n[0] + c * n[1], n[2]]).collect()
    } else {
        Vec::new()
    };
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in &positions {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let footprint = f
        .footprint
        .iter()
        .map(|t| {
            t.map(|p| {
                let q = at([p[0], p[1], 0.0]);
                [q[0], q[1]]
            })
        })
        .collect();
    let groups = f.groups.iter().zip(&f.transparency).map(|((t, a, n), h)| (t.clone(), *a, *n, *h)).collect();
    let what = format!("{}:{}:{}:{}:{}:{}", o.model, o.x, o.y, o.z, o.bearing, o.scale);
    Placed {
        positions,
        uvs: f.uvs.clone(),
        normals,
        groups,
        glow: f.glow.clone(),
        footprint,
        min,
        max,
        baked: o.baked,
        halo: false,
        what,
        model,
    }
}

/// The current terrain's objects placed, and an index of their footprints.
pub struct Scene {
    /// Every object, placed.
    pub objects: Vec<Placed>,
    /// Footprint triangles by 2 m cell.
    cells: std::collections::HashMap<(i32, i32), Vec<[[f32; 2]; 3]>>,
    /// The baked ones' triangles, for shadows.
    occluders: Option<crate::occluders::Occluders>,
    /// The area's lowest ground (m): how far below an object its shadow can fall.
    pub lowest: f32,
    /// The models that couldn't be read (sorted, each once): their objects stand with no triangles and block nothing.
    pub missing: Vec<String>,
    serial: u64,
}

const CELL: f32 = 2.0;
static SCENE: RwLock<Option<Arc<Scene>>> = RwLock::new(None);

/// The objects of the loaded terrain (built once per load).
pub fn scene() -> Option<Arc<Scene>> {
    let hm = crate::heightmap::current()?;
    if hm.objects.is_empty() {
        return None;
    }
    if let Some(s) = SCENE.read().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().filter(|s| s.serial == hm.serial) {
        return Some(s.clone());
    }
    let mut built = build(&hm.objects, hm.serial, area_of(&hm), hm.halo.is_some());
    built.lowest = hm.data.iter().copied().fold(f32::MAX, f32::min);
    let built = Arc::new(built);
    *SCENE.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(built.clone());
    Some(built)
}

/// The loaded objects' models that can't be read (sorted, each once): an area from another install or the web demo may use
/// models this one lacks, which then block nothing and are left out of the tiles.
pub fn missing() -> Vec<String> {
    scene().map_or_else(Vec::new, |s| s.missing.clone())
}

/// The area's size in metres, east and north.
fn area_of(hm: &crate::heightmap::Heightmap) -> [f32; 2] {
    [hm.tiles as f32 * crate::generator::TILE, hm.tiles_y as f32 * crate::generator::TILE]
}

/// The cells of size `cell` from `lo` to `hi` metres, no further than a cell beyond the area's `size` either way: nothing
/// outside it asks whether a footprint blocks it or what stands over it, so a large footprint costs no more than the area does.
fn cells_between(lo: f32, hi: f32, cell: f32, size: f32) -> std::ops::RangeInclusive<i32> {
    ((lo.max(-cell) / cell).floor() as i32)..=((hi.min(size + cell) / cell).floor() as i32)
}

/// `halo`: the area has one, so an object standing beyond its edge is the halo's ([`Placed::halo`]).
/// An object whose model can't be read stays in the list as one with no triangles (the generator matches objects to the area's by
/// index), and its model is named in [`Scene::missing`].
fn build(list: &[Object], serial: u64, [aw, ah]: [f32; 2], halo: bool) -> Scene {
    let game = models::game();
    let mut objects = Vec::new();
    let mut missing = std::collections::BTreeSet::new();
    let mut cells: std::collections::HashMap<(i32, i32), Vec<[[f32; 2]; 3]>> = Default::default();
    // (Each model's digest once: a forest places a few models thousands of times. The model is held, so its address isn't
    // another's later.)
    let mut digests: std::collections::HashMap<*const Flat, ([u8; 20], Arc<Flat>)> = Default::default();
    for o in list {
        let flat = match game.as_ref().map_err(|e| anyhow::anyhow!("{e:#}")).and_then(|g| models::flat(g, &o.model)) {
            Ok(f) => f,
            Err(e) => {
                if missing.insert(o.model.clone()) {
                    eprintln!("{}: model not readable, so it blocks nothing and isn't baked ({e:#})", o.model);
                }
                Arc::new(Flat::default())
            }
        };
        let model = digests.entry(Arc::as_ptr(&flat)).or_insert_with(|| (model_digest(&flat), flat.clone())).0;
        let mut p = place(o, &flat, model);
        if p.positions.is_empty() {
            (p.min, p.max) = ([o.x, o.y, o.z], [o.x, o.y, o.z]);
        }
        p.halo = halo && !((0.0..=aw).contains(&o.x) && (0.0..=ah).contains(&o.y));
        // Decoration (not solid) doesn't block walking, nor does the halo's.
        for t in p.footprint.iter().filter(|_| o.solid && !p.halo) {
            let (x0, x1) = (t.iter().map(|q| q[0]).fold(f32::MAX, f32::min), t.iter().map(|q| q[0]).fold(f32::MIN, f32::max));
            let (y0, y1) = (t.iter().map(|q| q[1]).fold(f32::MAX, f32::min), t.iter().map(|q| q[1]).fold(f32::MIN, f32::max));
            for cy in cells_between(y0, y1, CELL, ah) {
                for cx in cells_between(x0, x1, CELL, aw) {
                    cells.entry((cx, cy)).or_default().push(*t);
                }
            }
        }
        objects.push(p);
    }
    let occluders = crate::occluders::Occluders::build(&objects);
    Scene { objects, cells, occluders, lowest: 0.0, missing: missing.into_iter().collect(), serial }
}

fn inside(t: &[[f32; 2]; 3], x: f32, y: f32) -> bool {
    let d = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
    let (d0, d1, d2) = (d(t[0], t[1]), d(t[1], t[2]), d(t[2], t[0]));
    (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
}

impl Scene {
    /// Whether an object's footprint covers (x, y).
    pub fn blocks(&self, x: f32, y: f32) -> bool {
        self.cells.get(&((x / CELL).floor() as i32, (y / CELL).floor() as i32)).is_some_and(|ts| ts.iter().any(|t| inside(t, x, y)))
    }

    /// The baked objects' triangles, for shadows (None: none baked).
    pub fn occluders(&self) -> Option<&crate::occluders::Occluders> {
        self.occluders.as_ref()
    }
}

/// The tag of an object-mode placeable: how a re-export finds the ones it placed.
pub fn tag(id: u32) -> String {
    format!("{}_p{id}", crate::generator::tag_prefix())
}

/// The object-mode placeables, as the toolset places a blueprint: the blueprint with their appearance, not static, at their place and
/// bearing, tagged by `tag`, scaled by a visual transform as Aurora writes one.
pub fn instances(game: &mg_rules::GameData) -> Result<Vec<mg_gff::Struct>> {
    use anyhow::Context;
    use mg_gff::Value;
    let Some(hm) = crate::heightmap::current() else { return Ok(Vec::new()) };
    let placing = mg_module::instances::Placing { game, item: &|_| None };
    let mut out = Vec::new();
    for o in hm.objects.iter().filter(|o| !o.baked) {
        let (Some(row), Some(id)) = (o.appearance, o.id) else { continue };
        let (resref, blueprint) = models::blueprint(game, row)?;
        let at = mg_module::instances::Placement { position: [o.x, o.y, o.z], rotation: o.bearing };
        let mut s = mg_module::instances::instance(&placing, mg_core::ResType::UTP, &blueprint, at, &[])
            .with_context(|| format!("placing {resref}"))?;
        s.set("Tag", Value::String(tag(id).into_bytes()));
        s.set("Static", Value::Byte(0));
        if (o.scale - 1.0).abs() > 1e-3 {
            let axis = |v: f32| {
                let mut a = mg_gff::Struct::new(0);
                a.set("TimerType", Value::Int(0));
                a.set("ValueTo", Value::Float(v));
                a.set("LerpType", Value::Int(0));
                Value::Struct(a)
            };
            let mut entry = mg_gff::Struct::new(6);
            entry.set("Scope", Value::Int(0));
            entry.set("AnimationSpeed", axis(1.0));
            for (prefix, v) in [("Scale", o.scale), ("Rotate", 0.0), ("Translate", 0.0)] {
                for a in ["X", "Y", "Z"] {
                    entry.set(&format!("{prefix}{a}"), axis(v));
                }
            }
            s.set("VisTransformList", Value::List(vec![entry]));
        }
        out.push(s);
    }
    Ok(out)
}

/// A triangle's corner: its position, texture coordinates and normal.
type Corner = ([f32; 3], [f32; 2], [f32; 3]);

/// A triangle, its corners' texture coordinates and their normals (zero for a model with none: then they stay zero).
pub type Piece = ([[f32; 3]; 3], [[f32; 2]; 3], [[f32; 3]; 3]);

/// Where edge a-b crosses the plane `p[axis] = at`, on the plane exactly. The edge is taken from its lower end whichever way it is
/// walked, so the two pieces on either side of a cut get the same point, bit for bit.
fn crossing(a: &Corner, b: &Corner, axis: usize, at: f32) -> Corner {
    let (a, b) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    let t = (at - a.0[axis]) / (b.0[axis] - a.0[axis]);
    let mut p = [0.0; 3];
    for k in 0..3 {
        p[k] = a.0[k] + (b.0[k] - a.0[k]) * t;
    }
    p[axis] = at;
    // (The normal as the texture coordinates, then made unit again: the cut's corners shade as the surface between its ends.)
    let mut n = [0.0; 3];
    for k in 0..3 {
        n[k] = a.2[k] + (b.2[k] - a.2[k]) * t;
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 {
        n = n.map(|v| v / len);
    }
    (p, [a.1[0] + (b.1[0] - a.1[0]) * t, a.1[1] + (b.1[1] - a.1[1]) * t], n)
}

/// The part of polygon `poly` where `(p[axis] - at) * sign >= 0`.
fn keep(poly: &[Corner], axis: usize, at: f32, sign: f32) -> Vec<Corner> {
    let f = |v: &Corner| (v.0[axis] - at) * sign;
    let mut out = Vec::new();
    for k in 0..poly.len() {
        let (a, b) = (&poly[k], &poly[(k + 1) % poly.len()]);
        let (fa, fb) = (f(a), f(b));
        if fa >= 0.0 {
            out.push(*a);
        }
        // (A corner on the plane is kept as it is, not crossed again.)
        if (fa > 0.0 && fb < 0.0) || (fa < 0.0 && fb > 0.0) {
            out.push(crossing(a, b, axis, at));
        }
    }
    out
}

/// A convex polygon as triangles (a fan from its first corner).
fn fan(poly: &[Corner]) -> Vec<Piece> {
    (1..poly.len().saturating_sub(1))
        .map(|k| ([poly[0].0, poly[k].0, poly[k + 1].0], [poly[0].1, poly[k].1, poly[k + 1].1], [poly[0].2, poly[k].2, poly[k + 1].2]))
        .collect()
}

/// Clips a triangle (positions, UVs and normals) to the square [x0, x1] x [y0, y1]; the pieces as triangles.
pub fn clip((tri, uv, nrm): Piece, x0: f32, x1: f32, y0: f32, y1: f32) -> Vec<Piece> {
    let mut poly: Vec<Corner> = (0..3).map(|k| (tri[k], uv[k], nrm[k])).collect();
    for (axis, at, sign) in [(0, x0, 1.0), (0, x1, -1.0), (1, y0, 1.0), (1, y1, -1.0)] {
        if poly.is_empty() {
            break;
        }
        poly = keep(&poly, axis, at, sign);
    }
    fan(&poly)
}

/// Splits a triangle at height `z`: the pieces below and those above. A triangle on one side is that side's whole; the pieces of
/// one that crosses share their cut edge's points exactly, so nothing opens between them.
pub fn split_at_height((tri, uv, nrm): Piece, z: f32) -> (Vec<Piece>, Vec<Piece>) {
    if tri.iter().all(|p| p[2] <= z) {
        return (vec![(tri, uv, nrm)], Vec::new());
    }
    if tri.iter().all(|p| p[2] >= z) {
        return (Vec::new(), vec![(tri, uv, nrm)]);
    }
    let poly: Vec<Corner> = (0..3).map(|k| (tri[k], uv[k], nrm[k])).collect();
    (fan(&keep(&poly, 2, z, -1.0)), fan(&keep(&poly, 2, z, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_s_digest_changes_with_what_casts_its_shadow() {
        let f = Flat {
            mesh: nwn_bake::Flat {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 2.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                groups: vec![("leaves".into(), 0, 1)],
                transparency: vec![0],
                ..Default::default()
            },
            ..Default::default()
        };
        let base = model_digest(&f);
        assert_eq!(base, model_digest(&f.clone()));
        let changed = |change: &dyn Fn(&mut Flat)| {
            let mut g = f.clone();
            change(&mut g);
            model_digest(&g)
        };
        assert_ne!(base, changed(&|g| g.positions[2][2] = 3.0), "a corner moved");
        assert_ne!(base, changed(&|g| g.uvs[1][0] = 0.5), "its texture coordinates");
        assert_ne!(base, changed(&|g| g.groups[0].0 = "bark".into()), "its texture");
        assert_ne!(base, changed(&|g| g.transparency[0] = 1), "its transparency");
    }

    #[test]
    fn an_object_whose_model_can_t_be_read_stands_with_no_triangles_and_blocks_nothing() {
        let o = |model: &str, x: f32| Object {
            model: model.into(),
            x,
            y: 5.0,
            z: 0.0,
            bearing: 0.0,
            scale: 1.0,
            solid: true,
            baked: true,
            appearance: None,
            id: None,
        };
        let list = [o("ls_no_such_mdl", 5.0), o("ls_no_such_mdl", 15.0), o("ls_nor_this_one", 25.0)];
        let scene = build(&list, 0, [30.0, 10.0], false);
        // (Kept in the list: the generator matches objects to the area's by index.)
        assert_eq!(scene.objects.len(), 3);
        assert!(scene.objects.iter().all(|p| p.positions.is_empty() && p.footprint.is_empty()));
        assert_eq!(scene.objects[1].min, [15.0, 5.0, 0.0]);
        assert_eq!(scene.missing, ["ls_no_such_mdl", "ls_nor_this_one"]);
        assert!(!scene.blocks(5.0, 5.0));
    }

    #[test]
    fn a_triangle_across_a_tile_edge_is_split_between_the_tiles() {
        let tri = [[8.0, 2.0, 0.0], [12.0, 2.0, 1.0], [8.0, 6.0, 0.0]];
        let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        // (Normals: west at the first corner, east at the second.)
        let nrm = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let area = |ps: &[Piece]| {
            ps.iter()
                .map(|(t, ..)| ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])).abs() / 2.0)
                .sum::<f32>()
        };
        let left = clip((tri, uv, nrm), 0.0, 10.0, 0.0, 10.0);
        let right = clip((tri, uv, nrm), 10.0, 20.0, 0.0, 10.0);
        assert!((area(&left) + area(&right) - 8.0).abs() < 1e-4);
        // Past x = 10: the corner (10, 2), (12, 2), (10, 4).
        assert!((area(&right) - 2.0).abs() < 1e-4);
        // The cut's height, UV and normal are interpolated, the normal made unit again. Halfway from west to east it is zero, and
        // stays zero rather than pointing anywhere.
        let cut = right
            .iter()
            .flat_map(|(t, u, n)| t.iter().zip(u).zip(n))
            .find(|((p, _), _)| (p[0] - 10.0).abs() < 1e-5 && (p[1] - 2.0).abs() < 1e-5)
            .unwrap();
        assert!((cut.0.0[2] - 0.5).abs() < 1e-5 && (cut.0.1[0] - 0.5).abs() < 1e-5);
        assert_eq!(*cut.1, [0.0; 3]);
        // Where (10, 4) cuts edge 1-2: from east (1, 0, 0) halfway to up (0, 0, 1), made unit.
        let far = right.iter().flat_map(|(t, _, n)| t.iter().zip(n)).find(|(p, _)| (p[1] - 4.0).abs() < 1e-5).unwrap().1;
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((far[0] - h).abs() < 1e-5 && far[1].abs() < 1e-6 && (far[2] - h).abs() < 1e-5, "{far:?}");
    }
}
