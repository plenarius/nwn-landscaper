// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Baked objects as shadow casters: their own triangles in a bounding volume hierarchy, a cut-out texture's alpha letting light
//! through, so a palm frond casts a frond and a trunk a trunk. (They were once a height field of their tops: every crown a column
//! filled down to the ground, every leaf card a solid rectangle.)

use std::collections::HashMap;

use mg_core::ResType;

use crate::objects::Placed;

/// The baked objects' triangles, for rays.
pub struct Occluders {
    tris: Vec<Tri>,
    nodes: Vec<Node>,
    masks: Vec<Mask>,
}

/// A triangle: a corner, its two edges from it, the same for its texture coordinates, and its cut-out (`NONE`: solid).
#[derive(Clone, Copy)]
struct Tri {
    a: [f32; 3],
    e1: [f32; 3],
    e2: [f32; 3],
    uv: [f32; 2],
    du1: [f32; 2],
    du2: [f32; 2],
    mask: u32,
}

const NONE: u32 = u32::MAX;

/// A box of the hierarchy: a leaf (`count` triangles from `start`) or two children (`count` 0, the first at `start`, the second
/// after it).
struct Node {
    lo: [f32; 3],
    hi: [f32; 3],
    start: u32,
    count: u32,
}

/// A cut-out texture's opacity, rows from the bottom (v up, as the game stores them).
struct Mask {
    w: usize,
    h: usize,
    a: Vec<u8>,
}

impl Mask {
    /// Opacity at (u, v), bilinear, repeating as the game samples it.
    fn at(&self, u: f32, v: f32) -> f32 {
        let (fx, fy) = (u.rem_euclid(1.0) * self.w as f32 - 0.5, v.rem_euclid(1.0) * self.h as f32 - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (w, h) = (self.w as i64, self.h as i64);
        let px = |x: i64, y: i64| f32::from(self.a[(y.rem_euclid(h) * w + x.rem_euclid(w)) as usize]);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = px(x0, y0) * (1.0 - tx) + px(x0 + 1, y0) * tx;
        let bottom = px(x0, y0 + 1) * (1.0 - tx) + px(x0 + 1, y0 + 1) * tx;
        // (Nearly clear or nearly solid counts as clear or solid: a ray's answer is then often exactly 0 or 1, and the texel needs
        // no more rays.)
        let a = (top * (1.0 - ty) + bottom * ty) / 255.0;
        if a < 0.02 {
            0.0
        } else if a > 0.98 {
            1.0
        } else {
            a
        }
    }
}

/// How a texture stops light: not at all (additive glows), wholly, or by its alpha.
enum Stops {
    Nothing,
    All,
    Alpha(Mask),
}

/// What texture `name` does to light, as the game draws it: an environment map's alpha is its reflection mask, not transparency;
/// punch-through cuts out at half alpha.
fn stops(game: &mg_rules::GameData, name: &str) -> Stops {
    let txi = game.resman.get_named(name, ResType::TXI).ok().map(|d| mg_image::txi::Txi::parse(&d));
    if let Some(t) = &txi {
        if t.blending() == mg_image::txi::Blending::Additive {
            return Stops::Nothing;
        }
        if t.envmap().is_some() {
            return Stops::All;
        }
    }
    let image = crate::models::import_texture(name).or_else(|| nwn_bake::texture::image(game, name).map(|(t, d)| (t, d.into_owned())));
    let Some(tex) = image.and_then(|(t, d)| mg_image::read(t, &d).ok()) else { return Stops::All };
    if !tex.has_alpha {
        return Stops::All;
    }
    let rgba = tex.to_rgba();
    let punch = txi.as_ref().is_some_and(|t| t.blending() == mg_image::txi::Blending::Punchthrough);
    let a: Vec<u8> = rgba.data.as_chunks::<4>().0.iter().map(|p| if punch { if p[3] >= 128 { 255 } else { 0 } } else { p[3] }).collect();
    if a.iter().all(|&v| v >= 250) {
        return Stops::All;
    }
    Stops::Alpha(Mask { w: rgba.width as usize, h: rgba.height as usize, a })
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Occluders {
    /// The baked ones of `objects`. None when there are none.
    pub fn build(objects: &[Placed]) -> Option<Occluders> {
        let game = crate::models::game().ok();
        let mut by_name: HashMap<String, Option<u32>> = HashMap::new();
        let mut masks = Vec::new();
        let mut tris = Vec::new();
        for p in objects.iter().filter(|p| p.baked) {
            for (texture, first, count, _) in &p.groups {
                let mask = match by_name.get(texture) {
                    Some(m) => *m,
                    None => {
                        let m = match game.map_or(Stops::All, |g| stops(g, texture)) {
                            Stops::Nothing => None,
                            Stops::All => Some(NONE),
                            Stops::Alpha(m) => {
                                masks.push(m);
                                Some(masks.len() as u32 - 1)
                            }
                        };
                        by_name.insert(texture.clone(), m);
                        m
                    }
                };
                let Some(mask) = mask else { continue };
                for t in *first..first + count {
                    let (p0, p1, p2) = (p.positions[3 * t], p.positions[3 * t + 1], p.positions[3 * t + 2]);
                    let (u0, u1, u2) = (p.uvs[3 * t], p.uvs[3 * t + 1], p.uvs[3 * t + 2]);
                    let (e1, e2) = (sub(p1, p0), sub(p2, p0));
                    if dot(cross(e1, e2), cross(e1, e2)) < 1e-12 {
                        continue;
                    }
                    tris.push(Tri {
                        a: p0,
                        e1,
                        e2,
                        uv: u0,
                        du1: [u1[0] - u0[0], u1[1] - u0[1]],
                        du2: [u2[0] - u0[0], u2[1] - u0[1]],
                        mask,
                    });
                }
            }
        }
        if tris.is_empty() {
            return None;
        }
        let mut o = Occluders { tris, nodes: Vec::new(), masks };
        o.nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], start: 0, count: 0 });
        let n = o.tris.len();
        o.split(0, 0, n, 0);
        Some(o)
    }

    fn bounds(&self, from: usize, to: usize) -> ([f32; 3], [f32; 3]) {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for t in &self.tris[from..to] {
            for p in [t.a, [t.a[0] + t.e1[0], t.a[1] + t.e1[1], t.a[2] + t.e1[2]], [t.a[0] + t.e2[0], t.a[1] + t.e2[1], t.a[2] + t.e2[2]]] {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        (lo, hi)
    }

    /// Makes node `at` the box of triangles `from..to`, split where the two halves' boxes cost the fewest ray tests (binned
    /// surface area heuristic) until four or fewer. `depth`: its parents (deep down, the middle: [`Occluders::through`]'s stack
    /// holds 64).
    fn split(&mut self, at: usize, from: usize, to: usize, depth: usize) {
        let (lo, hi) = self.bounds(from, to);
        let leaf = |o: &mut Occluders| o.nodes[at] = Node { lo, hi, start: from as u32, count: (to - from) as u32 };
        if to - from <= 4 {
            return leaf(self);
        }
        let centre = |t: &Tri, k: usize| t.a[k] + (t.e1[k] + t.e2[k]) / 3.0;
        let (mut clo, mut chi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for t in &self.tris[from..to] {
            for k in 0..3 {
                clo[k] = clo[k].min(centre(t, k));
                chi[k] = chi[k].max(centre(t, k));
            }
        }
        const BINS: usize = 12;
        let area = |lo: [f32; 3], hi: [f32; 3]| {
            let d = [0, 1, 2].map(|k| (hi[k] - lo[k]).max(0.0));
            d[0] * d[1] + d[1] * d[2] + d[2] * d[0]
        };
        // (axis, bins below the cut, cost)
        let mut best: Option<(usize, usize, f32)> = None;
        for axis in 0..3 {
            let extent = chi[axis] - clo[axis];
            if extent <= 1e-6 {
                continue;
            }
            let mut bins = [([f32::MAX; 3], [f32::MIN; 3], 0usize); BINS];
            for t in &self.tris[from..to] {
                let b = (((centre(t, axis) - clo[axis]) / extent * BINS as f32) as usize).min(BINS - 1);
                let e = &mut bins[b];
                for p in [t.a, [0, 1, 2].map(|k| t.a[k] + t.e1[k]), [0, 1, 2].map(|k| t.a[k] + t.e2[k])] {
                    e.0 = [0, 1, 2].map(|k| e.0[k].min(p[k]));
                    e.1 = [0, 1, 2].map(|k| e.1[k].max(p[k]));
                }
                e.2 += 1;
            }
            // Costs of each cut from below and from above.
            let mut below = [0.0f32; BINS];
            let (mut l, mut h, mut c) = ([f32::MAX; 3], [f32::MIN; 3], 0);
            for b in 0..BINS - 1 {
                for k in 0..3 {
                    l[k] = l[k].min(bins[b].0[k]);
                    h[k] = h[k].max(bins[b].1[k]);
                }
                c += bins[b].2;
                below[b] = if c == 0 { 0.0 } else { area(l, h) * c as f32 };
            }
            let (mut l, mut h, mut c) = ([f32::MAX; 3], [f32::MIN; 3], 0);
            for b in (1..BINS).rev() {
                for k in 0..3 {
                    l[k] = l[k].min(bins[b].0[k]);
                    h[k] = h[k].max(bins[b].1[k]);
                }
                c += bins[b].2;
                let cost = below[b - 1] + if c == 0 { 0.0 } else { area(l, h) * c as f32 };
                if best.is_none_or(|(_, _, bc)| cost < bc) {
                    best = Some((axis, b, cost));
                }
            }
        }
        let mid = match best.filter(|_| depth < 40) {
            Some((axis, b, _)) => {
                let extent = chi[axis] - clo[axis];
                let goes_below = |t: &Tri| ((((centre(t, axis) - clo[axis]) / extent * BINS as f32) as usize).min(BINS - 1)) < b;
                let mut m = from;
                for i in from..to {
                    if goes_below(&self.tris[i]) {
                        self.tris.swap(i, m);
                        m += 1;
                    }
                }
                m
            }
            None => from,
        };
        // (Everything on one side, all at one point, or deep: split at the middle triangle along the longest side.)
        let mid = if mid == from || mid == to {
            let axis = (0..3).max_by(|&a, &b| (chi[a] - clo[a]).total_cmp(&(chi[b] - clo[b]))).unwrap_or(0);
            let mid = (from + to) / 2;
            self.tris[from..to].select_nth_unstable_by(mid - from, |a, b| centre(a, axis).total_cmp(&centre(b, axis)));
            mid
        } else {
            mid
        };
        let first = self.nodes.len();
        self.nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], start: 0, count: 0 });
        self.nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], start: 0, count: 0 });
        self.nodes[at] = Node { lo, hi, start: first as u32, count: 0 };
        self.split(first, from, mid, depth + 1);
        self.split(first + 1, mid, to, depth + 1);
    }

    /// Calls `hit(triangle, u, v)` for each triangle a ray from `o` along `d` (unit) crosses before `far`, in no order, until it
    /// returns true.
    fn hits(&self, o: [f32; 3], d: [f32; 3], far: f32, mut hit: impl FnMut(&Tri, f32, f32) -> bool) {
        let inv = d.map(|v| if v.abs() < 1e-9 { 1e9_f32.copysign(v) } else { 1.0 / v });
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            // The box: entered before `far` and left after the start?
            let (mut t0, mut t1) = (0.0f32, far);
            for k in 0..3 {
                let (a, b) = ((n.lo[k] - o[k]) * inv[k], (n.hi[k] - o[k]) * inv[k]);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
            if t0 > t1 {
                continue;
            }
            if n.count == 0 {
                stack[sp] = n.start;
                stack[sp + 1] = n.start + 1;
                sp += 2;
                continue;
            }
            for t in &self.tris[n.start as usize..(n.start + n.count) as usize] {
                // Möller-Trumbore, both sides.
                let p = cross(d, t.e2);
                let det = dot(t.e1, p);
                if det.abs() < 1e-10 {
                    continue;
                }
                let s = sub(o, t.a);
                let u = dot(s, p) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = cross(s, t.e1);
                let v = dot(d, q) / det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let dist = dot(t.e2, q) / det;
                if dist <= 1e-3 || dist >= far {
                    continue;
                }
                if hit(t, u, v) {
                    return;
                }
            }
        }
    }

    /// How much light gets from `o` along `d` (unit) to `far` metres: 1 all, 0 none (each surface crossed takes its opacity).
    pub fn through(&self, o: [f32; 3], d: [f32; 3], far: f32) -> f32 {
        let mut left = 1.0f32;
        self.hits(o, d, far, |t, u, v| {
            let opacity = match t.mask {
                NONE => 1.0,
                m => self.masks[m as usize].at(t.uv[0] + u * t.du1[0] + v * t.du2[0], t.uv[1] + u * t.du1[1] + v * t.du2[1]),
            };
            left *= 1.0 - opacity;
            left < 0.004
        });
        if left < 0.004 { 0.0 } else { left }
    }

    /// Whether a ray from `o` along `d` (unit) meets a solid surface (not a cut-out) before `far` metres.
    fn blocked(&self, o: [f32; 3], d: [f32; 3], far: f32) -> bool {
        let mut solid = false;
        self.hits(o, d, far, |t, _, _| {
            solid = t.mask == NONE;
            solid
        });
        solid
    }

    /// Whether the ground at `o` is shut in by solid surfaces: under one and walled all round, so no one sees it (the ground under
    /// an imported cliff's top, or inside a boulder). Ground under a tree or an overhang is open to the side.
    pub fn shut_in(&self, o: [f32; 3]) -> bool {
        const FAR: f32 = 40.0;
        let o = [o[0], o[1], o[2] + 0.05];
        // (Eight ways round, a little upwards: the ground itself doesn't count.)
        let (rise, flat) = (0.34f32.sin(), 0.34f32.cos());
        self.blocked(o, [0.0, 0.0, 1.0], FAR)
            && (0..8).all(|k| {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                self.blocked(o, [flat * a.cos(), flat * a.sin(), rise], FAR)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(positions: Vec<[f32; 3]>) -> Placed {
        let n = positions.len();
        Placed {
            uvs: vec![[0.0, 0.0]; n],
            groups: vec![("solid".into(), 0, n / 3, 0)],
            footprint: Vec::new(),
            min: [0.0; 3],
            max: [0.0; 3],
            baked: true,
            what: String::new(),
            positions,
        }
    }

    #[test]
    fn a_ray_is_stopped_by_a_triangle_in_its_way_and_not_by_one_beside_it_or_behind_it() {
        // A 1 m square at height 5 over (0..1, 0..1), and many small ones far away, so the hierarchy has levels.
        let mut ps = vec![[0.0, 0.0, 5.0], [1.0, 0.0, 5.0], [1.0, 1.0, 5.0], [0.0, 0.0, 5.0], [1.0, 1.0, 5.0], [0.0, 1.0, 5.0]];
        for k in 0..200 {
            let x = 20.0 + k as f32;
            ps.extend([[x, 0.0, 1.0], [x + 0.5, 0.0, 1.0], [x, 0.5, 1.0]]);
        }
        let o = Occluders::build(&[placed(ps)]).expect("triangles");
        let up = [0.0, 0.0, 1.0];
        assert_eq!(o.through([0.5, 0.5, 0.0], up, 100.0), 0.0);
        assert_eq!(o.through([1.5, 0.5, 0.0], up, 100.0), 1.0);
        // Not as far as it, or starting above it.
        assert_eq!(o.through([0.5, 0.5, 0.0], up, 4.0), 1.0);
        assert_eq!(o.through([0.5, 0.5, 6.0], up, 100.0), 1.0);
        assert_eq!(o.through([20.1, 0.1, 0.0], up, 100.0), 0.0);
    }

    #[test]
    fn ground_in_a_closed_box_is_shut_in_and_ground_beside_it_or_under_a_roof_alone_is_not() {
        // A box over (0..4, 0..4) from 0 to 3 m, and a roof alone over (10..14, 0..4) at 3 m.
        let quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| [a, b, c, a, c, d];
        let mut ps = Vec::new();
        ps.extend(quad([0.0, 0.0, 3.0], [4.0, 0.0, 3.0], [4.0, 4.0, 3.0], [0.0, 4.0, 3.0]));
        for (a, b) in [([0.0, 0.0], [4.0, 0.0]), ([4.0, 0.0], [4.0, 4.0]), ([4.0, 4.0], [0.0, 4.0]), ([0.0, 4.0], [0.0, 0.0])] {
            ps.extend(quad([a[0], a[1], 0.0], [b[0], b[1], 0.0], [b[0], b[1], 3.0], [a[0], a[1], 3.0]));
        }
        ps.extend(quad([10.0, 0.0, 3.0], [14.0, 0.0, 3.0], [14.0, 4.0, 3.0], [10.0, 4.0, 3.0]));
        let o = Occluders::build(&[placed(ps)]).expect("triangles");
        assert!(o.shut_in([2.0, 2.0, 0.0]));
        assert!(!o.shut_in([6.0, 2.0, 0.0]));
        assert!(!o.shut_in([12.0, 2.0, 0.0]));
    }

    #[test]
    fn a_cut_out_lets_light_through_where_its_alpha_is_clear() {
        let m = Mask { w: 2, h: 1, a: vec![0, 255] };
        // Texel centres: clear at u 0.25, opaque at u 0.75, half way between.
        assert!(m.at(0.25, 0.5) < 0.01 && m.at(0.75, 0.5) > 0.99 && (m.at(0.5, 0.5) - 0.5).abs() < 0.01);
    }
}
