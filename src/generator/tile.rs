// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A tile's files: its model, its walkmesh with the game's surfaces, its water, and the tileset's .set file.

use std::fmt::Write as _;

use mg_mdl::{Face, Mesh};

use super::{
    ENTRANCE_DARK, Grid, MAT_DEEP_WATER, MAT_GRASS, MAT_NONWALK, MAT_WATER, Painter, RENDER_STEP, TILE, TRANSITION, VOID_BELOW,
    WALKABLE_NZ, WATER_METRES, WATER_TEXTURE, Walk, aabb_body, baked_name, edge, entrance, entrance_rock, grid, height, minimap_name,
    normal_z, num, shadow_mtr, shadow_name, tile_level, tile_name, tiles_x, tiles_y, tileset_name, trimesh, trimesh_full, trimesh_listed,
    trimesh_with,
};
use crate::{heightmap, objects, paint, shared};

/// Tile (tx, ty)'s model; `mtrs` gets the materials of its shadowed meshes (when shadows are on: `shadow`).
pub(super) fn tile_model(
    name: &str,
    tx: usize,
    ty: usize,
    w: &Walk,
    painter: Option<&Painter>,
    shadow: bool,
    mtrs: &mut Vec<(String, String)>,
) -> String {
    let (ox, oy) = (tx as f32 * TILE + TILE / 2.0, ty as f32 * TILE + TILE / 2.0);
    let base = tile_level(tx, ty) as f32 * TRANSITION;
    let g = grid(tx, ty, RENDER_STEP);
    let (mut ground, mut rock, mut void) = (Vec::new(), Vec::new(), Vec::new());
    // A cave's rock tops (at its roof) are void: drawn black.
    let cave = heightmap::current().filter(|h| h.underground && h.roof.is_some());
    // (A vertex is void when it is at its ceiling: the roof's own height, or the one that follows the floor.)
    let void_vertex = |v: u32| {
        let [x, y, z] = g.verts[v as usize];
        cave.as_ref().is_some_and(|h| h.roof_at(x + ox, y + oy).is_some_and(|r| z + base >= r - VOID_BELOW))
    };
    // A chasm: how far each vertex is under its rim. Its floor is black (with the void); its walls fade to black going down.
    let chasm = heightmap::current().filter(|h| !h.rim.is_empty());
    let depth = |v: [f32; 3]| chasm.as_ref().map_or(0.0, |h| h.chasm_depth(v[0] + ox, v[1] + oy));
    let mut walls = Vec::new();
    for &t in &g.tris {
        let deep = t.map(|v| depth(g.verts[v as usize]));
        let in_void = cave.is_some() && t.iter().all(|&v| void_vertex(v));
        let on_floor = chasm.is_some() && deep.iter().all(|&d| d >= heightmap::CHASM_DEPTH - 0.01);
        if in_void || on_floor {
            void.push(t);
        } else if deep.iter().any(|&d| d > heightmap::CHASM_BELOW) {
            walls.push(t);
        } else if normal_z(&g.verts, t) >= WALKABLE_NZ {
            ground.push(t);
        } else {
            rock.push(t);
        }
    }
    let mut s = format!(
        "newmodel {name}\nsetsupermodel {name} NULL\nclassification tile\nsetanimationscale 1\n\
         beginmodelgeom {name}\nnode dummy {name}\n  parent NULL\nendnode\n"
    );
    for l in ["ml1", "ml2"] {
        let _ = writeln!(
            s,
            "node light {name}{l}\n  parent {name}\n  ambientonly 0\n  shadow 0\n  isdynamic 0\n  affectdynamic 1\n  lightpriority 5\n  \
             fadingLight 1\n  flareradius 0\n  position 0 0 5\n  radius 14\n  multiplier 1\n  color 0 0 0\nendnode"
        );
    }
    // Texture coordinates from area coordinates, so textures run on across
    // tiles; steep faces take them from the side (along x + y and up).
    let side = |v: [f32; 3]| [(v[0] + ox + v[1] + oy) / paint::TEXTURE_METRES, (v[2] + base) / paint::TEXTURE_METRES];
    // Ground and steep meshes; shadowed, each its own material and the tile's square as second texture coordinates.
    let shadow_map = shadow_name(tx, ty);
    let square = |v: [f32; 3]| [(v[0] + TILE / 2.0) / TILE, (v[1] + TILE / 2.0) / TILE];
    // The shadow map's coordinates (tverts1): the tile's square, as the colour's. Checked in game 2026-10-20 with
    // LS_SHADOW_DEBUG=half: each tile's dark south half lay south of a pillar standing on a tile corner.
    let shadow_uv = square;
    let mut lit = |s: &mut String, node: String, bitmap: &str, faces: &[[u32; 3]], uv: &dyn Fn([f32; 3]) -> [f32; 2]| {
        if shadow && !faces.is_empty() {
            let mat = format!("{name}m{}", mtrs.len());
            mtrs.push((mat.clone(), shadow_mtr(bitmap, &shadow_map)));
            trimesh_full(s, &node, name, bitmap, &format!("  materialname {mat}\n"), &g, faces, uv, Some(&shadow_uv));
        } else {
            trimesh_full(s, &node, name, bitmap, "", &g, faces, uv, None);
        }
    };
    match painter {
        None => {
            lit(&mut s, format!("{name}_gnd"), "ttr01_grass02", &ground, &|v| [(v[0] + ox) / 5.0, (v[1] + oy) / 5.0]);
            lit(&mut s, format!("{name}_rck"), "ttr01_rock00", &rock, &side);
        }
        Some(p) => {
            match p.tile_materials(tx, ty).as_slice() {
                // An import's picture: baked like several textures.
                _ if p.baked(tx, ty) => lit(&mut s, format!("{name}_gnd"), &baked_name(tx, ty), &ground, &square),
                // One texture: the library's, repeating.
                [m] => lit(&mut s, format!("{name}_gnd"), &paint::resref(&p.hm.palette[*m as usize]), &ground, &|v| {
                    [(v[0] + ox) / paint::TEXTURE_METRES, (v[1] + oy) / paint::TEXTURE_METRES]
                }),
                // Several: the tile's own baked texture over its square.
                _ => lit(&mut s, format!("{name}_gnd"), &baked_name(tx, ty), &ground, &square),
            }
            // Steep faces: the texture painted nearest each face's middle.
            let mut by_material: std::collections::BTreeMap<u8, Vec<[u32; 3]>> = Default::default();
            for &t in &rock {
                let c = t.map(|v| g.verts[v as usize]);
                let (cx, cy) = ((c[0][0] + c[1][0] + c[2][0]) / 3.0 + ox, (c[0][1] + c[1][1] + c[2][1]) / 3.0 + oy);
                by_material.entry(p.nearest(cx, cy)).or_default().push(t);
            }
            for (k, (m, faces)) in by_material.iter().enumerate() {
                if p.imported(*m) {
                    // The tileset's own cliff texture there, else the picture. (The original walls, in the tile's object, stand in
                    // front: the grid's slope fills what they don't cover.)
                    let Some(tiles) = p.textures[*m as usize].tiles.as_ref() else { continue };
                    match tiles.cliffs.get(ty * tiles.width + tx).cloned().flatten() {
                        Some(cliff) => lit(&mut s, format!("{name}_rck{k}"), &cliff, faces, &side),
                        None => lit(&mut s, format!("{name}_rck{k}"), &baked_name(tx, ty), faces, &square),
                    }
                } else {
                    lit(&mut s, format!("{name}_rck{k}"), &paint::resref(&p.hm.palette[*m as usize]), faces, &side);
                }
            }
        }
    }
    if !void.is_empty() {
        trimesh(&mut s, &format!("{name}_void"), name, ENTRANCE_DARK, &g, &void, |v| [v[0], v[1]]);
    }
    // A chasm's walls: one mesh in the texture painted nearest most of them (one material more a tile at most: an MTR's resref
    // is the tile's plus `m` and a number, and 16 characters is all a resref has), darkened by the fade in place of the shadow map
    // (a wall is a line seen from above, where the shadow map lies) by how far under the rim each vertex is.
    if !walls.is_empty() {
        let mut count: std::collections::BTreeMap<u8, usize> = Default::default();
        for &t in &walls {
            let c = t.map(|v| g.verts[v as usize]);
            let (cx, cy) = ((c[0][0] + c[1][0] + c[2][0]) / 3.0 + ox, (c[0][1] + c[1][1] + c[2][1]) / 3.0 + oy);
            *count.entry(painter.map_or(0, |p| p.nearest(cx, cy))).or_default() += 1;
        }
        let m = count.iter().max_by_key(|(_, n)| **n).map_or(0, |(m, _)| *m);
        // (an import's ground: its tileset's cliff texture there, as its steep faces take, else the tile's picture)
        let bitmap = match painter {
            Some(p) if p.imported(m) => p.textures[m as usize]
                .tiles
                .as_ref()
                .and_then(|t| t.cliffs.get(ty * t.width + tx).cloned().flatten())
                .unwrap_or_else(|| baked_name(tx, ty)),
            Some(p) => paint::resref(&p.hm.palette[m as usize]),
            None => "ttr01_rock00".to_string(),
        };
        let fade = |v: [f32; 3]| [shared::chasm_fade_u(depth(v) / heightmap::CHASM_DEPTH), 0.5];
        let mat = format!("{name}m{}", mtrs.len());
        mtrs.push((mat.clone(), shadow_mtr(&bitmap, shared::CHASM_FADE)));
        trimesh_full(&mut s, &format!("{name}_cw"), name, &bitmap, &format!("  materialname {mat}\n"), &g, &walls, &side, Some(&fade));
    }
    // Entrances whose mouth is in this tile: the roof's top in the hill's paint, its rock (underside and lip; shadowed, so dark
    // inside), the dark back.
    if let Some(hm) = heightmap::current() {
        for (k, e) in hm.entrances.iter().enumerate().filter(|(_, e)| e.tile() == (tx, ty)) {
            let sc = entrance::scenery(e, &height);
            let grid = |tris: &[[[f32; 3]; 3]]| Grid {
                verts: tris.iter().flatten().map(|v| [v[0] - ox, v[1] - oy, v[2] - base]).collect(),
                tris: (0..tris.len() as u32).map(|t| [3 * t, 3 * t + 1, 3 * t + 2]).collect(),
            };
            let flat = |v: [f32; 3]| [(v[0] + ox) / paint::TEXTURE_METRES, (v[1] + oy) / paint::TEXTURE_METRES];
            // Tilted with height, so the underside and the upright lip both take the texture unstretched enough.
            let tilted = |v: [f32; 3]| {
                let (x, y, z) = (v[0] + ox, v[1] + oy, v[2] + base);
                [(x + 0.6 * z) / paint::TEXTURE_METRES, (y - 0.4 * z) / paint::TEXTURE_METRES]
            };
            let (hill, rock) = match painter {
                Some(p) => {
                    let (bx, by) = e.at(e.depth + 2.0, 0.0);
                    let hill = paint::resref(&p.hm.palette[p.nearest(bx, by) as usize]);
                    let rock = e
                        .rock
                        .as_deref()
                        .filter(|r| p.hm.palette.iter().any(|id| id == r))
                        .map_or_else(|| paint::resref(&p.hm.palette[entrance_rock(p) as usize]), paint::resref);
                    (hill, rock)
                }
                None => ("ttr01_grass02".to_string(), "ttr01_rock00".to_string()),
            };
            let top = grid(&sc.top);
            // In a cave the roof's top is in the void: black like it.
            let hill = if hm.underground { ENTRANCE_DARK.to_string() } else { hill };
            trimesh_with(&mut s, &format!("{name}_e{k}top"), name, &hill, "", &top, &top.tris, flat);
            let rk = grid(&sc.rock);
            if shadow {
                let mat = format!("{name}m{}", mtrs.len());
                mtrs.push((mat.clone(), shadow_mtr(&rock, &shadow_map)));
                trimesh_full(
                    &mut s,
                    &format!("{name}_e{k}rock"),
                    name,
                    &rock,
                    &format!("  materialname {mat}\n"),
                    &rk,
                    &rk.tris,
                    &tilted,
                    Some(&shadow_uv),
                );
            } else {
                trimesh_full(&mut s, &format!("{name}_e{k}rock"), name, &rock, "", &rk, &rk.tris, &tilted, None);
            }
            let back = grid(&sc.back);
            trimesh_with(&mut s, &format!("{name}_e{k}dark"), name, ENTRANCE_DARK, "", &back, &back.tris, |v| [v[0], v[1]]);
        }
    }
    // Baked objects: each one's triangles in this tile, by texture.
    if let Some(scene) = objects::scene() {
        let (x0, y0) = (tx as f32 * TILE, ty as f32 * TILE);
        let (x1, y1) = (x0 + TILE, y0 + TILE);
        let mut node = 0;
        for o in scene.objects.iter().filter(|o| o.baked && o.max[0] > x0 && o.min[0] < x1 && o.max[1] > y0 && o.min[1] < y1) {
            for (texture, first, count, hint) in &o.groups {
                let (mut verts, mut uvs) = (Vec::new(), Vec::new());
                for t in *first..first + count {
                    let tri = [o.positions[3 * t], o.positions[3 * t + 1], o.positions[3 * t + 2]];
                    let uv = [o.uvs[3 * t], o.uvs[3 * t + 1], o.uvs[3 * t + 2]];
                    for (p, u) in objects::clip(tri, uv, x0, x1, y0, y1) {
                        verts.extend(p.map(|v| [v[0] - ox, v[1] - oy, v[2] - base]));
                        uvs.extend(u);
                    }
                }
                if !verts.is_empty() {
                    let props = if *hint != 0 { format!("  transparencyhint {hint}\n") } else { String::new() };
                    trimesh_listed(&mut s, &format!("{name}_o{node}"), name, texture, &props, &verts, &uvs);
                    node += 1;
                }
            }
        }
    }
    if let Some(water) = water_surface(tx, ty) {
        trimesh_with(&mut s, &format!("{name}_wat"), name, WATER_TEXTURE, "  transparencyhint 1\n", &water, &water.tris, |v| {
            [(v[0] + ox) / WATER_METRES, (v[1] + oy) / WATER_METRES]
        });
    }
    let _ = writeln!(
        s,
        "node aabb {name}_wg\n  parent {name}\n  ambient 0.2 0.2 0.2\n  diffuse 0.8 0.8 0.8\n  specular 0 0 0\n  shininess 1\n  render 0"
    );
    aabb_body(&mut s, w);
    let _ = writeln!(s, "endnode\nendmodelgeom {name}\ndonemodel {name}");
    s
}

/// Tile `name`'s walkmesh `w` as the text of a `.wok`.
pub(super) fn tile_wok(name: &str, w: &Walk) -> String {
    let mesh = Mesh {
        vertices: w.verts.clone(),
        faces: w.faces.iter().map(|(v, m)| Face { vertices: *v, material: *m }).collect(),
        ..Default::default()
    };
    let tree = mg_mdl::compile::aabb_tree(&mesh);
    let mut s = format!(
        "#MAXWALKMESH  ASCII\nbeginwalkmeshgeom {name}\nnode aabb {name}_wg\n  parent {name}\n  position 0 0 0\n  orientation 0 0 0 0\n"
    );
    aabb_body(&mut s, w);
    for (i, e) in tree.iter().enumerate() {
        let lead = if i == 0 { "  aabb" } else { "      " };
        let _ = writeln!(
            s,
            "{lead} {} {} {} {} {} {} {}",
            num(e.min[0]),
            num(e.min[1]),
            num(e.min[2]),
            num(e.max[0]),
            num(e.max[1]),
            num(e.max[2]),
            e.face
        );
    }
    let _ = writeln!(s, "endnode\nendwalkmeshgeom {name}");
    s
}

/// The engine's .set parser overflows a buffer on LF-only text: BioWare's text resources are CRLF.
pub(super) fn crlf(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

/// Tile models: `<name>_<xx><yy>` (up to 14 characters). The water over tile (tx, ty), in its own coordinates: a flat quad at the
/// water's level over each 0.625 m cell with a corner under water (the ground hides it beyond the shore).
fn water_surface(tx: usize, ty: usize) -> Option<Grid> {
    let hm = heightmap::current().filter(|h| !h.water.is_empty())?;
    let k = (TILE / hm.spacing).round() as i64;
    let base = tile_level(tx, ty) as f32 * TRANSITION;
    let (mut verts, mut tris) = (Vec::new(), Vec::new());
    for cj in 0..k {
        for ci in 0..k {
            let (i, j) = (tx as i64 * k + ci, ty as i64 * k + cj);
            let level =
                [(0, 0), (1, 0), (0, 1), (1, 1)].iter().filter_map(|(di, dj)| hm.water_level(i + di, j + dj)).fold(f32::NAN, f32::max);
            if level.is_nan() {
                continue;
            }
            let at = verts.len() as u32;
            let (x0, y0) = (ci as f32 * hm.spacing - TILE / 2.0, cj as f32 * hm.spacing - TILE / 2.0);
            let z = level - base;
            verts.extend([[x0, y0, z], [x0 + hm.spacing, y0, z], [x0 + hm.spacing, y0 + hm.spacing, z], [x0, y0 + hm.spacing, z]]);
            tris.extend([[at, at + 1, at + 2], [at, at + 2, at + 3]]);
        }
    }
    (!tris.is_empty()).then_some(Grid { verts, tris })
}

/// The walkmesh `w` of tile (tx, ty) with the surfaces the game gets: deep water where blocked ground is under water, wading water
/// where walkable ground is, else the painted texture's (grass unpainted).
pub(super) fn surfaced(w: &Walk, tx: usize, ty: usize, painter: Option<&Painter>) -> Walk {
    let (ox, oy) = (tx as f32 * TILE + TILE / 2.0, ty as f32 * TILE + TILE / 2.0);
    let hm = heightmap::current();
    let mut out = w.clone();
    for (f, m) in &mut out.faces {
        let c = f.map(|v| w.verts[v as usize]);
        let (x, y) = ((c[0][0] + c[1][0] + c[2][0]) / 3.0 + ox, (c[0][1] + c[1][1] + c[2][1]) / 3.0 + oy);
        let wet = hm.as_ref().is_some_and(|h| h.water_at(x, y).is_some());
        *m = match (*m == MAT_NONWALK, wet) {
            (true, true) => MAT_DEEP_WATER,
            (true, false) => MAT_NONWALK,
            (false, true) => MAT_WATER,
            (false, false) => painter.map_or(MAT_GRASS, |p| p.textures[p.nearest(x, y) as usize].surface),
        };
    }
    out
}

/// The generated tileset's `.set`, each tile with its path node letter and orientation (`nodes`, in tile order).
pub(super) fn set_file(nodes: &[(char, u8)]) -> String {
    let mut s = format!(
        "[GENERAL]\nName={}\nType=SET\nVersion=V1.0\nInterior=0\nHasHeightTransition=1\nEnvMap=ttr01__ref01\n\
         Transition=5\nDisplayName=-1\nUnlocalizedName=Landscaper\nBorder=Ground\nDefault=Ground\nFloor=Ground\n\n\
         [GRASS]\nGrass=0\n\n",
        tileset_name().to_uppercase(),
    );
    // Ground, and the area corners' own terrains and the boundary edges' own crossers, which only key the edge scenery (see edge.rs).
    let _ = write!(s, "[TERRAIN TYPES]\nCount={}\n\n[TERRAIN0]\nName=Ground\nStrRef=63635\n\n", 1 + edge::CORNERS.len());
    for (k, c) in edge::CORNERS.iter().enumerate() {
        let _ = write!(s, "[TERRAIN{}]\nName={c}\nStrRef=63635\n\n", k + 1);
    }
    // Each boundary edge's: north and south along x, east and west along y.
    let (w, h) = (tiles_x(), tiles_y());
    let crossers: Vec<String> = (0..w)
        .flat_map(|t| [edge::tile_entries(t, h - 1).0[0].clone(), edge::tile_entries(t, 0).0[2].clone()])
        .chain((0..h).flat_map(|t| [edge::tile_entries(w - 1, t).0[1].clone(), edge::tile_entries(0, t).0[3].clone()]))
        .collect();
    let _ = write!(s, "[CROSSER TYPES]\nCount={}\n\n", crossers.len());
    for (k, c) in crossers.iter().enumerate() {
        let _ = write!(s, "[CROSSER{k}]\nName={c}\nStrRef=-1\n\n");
    }
    let _ = write!(s, "[PRIMARY RULES]\nCount=0\n\n[SECONDARY RULES]\nCount=0\n\n[TILES]\nCount={}\n\n", tiles_x() * tiles_y());
    for ty in 0..tiles_y() {
        for tx in 0..tiles_x() {
            let i = ty * tiles_x() + tx;
            let ([top, right, bottom, left], [tl, tr, bl, br]) = edge::tile_entries(tx, ty);
            let _ = write!(
                s,
                "[TILE{i}]\nModel={}\nWalkMesh=msb01\nTopLeft={tl}\nTopLeftHeight=0\nTopRight={tr}\nTopRightHeight=0\n\
                 BottomLeft={bl}\nBottomLeftHeight=0\nBottomRight={br}\nBottomRightHeight=0\n\
                 Top={top}\nRight={right}\nBottom={bottom}\nLeft={left}\n\
                 MainLight1=1\nMainLight2=1\nSourceLight1=0\nSourceLight2=0\nAnimLoop1=0\nAnimLoop2=0\nAnimLoop3=0\n\
                 Doors=0\nSounds=0\nPathNode={}\nOrientation={}\nVisibilityNode=A\nVisibilityOrientation=0\nImageMap2D={}\n\n",
                if std::env::var_os("LS_REUSE").is_some() { "ttr01_o01_01".to_string() } else { tile_name(tx, ty) },
                // .set orientations are degrees.
                nodes[i].0,
                u32::from(nodes[i].1) * 90,
                if std::env::var_os("LS_REUSE").is_some() { "mitr01_A01".to_string() } else { minimap_name(tx, ty) }
            );
        }
    }
    s.push_str("[GROUPS]\nCount=0\n");
    s
}
