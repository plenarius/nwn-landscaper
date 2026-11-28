// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The generator: a heightfield becomes an NWN area with a tileset of its own. Each 10 m tile of the area gets a model and
//! walkmesh cut from the same continuous terrain, so neighbouring edges always match. `export` and `check` (the editor's
//! Export and Check) run it on a painted terrain; with none loaded it builds the built-in test terrain (`generate`).
//!
//! This file holds what every stage shares: the area's size, the resref names and the surface materials. The stages are in
//! `generator/`: the ground (`terrain`), the area plan of walkmeshes and path nodes (`plan`), the painted textures and shadows
//! (`painter`), ASCII model writing (`mdl`), each tile's files (`tile`, with `edge`, `entrance` and `split`), the hak and module
//! (`generate`), the minimaps drawn from above by nwn-bake (`above`), an export's or a check's analysis (`analyse`), and the
//! module's install in the user folder (`install`).

mod above;
mod analyse;
mod cards;
mod edge;
mod entrance;
mod generate;
mod hops;
mod install;
mod mdl;
mod overhang;
mod painter;
mod plan;
mod soft;
mod split;
mod terrain;
mod tile;

use self::analyse::start_point;
pub(crate) use self::analyse::{START, check, describe, export, long_note, routes_work, sim_area};
pub(crate) use self::entrance::{DOOR_MARK, door_mark};
pub(crate) use self::generate::{generate, module_haks, our_haks};
pub(crate) use self::install::Before;
use self::mdl::{Grid, Mapping, aabb_body, grid, normal_z, num, terrain_mesh, trimesh, trimesh_full, trimesh_listed, trimesh_with};
use self::painter::{
    MINIMAP, Painter, SHADOW_MAP_FINE, bake_size, entrance_rock, minimap, mtr_name, shadow_map, shadow_name, shadow_strength, tile_mtr,
};
use self::plan::{CROSSING_LIMIT, choose_path_nodes, plan_digest, sim_area_of, simulate_all, tile_crossings_for, tile_mesh, walkmesh_raw};
pub(crate) use self::plan::{Walk, area_plan, in_parallel, in_parallel_on, walkmesh};
pub(crate) use self::terrain::{CLIFF_OVERRIDE, FLAT_OVERRIDE, RUGGED_OVERRIDE, blocked, height};
use self::terrain::{TRANSITION, flat, ground_normal, in_void, render_point, rugged, tile_level};
use self::tile::{crlf, set_file, surfaced, tile_model, tile_wok};
use crate::{heightmap, paint, shared};
use std::fmt::Write as _;

/// A tile's side (m), as the game's.
pub(crate) const TILE: f32 = 10.0;
/// Area size in tiles across (west to east) and up (south to north): the
/// painted terrain's, else 4 x 4, or 8 x 8 for the rugged variant.
pub(crate) fn tiles_x() -> usize {
    heightmap::current().map_or(if rugged() { 8 } else { 4 }, |h| h.tiles)
}
/// Area size in tiles up (see [`tiles_x`]).
pub(crate) fn tiles_y() -> usize {
    heightmap::current().map_or(if rugged() { 8 } else { 4 }, |h| h.tiles_y)
}
/// Render mesh spacing (m).
const RENDER_STEP: f32 = 0.5;
/// A face whose normal's z is below this is a cliff: rock, not walkable.
pub(crate) const WALKABLE_NZ: f32 = 0.64; // acos(0.64): about 50 degrees

/// Resref prefix of the generated tileset and tiles: the painted terrain's name, else the built-in test terrain's.
pub(crate) fn tileset_name() -> String {
    heightmap::current().map_or_else(|| "lsp01".to_string(), |h| h.name.clone())
}

/// What the engine's own tags start with: the area's `tagPrefix` (a canvas's name), else its name ([`tileset_name`]).
pub(crate) fn tag_prefix() -> String {
    heightmap::current().map_or_else(tileset_name, |h| if h.tag_prefix.is_empty() { h.name.clone() } else { h.tag_prefix.clone() })
}

/// Name of the hak (and module) written.
pub(crate) fn hak_name() -> String {
    heightmap::current().map_or_else(|| "landscaper".to_string(), |h| h.name.clone())
}

const MAT_GRASS: u32 = 3;
/// surfacemat.2da's non-walkable row: cliffs and other blocked ground.
pub(crate) const MAT_NONWALK: u32 = 7;
/// surfacemat.2da: wading water (walkable, splashes) and deep water.
const MAT_WATER: u32 = 6;
const MAT_DEEP_WATER: u32 = 17;
/// Water deeper than this (m) can't be walked.
const SHALLOW: f32 = 0.5;
/// The game's water texture (its TXI asks for EE's water shader) and the metres one repeat of it covers, as in the rural tiles.
const WATER_TEXTURE: &str = "ttr01_water01";
const WATER_METRES: f32 = 3.3;

/// The area's water as its meshes are written: the texture, and the properties after the transparency hint (its tint as the
/// mesh's ambient and diffuse colour, its opacity as alpha); without a water look ([`crate::atmosphere::water_look`]), the
/// rural tiles' clear water as before.
fn water_mesh() -> (String, String) {
    let look = heightmap::current().and_then(|h| crate::atmosphere::water_look(h.atmosphere.as_ref()).ok().flatten());
    water_mesh_of(look.as_ref())
}

fn water_mesh_of(look: Option<&crate::atmosphere::WaterLook>) -> (String, String) {
    let mut props = "  transparencyhint 1\n".to_string();
    let Some(w) = look else { return (WATER_TEXTURE.to_string(), props) };
    if let Some([r, g, b]) = w.tint {
        let _ = write!(props, "  ambient {0} {1} {2}\n  diffuse {0} {1} {2}\n", num(r), num(g), num(b));
    }
    if let Some(o) = w.opacity.filter(|o| *o < 1.0) {
        let _ = writeln!(props, "  alpha {}", num(o));
    }
    (w.texture.clone().unwrap_or_else(|| WATER_TEXTURE.to_string()), props)
}
/// A black texture for the inside of entrances (written to the hak).
const ENTRANCE_DARK: &str = shared::BLACK;
/// A cave's rock is void (black) from this far below its roof up.
const VOID_BELOW: f32 = 0.3;

/// The loaded area's ambience sorted into its tiles, from the installed particle pack (see `ambience`).
fn ambience_plan() -> crate::ambience::Plan {
    let list = heightmap::current().map_or_default(|h| h.ambience.clone());
    crate::ambience::plan(&list, crate::ambience::installed(), (tiles_x(), tiles_y()), |tx, ty| tile_level(tx, ty) as f32 * TRANSITION)
}

fn tile_name(tx: usize, ty: usize) -> String {
    format!("{}_{tx:02}{ty:02}", tileset_name())
}

/// A tile's baked texture (when painted with more than one).
fn baked_name(tx: usize, ty: usize) -> String {
    format!("{}t", tile_name(tx, ty))
}

/// A tile's minimap picture.
fn minimap_name(tx: usize, ty: usize) -> String {
    format!("{}m", tile_name(tx, ty))
}

#[cfg(test)]
mod water_look_tests {
    use super::water_mesh_of;
    use crate::atmosphere::WaterLook;

    #[test]
    fn without_a_look_the_water_is_written_as_before() {
        assert_eq!(water_mesh_of(None), ("ttr01_water01".to_string(), "  transparencyhint 1\n".to_string()));
        let murky = WaterLook { texture: Some("tds01_water01".into()), tint: Some([0.5, 1.0, 0.25]), opacity: Some(0.8) };
        let (texture, props) = water_mesh_of(Some(&murky));
        assert_eq!(texture, "tds01_water01");
        assert!(props.contains("  diffuse 0.5 1 0.25\n") && props.contains("  ambient 0.5 1 0.25\n") && props.contains("  alpha 0.8\n"));
        // (a tinted mesh's own colours replace the white ones; an untinted one keeps them)
        let mut s = String::new();
        super::mdl::trimesh_listed(&mut s, "w", "r", &texture, &props, &[[0.0; 3]; 3], &[[0.0; 2]; 3]);
        assert!(!s.contains("diffuse 1 1 1") && s.contains("diffuse 0.5 1 0.25"));
        let mut s = String::new();
        super::mdl::trimesh_listed(&mut s, "w", "r", "ttr01_water01", "  transparencyhint 1\n", &[[0.0; 3]; 3], &[[0.0; 2]; 3]);
        assert!(s.contains("  ambient 1 1 1\n  diffuse 1 1 1\n"));
    }
}
