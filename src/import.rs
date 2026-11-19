// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Importing an NWN area built from a tileset: its ground as a landscaper height grid, and each tile's ground as a picture drawn from
//! the tileset's own textures (transitions and all), top down.
//!
//! Heights come from the tile walkmeshes (every face, walkable or not: a cliff is ground too). The picture is the tile model's faces
//! that lie on that ground (within `ON_GROUND`): what stands on it (trees, houses) is left out, for importing as objects.
//!
//! This file holds the import's shared types and [`import`], which reads each tile. The rest is in `import/`: reading a tile
//! model (`tile`), drawing its picture (`picture`), writing the import's files (`files`), the kept original (`original`), and
//! where an import comes from and the whole import of one (`source`).

mod files;
mod original;
mod picture;
mod source;
mod tile;

use std::collections::HashMap;

use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mg_image::Rgba;
use mg_rules::GameData;

pub use self::files::pick_at;
pub use self::original::{KEPT, adopt, original, production_name, recorded, remove_imports_of};
use self::picture::{fill_gaps, fill_holes, inpaint, load_texture, patch, pick_blocks, raster, sample};
pub use self::source::{Source, import_to, reimport_to, run, sources};
use self::tile::{bounds, cluster, flat_of, tile_parts};
use crate::models;

/// How many threads an import works on: [`crate::config::threads`] (`--jobs`, else `LANDSCAPER_JOBS`, else half the
/// machine's cores). The imported files are the same whatever it is.
pub fn threads() -> usize {
    crate::config::threads()
}

/// Seconds spent in each stage of an import, printed to stderr when `LS_IMPORT_TIMES` is set (for profiling). A stage indented
/// under another is part of it; those of the tiles run on several cores at once, so their times add up across the cores.
mod times {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static SPENT: Mutex<Vec<(&'static str, Duration, usize)>> = Mutex::new(Vec::new());
    /// `f()`, its time added to the stage `name`.
    pub fn span<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
        let t = begin(name);
        let r = f();
        add(name, t);
        r
    }
    /// The start of a stage `name` (listed where it first starts), for [`add`].
    pub fn begin(name: &'static str) -> Instant {
        let mut spent = SPENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !spent.iter().any(|e| e.0 == name) {
            spent.push((name, Duration::ZERO, 0));
        }
        Instant::now()
    }
    /// The time since `t` added to the stage `name`.
    pub fn add(name: &'static str, t: Instant) {
        let d = t.elapsed();
        if let Some(e) = SPENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter_mut().find(|e| e.0 == name) {
            e.1 += d;
            e.2 += 1;
        }
    }
    /// Prints the time each stage took, with `LS_IMPORT_TIMES` set: where an import's seconds go.
    pub fn report() {
        if std::env::var_os("LS_IMPORT_TIMES").is_none() {
            return;
        }
        for (name, d, n) in SPENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter() {
            eprintln!("time {name:<20} {:9.3} s  x{n}", d.as_secs_f64());
        }
    }
}
use times::span;

/// Metres a sample (the editor's grid).
const SPACING: f32 = 0.625;
const TILE: f32 = 10.0;
/// Pixels a tile's picture (as baked tiles).
pub const PICTURE: u32 = 512;
/// A model face counts as ground within this of the walkmesh under it.
const ON_GROUND: f32 = 0.35;

/// Walkmesh surface: deep water (its depth blocks it here).
const DEEP_WATER: u32 = 17;
/// Faces steeper than this (their normal's up part) are cliffs.
const STEEP_NZ: f32 = 0.64;

/// A body of water: filled from sample (i, j) to `level`, no further than
/// the samples `bounds` (i0, j0, i1, j1) its tile's water faces cover.
pub struct Water {
    /// The sample it fills from: column.
    pub i: usize,
    /// The sample it fills from: row.
    pub j: usize,
    /// The water's height (m).
    pub level: f32,
    /// The samples it may fill (i0, j0, i1, j1, inclusive).
    pub bounds: [usize; 4],
}

/// A thing an import found standing on a tile (or the tile's cliffs and
/// blocked ground): its model about (x, y, z), and its patch of ground.
pub struct ImportedObject {
    /// `<xx><yy>` (the tile's fixed object) or `<xx><yy>_<k>`.
    pub key: String,
    /// Where its model is centred, east (area metres).
    pub x: f32,
    /// Where its model is centred, north (area metres).
    pub y: f32,
    /// The height its model stands at (m).
    pub z: f32,
    /// Its model as triangles, to bake into the terrain.
    pub flat: models::Flat,
    /// The picture of the ground under it, when it covers some: the editor paints it on.
    pub decal: Option<Rgba>,
}

/// Samples a tile's side in [`Pick`]'s map (15.6 cm each).
pub const PICK: usize = 64;

/// Which tileset texture covers each bit of the ground, so the editor can pick one up and paint with it (see [`pick_at`]).
#[derive(Default)]
pub struct Pick {
    /// The textures that cover ground, by their resref.
    pub names: Vec<String>,
    /// The `surfacemat.2da` row most of each one's ground walks as (0 when none of it was walkable).
    pub surface: Vec<u32>,
    /// `width` by `height` samples, rows from the south: 0 where no texture was seen, else the index in `names` plus 1.
    pub map: Vec<u8>,
    /// Samples across (`PICK` a tile).
    pub width: usize,
    /// Samples up (`PICK` a tile).
    pub height: usize,
}

/// An area's ground, imported.
pub struct Imported {
    /// Tiles across.
    pub width: u32,
    /// Tiles up.
    pub height: u32,
    /// (width * 16 + 1) by (height * 16 + 1) heights, rows from the south.
    pub heights: Vec<f32>,
    /// Each tile's ground picture (rows from the south), row by row.
    pub pictures: Vec<Rgba>,
    /// Water bodies, as the editor's.
    pub water: Vec<Water>,
    /// Each tile's cliff texture: what its steep ground is drawn with.
    pub cliffs: Vec<Option<String>>,
    /// What stands on each tile (trees, rocks, buildings; its cliffs), as models to bake.
    pub objects: Vec<ImportedObject>,
    /// The tileset textures the cliffs and objects use.
    pub textures: Vec<String>,
    /// The area's tileset resref.
    pub tileset: String,
    /// The area's light and fog, as the editor keeps them.
    pub atmosphere: serde_json::Value,
    /// The area's own settings (scripts, weather, music...), as the editor keeps them.
    pub settings: serde_json::Value,
    /// 'interior', 'underground' or 'surface', from its flags.
    pub kind: &'static str,
    /// What the import couldn't carry over, in words for the builder.
    pub notes: Vec<String>,
    /// Which tileset texture covers each bit of the ground, for the editor's texture picker.
    pub pick: Pick,
}

#[derive(Clone)]
struct Face {
    p: [Vec3; 3],
    uv: [[f32; 2]; 3],
    tex: String,
    /// The mesh's transparency hint (cut-out leaves keep theirs).
    hint: u32,
}

#[derive(Clone)]
struct WalkFace {
    p: [Vec3; 3],
    material: u32,
}

/// One tile, placed: its model's faces and walkmesh in the tile's own square (x, y in -5..5 turned as placed, z above the area's 0).
#[derive(Default)]
struct Placed {
    walk: Vec<WalkFace>,
    /// Water surfaces (the tileset's water textures), at their level.
    water: Vec<[Vec3; 3]>,
    faces: Vec<Face>,
}

/// What [`import`] makes of one tile.
struct TileMade {
    picture: Rgba,
    cliff: Option<String>,
    objects: Vec<ImportedObject>,
    /// The textures its objects and cliff use.
    used: std::collections::BTreeSet<String>,
    /// Its picture's pixels by texture number and walkmesh surface.
    walks: HashMap<(u8, u8), u32>,
    /// Its part of the pick map, `PICK` rows of `PICK`.
    map: Vec<u8>,
}

/// Imports the area whose ARE root is `are`.
pub fn import(game: &GameData, are: &mg_gff::Struct) -> Result<Imported> {
    let tileset_ref = are.resref("Tileset").context("the area has no tileset")?;
    let read = span("  tileset", || nwn_bake::area::tileset(game, tileset_ref)).with_context(|| {
        format!(
            "the tileset {tileset_ref} isn't in the game or in the haks this area comes with; its hak has to be in your hak folder \
             and in the module's hak list (the file's own, else the module's: LANDSCAPER_MODULE_REPO or the review service)"
        )
    })?;
    let tileset = &read.tileset;
    let git = mg_gff::Struct::new(0);
    let model = span("  area model", || mg_area::AreaModel::read(game, are, &git, Some(tileset)));
    let (w, h) = (model.width, model.height);
    let mut notes = model.problems.clone();
    notes.extend(read.notes(&tileset_ref.to_string()));
    let mut cache: HashMap<String, Option<(Vec<WalkFace>, Vec<Face>)>> = HashMap::new();
    let mut placed: Vec<Placed> = Vec::new();
    let is_water = |tex: &str| tex.contains("water") && !tex.contains("watcliff") && !tex.contains("fall");
    for t in &model.tiles {
        let Some(name) = &t.model else {
            placed.push(Placed::default());
            continue;
        };
        let parts = cache
            .entry(name.clone())
            .or_insert_with(|| span("  tile parts", || tile_parts(game, name)).map_err(|e| notes.push(format!("{name}: {e:#}"))).ok())
            .clone();
        let (walk, faces) = parts.unwrap_or_default();
        // Turned as placed, lifted to its height (centred on the tile).
        let turn = Mat4::from_rotation_z(f32::from(t.orientation) * std::f32::consts::FRAC_PI_2);
        let lift = Vec3::new(0.0, 0.0, t.position.z);
        let place = |p: Vec3| turn.transform_point3(p) + lift;
        placed.push(Placed {
            walk: walk.iter().map(|f| WalkFace { p: f.p.map(place), material: f.material }).collect(),
            water: faces
                .iter()
                .filter(|f| is_water(&f.tex))
                .inspect(|f| {
                    if std::env::var_os("LS_IMPORT_WATER").is_some() {
                        eprintln!("water face: tile ({},{}) {name} {} at {:.2}", t.column, t.row, f.tex, place(f.p[0]).z);
                    }
                })
                .map(|f| f.p.map(place))
                .collect(),
            faces: faces.iter().filter(|f| !is_water(&f.tex)).map(|f| Face { p: f.p.map(place), ..f.clone() }).collect(),
        });
    }

    // Ground a tile's model draws beyond its own square, on the tiles there: a group's first tile can hold the whole group's
    // model (wsf10's 2 x 2 tower, wsf10_l10_01, the other three only walkmeshes), so their pictures come from it.
    let spill = spilled(&placed, w as usize, h as usize);

    if std::env::var_os("LS_IMPORT_MATS").is_some() {
        // Walkmesh surfaces by area: (flat m², steep m²) per surfacemat row.
        let mut by: std::collections::BTreeMap<u32, (f32, f32)> = Default::default();
        for f in placed.iter().flat_map(|p| &p.walk) {
            let n = (f.p[1] - f.p[0]).cross(f.p[2] - f.p[0]);
            let e = by.entry(f.material).or_default();
            if n.length() > 1e-6 && (n.z / n.length()).abs() >= STEEP_NZ { e.0 += n.z.abs() / 2.0 } else { e.1 += n.length() / 2.0 }
        }
        eprintln!("walkmesh surfaces (flat m2, steep m2): {by:?}");
    }
    // Whether the game walks a walkmesh surface (surfacemat.2da Walk).
    let surfacemat = game.table("surfacemat").ok();
    let walkable = |m: u32| surfacemat.as_ref().and_then(|t| t.get(m as usize, "Walk")).is_some_and(|v| v.trim() == "1");
    // Ground height under each picture pixel, per tile: the top of the walkmesh the game walks (and lake beds, for the water's depth).
    // What it can't walk (cliffs, walls, pits, trees) is blocked by the tile's footprint and drawn by its own faces in the tile's
    // object; its ground comes from around it, so no drop can eat a walkway beside it (the grid only slopes, 0.625 m a sample).
    let s = PICTURE as usize;
    let px = |v: f32| (v + TILE / 2.0) / TILE * PICTURE as f32;
    // (And the walkmesh surface that top face has there, for the footsteps of a texture picked up from it.)
    let t_ground = times::begin("  ground heights");
    // (Each tile on its own, on the import's threads.)
    let tiles: Vec<usize> = (0..placed.len()).collect();
    let (grounds, ground_mats): (Vec<Vec<f32>>, Vec<Vec<u8>>) = crate::generator::in_parallel_on(threads(), &tiles, |t| {
        let p = &placed[t];
        let mut z = vec![f32::NAN; s * s];
        let mut mats = vec![0u8; s * s];
        // (A lake bed some tilesets give a non-walkable surface rather than deep water is ground under the water all the same.)
        for f in p.walk.iter().filter(|f| walkable(f.material) || f.material == DEEP_WATER || under_water(f, &p.water)) {
            let material = if walkable(f.material) { f.material } else { DEEP_WATER };
            let (face, material) = (f.p, material.min(255) as u8);
            raster(face.map(|v| Vec3::new(px(v.x), px(v.y), v.z)), s, |i, j, b| {
                let zz = b[0] * face[0].z + b[1] * face[1].z + b[2] * face[2].z;
                let k = j * s + i;
                if z[k].is_nan() || zz > z[k] {
                    z[k] = zz;
                    mats[k] = material;
                }
            });
        }
        span("    fill gaps", || fill_gaps(&mut z, s));
        (z, mats)
    })
    .into_iter()
    .unzip();
    times::add("  ground heights", t_ground);
    // Heights at the editor's samples, from the tile under each.
    let k = (TILE / SPACING).round() as usize;
    let (nw, nh) = (w as usize * k + 1, h as usize * k + 1);
    let mut heights = vec![0.0f32; nw * nh];
    for j in 0..nh {
        for i in 0..nw {
            let (tx, ty) = ((i / k).min(w as usize - 1), (j / k).min(h as usize - 1));
            let (lx, ly) = ((i - tx * k) as f32 * SPACING, (j - ty * k) as f32 * SPACING);
            let g = &grounds[ty * w as usize + tx];
            let (pi, pj) = (((lx / TILE) * PICTURE as f32) as usize, ((ly / TILE) * PICTURE as f32) as usize);
            let z = g[pj.min(s - 1) * s + pi.min(s - 1)];
            heights[j * nw + i] = if z.is_nan() { model.tiles[ty * w as usize + tx].position.z } else { z };
        }
    }

    // Water: each tile's water surface at a level is a body (the editor's water), filled from a sample under it and bounded by the
    // samples its faces cover: water in a game tile is only where its faces are, it doesn't run off into lower ground beside it.
    let mut water: Vec<Water> = Vec::new();
    for (t, p) in placed.iter().enumerate() {
        let (tx, ty) = (t % w as usize, t / w as usize);
        let (ox, oy) = (tx as f32 * TILE + TILE / 2.0, ty as f32 * TILE + TILE / 2.0);
        let mut levels: Vec<(f32, Option<(usize, usize)>, [usize; 4])> = Vec::new();
        for f in &p.water {
            let level = (f[0].z + f[1].z + f[2].z) / 3.0;
            let sample = |v: f32, o: f32, max: usize| (((v + o) / SPACING).round().max(0.0) as usize).min(max - 1);
            let (xs, ys) = (f.map(|v| sample(v.x, ox, nw)), f.map(|v| sample(v.y, oy, nh)));
            let span = |a: [usize; 3]| (a[0].min(a[1]).min(a[2]), a[0].max(a[1]).max(a[2]));
            let ((x0, x1), (y0, y1)) = (span(xs), span(ys));
            let b = [x0, y0, x1, y1];
            // A seed: the sample under the face's middle, if below the water.
            let c = (f[0] + f[1] + f[2]) / 3.0;
            let (i, j) = (sample(c.x, ox, nw), sample(c.y, oy, nh));
            let seed = (heights[j * nw + i] < level).then_some((i, j));
            match levels.iter_mut().find(|l| (l.0 - level).abs() < 0.01) {
                Some(l) => {
                    l.1 = l.1.or(seed);
                    l.2 = [l.2[0].min(b[0]), l.2[1].min(b[1]), l.2[2].max(b[2]), l.2[3].max(b[3])];
                }
                None => levels.push((level, seed, b)),
            }
        }
        for (level, seed, bounds) in levels {
            if let Some((i, j)) = seed {
                water.push(Water { i, j, level, bounds });
            }
        }
    }

    let steep = |f: &Face| {
        let n = (f.p[1] - f.p[0]).cross(f.p[2] - f.p[0]);
        n.length() > 1e-6 && (n.z / n.length()).abs() < STEEP_NZ
    };
    // Each model face is ground (on it, or a cliff between its levels) or
    // stands on it: above the highest ground within 0.75 m of its middle.
    let reach = (0.75 / TILE * PICTURE as f32) as i64;
    // A steep face is ground only where the ground steps (a cliff): on flat
    // ground it is something standing there (a trunk's foot, a wall).
    let on_ground = |ground: &[f32], f: &Face| -> bool {
        let c = (f.p[0] + f.p[1] + f.p[2]) / 3.0;
        let (ci, cj) = (px(c.x) as i64, px(c.y) as i64);
        let (mut low, mut top) = (f32::MAX, f32::MIN);
        for dj in (-reach..=reach).step_by(4) {
            for di in (-reach..=reach).step_by(4) {
                let (i, j) = ((ci + di).clamp(0, s as i64 - 1) as usize, (cj + dj).clamp(0, s as i64 - 1) as usize);
                low = low.min(ground[j * s + i]);
                top = top.max(ground[j * s + i]);
            }
        }
        c.z <= top + ON_GROUND && (!steep(f) || top - low > 1.0)
    };

    // Each tile's ground picture from the tileset's textures, its cliff texture, and what stands on it. The picture is drawn from what
    // lies on the ground (cliffs too, seen from above); the object is everything else and the cliffs (the tileset's own walls, sharper
    // than the height grid can be).
    let split: Vec<(Vec<&Face>, Vec<&Face>)> = span("  on ground", || {
        crate::generator::in_parallel_on(threads(), &tiles, |t| {
            let (p, ground) = (&placed[t], &grounds[t]);
            let ground_faces: Vec<&Face> = p.faces.iter().chain(&spill[t]).filter(|f| on_ground(ground, f)).collect();
            let standing: Vec<&Face> = p.faces.iter().filter(|f| !on_ground(ground, f) || steep(f)).collect();
            (ground_faces, standing)
        })
    });
    // The textures the ground faces use, and each its number in the pick map (none past 255 of them), in the order the tiles and
    // their faces first use them (one after the other: the numbers are given by that order).
    let mut textures: HashMap<String, Option<Rgba>> = HashMap::new();
    let mut pick = Pick {
        width: w as usize * PICK,
        height: h as usize * PICK,
        map: vec![0; w as usize * PICK * h as usize * PICK],
        ..Pick::default()
    };
    for f in split.iter().flat_map(|(ground_faces, _)| ground_faces) {
        let tex = textures.entry(f.tex.clone()).or_insert_with(|| span("  load texture", || load_texture(game, &f.tex)));
        if tex.is_some() && pick.names.len() < 255 && !pick.names.contains(&f.tex) {
            pick.names.push(f.tex.clone());
        }
    }
    let ids_of: HashMap<&str, u8> = pick.names.iter().enumerate().map(|(at, n)| (n.as_str(), at as u8 + 1)).collect();
    // Then the tiles, each on its own (on the import's threads), put together in their order.
    let t_tiles = times::begin("  tiles");
    let made: Vec<TileMade> = crate::generator::in_parallel_on(threads(), &tiles, |t| {
        let (p, ground, (ground_faces, standing)) = (&placed[t], &grounds[t], &split[t]);
        let mut used: std::collections::BTreeSet<String> = Default::default();
        let mut objects = Vec::new();
        let mut ids = vec![0u8; s * s];
        let mut img = Rgba::new(PICTURE, PICTURE);
        let mut depth = vec![f32::NEG_INFINITY; s * s];
        for f in ground_faces {
            let Some(Some(tex)) = textures.get(&f.tex) else { continue };
            let id = ids_of.get(f.tex.as_str()).copied().unwrap_or(0);
            let (fp, uv) = (f.p, f.uv);
            let t_draw = times::begin("    draw ground");
            raster(fp.map(|v| Vec3::new(px(v.x), px(v.y), v.z)), s, |i, j, b| {
                let k = j * s + i;
                let z = b[0] * fp[0].z + b[1] * fp[1].z + b[2] * fp[2].z;
                // The top of the ground there.
                if (!ground[k].is_nan() && z > ground[k] + ON_GROUND) || z <= depth[k] {
                    return;
                }
                let (u, v) = (b[0] * uv[0][0] + b[1] * uv[1][0] + b[2] * uv[2][0], b[0] * uv[0][1] + b[1] * uv[1][1] + b[2] * uv[2][1]);
                let c = sample(tex, u, v);
                if c[3] < 128 {
                    return; // a cut-out's hole
                }
                depth[k] = z;
                img.data[k * 4..k * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                // A solid bit of it, not the soft edge of a fringe or a decal, is what a pick finds.
                if c[3] >= 250 {
                    ids[k] = id;
                }
            });
            times::add("    draw ground", t_draw);
        }
        span("    fill holes", || fill_holes(&mut img));
        let t_pick = times::begin("    pick map");
        // What the ground there walks as, per texture (texture number, walkmesh surface); and the tile's map, a block of samples by
        // its most common texture.
        let mut walks: HashMap<(u8, u8), u32> = HashMap::new();
        for (k, &id) in ids.iter().enumerate() {
            if id != 0 && ground_mats[t][k] != 0 {
                *walks.entry((id, ground_mats[t][k])).or_default() += 1;
            }
        }
        let map = pick_blocks(&ids, s);
        times::add("    pick map", t_pick);
        // The cliff texture: the one covering most of the steep ground. (A BTreeMap, so a tie goes the same way every run.)
        let t_things = times::begin("    things");
        let mut by_texture: std::collections::BTreeMap<&str, f32> = Default::default();
        for f in ground_faces.iter().filter(|f| steep(f)) {
            *by_texture.entry(&f.tex).or_default() += (f.p[1] - f.p[0]).cross(f.p[2] - f.p[0]).length() / 2.0;
        }
        let cliff = by_texture.into_iter().filter(|(_, a)| *a > 0.5).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(t, _)| t.to_string());
        if let Some(c) = &cliff {
            used.insert(c.clone());
        }

        // What stands on the tile: each thing (a tree, a rock, a house) its own object, standing on the ground at its spot, with its
        // patch of ground cut from the picture to go where it goes; the cliffs and the blocked ground no thing owns (wall tops, pits)
        // one fixed object. Footprints: every walkmesh face the game can't walk (deep water is the water's), so the ground is blocked
        // as in the original. (Steep ones are cliff walls: they stay with the tile.)
        let flat_walk = |f: &WalkFace| {
            let n = (f.p[1] - f.p[0]).cross(f.p[2] - f.p[0]);
            n.length() < 1e-6 || (n.z / n.length()).abs() >= STEEP_NZ
        };
        let blocked: Vec<&WalkFace> = p.walk.iter().filter(|f| f.material != DEEP_WATER && !walkable(f.material)).collect();
        let footprint: Vec<[[f32; 2]; 3]> = blocked.iter().filter(|f| flat_walk(f)).map(|f| f.p.map(|v| [v.x, v.y])).collect();
        let cliff_footprint: Vec<[[f32; 2]; 3]> = blocked.iter().filter(|f| !flat_walk(f)).map(|f| f.p.map(|v| [v.x, v.y])).collect();
        let (cliff_faces, things): (Vec<&Face>, Vec<&Face>) = standing.iter().copied().partition(|f| steep(f) && on_ground(ground, f));
        let ground_at = |x: f32, y: f32| {
            let (i, j) = ((px(x) as usize).min(s - 1), (px(y) as usize).min(s - 1));
            Some(ground[j * s + i]).filter(|z| !z.is_nan()).unwrap_or(model.tiles[t].position.z)
        };
        let clusters = span("      cluster", || cluster(&things));
        // Each footprint triangle to the thing over it, else the fixed object.
        let boxes: Vec<[f32; 4]> = clusters.iter().map(|c| bounds(c.iter().map(|f| f.p))).collect();
        let mut owned: Vec<Vec<[[f32; 2]; 3]>> = vec![Vec::new(); clusters.len()];
        let mut fixed_footprint = cliff_footprint;
        for tri in footprint {
            let (cx, cy) = ((tri[0][0] + tri[1][0] + tri[2][0]) / 3.0, (tri[0][1] + tri[1][1] + tri[2][1]) / 3.0);
            let owner = boxes
                .iter()
                .enumerate()
                .filter(|(_, b)| cx >= b[0] - 0.5 && cx <= b[2] + 0.5 && cy >= b[1] - 0.5 && cy <= b[3] + 0.5)
                .min_by(|a, b| ((a.1[2] - a.1[0]) * (a.1[3] - a.1[1])).total_cmp(&((b.1[2] - b.1[0]) * (b.1[3] - b.1[1]))))
                .map(|(k, _)| k);
            match owner {
                Some(k) => owned[k].push(tri),
                None => fixed_footprint.push(tri),
            }
        }
        let (tx, ty) = (t % w as usize, t / w as usize);
        // A thing moves if it blocks walking itself (a tree, a rock, a house); what doesn't (the grass fringe along a cliff top,
        // tufts) stays with the tile, as its cliffs do.
        let mut fixed_faces: Vec<&Face> = cliff_faces.clone();
        let mut movable: Vec<(Vec<&Face>, Vec<[[f32; 2]; 3]>)> = Vec::new();
        for (faces, own) in clusters.into_iter().zip(owned) {
            if own.is_empty() {
                fixed_faces.extend(faces);
            } else {
                movable.push((faces, own));
            }
        }
        // The fixed object: at the tile's middle, heights its own.
        if !fixed_faces.is_empty() || !fixed_footprint.is_empty() {
            let mut flat = flat_of(&fixed_faces, [0.0, 0.0, 0.0], &mut used);
            flat.footprint = fixed_footprint;
            flat.fixed = true;
            objects.push(ImportedObject {
                key: format!("{tx:02}{ty:02}"),
                x: tx as f32 * TILE + TILE / 2.0,
                y: ty as f32 * TILE + TILE / 2.0,
                z: 0.0,
                flat,
                decal: None,
            });
        }
        // Each thing, its ground patch's alpha in the tile's picture.
        let mut patches: Vec<(Vec<f32>, [usize; 4])> = Vec::new();
        for (k, (faces, own)) in movable.iter().enumerate() {
            // It stands where its footprint is.
            let base = own.clone();
            let b = bounds(base.iter().map(|t| t.map(|v| Vec3::new(v[0], v[1], 0.0))));
            let (ox, oy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            let oz = ground_at(ox, oy);
            let mut flat = flat_of(faces, [ox, oy, oz], &mut used);
            flat.footprint = own.iter().map(|t| t.map(|v| [v[0] - ox, v[1] - oy])).collect();
            patches.push(span("      patch", || patch(&base, s, px)));
            objects.push(ImportedObject {
                key: format!("{tx:02}{ty:02}_{k}"),
                x: tx as f32 * TILE + TILE / 2.0 + ox,
                y: ty as f32 * TILE + TILE / 2.0 + oy,
                z: oz,
                flat,
                decal: None,
            });
        }
        times::add("    things", t_things);
        let t_decals = times::begin("    decals");
        // The patches out of the picture (the ground under each, fading out), and the picture filled from around where they were.
        let first = objects.len() - movable.len();
        let mut hole = vec![false; s * s];
        for (k, (alpha, [x0, y0, x1, y1])) in patches.iter().enumerate() {
            let (mut i0, mut j0, mut i1, mut j1) = (s, s, 0, 0);
            for j in *y0..*y1 {
                for i in *x0..*x1 {
                    let q = j * s + i;
                    if alpha[q] > 0.0 {
                        (i0, j0, i1, j1) = (i0.min(i), j0.min(j), i1.max(i), j1.max(j));
                        hole[q] = true;
                    }
                }
            }
            if i0 > i1 {
                continue;
            }
            let (dw, dh) = ((i1 - i0 + 1) as u32, (j1 - j0 + 1) as u32);
            let mut d = Rgba::new(dw, dh);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (src, dst) = ((j * s + i) * 4, (((j - j0) * dw as usize) + (i - i0)) * 4);
                    d.data[dst..dst + 3].copy_from_slice(&img.data[src..src + 3]);
                    d.data[dst + 3] = (alpha[j * s + i] * 255.0).round() as u8;
                }
            }
            let o = &mut objects[first + k];
            let (ox, oy) = (o.x - (tx as f32 * TILE + TILE / 2.0), o.y - (ty as f32 * TILE + TILE / 2.0));
            let m = |p: usize| p as f32 / PICTURE as f32 * TILE - TILE / 2.0;
            o.flat.decal = Some(models::Decal {
                file: format!("decal_{}.png", o.key),
                rect: [m(i0) - ox, m(j0) - oy, m(i1 + 1) - ox, m(j1 + 1) - oy],
            });
            o.decal = Some(d);
        }
        times::add("    decals", t_decals);
        span("    inpaint", || inpaint(&mut img, &hole, s));
        TileMade { picture: img, cliff, objects, used, walks, map }
    });
    times::add("  tiles", t_tiles);
    let mut used: std::collections::BTreeSet<String> = Default::default();
    let (mut pictures, mut cliffs, mut objects) = (Vec::new(), Vec::new(), Vec::new());
    let mut walks: Vec<HashMap<u8, u32>> = vec![HashMap::new(); pick.names.len()];
    for (t, m) in made.into_iter().enumerate() {
        let (tx, ty) = (t % w as usize, t / w as usize);
        for (bj, row) in m.map.chunks(PICK).enumerate() {
            let at = (ty * PICK + bj) * pick.width + tx * PICK;
            pick.map[at..at + PICK].copy_from_slice(row);
        }
        for ((id, material), n) in m.walks {
            *walks[id as usize - 1].entry(material).or_default() += n;
        }
        pictures.push(m.picture);
        cliffs.push(m.cliff);
        objects.extend(m.objects);
        used.extend(m.used);
    }
    // (By name: a HashMap's order changes from run to run.)
    let mut missing: Vec<&String> = textures.iter().filter(|(_, t)| t.is_none()).map(|(name, _)| name).collect();
    missing.sort();
    for name in missing {
        notes.push(format!("texture {name} not found"));
    }
    pick.surface = walks
        .iter()
        .map(|m| m.iter().max_by_key(|(row, n)| (**n, std::cmp::Reverse(**row))).map_or(0, |(row, _)| u32::from(*row)))
        .collect();
    Ok(Imported {
        width: w,
        height: h,
        heights,
        pictures,
        water,
        cliffs,
        objects,
        textures: used.into_iter().collect(),
        tileset: tileset_ref.to_string(),
        atmosphere: crate::atmosphere::read(are),
        settings: serde_json::Value::Null,
        // Flags: 1 interior, 2 underground, 4 natural.
        kind: match are.integer("Flags").unwrap_or(4) {
            f if f & 1 != 0 => "interior",
            f if f & 2 != 0 => "underground",
            _ => "surface",
        },
        notes,
        pick,
    })
}

/// The model faces each tile gets from the others' (`w` by `h` tiles): those whose middle is in its square, moved into its own.
/// (They are only drawn on its picture: what stands stays an object of the tile whose model it is.)
fn spilled(placed: &[Placed], w: usize, h: usize) -> Vec<Vec<Face>> {
    let mut spill: Vec<Vec<Face>> = vec![Vec::new(); placed.len()];
    for (t, p) in placed.iter().enumerate() {
        let (tx, ty) = ((t % w) as i64, (t / w) as i64);
        for f in &p.faces {
            let c = (f.p[0] + f.p[1] + f.p[2]) / 3.0;
            let (dx, dy) = (((c.x + TILE / 2.0) / TILE).floor() as i64, ((c.y + TILE / 2.0) / TILE).floor() as i64);
            let (x, y) = (tx + dx, ty + dy);
            if (dx, dy) == (0, 0) || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                continue;
            }
            let by = Vec3::new(dx as f32 * TILE, dy as f32 * TILE, 0.0);
            spill[y as usize * w + x as usize].push(Face { p: f.p.map(|v| v - by), ..f.clone() });
        }
    }
    spill
}

/// Whether walkmesh face `f` lies under one of its tile's `water` surfaces: its middle inside one, and all of it below that
/// water's level. (wsf10's lake tiles, TILE215 and its kind, have a Nonwalk walkmesh under their water, not DeepWater.)
fn under_water(f: &WalkFace, water: &[[Vec3; 3]]) -> bool {
    let c = (f.p[0] + f.p[1] + f.p[2]) / 3.0;
    let top = f.p[0].z.max(f.p[1].z).max(f.p[2].z);
    water.iter().any(|w| {
        let level = (w[0].z + w[1].z + w[2].z) / 3.0;
        let side = |a: Vec3, b: Vec3| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        let (d0, d1, d2) = (side(w[0], w[1]), side(w[1], w[2]), side(w[2], w[0]));
        let inside = (d0 >= -1e-4 && d1 >= -1e-4 && d2 >= -1e-4) || (d0 <= 1e-4 && d1 <= 1e-4 && d2 <= 1e-4);
        inside && top < level
    })
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{Face, Placed, WalkFace, spilled, under_water};

    #[test]
    fn a_group_model_s_ground_beyond_its_tile_goes_to_the_tiles_it_lies_on() {
        let face = |x: f32, y: f32| Face {
            p: [Vec3::new(x - 1.0, y - 1.0, 0.5), Vec3::new(x + 1.0, y - 1.0, 0.5), Vec3::new(x, y + 1.0, 0.5)],
            uv: [[0.0; 2]; 3],
            tex: format!("at {x} {y}"),
            hint: 0,
        };
        // 3 by 2 tiles; the model at (1, 0) reaches into (2, 0), (1, 1) and (2, 1), and past the area's edge (dropped).
        let mut placed: Vec<Placed> = (0..6).map(|_| Placed::default()).collect();
        placed[1].faces = vec![face(0.0, 0.0), face(10.0, 0.0), face(0.0, 10.0), face(12.0, 14.0), face(0.0, -10.0), face(30.0, 0.0)];
        let spill = spilled(&placed, 3, 2);
        let names = |t: usize| spill[t].iter().map(|f| f.tex.as_str()).collect::<Vec<_>>();
        assert_eq!(names(2), ["at 10 0"]);
        assert_eq!(names(4), ["at 0 10"]);
        assert_eq!(names(5), ["at 12 14"]);
        assert!(names(0).is_empty() && names(1).is_empty() && names(3).is_empty());
        // Moved into the square of the tile it lies on.
        let c = spill[5][0].p.iter().fold(Vec3::ZERO, |a, &b| a + b) / 3.0;
        assert!((c - Vec3::new(2.0, 4.0 - 1.0 / 3.0, 0.5)).length() < 1e-5, "{c}");
    }

    #[test]
    fn a_lake_bed_under_its_tile_s_water_is_ground_whatever_its_surface() {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        // wsf10_e12_01 (TILE215): water at -1 over the whole tile, its walkmesh Nonwalk (7) from -2 to -1.41.
        let water =
            [[v(-5.0, -5.0, -1.0), v(5.0, -5.0, -1.0), v(5.0, 5.0, -1.0)], [v(-5.0, -5.0, -1.0), v(5.0, 5.0, -1.0), v(-5.0, 5.0, -1.0)]];
        let bed = WalkFace { p: [v(-3.0, -2.0, -2.0), v(-5.0, -5.0, -2.0), v(-2.0, -2.0, -1.92)], material: 7 };
        assert!(under_water(&bed, &water));
        // Turned the other way round (a face wound clockwise), the same.
        let turned = WalkFace { p: [bed.p[0], bed.p[2], bed.p[1]], ..bed.clone() };
        assert!(under_water(&turned, &water));
        // Above the water (a bank, a rock in the lake), or beside it, it isn't.
        assert!(!under_water(&WalkFace { p: bed.p.map(|p| p + v(0.0, 0.0, 1.5)), material: 7 }, &water));
        let half =
            [[v(0.0, -5.0, -1.0), v(5.0, -5.0, -1.0), v(5.0, 5.0, -1.0)], [v(0.0, -5.0, -1.0), v(5.0, 5.0, -1.0), v(0.0, 5.0, -1.0)]];
        assert!(!under_water(&bed, &half));
        // No water on the tile at all.
        assert!(!under_water(&bed, &[]));
    }

    #[test]
    fn work_on_a_few_threads_comes_back_in_order_and_uses_no_more_of_them() {
        use std::collections::HashSet;
        use std::sync::Mutex;
        let items: Vec<usize> = (0..200).collect();
        for threads in [1, 2, 3, 64] {
            let seen = Mutex::new(HashSet::new());
            let out = crate::generator::in_parallel_on(threads, &items, |i| {
                seen.lock().unwrap().insert(std::thread::current().id());
                std::thread::sleep(std::time::Duration::from_micros(200));
                i * 3
            });
            assert_eq!(out, items.iter().map(|i| i * 3).collect::<Vec<_>>());
            let used = seen.lock().unwrap().len();
            assert!(used >= 1 && used <= threads, "{threads} threads asked, {used} used");
        }
    }
}
