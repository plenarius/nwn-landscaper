// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Entrances: cave mouths for area transitions. The editor carves each one's notch into the heights (a flat floor into the slope, rock
//! walls and a rim round it) and records it; here it gets what a heightfield can't have: a rock hood over the mouth and a black
//! backdrop at the back (scenery only), its inside darkened in the shadow map, and a door at the back to link: doortypes.2da's
//! RuralCave transition (invisible, as BioWare's rural cave tiles use).
//!
//! An entrance's own frame: u into the slope along `dir`, v across it (to the left of u), from the middle of the mouth on the floor.

use super::{TILE, heightmap, tiles_x, tiles_y};
use crate::heightmap::Entrance;

/// The shape of a natural cave mouth (the editor's js/entrance.js has the same; keep them in step). Heights are over the entrance's
/// floor; u and v in its own frame.
///
/// The hill is drawn on over the cave: the roof's top blends the slope's surface across the opening (a Coons patch of the ground
/// beside and behind it and the lip in front), so the paint runs on to the lip. Its underside is a rough arch. (The editor's carve
/// raises lower ground round a cave to ROOF, 4.5 m over its floor, as a rocky outcrop, so there is a hill to draw on.) Where the lip
/// is (u), on average.
const LIP: f32 = -0.6;
/// How far the roof reaches past the opening's sides (into the walls).
const SIDE: f32 = 0.3;
/// The roof's least thickness.
const THICK: f32 = 0.7;
/// How far out beside and behind the opening the ground is read for the roof.
const READ_OUT: f32 = 1.2;

fn seed(e: &Entrance) -> f32 {
    e.x * 0.37 + e.y * 0.61
}

/// u of the lip's edge at v: ragged.
pub fn lip(e: &Entrance, v: f32) -> f32 {
    let s = seed(e);
    LIP + 0.22 * (2.3 * v + s).sin() + 0.13 * (5.1 * v + 2.0 * s).sin()
}

/// The arch's underside over the floor at (u, v): walls to about 1 m, an
/// arch to about 2.9 m in the middle, a little rough, drooping at the lip.
pub fn under(e: &Entrance, u: f32, v: f32) -> f32 {
    let t = (v / (e.width / 2.0)).clamp(-1.0, 1.0);
    let arch = 1.0 + 1.9 * (1.0 - t * t).max(0.0).sqrt();
    let rough = 0.12 * (3.7 * v + 1.3 * u + seed(e)).sin();
    let droop = 0.25 * ((0.4 - u) / 1.0).clamp(0.0, 1.0);
    arch + rough - droop
}

/// The roof's top over the floor at (u, v), from the ground `height` (area metres to height) round the opening.
pub fn top(e: &Entrance, u: f32, v: f32, height: &dyn Fn(f32, f32) -> f32) -> f32 {
    let vr = e.width / 2.0 + SIDE;
    let (u0, u1) = (LIP, e.depth);
    let s = ((u - u0) / (u1 - u0)).clamp(0.0, 1.0);
    let t = ((v + vr) / (2.0 * vr)).clamp(0.0, 1.0);
    let ground = |u: f32, v: f32| {
        let (x, y) = e.at(u, v);
        height(x, y) - e.floor
    };
    let along = |s: f32| u0 + s * (u1 - u0);
    let across = |t: f32| -vr + t * 2.0 * vr;
    let front = |t: f32| under(e, u0, across(t)) + THICK;
    let back = |t: f32| ground(e.depth + READ_OUT, across(t));
    let left = |s: f32| ground(along(s), -(vr + READ_OUT));
    let right = |s: f32| ground(along(s), vr + READ_OUT);
    let (p00, p01) = ((front(0.0) + left(0.0)) / 2.0, (front(1.0) + right(0.0)) / 2.0);
    let (p10, p11) = ((back(0.0) + left(1.0)) / 2.0, (back(1.0) + right(1.0)) / 2.0);
    let coons = (1.0 - s) * front(t) + s * back(t) + (1.0 - t) * left(s) + t * right(s)
        - ((1.0 - s) * (1.0 - t) * p00 + (1.0 - s) * t * p01 + s * (1.0 - t) * p10 + s * t * p11);
    coons.max(under(e, u, v) + THICK)
}

/// An entrance's scenery, in area metres (x, y, z): the roof's top (the hill's paint), its rock (underside and lip) and the dark back.
pub struct Scenery {
    /// The roof's top, painted as the hill around it.
    pub top: Vec<[[f32; 3]; 3]>,
    /// The rock: the roof's underside and its lip.
    pub rock: Vec<[[f32; 3]; 3]>,
    /// The dark back, which hides that the cave ends there.
    pub back: Vec<[[f32; 3]; 3]>,
}

/// Triangles of a quad a, b, c, d, turned to face `up` (z) or else outward from the cave (-u).
fn quad(out: &mut Vec<[[f32; 3]; 3]>, q: [[f32; 3]; 4], facing: [f32; 3]) {
    for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
        let (a, b, c) = (t[0], t[1], t[2]);
        let n = [
            (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
            (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
        ];
        let dot = n[0] * facing[0] + n[1] * facing[1] + n[2] * facing[2];
        out.push(if dot < 0.0 { [a, c, b] } else { t });
    }
}

/// The scenery of entrance `e` on the ground `height` gives: the roof follows the slope it cuts into.
pub fn scenery(e: &Entrance, height: &dyn Fn(f32, f32) -> f32) -> Scenery {
    let (nu, nv) = (((e.depth - LIP) / 0.5).ceil() as usize, ((e.width + 2.0 * SIDE) / 0.5).ceil() as usize);
    let vr = e.width / 2.0 + SIDE;
    let (a, _) = e.axes();
    let out = [-a[0], -a[1], 0.0];
    let world = |u: f32, v: f32, z: f32| {
        let (x, y) = e.at(u, v);
        [x, y, e.floor + z]
    };
    // The grid: v across, u from the ragged lip to the back.
    let uv_at = |i: usize, j: usize| {
        let v = -vr + j as f32 * 2.0 * vr / nv as f32;
        let l = lip(e, v);
        (l + i as f32 * (e.depth - l) / nu as f32, v)
    };
    let mut sc = Scenery { top: Vec::new(), rock: Vec::new(), back: Vec::new() };
    for i in 0..nu {
        for j in 0..nv {
            let c = [uv_at(i, j), uv_at(i + 1, j), uv_at(i + 1, j + 1), uv_at(i, j + 1)];
            quad(&mut sc.top, c.map(|(u, v)| world(u, v, top(e, u, v, height))), [0.0, 0.0, 1.0]);
            quad(&mut sc.rock, c.map(|(u, v)| world(u, v, under(e, u, v))), [0.0, 0.0, -1.0]);
        }
    }
    // The lip: from the underside up to the top, facing out.
    for j in 0..nv {
        let (u0, v0) = uv_at(0, j);
        let (u1, v1) = uv_at(0, j + 1);
        let q = [
            world(u0, v0, under(e, u0, v0)),
            world(u1, v1, under(e, u1, v1)),
            world(u1, v1, top(e, u1, v1, height)),
            world(u0, v0, top(e, u0, v0, height)),
        ];
        quad(&mut sc.rock, q, out);
    }
    // The dark back: the arch's shape, at the back wall.
    let ub = e.depth - 0.05;
    let hw = e.width / 2.0;
    for j in 0..nv {
        let (v0, v1) = (-hw + j as f32 * e.width / nv as f32, -hw + (j + 1) as f32 * e.width / nv as f32);
        let q = [world(ub, v0, -0.1), world(ub, v1, -0.1), world(ub, v1, under(e, ub, v1)), world(ub, v0, under(e, ub, v0))];
        quad(&mut sc.back, q, out);
    }
    sc
}

/// The doortypes.2da row of the door.
pub const DOOR_TYPE: u32 = 57;
/// The door's blueprint: the stock one that has that row.
pub const DOOR_BLUEPRINT: &str = "nw_door_ttr_12";

impl Entrance {
    /// Unit vectors into the slope and across the mouth (to the left, looking in).
    pub fn axes(&self) -> ([f32; 2], [f32; 2]) {
        let (s, c) = self.dir.sin_cos();
        ([c, s], [-s, c])
    }

    /// Area point of (u, v).
    pub fn at(&self, u: f32, v: f32) -> (f32, f32) {
        let (a, b) = self.axes();
        (self.x + a[0] * u + b[0] * v, self.y + a[1] * u + b[1] * v)
    }

    /// (u, v) of an area point.
    pub fn local(&self, x: f32, y: f32) -> (f32, f32) {
        let (a, b) = self.axes();
        let (dx, dy) = (x - self.x, y - self.y);
        (dx * a[0] + dy * a[1], dx * b[0] + dy * b[1])
    }

    /// The tile the mouth is in (its hood and backdrop go in that tile's model).
    pub fn tile(&self) -> (usize, usize) {
        (((self.x / TILE).max(0.0) as usize).min(tiles_x() - 1), ((self.y / TILE).max(0.0) as usize).min(tiles_y() - 1))
    }

    /// Where its door stands (0.15 m in front of the back wall) and its bearing (the door's width across the notch).
    pub fn door(&self) -> ([f32; 3], f32) {
        let (x, y) = self.at(self.depth - 0.15, 0.0);
        ([x, y, self.floor], self.dir + std::f32::consts::FRAC_PI_2)
    }
}

/// How much darker the shadow map is at (x, y) inside an entrance: the mouth already in shade, the back darker still. 1 outside.
pub fn darkness(x: f32, y: f32) -> f32 {
    let Some(hm) = heightmap::current() else { return 1.0 };
    let mut f = 1.0f32;
    for e in &hm.entrances {
        let (u, v) = e.local(x, y);
        if u < -1.0 || u > e.depth + 1.5 || v.abs() > e.width / 2.0 + 1.5 {
            continue;
        }
        let smooth = |a: f32, b: f32, t: f32| {
            let s = ((t - a) / (b - a)).clamp(0.0, 1.0);
            s * s * (3.0 - 2.0 * s)
        };
        let inside = 1.0 - 0.45 * smooth(-0.8, 0.4, u) - 0.25 * (u / e.depth).clamp(0.0, 1.0);
        f = f.min(inside);
    }
    f
}

/// The local variable the engine puts on each entrance door it makes (an int: the entrance's id), so a later export can tell its
/// own doors from one a builder placed by hand with a tag like it, and take out the door of an entrance the area no longer has.
pub const DOOR_MARK: &str = "ls_entrance";

/// The entrance id a door carries in its [`DOOR_MARK`], if the engine made it.
pub fn door_mark(door: &mg_gff::Struct) -> Option<i64> {
    door.list("VarTable")?.iter().find(|v| v.string("Name") == Some(DOOR_MARK.as_bytes())).and_then(|v| v.integer("Value"))
}

/// The tag of the door of the entrance whose id is `id` (`<area>_ent<id>`): how a re-export finds the doors it placed, and
/// what transitions and scripts name them by. From the entrance's id, not its place in the list, so taking one entrance out
/// leaves the others' tags as they were.
pub fn door_tag(id: u32) -> String {
    format!("{}_ent{id}", crate::generator::tag_prefix())
}

/// The doors of the entrances, made from DOOR_BLUEPRINT as the toolset
/// places doors, as the RuralCave transition, each tagged by door_tag.
pub fn doors(game: &mg_rules::GameData) -> anyhow::Result<Vec<mg_gff::Struct>> {
    use anyhow::Context;
    let Some(hm) = heightmap::current().filter(|h| !h.entrances.is_empty()) else { return Ok(Vec::new()) };
    let key = mg_resman::ResKey { resref: mg_core::ResRef::from_str(DOOR_BLUEPRINT)?, restype: mg_core::ResType::UTD };
    let blueprint = mg_gff::Gff::read(&game.resman.get(&key).with_context(|| format!("no {DOOR_BLUEPRINT}.utd"))?)?;
    let placing = mg_module::instances::Placing { game, item: &|_| None };
    let mut out = Vec::new();
    for e in &hm.entrances {
        let (position, rotation) = e.door();
        let at = mg_module::instances::Placement { position, rotation };
        let mut door =
            mg_module::instances::instance(&placing, mg_core::ResType::UTD, &blueprint.root, at, &[]).context("placing the door")?;
        door.set("Tag", mg_gff::Value::String(door_tag(e.id).into_bytes()));
        crate::things::add_var(&mut door, DOOR_MARK, &serde_json::json!({ "type": "int", "value": e.id }));
        // The doortypes.2da row, in the field's own type.
        let appearance = match door.get("Appearance") {
            Some(mg_gff::Value::Byte(_)) => mg_gff::Value::Byte(DOOR_TYPE as u8),
            Some(mg_gff::Value::Int(_)) => mg_gff::Value::Int(DOOR_TYPE as i32),
            _ => mg_gff::Value::Dword(DOOR_TYPE),
        };
        door.set("Appearance", appearance);
        // Linked to another entrance's door (1: a door).
        if let Some(link) = &e.link {
            door.set("LinkedTo", mg_gff::Value::String(link.clone().into_bytes()));
            door.set("LinkedToFlags", mg_gff::Value::Byte(1));
        }
        out.push(door);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_door_is_tagged_by_its_entrances_id_not_its_place() {
        // Entrances 1 and 3 (2 taken out): the second in the list is still <area>_ent3.
        let list = serde_json::json!([
            { "id": 1, "x": 10, "y": 0, "dir": 0, "width": 4, "depth": 4, "floor": 0 },
            { "id": 3, "x": 20, "y": 0, "dir": 0, "width": 4, "depth": 4, "floor": 0 },
        ]);
        let tags: Vec<String> = heightmap::entrances_of(&list).unwrap().iter().map(|e| door_tag(e.id)).collect();
        assert!(tags[0].ends_with("_ent1") && tags[1].ends_with("_ent3"), "{tags:?}");
    }

    #[test]
    fn frame_round_trips_and_the_door_faces_across() {
        let e = Entrance { id: 1, x: 20.0, y: 30.0, dir: 0.7, width: 4.0, depth: 4.0, floor: 5.0, rock: None, link: None };
        let (x, y) = e.at(2.0, -1.0);
        let (u, v) = e.local(x, y);
        assert!((u - 2.0).abs() < 1e-5 && (v + 1.0).abs() < 1e-5);
        let ([dx, dy, dz], bearing) = e.door();
        let (u, v) = e.local(dx, dy);
        assert!((u - 3.85).abs() < 1e-4 && v.abs() < 1e-4 && dz == 5.0);
        assert!((bearing - 0.7 - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn the_roof_follows_the_hill_and_the_editor_agrees() {
        // A hill rising 1 m a metre east of x = 20; a cave facing into it.
        let e = Entrance { id: 1, x: 20.0, y: 30.0, dir: 0.0, width: 4.0, depth: 4.0, floor: 0.0, rock: None, link: None };
        let h = |x: f32, _y: f32| (x - 20.0).max(0.0);
        let (under_mid, top_mid, top_back) = (under(&e, 2.0, 0.0), top(&e, 2.0, 0.0, &h), top(&e, 3.9, 0.0, &h));
        assert!(top_mid > under_mid + 0.69 && top_back > top_mid);
        // js/entrance.js gives the same: keep them in step.
        assert_eq!(format!("{:.4} {:.4} {:.4} {:.4}", lip(&e, 0.5), under_mid, top_mid, top_back), "-0.4495 2.8969 3.5969 4.5433");
    }
}
