// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Placeables baked into the terrain: the editor's objects, their models' triangles copied into the tiles (clipped at tile edges, each
//! tile its share, textures referenced by name), their walkmesh footprints made ground nothing can walk on (so the path-node planner
//! and the simulator see them), and their tops casting shadows.

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
    /// (texture, first triangle, count, transparency hint).
    pub groups: Vec<(String, usize, usize, u32)>,
    /// The triangles it covers on the ground (x, y): where it blocks walking or casts its patch.
    pub footprint: Vec<[[f32; 2]; 3]>,
    /// Its bounding box's low corner (area metres).
    pub min: [f32; 3],
    /// Its bounding box's high corner (area metres).
    pub max: [f32; 3],
    /// Baked into the tiles (else an object: footprint only).
    pub baked: bool,
}

fn place(o: &Object, f: &Flat) -> Placed {
    let (s, c) = o.bearing.sin_cos();
    let at = |p: [f32; 3]| {
        let (x, y, z) = (p[0] * o.scale, p[1] * o.scale, p[2] * o.scale);
        [o.x + c * x - s * y, o.y + s * x + c * y, o.z + z]
    };
    let positions: Vec<[f32; 3]> = f.positions.iter().map(|p| at(*p)).collect();
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
    Placed { positions, uvs: f.uvs.clone(), groups, footprint, min, max, baked: o.baked }
}

/// The current terrain's objects placed, and an index of their footprints.
pub struct Scene {
    /// Every object, placed.
    pub objects: Vec<Placed>,
    /// Footprint triangles by 2 m cell.
    cells: std::collections::HashMap<(i32, i32), Vec<[[f32; 2]; 3]>>,
    /// The objects' tops on a 0.25 m grid (area metres), for shadows.
    tops: Tops,
    serial: u64,
}

const CELL: f32 = 2.0;
/// The tops' grid (m).
pub const TOP_CELL: f32 = 0.25;

/// The tops' cells, from a cell beyond the area's south-west corner to one beyond its north-east one (no others are marked), as one
/// array: the shadow maps ask hundreds of millions of times. NaN: nothing there.
struct Tops {
    w: i32,
    h: i32,
    cells: Vec<f32>,
}

impl Tops {
    fn new([aw, ah]: [f32; 2]) -> Tops {
        let (w, h) = ((aw / TOP_CELL).floor() as i32 + 3, (ah / TOP_CELL).floor() as i32 + 3);
        Tops { w, h, cells: vec![f32::NAN; (w * h) as usize] }
    }

    fn index(&self, (cx, cy): (i32, i32)) -> Option<usize> {
        let (i, j) = (cx + 1, cy + 1);
        ((0..self.w).contains(&i) && (0..self.h).contains(&j)).then(|| (j * self.w + i) as usize)
    }

    /// Raises cell `c` to `z` (as a map entry starting at `f32::MIN`).
    fn mark(&mut self, c: (i32, i32), z: f32) {
        if let Some(k) = self.index(c) {
            let e = &mut self.cells[k];
            *e = if e.is_nan() { f32::MIN.max(z) } else { e.max(z) };
        }
    }

    fn get(&self, c: (i32, i32)) -> Option<f32> {
        self.index(c).map(|k| self.cells[k]).filter(|v| !v.is_nan())
    }
}

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
    let built = Arc::new(build(&hm.objects, hm.serial, area_of(&hm)).ok()?);
    *SCENE.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(built.clone());
    Some(built)
}

/// Whether the loaded objects' models can all be read (else the reason).
pub fn check() -> Result<()> {
    let Some(hm) = crate::heightmap::current() else { return Ok(()) };
    if hm.objects.is_empty() {
        return Ok(());
    }
    build(&hm.objects, hm.serial, area_of(&hm)).map(|_| ())
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

fn build(list: &[Object], serial: u64, [aw, ah]: [f32; 2]) -> Result<Scene> {
    let game = models::game()?;
    let mut objects = Vec::new();
    let mut cells: std::collections::HashMap<(i32, i32), Vec<[[f32; 2]; 3]>> = Default::default();
    let mut tops = Tops::new([aw, ah]);
    for o in list {
        let p = place(o, &*models::flat(game, &o.model)?);
        // Decoration (not solid) doesn't block walking.
        for t in p.footprint.iter().filter(|_| o.solid) {
            let (x0, x1) = (t.iter().map(|q| q[0]).fold(f32::MAX, f32::min), t.iter().map(|q| q[0]).fold(f32::MIN, f32::max));
            let (y0, y1) = (t.iter().map(|q| q[1]).fold(f32::MAX, f32::min), t.iter().map(|q| q[1]).fold(f32::MIN, f32::max));
            for cy in cells_between(y0, y1, CELL, ah) {
                for cx in cells_between(x0, x1, CELL, aw) {
                    cells.entry((cx, cy)).or_default().push(*t);
                }
            }
        }
        // Tops: each triangle over the cells it covers, at its own height there (an upright face, fence boards say, along its edges).
        // Only baked ones: an object can be moved or destroyed in play.
        for t in p.positions.as_chunks::<3>().0.iter().filter(|_| p.baked) {
            let mut mark = |x: f32, y: f32, z: f32| {
                if !(-TOP_CELL..=aw + TOP_CELL).contains(&x) || !(-TOP_CELL..=ah + TOP_CELL).contains(&y) {
                    return;
                }
                tops.mark(((x / TOP_CELL).floor() as i32, (y / TOP_CELL).floor() as i32), z);
            };
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let steps = ((b[0] - a[0]).hypot(b[1] - a[1]) / (TOP_CELL / 2.0)).ceil().max(1.0) as usize;
                for i in 0..=steps {
                    let f = i as f32 / steps as f32;
                    mark(a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f);
                }
            }
            let det = (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1]);
            if det.abs() < 1e-6 {
                continue;
            }
            let (x0, x1) = (t.iter().map(|q| q[0]).fold(f32::MAX, f32::min), t.iter().map(|q| q[0]).fold(f32::MIN, f32::max));
            let (y0, y1) = (t.iter().map(|q| q[1]).fold(f32::MAX, f32::min), t.iter().map(|q| q[1]).fold(f32::MIN, f32::max));
            for cy in cells_between(y0, y1, TOP_CELL, ah) {
                for cx in cells_between(x0, x1, TOP_CELL, aw) {
                    let (x, y) = ((cx as f32 + 0.5) * TOP_CELL, (cy as f32 + 0.5) * TOP_CELL);
                    let l1 = ((t[1][1] - t[2][1]) * (x - t[2][0]) + (t[2][0] - t[1][0]) * (y - t[2][1]))
                        / ((t[1][1] - t[2][1]) * (t[0][0] - t[2][0]) + (t[2][0] - t[1][0]) * (t[0][1] - t[2][1]));
                    let l2 = ((t[2][1] - t[0][1]) * (x - t[2][0]) + (t[0][0] - t[2][0]) * (y - t[2][1]))
                        / ((t[1][1] - t[2][1]) * (t[0][0] - t[2][0]) + (t[2][0] - t[1][0]) * (t[0][1] - t[2][1]));
                    let l3 = 1.0 - l1 - l2;
                    if l1 >= 0.0 && l2 >= 0.0 && l3 >= 0.0 {
                        mark(x, y, l1 * t[0][2] + l2 * t[1][2] + l3 * t[2][2]);
                    }
                }
            }
        }
        objects.push(p);
    }
    Ok(Scene { objects, cells, tops, serial })
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

    /// The highest object surface over (x, y), if any.
    pub fn top(&self, x: f32, y: f32) -> Option<f32> {
        self.tops.get(((x / TOP_CELL).floor() as i32, (y / TOP_CELL).floor() as i32))
    }
}

/// The tag of an object-mode placeable: how a re-export finds the ones it placed.
pub fn tag(id: u32) -> String {
    format!("{}_p{id}", crate::generator::tileset_name())
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

/// Clips a triangle (positions and UVs) to the square [x0, x1] x [y0, y1]; the pieces as triangles.
pub fn clip(tri: [[f32; 3]; 3], uv: [[f32; 2]; 3], x0: f32, x1: f32, y0: f32, y1: f32) -> Vec<([[f32; 3]; 3], [[f32; 2]; 3])> {
    type V = ([f32; 3], [f32; 2]);
    let mut poly: Vec<V> = (0..3).map(|k| (tri[k], uv[k])).collect();
    // Keep the side where f(p) >= 0, for each edge of the square.
    let planes: [(usize, f32, f32); 4] = [(0, x0, 1.0), (0, x1, -1.0), (1, y0, 1.0), (1, y1, -1.0)];
    for (axis, at, sign) in planes {
        if poly.is_empty() {
            break;
        }
        let f = |v: &V| (v.0[axis] - at) * sign;
        let mut out = Vec::new();
        for k in 0..poly.len() {
            let (a, b) = (&poly[k], &poly[(k + 1) % poly.len()]);
            let (fa, fb) = (f(a), f(b));
            if fa >= 0.0 {
                out.push(*a);
            }
            if (fa >= 0.0) != (fb >= 0.0) {
                let t = fa / (fa - fb);
                let lerp3 = |p: [f32; 3], q: [f32; 3]| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t];
                let lerp2 = |p: [f32; 2], q: [f32; 2]| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
                out.push((lerp3(a.0, b.0), lerp2(a.1, b.1)));
            }
        }
        poly = out;
    }
    (1..poly.len().saturating_sub(1)).map(|k| ([poly[0].0, poly[k].0, poly[k + 1].0], [poly[0].1, poly[k].1, poly[k + 1].1])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tops_grid_keeps_the_highest_mark_of_each_cell_within_a_cell_of_the_area() {
        let mut tops = Tops::new([20.0, 10.0]);
        assert_eq!(tops.get((3, 4)), None);
        tops.mark((3, 4), 1.5);
        tops.mark((3, 4), 0.5);
        assert_eq!(tops.get((3, 4)), Some(1.5));
        // A cell beyond each corner is on the grid; further is not, and isn't marked.
        for c in [(-1, -1), (81, 41)] {
            tops.mark(c, -2.0);
            assert_eq!(tops.get(c), Some(-2.0));
        }
        tops.mark((82, 0), 1.0);
        assert_eq!((tops.get((82, 0)), tops.get((-2, 0))), (None, None));
        // A NaN height marks the cell as a map entry starting at f32::MIN would.
        tops.mark((5, 5), f32::NAN);
        assert_eq!(tops.get((5, 5)), Some(f32::MIN));
    }

    #[test]
    fn a_triangle_across_a_tile_edge_is_split_between_the_tiles() {
        let tri = [[8.0, 2.0, 0.0], [12.0, 2.0, 1.0], [8.0, 6.0, 0.0]];
        let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let area = |ps: &[([[f32; 3]; 3], [[f32; 2]; 3])]| {
            ps.iter()
                .map(|(t, _)| ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])).abs() / 2.0)
                .sum::<f32>()
        };
        let left = clip(tri, uv, 0.0, 10.0, 0.0, 10.0);
        let right = clip(tri, uv, 10.0, 20.0, 0.0, 10.0);
        assert!((area(&left) + area(&right) - 8.0).abs() < 1e-4);
        // Past x = 10: the corner (10, 2), (12, 2), (10, 4).
        assert!((area(&right) - 2.0).abs() < 1e-4);
        // The cut's height and UV are interpolated.
        let cut =
            right.iter().flat_map(|(t, u)| t.iter().zip(u)).find(|(p, _)| (p[0] - 10.0).abs() < 1e-5 && (p[1] - 2.0).abs() < 1e-5).unwrap();
        assert!((cut.0[2] - 0.5).abs() < 1e-5 && (cut.1[0] - 0.5).abs() < 1e-5);
    }
}
