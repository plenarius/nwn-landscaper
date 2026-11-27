// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing ASCII models: a tile's grid of vertices and the trimesh and walkmesh nodes made from it.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::{TILE, TRANSITION, WALKABLE_NZ, Walk, ground_normal, height, render_point, tile_level};
use crate::triplanar;

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
            let (p, _) = render_point(ox + i as f32 * step, oy + j as f32 * step, step);
            verts.push([p[0] - ox - TILE / 2.0, p[1] - oy - TILE / 2.0, p[2] - base]);
        }
    }
    let at = |i: usize, j: usize| (j * (n + 1) + i) as u32;
    let mut tris = Vec::with_capacity(n * n * 2);
    for j in 0..n {
        for i in 0..n {
            // Counter-clockwise seen from above. (An interior's upright wall folds some flat: those are left out.)
            for t in [[at(i, j), at(i + 1, j), at(i + 1, j + 1)], [at(i, j), at(i + 1, j + 1), at(i, j + 1)]] {
                if !flat_face(&verts, t) {
                    tris.push(t);
                }
            }
        }
    }
    Grid { verts, tris }
}

/// Whether triangle `t` of `v` has no area (its corners in a line).
pub(super) fn flat_face(v: &[[f32; 3]], t: [u32; 3]) -> bool {
    let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    n[0] * n[0] + n[1] * n[1] + n[2] * n[2] < 1e-8
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

/// A mesh's colours: white, unless its properties give their own (a tinted water look).
fn lit(props: &str) -> &'static str {
    if props.contains("  diffuse ") { "" } else { "  ambient 1 1 1\n  diffuse 1 1 1\n" }
}

/// A trimesh of the triangles listed (three vertices each, with their own texture coordinates), as copied from another model.
pub(super) fn trimesh_listed(out: &mut String, name: &str, root: &str, bitmap: &str, props: &str, verts: &[[f32; 3]], uvs: &[[f32; 2]]) {
    let _ = writeln!(
        out,
        "node trimesh {name}\n  parent {root}\n{}  specular 0 0 0\n  shininess 1\n  shadow 0\n{props}  bitmap {bitmap}",
        lit(props)
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
    let (verts, local) = used(g, faces);
    let tverts: Vec<[f32; 2]> = verts.iter().map(|v| uv(*v)).collect();
    let tverts1 = uv1.map(|f| verts.iter().map(|v| f(*v)).collect::<Vec<_>>());
    write_node(out, name, root, bitmap, props, &verts, None, &tverts, tverts1.as_deref(), None, &local, &local, &[]);
}

/// How a tile's ground mesh takes its texture coordinates ([`terrain_mesh`]).
pub(super) enum Mapping<'a> {
    /// From each vertex (tile-centred, as [`Grid`] has it): a picture over the tile's square.
    Vertex(&'a dyn Fn([f32; 3]) -> [f32; 2]),
    /// A repeating library texture, box mapped: each face from the side it faces most ([`triplanar::box_projection`]), so no
    /// face is stretched more than 1.6 times whichever way it faces. The game's shader blends the three sides over this (`ls_tp_fs`,
    /// through the mesh's material); these coordinates are what a viewer without it draws (Moonglow, nwn-bake). How the texture
    /// repeats ([`crate::paint::Repeat`]): the coordinates take its metres; the material, both.
    Boxed(crate::paint::Repeat),
}

/// A trimesh of the ground faces `faces` of `g`, a tile's grid whose (0, 0, 0) is area point `origin`: the ground's own normals
/// ([`ground_normal`]), so light and the triplanar blend run on smoothly across meshes and tiles where the game's smoothing stops
/// at each mesh's edge; texture coordinates by `mapping`; second ones (`tverts1`) from each vertex by `uv1`, if given; each face's
/// material (the game grows its grass on faces of a grass surface) from `surfaces`, else 0.
pub(super) fn terrain_mesh(
    out: &mut String,
    name: &str,
    root: &str,
    bitmap: &str,
    props: &str,
    g: &Grid,
    faces: &[[u32; 3]],
    origin: [f32; 3],
    mapping: Mapping<'_>,
    uv1: Option<&dyn Fn([f32; 3]) -> [f32; 2]>,
    surfaces: &[u32],
) {
    if faces.is_empty() {
        return;
    }
    let (verts, local) = used(g, faces);
    let area = |v: [f32; 3]| [v[0] + origin[0], v[1] + origin[1], v[2] + origin[2]];
    let normals: Vec<[f32; 3]> = verts.iter().map(|v| ground_normal(v[0] + origin[0], v[1] + origin[1])).collect();
    // Texture vertices: one a vertex, or (box mapped) one a vertex and side, each face's corners on its own side's.
    let mut tvert_of: Vec<u32> = Vec::new();
    let (tverts, tfaces) = match mapping {
        Mapping::Vertex(uv) => {
            tvert_of = (0..verts.len() as u32).collect();
            (verts.iter().map(|v| uv(*v)).collect(), local.clone())
        }
        Mapping::Boxed(repeat) => {
            let mut index: HashMap<(u32, usize), u32> = HashMap::new();
            let mut tverts = Vec::new();
            let tfaces = local
                .iter()
                .map(|f| {
                    let axis = triplanar::box_projection(face_normal(f.map(|v| verts[v as usize])));
                    f.map(|v| {
                        *index.entry((v, axis)).or_insert_with(|| {
                            tverts.push(triplanar::uv(axis, area(verts[v as usize]), repeat.metres));
                            tvert_of.push(v);
                            (tverts.len() - 1) as u32
                        })
                    })
                })
                .collect();
            (tverts, tfaces)
        }
    };
    let tverts1 = uv1.map(|f| tvert_of.iter().map(|&v| f(verts[v as usize])).collect::<Vec<_>>());
    // A library texture's blend normal ([`triplanar::blend_normal`]) in the third coordinates, for the shader: walkable ground
    // takes the gentler slope at each vertex, a steep face the steeper, so neither takes the other's projection at the crease.
    let tverts2 = matches!(mapping, Mapping::Boxed(_)).then(|| {
        let mut nz = vec![(0.0f32, 0u32); verts.len()];
        for f in &local {
            let n = face_normal(f.map(|v| verts[v as usize]));
            let z = n[2].abs() / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            for &v in f {
                nz[v as usize].0 += z;
                nz[v as usize].1 += 1;
            }
        }
        let blend: Vec<[f32; 2]> = verts
            .iter()
            .zip(&nz)
            .map(|(v, (z, k))| {
                let a = area(*v);
                let n = triplanar::blend_normal(height, a[0], a[1], BLEND_STEP, z / ((*k).max(1) as f32) < WALKABLE_NZ);
                blend_uv(n)
            })
            .collect();
        tvert_of.iter().map(|&v| blend[v as usize]).collect::<Vec<_>>()
    });
    write_node(
        out,
        name,
        root,
        bitmap,
        props,
        &verts,
        Some(&normals),
        &tverts,
        tverts1.as_deref(),
        tverts2.as_deref(),
        &local,
        &tfaces,
        surfaces,
    );
}

/// How far each way the blend normal's slopes reach (m): a heightmap sample's spacing.
const BLEND_STEP: f32 = 0.625;

/// A blend normal as the third texture coordinates: its x and y, each plus 2, so a mesh without them (read as 0) is told apart
/// and the shader falls back to the vertex normal there. Its z is the rest of the unit length (the blend weighs by size only).
pub(super) fn blend_uv(n: [f32; 3]) -> [f32; 2] {
    [n[0] + 2.0, n[1] + 2.0]
}

/// The vertices of `g` that `faces` use, and the faces renumbered to them.
fn used(g: &Grid, faces: &[[u32; 3]]) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let mut remap = HashMap::new();
    let mut verts = Vec::new();
    let local = faces
        .iter()
        .map(|f| {
            f.map(|v| {
                *remap.entry(v).or_insert_with(|| {
                    verts.push(g.verts[v as usize]);
                    (verts.len() - 1) as u32
                })
            })
        })
        .collect();
    (verts, local)
}

/// A triangle's normal (not unit), counter-clockwise.
fn face_normal([a, b, c]: [[f32; 3]; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]]
}

/// Writes a trimesh node: `faces` index `verts` (and `normals`, if given), `tfaces` the texture vertices (`tverts`, and `tverts1` and
/// `tverts2` alike, if given), `materials` each face's material (0 past its end).
pub(super) fn write_node(
    out: &mut String,
    name: &str,
    root: &str,
    bitmap: &str,
    props: &str,
    verts: &[[f32; 3]],
    normals: Option<&[[f32; 3]]>,
    tverts: &[[f32; 2]],
    tverts1: Option<&[[f32; 2]]>,
    tverts2: Option<&[[f32; 2]]>,
    faces: &[[u32; 3]],
    tfaces: &[[u32; 3]],
    materials: &[u32],
) {
    let _ = writeln!(
        out,
        "node trimesh {name}\n  parent {root}\n{}  specular 0 0 0\n  shininess 1\n  shadow 0\n{props}  bitmap {bitmap}",
        lit(props)
    );
    let _ = writeln!(out, "  verts {}", verts.len());
    for v in verts {
        let _ = writeln!(out, "    {} {} {}", num(v[0]), num(v[1]), num(v[2]));
    }
    if let Some(normals) = normals {
        let _ = writeln!(out, "  normals {}", normals.len());
        for n in normals {
            let _ = writeln!(out, "    {} {} {}", num(n[0]), num(n[1]), num(n[2]));
        }
    }
    let _ = writeln!(out, "  tverts {}", tverts.len());
    for t in tverts {
        let _ = writeln!(out, "    {} {} 0", num(t[0]), num(t[1]));
    }
    for (key, list) in [("tverts1", tverts1), ("tverts2", tverts2)] {
        if let Some(list) = list {
            let _ = writeln!(out, "  {key} {}", list.len());
            for t in list {
                let _ = writeln!(out, "    {} {} 0", num(t[0]), num(t[1]));
            }
        }
    }
    let _ = writeln!(out, "  faces {}", faces.len());
    for (k, (f, t)) in faces.iter().zip(tfaces).enumerate() {
        let m = materials.get(k).copied().unwrap_or(0);
        let _ = writeln!(out, "    {} {} {} 1 {} {} {} {m}", f[0], f[1], f[2], t[0], t[1], t[2]);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines of `node`'s list `key` (`tverts`, `faces`...), split into words.
    fn list(node: &str, key: &str) -> Vec<Vec<String>> {
        let mut lines = node.lines().skip_while(|l| l.split_whitespace().next() != Some(key));
        let n: usize = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|n| n.parse().ok()).unwrap_or(0);
        lines.take(n).map(|l| l.split_whitespace().map(str::to_string).collect()).collect()
    }

    #[test]
    fn box_mapping_gives_each_face_its_own_side_and_splits_the_vertices_between_them() {
        // Two walls meeting at an edge: one facing along x, one along y. Their two shared vertices take a texture vertex each side.
        let g = Grid { verts: vec![[0.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 4.0], [4.0, 0.0, 0.0]], tris: vec![[0, 1, 2], [0, 2, 3]] };
        let mut s = String::new();
        let origin = [10.0, 20.0, 5.0];
        terrain_mesh(
            &mut s,
            "t",
            "r",
            "ls_rock",
            "",
            &g,
            &g.tris,
            origin,
            Mapping::Boxed(crate::paint::Repeat::PLAIN),
            Some(&|v: [f32; 3]| [v[0], v[1]]),
            &[],
        );
        assert_eq!(list(&s, "verts").len(), 4);
        assert_eq!(list(&s, "normals").len(), 4, "the ground's own normals, one a vertex");
        let tverts = list(&s, "tverts");
        assert_eq!(tverts.len(), 6);
        assert_eq!(list(&s, "tverts1").len(), 6, "the second coordinates follow the first's vertices");
        let faces = list(&s, "faces");
        let uv = |f: usize, c: usize| -> [f32; 2] {
            let t: usize = faces[f][4 + c].parse().unwrap();
            [tverts[t][0].parse().unwrap(), tverts[t][1].parse().unwrap()]
        };
        // The wall facing x runs on along y and up (area y 20..24, z 5..9: 5..6 and 1.25..2.25 repeats).
        assert_eq!([uv(0, 0), uv(0, 1), uv(0, 2)], [[5.0, 1.25], [6.0, 1.25], [5.0, 2.25]]);
        // The wall facing y along x and up.
        assert_eq!([uv(1, 0), uv(1, 1), uv(1, 2)], [[2.5, 1.25], [2.5, 2.25], [3.5, 1.25]]);
    }

    #[test]
    fn a_vertex_mapping_keeps_one_texture_vertex_a_vertex() {
        let g = Grid { verts: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 3.0]], tris: vec![[0, 1, 2]] };
        let mut s = String::new();
        terrain_mesh(&mut s, "t", "r", "pic", "", &g, &g.tris, [0.0; 3], Mapping::Vertex(&|v| [v[0], v[1]]), None, &[]);
        assert_eq!(list(&s, "tverts").len(), 3);
        assert!(list(&s, "tverts1").is_empty());
        assert_eq!(list(&s, "faces")[0], ["0", "1", "2", "1", "0", "1", "2", "0"]);
    }
}
