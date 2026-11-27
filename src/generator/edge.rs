// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The scenery the game client draws beyond the area's edge, as the client
//! builds it (reverse-engineered from the client binary): 5 cells deep, each strip cell showing the model that
//! `<tileset>_edge.2da` gives the boundary tile beside it, keyed by that
//! tile's two corner terrains and its crosser on that side; each 5 x 5
//! corner block keyed by one area corner's terrain.
//!
//! Every boundary edge gets a crosser of its own (`LSE_W07`...) and each area corner a terrain of its own, so each has a row and a
//! model: the boundary's profile (heights, painted textures, water) carried straight outward, the same at every depth, and the corner
//! ground flat at the corner's height. Rows are written as the client reads them (corner 1 north or west), so it places them unturned
//! on the west and east strips and turned once (90 degrees anticlockwise) on the south and north.

use std::fmt::Write as _;

use super::{
    Grid, Look, Painter, TILE, TRANSITION, WATER_METRES, WATER_TEXTURE, baked_name, height, heightmap, mtr_name, normal_z, paint, shared,
    tile_level, tile_mtr, tiles_x, tiles_y, tileset_name, trimesh, trimesh_with,
};

/// A side of a tile (or of the area), by compass direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    West,
    East,
    South,
    North,
}

impl Side {
    const ALL: [Side; 4] = [Side::West, Side::East, Side::South, Side::North];

    fn letter(self) -> char {
        match self {
            Side::West => 'w',
            Side::East => 'e',
            Side::South => 's',
            Side::North => 'n',
        }
    }

    /// The boundary tile beside strip cell `k`.
    fn tile(self, k: usize) -> (usize, usize) {
        match self {
            Side::West => (0, k),
            Side::East => (tiles_x() - 1, k),
            Side::South => (k, 0),
            Side::North => (k, tiles_y() - 1),
        }
    }

    /// How many tiles the side runs along.
    fn cells(self) -> usize {
        match self {
            Side::West | Side::East => tiles_y(),
            Side::South | Side::North => tiles_x(),
        }
    }

    /// The area point on the boundary `along` metres along it.
    fn boundary(self, along: f32) -> (f32, f32) {
        match self {
            Side::West => (0.0, along),
            Side::East => (tiles_x() as f32 * TILE, along),
            Side::South => (along, 0.0),
            Side::North => (along, tiles_y() as f32 * TILE),
        }
    }

    /// Where along the boundary a model's local point lies: the model is unturned on west and east (local y runs north) and turned 90
    /// degrees anticlockwise on south and north (local y runs west).
    fn along(self, centre: f32, local: [f32; 3]) -> f32 {
        match self {
            Side::West | Side::East => centre + local[1],
            Side::South | Side::North => centre - local[1],
        }
    }
}

/// The crosser naming boundary edge `k` of `side` in the .set.
pub fn crosser(side: Side, k: usize) -> String {
    format!("LSE_{}{k:02}", side.letter().to_ascii_uppercase())
}

/// The terrains of the area's corners: south-west, north-west, south-east, north-east.
pub const CORNERS: [&str; 4] = ["CornerSW", "CornerNW", "CornerSE", "CornerNE"];

/// Tile (tx, ty)'s edges (Top, Right, Bottom, Left) and corner terrains (TopLeft, TopRight, BottomLeft, BottomRight) in the .set.
pub fn tile_entries(tx: usize, ty: usize) -> ([String; 4], [&'static str; 4]) {
    let (last_x, last_y) = (tiles_x() - 1, tiles_y() - 1);
    let mut edges: [String; 4] = Default::default();
    if ty == last_y {
        edges[0] = crosser(Side::North, tx);
    }
    if tx == last_x {
        edges[1] = crosser(Side::East, ty);
    }
    if ty == 0 {
        edges[2] = crosser(Side::South, tx);
    }
    if tx == 0 {
        edges[3] = crosser(Side::West, ty);
    }
    let mut corners = ["Ground"; 4];
    if (tx, ty) == (0, last_y) {
        corners[0] = CORNERS[1];
    }
    if (tx, ty) == (last_x, last_y) {
        corners[1] = CORNERS[3];
    }
    if (tx, ty) == (0, 0) {
        corners[2] = CORNERS[0];
    }
    if (tx, ty) == (last_x, 0) {
        corners[3] = CORNERS[2];
    }
    (edges, corners)
}

fn strip_name(side: Side, k: usize) -> String {
    format!("{}_e{}{k:02}", tileset_name(), side.letter())
}

fn corner_name(k: usize) -> String {
    format!("{}_c{}", tileset_name(), ["sw", "nw", "se", "ne"][k])
}

/// `<tileset>_edge.2da`: a row for every boundary edge and corner.
pub fn table() -> String {
    let mut s = String::from("2DA V2.0\n\n   Corner1  Edge  Corner2  Height  Model\n");
    let mut row = 0;
    for side in Side::ALL {
        for k in 0..side.cells() {
            let (tx, ty) = side.tile(k);
            let (_, c) = tile_entries(tx, ty);
            // Corner 1 is the north end on west and east, the west end on south and north.
            let (c1, c2) = match side {
                Side::West => (c[0], c[2]),
                Side::East => (c[1], c[3]),
                Side::South => (c[2], c[3]),
                Side::North => (c[0], c[1]),
            };
            let _ = writeln!(s, "{row} {c1} {} {c2} 0 {}", crosser(side, k), strip_name(side, k));
            row += 1;
        }
    }
    // Corner blocks: one corner's terrain, in the slot the client fills.
    for (k, (c1, c2)) in [("****", CORNERS[0]), (CORNERS[1], "****"), (CORNERS[2], "****"), ("****", CORNERS[3])].into_iter().enumerate() {
        let _ = writeln!(s, "{row} {c1} **** {c2} 0 {}", corner_name(k));
        row += 1;
    }
    s
}

fn header(name: &str) -> String {
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
    s
}

fn footer(s: &mut String, name: &str) {
    let _ = writeln!(s, "endmodelgeom {name}\ndonemodel {name}");
}

/// The bitmap of ground painted `m`, or the built-in test terrain's grass unpainted.
fn bitmap(painter: Option<&Painter>, m: u8) -> String {
    painter.map_or_else(|| "ttr01_grass02".to_string(), |p| paint::resref(&p.hm.palette[m as usize]))
}

/// Faces counter-clockwise from above.
fn upward(g: &Grid, t: [u32; 3]) -> [u32; 3] {
    if normal_z(&g.verts, t) < 0.0 { [t[0], t[2], t[1]] } else { t }
}

/// Strip cell `k` of `side`: the boundary's profile at 0.625 m steps, carried across the cell (local x, outward or inward alike).
fn strip_model(side: Side, k: usize, painter: Option<&Painter>, mtrs: &mut Vec<(String, String)>) -> String {
    let name = strip_name(side, k);
    let (tx, ty) = side.tile(k);
    let base = tile_level(tx, ty) as f32 * TRANSITION;
    let centre = k as f32 * TILE + TILE / 2.0;
    let steps = 16;
    let step = TILE / steps as f32;
    // Local y from -5 to 5; the profile's along-coordinate for each.
    let mut g = Grid { verts: Vec::new(), tris: Vec::new() };
    // Three columns across (the middle one lets an import's picture fold).
    for i in 0..=steps {
        let ly = -TILE / 2.0 + i as f32 * step;
        let (bx, by) = side.boundary(side.along(centre, [0.0, ly, 0.0]));
        let z = height(bx, by) - base;
        g.verts.push([-TILE / 2.0, ly, z]);
        g.verts.push([0.0, ly, z]);
        g.verts.push([TILE / 2.0, ly, z]);
    }
    let hm = heightmap::current();
    let mut by_material: std::collections::BTreeMap<u8, Vec<[u32; 3]>> = Default::default();
    let mut water = Grid { verts: Vec::new(), tris: Vec::new() };
    let mut chasm = Vec::new();
    for i in 0..steps {
        let (a, b) = (3 * i as u32, 3 * i as u32 + 3);
        let quad = [
            upward(&g, [a, a + 1, b + 1]),
            upward(&g, [a, b + 1, b]),
            upward(&g, [a + 1, a + 2, b + 2]),
            upward(&g, [a + 1, b + 2, b + 1]),
        ];
        let mid = side.along(centre, [0.0, -TILE / 2.0 + (i as f32 + 0.5) * step, 0.0]);
        let (bx, by) = side.boundary(mid);
        // A chasm on the boundary carries on out, black.
        if hm.as_ref().is_some_and(|h| h.in_chasm(bx, by)) {
            chasm.extend(quad);
            continue;
        }
        let m = painter.map_or(0, |p| p.nearest(bx, by));
        by_material.entry(m).or_default().extend(quad);
        // Water carried out where the boundary is under it.
        if let Some(level) = hm.as_ref().and_then(|h| h.surface_at(bx, by)) {
            let z = level - base;
            let (y0, y1) = (-TILE / 2.0 + i as f32 * step, -TILE / 2.0 + (i + 1) as f32 * step);
            let at = water.verts.len() as u32;
            water.verts.extend([[-TILE / 2.0, y0, z], [TILE / 2.0, y0, z], [TILE / 2.0, y1, z], [-TILE / 2.0, y1, z]]);
            water.tris.extend([upward(&water, [at, at + 1, at + 2]), upward(&water, [at, at + 2, at + 3])]);
        }
    }
    let mut s = header(&name);
    // Along the boundary as the area's tiles have it (4 m repeats); across, a whole number of repeats a cell so the five copies join.
    // (For viewers without the game's shaders: the game draws a library texture from three sides at the world position, so it runs
    // on from the area's ground into every copy.)
    let uv = |v: [f32; 3]| [side.along(centre, v) / paint::TEXTURE_METRES, v[0] / 5.0];
    for (n, (m, faces)) in by_material.iter().enumerate() {
        if painter.is_some_and(|p| p.imported(*m)) {
            // An import's picture: the border tile's outer 5 m, mirrored out from the boundary and back (each cell's two ends are the
            // boundary, so the copies join).
            let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
            let (size_x, size_y) = (tiles_x() as f32 * TILE, tiles_y() as f32 * TILE);
            let square = |v: [f32; 3]| {
                let (bx, by) = side.boundary(side.along(centre, v));
                let d = TILE / 2.0 - v[0].abs();
                let (x, y) = match side {
                    Side::West => (d, by),
                    Side::East => (size_x - d, by),
                    Side::South => (bx, d),
                    Side::North => (bx, size_y - d),
                };
                [((x - ox) / TILE).clamp(0.002, 0.998), ((y - oy) / TILE).clamp(0.002, 0.998)]
            };
            trimesh(&mut s, &format!("{name}_g{n}"), &name, &baked_name(tx, ty), &g, faces, square);
        } else {
            let props = triplanar(&name, &bitmap(painter, *m), mtrs);
            trimesh_with(&mut s, &format!("{name}_g{n}"), &name, &bitmap(painter, *m), &props, &g, faces, uv);
        }
    }
    if !chasm.is_empty() {
        trimesh(&mut s, &format!("{name}_ch"), &name, shared::BLACK, &g, &chasm, |v| [v[0], v[1]]);
    }
    if !water.tris.is_empty() {
        let wuv = |v: [f32; 3]| [side.along(centre, v) / WATER_METRES, v[0] / 5.0];
        trimesh_with(&mut s, &format!("{name}_wat"), &name, WATER_TEXTURE, "  transparencyhint 1\n", &water, &water.tris, wuv);
    }
    footer(&mut s, &name);
    s
}

/// Corner block `k` (south-west, north-west, south-east, north-east): flat ground at the area corner's height, with its water.
fn corner_model(k: usize, painter: Option<&Painter>, mtrs: &mut Vec<(String, String)>) -> String {
    let name = corner_name(k);
    let (lx, ly) = (tiles_x() - 1, tiles_y() - 1);
    let (sx, sy) = (tiles_x() as f32 * TILE, tiles_y() as f32 * TILE);
    let ((tx, ty), (x, y)) = [((0, 0), (0.0, 0.0)), ((0, ly), (0.0, sy)), ((lx, 0), (sx, 0.0)), ((lx, ly), (sx, sy))][k];
    let base = tile_level(tx, ty) as f32 * TRANSITION;
    let z = height(x, y) - base;
    let h = TILE / 2.0;
    let quad = |z: f32| Grid { verts: vec![[-h, -h, z], [h, -h, z], [h, h, z], [-h, h, z]], tris: vec![[0, 1, 2], [0, 2, 3]] };
    let mut s = header(&name);
    let g = quad(z);
    let m = painter.map_or(0, |p| p.nearest(x, y));
    if heightmap::current().is_some_and(|h| h.in_chasm(x, y)) {
        // A chasm at the corner: black.
        trimesh(&mut s, &format!("{name}_g"), &name, shared::BLACK, &g, &g.tris, |v| [v[0] / 5.0, v[1] / 5.0]);
    } else if painter.is_some_and(|p| p.imported(m)) {
        // The corner tile's outer 5 m square, mirrored both ways (a 3 x 3 grid, so it can fold at the middle).
        let mut g3 = Grid { verts: Vec::new(), tris: Vec::new() };
        for j in 0..3 {
            for i in 0..3 {
                g3.verts.push([-h + i as f32 * h, -h + j as f32 * h, z]);
            }
        }
        for j in 0..2u32 {
            for i in 0..2u32 {
                let a = j * 3 + i;
                g3.tris.push(upward(&g3, [a, a + 1, a + 4]));
                g3.tris.push(upward(&g3, [a, a + 4, a + 3]));
            }
        }
        let (ox, oy) = (tx as f32 * TILE, ty as f32 * TILE);
        let (inx, iny) = (if x > 0.0 { -1.0 } else { 1.0 }, if y > 0.0 { -1.0 } else { 1.0 });
        let uv = |v: [f32; 3]| {
            let (dx, dy) = (h - v[0].abs(), h - v[1].abs());
            [((x + inx * dx - ox) / TILE).clamp(0.002, 0.998), ((y + iny * dy - oy) / TILE).clamp(0.002, 0.998)]
        };
        trimesh(&mut s, &format!("{name}_g"), &name, &baked_name(tx, ty), &g3, &g3.tris, uv);
    } else {
        let props = triplanar(&name, &bitmap(painter, m), mtrs);
        trimesh_with(&mut s, &format!("{name}_g"), &name, &bitmap(painter, m), &props, &g, &g.tris, |v| [v[0] / 5.0, v[1] / 5.0]);
    }
    if let Some(level) = heightmap::current().and_then(|hm| hm.surface_at(x, y)) {
        let w = quad(level - base);
        trimesh_with(&mut s, &format!("{name}_wat"), &name, WATER_TEXTURE, "  transparencyhint 1\n", &w, &w.tris, |v| {
            [v[0] / 5.0, v[1] / 5.0]
        });
    }
    footer(&mut s, &name);
    s
}

/// A material drawing library texture `bitmap` from three sides by the slope ([`crate::triplanar`]), as the area's tiles draw it, for
/// model `model`; its `materialname` line.
fn triplanar(model: &str, bitmap: &str, mtrs: &mut Vec<(String, String)>) -> String {
    let mat = mtr_name(model, mtrs.len());
    mtrs.push((mat.clone(), tile_mtr(bitmap, None, Look::Triplanar, 0.0)));
    format!("  materialname {mat}\n")
}

/// Every edge model: (resref, ASCII model, its materials as (resref, MTR)). `empty`: each without geometry, for an area whose
/// boundary tiles reach out over its halo (overhang.rs): the client repeats an edge model 5 deep, under that real ground.
pub fn models(painter: Option<&Painter>, empty: bool) -> Vec<(String, String, Vec<(String, String)>)> {
    let blank = |name: &str| {
        let mut s = header(name);
        footer(&mut s, name);
        s
    };
    let mut out = Vec::new();
    for side in Side::ALL {
        for k in 0..side.cells() {
            let mut mtrs = Vec::new();
            let text = if empty { blank(&strip_name(side, k)) } else { strip_model(side, k, painter, &mut mtrs) };
            out.push((strip_name(side, k), text, mtrs));
        }
    }
    for k in 0..4 {
        let mut mtrs = Vec::new();
        let text = if empty { blank(&corner_name(k)) } else { corner_model(k, painter, &mut mtrs) };
        out.push((corner_name(k), text, mtrs));
    }
    out
}
