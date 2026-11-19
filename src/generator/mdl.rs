// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing ASCII models: a tile's grid of vertices and the trimesh and walkmesh nodes made from it.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::{TILE, TRANSITION, Walk, height, tile_level};

/// A grid of vertices over one tile, in the tile's own coordinates (centred: -5..5), and its triangles.
pub(super) struct Grid {
    /// The vertices, tile-centred (m).
    pub(super) verts: Vec<[f32; 3]>,
    /// The triangles, as indices into `verts`.
    pub(super) tris: Vec<[u32; 3]>,
}

/// Tile (`tx`, `ty`)'s ground as a regular grid of `step` metres, at the terrain's heights.
pub(super) fn grid(tx: usize, ty: usize, step: f32) -> Grid {
    let n = (TILE / step).round() as usize;
    let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
    let base = tile_level(tx, ty) as f32 * TRANSITION;
    let mut verts = Vec::with_capacity((n + 1) * (n + 1));
    for j in 0..=n {
        for i in 0..=n {
            let (lx, ly) = (i as f32 * step, j as f32 * step);
            verts.push([lx - TILE / 2.0, ly - TILE / 2.0, height(ox + lx, oy + ly) - base]);
        }
    }
    let at = |i: usize, j: usize| (j * (n + 1) + i) as u32;
    let mut tris = Vec::with_capacity(n * n * 2);
    for j in 0..n {
        for i in 0..n {
            // Counter-clockwise seen from above.
            tris.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            tris.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    Grid { verts, tris }
}

/// The up part of triangle `t`'s unit normal: how flat it is (1 flat, 0 a wall), the test for walkable ground.
pub(super) fn normal_z(v: &[[f32; 3]], t: [u32; 3]) -> f32 {
    let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    n[2] / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

/// A number as the ASCII model writes it (Moonglow's form, so models compile the same).
pub(super) fn num(f: f32) -> String {
    mg_mdl::ascii_write::num(f)
}

/// A trimesh node of the faces given, with only the vertices they use.
pub(super) fn trimesh(
    out: &mut String,
    name: &str,
    root: &str,
    bitmap: &str,
    g: &Grid,
    faces: &[[u32; 3]],
    uv: impl Fn([f32; 3]) -> [f32; 2],
) {
    trimesh_with(out, name, root, bitmap, "", g, faces, uv);
}

/// [`trimesh`] with more node properties (`props`, whole lines).
pub(super) fn trimesh_with(
    out: &mut String,
    name: &str,
    root: &str,
    bitmap: &str,
    props: &str,
    g: &Grid,
    faces: &[[u32; 3]],
    uv: impl Fn([f32; 3]) -> [f32; 2],
) {
    trimesh_full(out, name, root, bitmap, props, g, faces, &uv, None);
}

/// A trimesh of the triangles listed (three vertices each, with their own texture coordinates), as copied from another model.
pub(super) fn trimesh_listed(out: &mut String, name: &str, root: &str, bitmap: &str, props: &str, verts: &[[f32; 3]], uvs: &[[f32; 2]]) {
    let _ = writeln!(
        out,
        "node trimesh {name}\n  parent {root}\n  ambient 1 1 1\n  diffuse 1 1 1\n  specular 0 0 0\n  shininess 1\n  \
         shadow 0\n{props}  bitmap {bitmap}"
    );
    let _ = writeln!(out, "  verts {}", verts.len());
    for v in verts {
        let _ = writeln!(out, "    {} {} {}", num(v[0]), num(v[1]), num(v[2]));
    }
    let _ = writeln!(out, "  tverts {}", uvs.len());
    for t in uvs {
        let _ = writeln!(out, "    {} {} 0", num(t[0]), num(t[1]));
    }
    let _ = writeln!(out, "  faces {}", verts.len() / 3);
    for k in 0..verts.len() / 3 {
        let (a, b, c) = (3 * k, 3 * k + 1, 3 * k + 2);
        let _ = writeln!(out, "    {a} {b} {c} 1 {a} {b} {c} 0");
    }
    let _ = writeln!(out, "endnode");
}

/// [`trimesh_with`] with second texture coordinates (`tverts1`), if given.
pub(super) fn trimesh_full(
    out: &mut String,
    name: &str,
    root: &str,
    bitmap: &str,
    props: &str,
    g: &Grid,
    faces: &[[u32; 3]],
    uv: &dyn Fn([f32; 3]) -> [f32; 2],
    uv1: Option<&dyn Fn([f32; 3]) -> [f32; 2]>,
) {
    if faces.is_empty() {
        return;
    }
    let mut remap = HashMap::new();
    let mut verts = Vec::new();
    let mut local = Vec::with_capacity(faces.len());
    for f in faces {
        local.push(f.map(|v| {
            *remap.entry(v).or_insert_with(|| {
                verts.push(g.verts[v as usize]);
                (verts.len() - 1) as u32
            })
        }));
    }
    let _ = writeln!(
        out,
        "node trimesh {name}\n  parent {root}\n  ambient 1 1 1\n  diffuse 1 1 1\n  specular 0 0 0\n  shininess 1\n  \
         shadow 0\n{props}  bitmap {bitmap}"
    );
    let _ = writeln!(out, "  verts {}", verts.len());
    for v in &verts {
        let _ = writeln!(out, "    {} {} {}", num(v[0]), num(v[1]), num(v[2]));
    }
    let _ = writeln!(out, "  tverts {}", verts.len());
    for v in &verts {
        let t = uv(*v);
        let _ = writeln!(out, "    {} {} 0", num(t[0]), num(t[1]));
    }
    if let Some(uv1) = uv1 {
        let _ = writeln!(out, "  tverts1 {}", verts.len());
        for v in &verts {
            let t = uv1(*v);
            let _ = writeln!(out, "    {} {} 0", num(t[0]), num(t[1]));
        }
    }
    let _ = writeln!(out, "  faces {}", local.len());
    for f in &local {
        let _ = writeln!(out, "    {} {} {} 1 {} {} {} 0", f[0], f[1], f[2], f[0], f[1], f[2]);
    }
    let _ = writeln!(out, "endnode");
}

/// Writes the walkmesh `w`'s vertices and faces into an aabb node, as the game's walkmesh reads them.
pub(super) fn aabb_body(out: &mut String, w: &Walk) {
    let _ = writeln!(out, "  verts {}", w.verts.len());
    for v in &w.verts {
        let _ = writeln!(out, "    {} {} {}", num(v[0]), num(v[1]), num(v[2]));
    }
    let _ = writeln!(out, "  faces {}", w.faces.len());
    for (f, m) in &w.faces {
        let _ = writeln!(out, "    {} {} {} 1 0 0 0 {m}", f[0], f[1], f[2]);
    }
}
