// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The route simulator: our own reimplementation of how the NWN:EE game server plans a creature's walk inside one area, written so
//! that the areas we generate and export walk the way the server will route them (for interoperability with the game). It is checked
//! against routes recorded on a real server (the `oracle` and `sim-check` research commands).
//!
//! Given a generated area (walkmeshes and path node tables) and a start and a goal, it predicts what an NPC's walk request comes to: a
//! direct line, a complete waypoint path, a partial straight line or nothing, and where the creature stops.
//!
//! The arithmetic is f32 and the order of operations, tie-breaks and limits follow the server's results closely, because a route that
//! differs by a rounding step can take another tile exit. That is why some code below looks more literal than it needs to be: it is
//! kept that way on purpose.
//!
//! Layout:
//! * `mesh`  - one tile's walkmesh: its triangles at three levels of detail,
//!   point lookup and the line and clearance tests.
//! * `intra` - the search for a path across one tile, from the entry point to
//!   a goal box on the exit edge (iterative deepening over triangles).
//! * `inter` - the search for a route of tiles, from exit to exit, and the
//!   replanning when a tile can't be crossed.
//! * `area`  - the straight-line test across tiles, the request's outcome and
//!   the smoothing of the waypoints.
//! * `grid`  - the short-range fallback: a search on a fine grid.
//! * `spawn` - where a newly created creature is put.

pub mod area;
pub mod grid;
pub mod inter;
pub mod intra;
pub mod mesh;
pub mod spawn;

use std::collections::HashMap;

pub use area::{Outcome, Query, SimResult};
use mesh::Mesh;

/// The tolerance of point comparisons (exactly the f32 nearest 1e-4).
pub const EPSILON: f32 = 1.0e-4;
/// A tile-local coordinate at or below this is on the tile's low edge.
pub const MIN_TILE_EPS: f32 = 1.0e-4;
/// A tile-local coordinate at or above this is on the tile's high edge.
pub const MAX_TILE_EPS: f32 = 9.9999;
/// A tile's side (m), as the game's.
pub const TILE_SIZE: f32 = 10.0;
/// A triangle edge with no neighbour inside the tile: the walkmesh ends there.
pub const TRIADJ_NO_EDGE: i32 = -3;
/// A triangle edge on the tile's border: the walk goes on in the next tile.
pub const TRIADJ_TILE_END: i32 = -2;
/// "No exit": a point on no exit of the tile, also meaning "the goal".
pub const UNKNOWN_EXIT: i32 = 12;
/// The most tiles an area has across, as the game allows: the route search's tables are this size.
pub const MAX_X_TILES: i32 = 32;
/// The most tiles an area has up.
pub const MAX_Y_TILES: i32 = 32;

/// A planning step ran out of time: the request goes on in the next call.
pub const ACTION_IN_PROGRESS: i32 = 1;
/// A planning step found its path.
pub const ACTION_COMPLETE: i32 = 2;
/// A planning step found no path.
pub const ACTION_FAILED: i32 = 3;

/// Turns a point in a tile's canonical frame (as its model is made) into the placed tile's real frame, tile-local, for placement
/// orientation `o` (quarter turns counter-clockwise).
pub fn rot_c2r(o: u8, x: f32, y: f32) -> (f32, f32) {
    match o {
        0 => (x, y),
        2 => (10.0 - x, 10.0 - y),
        3 => (y, 10.0 - x),
        _ => (10.0 - y, x),
    }
}

/// The inverse of `rot_c2r`, tile-local.
pub fn rot_r2c(o: u8, x: f32, y: f32) -> (f32, f32) {
    match o {
        0 => (x, y),
        1 => (y, 10.0 - x),
        2 => (10.0 - x, 10.0 - y),
        _ => (10.0 - y, x),
    }
}

/// A path node table for one letter and orientation: the tile's regions (node points), its exit points on the edges and the region
/// owning each exit. A region is referred to by its node index.
#[derive(Debug, Clone)]
pub struct PathNode {
    /// Each region's node point (tile-local, canonical frame).
    pub nodes: Vec<(f32, f32)>,
    /// The exit points on the tile's edges (tile-local, canonical frame).
    pub exits: Vec<(f32, f32)>,
    /// The region (node index) each exit belongs to.
    pub exit_node: Vec<i32>,
}

/// The path node table of every letter and orientation (src/pathnodes.json, see [`crate::pathnode::measured`]); made once.
pub fn path_node_tables() -> &'static HashMap<(u8, u8), PathNode> {
    static TABLES: std::sync::OnceLock<HashMap<(u8, u8), PathNode>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        crate::pathnode::measured()
            .iter()
            .map(|m| {
                let table =
                    PathNode { nodes: m.nodes.clone(), exits: m.exits.clone(), exit_node: m.exit_node.iter().map(|&n| n as i32).collect() };
                ((m.letter as u8, m.orientation), table)
            })
            .collect()
    })
}

/// One generated tile, as the generator describes it.
pub struct TileInput {
    /// Tile-centred walkmesh vertices (-5..5), as written to the .wok.
    pub verts: Vec<[f32; 3]>,
    /// Faces with their surfacemat Walk flag.
    pub faces: Vec<([u32; 3], bool)>,
    /// The path node letter of the tile's layout (`pathnodes.json`).
    pub letter: char,
    /// PathNode orientation index 0..3 (.set degrees / 90).
    pub pn_orient: u8,
    /// Area Tile_Orientation 0..3.
    pub orient: u8,
    /// Tile_Height.
    pub level: i32,
}

/// A placed tile.
pub struct Tile {
    /// The tile's column, from the west.
    pub x: i32,
    /// The tile's row, from the south.
    pub y: i32,
    /// Placement orientation, quarter turns.
    pub o: u8,
    /// Tile z offset: Tile_Height * Transition.
    pub z: f32,
    /// The tile's walkmesh, in its canonical frame.
    pub mesh: Mesh,
}

/// What the planner reads of an area.
pub struct Area {
    /// Tiles across.
    pub width: i32,
    /// Tiles up.
    pub height: i32,
    /// The tiles, row-major from the south-west.
    pub tiles: Vec<Tile>,
    /// The path node tables, by letter and orientation.
    pub tables: &'static HashMap<(u8, u8), PathNode>,
}

impl Area {
    /// `tiles` row-major from the south-west; `transition` = .set Transition.
    pub fn new(width: i32, height: i32, tiles: Vec<TileInput>, transition: f32) -> Area {
        let tables = path_node_tables();
        let tiles = tiles
            .into_iter()
            .enumerate()
            .map(|(i, t)| Tile {
                x: i as i32 % width,
                y: i as i32 / width,
                o: t.orient,
                z: t.level as f32 * transition,
                mesh: Mesh::load(t.letter as u8, t.pn_orient, &t.verts, &t.faces),
            })
            .collect();
        Area { width, height, tiles, tables }
    }

    /// The tile at column `x`, row `y`; None outside the area.
    pub fn tile(&self, x: i32, y: i32) -> Option<&Tile> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        self.tiles.get((y * self.width + x) as usize)
    }

    /// Whether a world point lies on a walkable triangle.
    pub fn on_walkmesh(&self, x: f32, y: f32) -> bool {
        let Some(t) = self.tile((x / TILE_SIZE) as i32, (y / TILE_SIZE) as i32) else { return false };
        let (cx, cy) = t.r2c_tile(x, y);
        let (ox, oy) = t.origin();
        t.mesh.find_triangle(0, 0, cx - ox, cy - oy, -1) != -1
    }

    /// The tile's path node table.
    pub fn table(&self, t: &Tile) -> &PathNode {
        &self.tables[&(t.mesh.letter, t.mesh.pn_orient)]
    }
}

impl Tile {
    /// The tile's south-west corner (world).
    pub fn origin(&self) -> (f32, f32) {
        (self.x as f32 * TILE_SIZE, self.y as f32 * TILE_SIZE)
    }

    /// `rot_c2r` for this tile: area coords in, area coords out.
    pub fn c2r_tile(&self, x: f32, y: f32) -> (f32, f32) {
        let (ox, oy) = self.origin();
        let r = rot_c2r(self.o, x - ox, y - oy);
        (ox + r.0, oy + r.1)
    }

    /// `rot_r2c` for this tile: area coords in, area coords out.
    pub fn r2c_tile(&self, x: f32, y: f32) -> (f32, f32) {
        let (ox, oy) = self.origin();
        let r = rot_r2c(self.o, x - ox, y - oy);
        (ox + r.0, oy + r.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_have_exact_bits() {
        // The exact f32 values the tolerances must have.
        assert_eq!(EPSILON.to_bits(), 0x38d1_b717);
        assert_eq!(MAX_TILE_EPS.to_bits(), 0x411f_ff97);
    }

    #[test]
    fn rotations_are_inverse_and_ccw() {
        for o in 0..4u8 {
            for &(x, y) in &[(1.0f32, 2.5f32), (0.0, 7.5), (9.0, 10.0), (5.0, 5.0)] {
                let (rx, ry) = rot_c2r(o, x, y);
                assert_eq!(rot_r2c(o, rx, ry), (x, y), "o={o}");
            }
        }
        // A quarter turn counter-clockwise about (5,5): east edge -> north edge.
        assert_eq!(rot_c2r(1, 10.0, 5.0), (5.0, 10.0));
        assert_eq!(rot_c2r(2, 10.0, 5.0), (0.0, 5.0));
        assert_eq!(rot_c2r(3, 10.0, 5.0), (5.0, 0.0));
        // Same sense as the measured tables: table r1 is table r0 turned once.
        let t = path_node_tables();
        let (b0, b1) = (&t[&(b'B', 0)], &t[&(b'B', 1)]);
        for (i, &(x, y)) in b0.exits.iter().enumerate() {
            let r = rot_c2r(1, x, y);
            assert!((r.0 - b1.exits[i].0).abs() < 1e-5 && (r.1 - b1.exits[i].1).abs() < 1e-5, "exit {i}: {r:?} vs {:?}", b1.exits[i]);
        }
    }
}
