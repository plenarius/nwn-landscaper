// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A tile's files: its model, its walkmesh with the game's surfaces, its water, and the tileset's .set file.

use std::fmt::Write as _;

use mg_mdl::{Face, Mesh};

use super::overhang;
use super::{
    ENTRANCE_DARK, Grid, MAT_DEEP_WATER, MAT_GRASS, MAT_NONWALK, MAT_WATER, Mapping, Painter, RENDER_STEP, TILE, TRANSITION, VOID_BELOW,
    WALKABLE_NZ, WATER_METRES, WATER_TEXTURE, Walk, aabb_body, baked_name, edge, entrance, entrance_rock, grid, height, minimap_name,
    mtr_name, normal_z, num, shadow_name, split, terrain_mesh, tile_level, tile_mtr, tile_name, tiles_x, tiles_y, tileset_name, trimesh,
    trimesh_full, trimesh_listed, trimesh_with,
};
use crate::{ambience, heightmap, objects, paint, shared};

/// Tile (tx, ty)'s model; `mtrs` gets the materials of its shadowed meshes (when shadows are on: `shadow`). `ambience`: the
/// particle placements baked into it, and the pack they come from. `overhang`: the edge scenery a boundary tile draws over the
/// halo.
pub(super) fn tile_model(
    name: &str,
    tx: usize,
    ty: usize,
    w: &Walk,
    painter: Option<&Painter>,
    shadow: bool,
    mtrs: &mut Vec<(String, String)>,
    ambience: Option<(&[ambience::Baked], &ambience::Pack)>,
    overhang: Option<&overhang::Overhang>,
) -> String {
    let (ox, oy) = (tx as f32 * TILE + TILE / 2.0, ty as f32 * TILE + TILE / 2.0);
    let base = tile_level(tx, ty) as f32 * TRANSITION;
    let mut g = grid(tx, ty, RENDER_STEP);
    let hm = heightmap::current();
    // A chasm: how far each vertex is under its rim. Its floor is black (with the void); its walls fade to black going down.
    let chasm = heightmap::current().filter(|h| !h.rim.is_empty());
    let depth = |v: [f32; 3]| chasm.as_ref().map_or(0.0, |h| h.chasm_depth(v[0] + ox, v[1] + oy));
    let mut parts: [Vec<[u32; 3]>; 4] = Default::default();
    for &t in &g.tris {
        parts[part(hm.as_deref(), t.map(|v| g.verts[v as usize]), [ox, oy, base]) as usize].push(t);
    }
    // Steep ground where its paint changes: cut so each piece has one texture, and each steep face's texture (split.rs).
    let mut rock_paint = None;
    if let Some(h) = hm.as_deref().filter(|h| painter.is_some() && h.painted()) {
        let mut levels = std::collections::HashMap::new();
        for (x, y) in (tx.saturating_sub(1)..=tx + 1).flat_map(|x| (ty.saturating_sub(1)..=ty + 1).map(move |y| (x, y))) {
            levels.insert((x, y), if (x, y) == (tx, ty) { base } else { tile_level(x, y) as f32 * TRANSITION });
        }
        let level = |x: usize, y: usize| levels.get(&(x, y)).copied().unwrap_or_else(|| tile_level(x, y) as f32 * TRANSITION);
        let centre = |t: usize| t as f32 * TILE + TILE / 2.0;
        let area = split::Area {
            step: RENDER_STEP,
            tile: TILE,
            tiles: (tiles_x(), tiles_y()),
            spacing: h.spacing,
            material: &|i, j| h.material(i, j),
            height: &height,
            base: &level,
            steep: &|x, y, f| part(Some(h), f, [centre(x), centre(y), level(x, y)]) == Part::Rock,
        };
        rock_paint = Some(split::split(&mut g, (tx, ty), &mut parts, Part::Rock as usize, &area));
    }
    let [ground, rock, void, walls] = parts;
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
    // The tile's (0, 0, 0) in the area, for texture coordinates from area coordinates, so textures run on across tiles.
    let origin = [ox, oy, base];
    // Ground and steep meshes, each with its own material when drawn triplanar (a library texture) or shadowed, and then the tile's
    // square as second texture coordinates.
    let shadow_map = shadow_name(tx, ty);
    let square = |v: [f32; 3]| [(v[0] + TILE / 2.0) / TILE, (v[1] + TILE / 2.0) / TILE];
    // The shadow map's coordinates (tverts1): the tile's square, as the colour's. Checked in game 2026-10-20 with
    // LS_SHADOW_DEBUG=half: each tile's dark south half lay south of a pillar standing on a tile corner.
    let shadow_uv = square;
    // A baked picture's: the square inside its margin (painter.rs, `BAKE_MARGIN`).
    let on_picture = |v: [f32; 3]| super::painter::picture_uv(square(v));
    let mut lit = |s: &mut String, node: String, bitmap: &str, faces: &[[u32; 3]], mapping: Mapping<'_>| {
        if faces.is_empty() {
            return;
        }
        let triplanar = match mapping {
            Mapping::Boxed(metres) => Some(metres),
            Mapping::Vertex(_) => None,
        };
        let props = if shadow || triplanar.is_some() {
            let mat = mtr_name(name, mtrs.len());
            mtrs.push((mat.clone(), tile_mtr(bitmap, shadow.then_some(shadow_map.as_str()), triplanar)));
            format!("  materialname {mat}\n")
        } else {
            String::new()
        };
        terrain_mesh(s, &node, name, bitmap, &props, &g, faces, origin, mapping, shadow.then_some(&shadow_uv));
    };
    match painter {
        None => {
            lit(&mut s, format!("{name}_gnd"), "ttr01_grass02", &ground, Mapping::Vertex(&|v| [(v[0] + ox) / 5.0, (v[1] + oy) / 5.0]));
            lit(&mut s, format!("{name}_rck"), "ttr01_rock00", &rock, Mapping::Boxed(paint::TEXTURE_METRES));
        }
        Some(p) => {
            match p.tile_materials(tx, ty).as_slice() {
                // An import's picture: baked like several textures.
                _ if p.baked(tx, ty) => lit(&mut s, format!("{name}_gnd"), &baked_name(tx, ty), &ground, Mapping::Vertex(&on_picture)),
                // One texture: the library's, repeating, drawn from three sides.
                [m] => {
                    let bitmap = paint::resref(&p.hm.palette[*m as usize]);
                    lit(&mut s, format!("{name}_gnd"), &bitmap, &ground, Mapping::Boxed(p.metres(*m)));
                }
                // Several: the tile's own baked texture over its square (baked from three sides, as the one-texture tiles draw it).
                _ => lit(&mut s, format!("{name}_gnd"), &baked_name(tx, ty), &ground, Mapping::Vertex(&on_picture)),
            }
            // Steep faces: each one's texture from split.rs, else the texture painted nearest the face's middle.
            let mut by_material: std::collections::BTreeMap<u8, Vec<[u32; 3]>> = Default::default();
            for (k, &t) in rock.iter().enumerate() {
                let m = rock_paint.as_ref().and_then(|r| r.get(k).copied()).unwrap_or_else(|| {
                    let c = t.map(|v| g.verts[v as usize]);
                    p.nearest((c[0][0] + c[1][0] + c[2][0]) / 3.0 + ox, (c[0][1] + c[1][1] + c[2][1]) / 3.0 + oy)
                });
                by_material.entry(m).or_default().push(t);
            }
            for (k, (m, faces)) in by_material.iter().enumerate() {
                if p.imported(*m) {
                    // The tileset's own cliff texture there, else the picture. (The original walls, in the tile's object, stand in
                    // front: the grid's slope fills what they don't cover.)
                    let Some(tiles) = p.textures[*m as usize].tiles.as_ref() else { continue };
                    match tiles.cliffs.get(ty * tiles.width + tx).cloned().flatten() {
                        Some(cliff) => lit(&mut s, format!("{name}_rck{k}"), &cliff, faces, Mapping::Boxed(paint::TEXTURE_METRES)),
                        None => lit(&mut s, format!("{name}_rck{k}"), &baked_name(tx, ty), faces, Mapping::Vertex(&on_picture)),
                    }
                } else {
                    lit(&mut s, format!("{name}_rck{k}"), &paint::resref(&p.hm.palette[*m as usize]), faces, Mapping::Boxed(p.metres(*m)));
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
        // (a tile's own picture lies over its square; a texture, from three sides like the steep faces)
        let picture = bitmap == baked_name(tx, ty);
        let mat = mtr_name(name, mtrs.len());
        let metres = painter.filter(|p| !p.imported(m)).map_or(paint::TEXTURE_METRES, |p| p.metres(m));
        mtrs.push((mat.clone(), tile_mtr(&bitmap, Some(shared::CHASM_FADE), (!picture).then_some(metres))));
        let mapping = if picture { Mapping::Vertex(&on_picture) } else { Mapping::Boxed(metres) };
        terrain_mesh(
            &mut s,
            &format!("{name}_cw"),
            name,
            &bitmap,
            &format!("  materialname {mat}\n"),
            &g,
            &walls,
            origin,
            mapping,
            Some(&fade),
        );
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
            // (each texture at its own size a repeat, as the ground round it draws it)
            let (hill, rock, hill_m, rock_m) = match painter {
                Some(p) => {
                    let (bx, by) = e.at(e.depth + 2.0, 0.0);
                    let h = p.nearest(bx, by);
                    let r = e
                        .rock
                        .as_deref()
                        .and_then(|r| p.hm.palette.iter().position(|id| id == r))
                        .map_or_else(|| entrance_rock(p), |i| i as u8);
                    (paint::resref(&p.hm.palette[h as usize]), paint::resref(&p.hm.palette[r as usize]), p.metres(h), p.metres(r))
                }
                None => ("ttr01_grass02".to_string(), "ttr01_rock00".to_string(), paint::TEXTURE_METRES, paint::TEXTURE_METRES),
            };
            let flat = |v: [f32; 3]| [(v[0] + ox) / hill_m, (v[1] + oy) / hill_m];
            // Tilted with height, so the underside and the upright lip both take the texture unstretched enough.
            let tilted = |v: [f32; 3]| {
                let (x, y, z) = (v[0] + ox, v[1] + oy, v[2] + base);
                [(x + 0.6 * z) / rock_m, (y - 0.4 * z) / rock_m]
            };
            let top = grid(&sc.top);
            // In a cave the roof's top is in the void: black like it.
            let hill = if hm.underground { ENTRANCE_DARK.to_string() } else { hill };
            trimesh_with(&mut s, &format!("{name}_e{k}top"), name, &hill, "", &top, &top.tris, flat);
            let rk = grid(&sc.rock);
            if shadow {
                let mat = mtr_name(name, mtrs.len());
                mtrs.push((mat.clone(), tile_mtr(&rock, Some(&shadow_map), None)));
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
        for o in scene.objects.iter().filter(|o| o.baked && !o.halo && o.max[0] > x0 && o.min[0] < x1 && o.max[1] > y0 && o.min[1] < y1) {
            for (k, (texture, first, count, hint)) in o.groups.iter().enumerate() {
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
                    let mut props = if *hint != 0 { format!("  transparencyhint {hint}\n") } else { String::new() };
                    // (a glowing mesh keeps its glow: crystals, mushrooms, embers)
                    if let Some(c) = o.glow.get(k).filter(|c| c.iter().any(|&x| x > 0.0)) {
                        let _ = writeln!(props, "  selfillumcolor {} {} {}", num(c[0]), num(c[1]), num(c[2]));
                    }
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
    // The halo's ground and trees beyond the area's edge, meeting this tile's outline.
    if let Some(o) = overhang {
        s.push_str(&o.nodes(name, tx, ty, &g, origin));
    }
    // Particles: under the tile's `<name>a` dummy, keyed in its animations (`ambience`).
    if let Some((baked, pack)) = ambience {
        s.push_str(&ambience::tile_nodes(name, baked, pack));
    }
    let _ = writeln!(
        s,
        "node aabb {name}_wg\n  parent {name}\n  ambient 0.2 0.2 0.2\n  diffuse 0.8 0.8 0.8\n  specular 0 0 0\n  shininess 1\n  render 0"
    );
    aabb_body(&mut s, w);
    let _ = writeln!(s, "endnode\nendmodelgeom {name}");
    if let Some((baked, pack)) = ambience {
        let animations = ambience::tile_animations(name, &s, baked, pack);
        s.push_str(&animations);
    }
    let _ = writeln!(s, "donemodel {name}");
    s
}

/// What a face of a tile's ground is drawn as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// Walkable: the tile's texture.
    Ground,
    /// Too steep to walk: a library texture, the paint's nearest.
    Rock,
    /// A cave's rock top at its roof, or a chasm's floor: black.
    Void,
    /// A chasm's wall, fading to black going down.
    Wall,
}

/// What face `t` (a tile's own coordinates; `origin` is the tile's middle and base in the area) is drawn as, in the terrain `hm`.
fn part(hm: Option<&heightmap::Heightmap>, t: [[f32; 3]; 3], origin: [f32; 3]) -> Part {
    let [ox, oy, base] = origin;
    // (A vertex is void when it is at its ceiling: the roof's own height, or the one that follows the floor.)
    let cave = hm.filter(|h| h.underground && h.roof.is_some());
    let in_void = cave.is_some_and(|h| t.iter().all(|&[x, y, z]| h.roof_at(x + ox, y + oy).is_some_and(|r| z + base >= r - VOID_BELOW)));
    let chasm = hm.filter(|h| !h.rim.is_empty());
    let deep = t.map(|v| chasm.map_or(0.0, |h| h.chasm_depth(v[0] + ox, v[1] + oy)));
    let on_floor = chasm.is_some() && deep.iter().all(|&d| d >= heightmap::CHASM_DEPTH - 0.01);
    if in_void || on_floor {
        Part::Void
    } else if deep.iter().any(|&d| d > heightmap::CHASM_BELOW) {
        Part::Wall
    } else if normal_z(&t, [0, 1, 2]) >= WALKABLE_NZ {
        Part::Ground
    } else {
        Part::Rock
    }
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
/// water's level over each 0.625 m cell with a corner under water, the shallows too, so it runs on under the bank and the ground
/// hides it beyond the shore: the shore is where they meet, not the cells' edges.
fn water_surface(tx: usize, ty: usize) -> Option<Grid> {
    let hm = heightmap::current().filter(|h| !h.water.is_empty())?;
    water_cells(&hm, tx, ty, tile_level(tx, ty) as f32 * TRANSITION)
}

/// [`water_surface`] over `hm`, for a tile whose ground is `base` metres up.
fn water_cells(hm: &heightmap::Heightmap, tx: usize, ty: usize, base: f32) -> Option<Grid> {
    let k = (TILE / hm.spacing).round() as i64;
    let (mut verts, mut tris) = (Vec::new(), Vec::new());
    for cj in 0..k {
        for ci in 0..k {
            let (i, j) = (tx as i64 * k + ci, ty as i64 * k + cj);
            let level =
                [(0, 0), (1, 0), (0, 1), (1, 1)].iter().filter_map(|(di, dj)| hm.surface_level(i + di, j + dj)).fold(f32::NAN, f32::max);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_water_runs_over_the_dry_shallows_up_to_the_bank() {
        // 3 by 3 tiles: a lake to 1.5 m over ground at 1 m west of x = 15 m, a shelf under 7.5 cm of water to x = 25 m, then dry
        // ground at 2 m.
        let dir = std::env::temp_dir().join(format!("ls-tile-water-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let n = 3 * 16 + 1;
        let heights: Vec<f32> = (0..n * n)
            .map(|k| match k % n {
                0..24 => 1.0,
                24..40 => 1.5 - heightmap::WET_DEPTH / 2.0,
                _ => 2.0,
            })
            .collect();
        std::fs::write(dir.join("t.f32"), heights.iter().flat_map(|h| h.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let header = serde_json::json!({
            "name": "t", "tiles": 3, "spacing": 0.625, "width": n, "height": n, "data": "t.f32",
            "water": [{ "i": 2, "j": 2, "level": 1.5 }],
        });
        std::fs::write(dir.join("t.json"), header.to_string()).unwrap();
        let hm = heightmap::read(&dir.join("t.json")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        // The shelf is dry to walk on...
        assert!(hm.water_at(20.0, 15.0).is_none());
        // ...but the water's surface runs on over it, at the lake's level, to the cell where the bank comes up through it: the
        // ground hides it from there, so the shore isn't cut along the cells where the shallows begin.
        let east = water_cells(&hm, 2, 1, 0.0).expect("water over the shelf");
        let reach = east.verts.iter().map(|v| v[0] + 2.5 * TILE).fold(f32::MIN, f32::max);
        assert!((reach - 25.0).abs() < 1e-3, "reaches x = {reach}");
        assert!(east.verts.iter().all(|v| (v[2] - 1.5).abs() < 1e-6));
    }
}
