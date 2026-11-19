// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A tile model as an import reads it: its walkmesh faces and textured triangles at rest, and its triangles grouped into the
//! things that stand on the ground (a trunk and its leaves), each as a model about a point.

use std::collections::HashMap;

use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mg_core::ResType;
use mg_mdl::walkmesh::{Walkmesh, WalkmeshKind};
use mg_mdl::{Model, NodeKind};
use mg_rules::GameData;

use super::{Face, WalkFace};
use crate::models;

/// A tile model's walkmesh faces and textured triangles, at rest, in the model's space (centred on the tile).
pub(super) fn tile_parts(game: &GameData, name: &str) -> Result<(Vec<WalkFace>, Vec<Face>)> {
    let flat = models::flat(game, name)?;
    let mut faces = Vec::new();
    for ((tex, first, count), hint) in flat.groups.iter().zip(&flat.transparency) {
        for t in *first..first + count {
            let p = [0, 1, 2].map(|k| Vec3::from_array(flat.positions[t * 3 + k]));
            let uv = [0, 1, 2].map(|k| flat.uvs[t * 3 + k]);
            faces.push(Face { p, uv, tex: tex.clone(), hint: *hint });
        }
    }
    let data = game.resman.get_named(name, ResType::WOK).with_context(|| format!("no walkmesh {name}"))?;
    let wok = Walkmesh::read(&data, WalkmeshKind::Tile)?;
    let pose = rest_pose(&wok.model);
    let mut walk = Vec::new();
    for (i, node) in wok.model.nodes.iter().enumerate() {
        let NodeKind::Mesh(mesh) = &node.kind else { continue };
        for f in &mesh.faces {
            walk.push(WalkFace {
                p: f.vertices.map(|v| pose[i].transform_point3(Vec3::from_array(mesh.vertices[v as usize]))),
                material: f.material,
            });
        }
    }
    Ok((walk, faces))
}

/// Faces as a model about `origin` (tile-local x, y; absolute z).
pub(super) fn flat_of(faces: &[&Face], origin: [f32; 3], used: &mut std::collections::BTreeSet<String>) -> models::Flat {
    let mut sorted: Vec<&Face> = faces.to_vec();
    sorted.sort_by(|a, b| (&a.tex, a.hint).cmp(&(&b.tex, b.hint)));
    let mut flat = nwn_bake::Flat { min: [f32::MAX; 3], max: [f32::MIN; 3], ..Default::default() };
    for f in &sorted {
        let tri = flat.positions.len() / 3;
        match flat.groups.last_mut() {
            Some(g) if g.0 == f.tex && flat.transparency.last() == Some(&f.hint) => g.2 += 1,
            _ => {
                flat.groups.push((f.tex.clone(), tri, 1));
                flat.transparency.push(f.hint);
            }
        }
        for k in 0..3 {
            let v = [f.p[k].x - origin[0], f.p[k].y - origin[1], f.p[k].z - origin[2]];
            for a in 0..3 {
                flat.min[a] = flat.min[a].min(v[a]);
                flat.max[a] = flat.max[a].max(v[a]);
            }
            flat.positions.push(v);
            flat.uvs.push(f.uv[k]);
        }
        used.insert(f.tex.clone());
    }
    if sorted.is_empty() {
        flat.min = [0.0; 3];
        flat.max = [0.0; 3];
    }
    flat.into()
}

/// x0, y0, x1, y1 of triangles.
pub(super) fn bounds(tris: impl Iterator<Item = [Vec3; 3]>) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for t in tris {
        for v in t {
            b = [b[0].min(v.x), b[1].min(v.y), b[2].max(v.x), b[3].max(v.y)];
        }
    }
    b
}

/// Faces grouped into things: faces whose outlines from above come within 10 cm of each other are one thing (a trunk and its leaves);
/// tiny things (a tuft) join the nearest thing within 3 m.
pub(super) fn cluster<'a>(faces: &[&'a Face]) -> Vec<Vec<&'a Face>> {
    const CELL: f32 = 0.5;
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut cells: HashMap<(i32, i32), usize> = HashMap::new();
    for (k, f) in faces.iter().enumerate() {
        let b = bounds(std::iter::once(f.p));
        for cy in ((b[1] - 0.1) / CELL).floor() as i32..=((b[3] + 0.1) / CELL).floor() as i32 {
            for cx in ((b[0] - 0.1) / CELL).floor() as i32..=((b[2] + 0.1) / CELL).floor() as i32 {
                match cells.get(&(cx, cy)) {
                    Some(&o) => {
                        let (a, b) = (find(&mut parent, k), find(&mut parent, o));
                        parent[a] = b;
                    }
                    None => {
                        cells.insert((cx, cy), k);
                    }
                }
            }
        }
    }
    // By their root face's number (a BTreeMap), then the biggest first: the same order every run. (With a HashMap, things of one
    // size came out in a different order each run, and so did the faces of a tile's fixed object and the objects' numbers.)
    let mut groups: std::collections::BTreeMap<usize, Vec<&'a Face>> = Default::default();
    for (k, f) in faces.iter().enumerate() {
        let r = find(&mut parent, k);
        groups.entry(r).or_default().push(f);
    }
    let mut out: Vec<Vec<&'a Face>> = groups.into_values().collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.len()));
    // Tiny ones into the nearest bigger one.
    let centre = |g: &[&Face]| {
        let b = bounds(g.iter().map(|f| f.p));
        ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0, (b[2] - b[0]).max(b[3] - b[1]))
    };
    let mut k = out.len();
    while k > 0 {
        k -= 1;
        let (cx, cy, size) = centre(&out[k]);
        if size >= 0.6 {
            continue;
        }
        let near = (0..out.len())
            .filter(|&o| o != k && centre(&out[o]).2 >= 0.6)
            .map(|o| {
                (o, {
                    let c = centre(&out[o]);
                    (c.0 - cx).hypot(c.1 - cy)
                })
            })
            .filter(|(_, d)| *d <= 3.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((o, _)) = near {
            let tiny = out.remove(k);
            let o = if o > k { o - 1 } else { o };
            out[o].extend(tiny);
        }
    }
    out
}

/// Each node's transform to model space in the rest pose: a tile's parts are placed by their parent nodes.
pub(super) fn rest_pose(model: &Model) -> Vec<Mat4> {
    let mut out: Vec<Mat4> = Vec::with_capacity(model.nodes.len());
    for (i, n) in model.nodes.iter().enumerate() {
        let local = Mat4::from_scale_rotation_translation(
            Vec3::splat(n.scale),
            glam::Quat::from_array(n.orientation).normalize(),
            Vec3::from_array(n.position),
        );
        out.push(match n.parent.filter(|&p| p < i) {
            Some(p) => out[p] * local,
            None => local,
        });
    }
    out
}
