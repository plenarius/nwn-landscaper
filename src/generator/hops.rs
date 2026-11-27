// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Routes too long for one click. NWN's tile route search gives up on a way about 20 tiles longer than the straight count of
//! tiles between its ends (a long descent's switchbacks, say), so the walker stops a few metres out, though shorter clicks
//! along the way get there. This finds out which a failed route is: clicks along its shortest walk, each a route the
//! simulator completes, or no way through.

use super::TILE;
use crate::{oracle, seams, sim};

/// How far along the walk each click is tried (m), farthest first.
const TRIES: [f32; 7] = [240.0, 200.0, 160.0, 120.0, 80.0, 40.0, 20.0];
/// Past this many clicks the route counts as having no way through.
const MAX_HOPS: usize = 24;

/// The samples of the shortest walk over `ground` from sample `from` to sample `to` (four neighbours, `from` first), None when
/// `ground` doesn't join them.
pub(super) fn walk(ground: &oracle::Reach, from: usize, to: usize) -> Option<Vec<usize>> {
    let (w, h) = (ground.w, ground.h);
    let joined = |k: usize| ground.comp[k] >= 0 && ground.comp[k] == ground.comp[to];
    if !joined(from) {
        return None;
    }
    let next = |k: usize| {
        let (i, j) = (k % w, k / w);
        [(i > 0).then(|| k - 1), (i + 1 < w).then(|| k + 1), (j > 0).then(|| k - w), (j + 1 < h).then(|| k + w)].into_iter().flatten()
    };
    // Breadth first from the goal, so each sample's step towards it is to a neighbour one nearer.
    let mut dist = vec![u32::MAX; w * h];
    dist[to] = 0;
    let mut queue = std::collections::VecDeque::from([to]);
    while let Some(k) = queue.pop_front() {
        if k == from {
            break;
        }
        for q in next(k) {
            if dist[q] == u32::MAX && joined(q) {
                dist[q] = dist[k] + 1;
                queue.push_back(q);
            }
        }
    }
    let mut path = vec![from];
    let mut k = from;
    while k != to {
        k = next(k).find(|&q| dist[q] < dist[k])?;
        path.push(k);
    }
    Some(path)
}

/// How many clicks get from `from` to `to` (area metres), each a route the simulator completes, None when none found does. The
/// clicks follow the shortest walk over `ground`, each as far along it as one click gets ([`TRIES`]); the points clicked are on
/// the walkmesh's part that holds `to` and away from tile edges, as the route check's own ends are.
pub(super) fn hops(
    area: &sim::Area,
    ground: &oracle::Reach,
    conn: &seams::Connectivity,
    from: (f32, f32),
    to: (f32, f32),
) -> Option<usize> {
    let sample = |(x, y): (f32, f32)| {
        let (i, j) = ((x / ground.step).round() as usize, (y / ground.step).round() as usize);
        (i < ground.w && j < ground.h).then(|| j * ground.w + i)
    };
    let path = walk(ground, sample(from)?, sample(to)?)?;
    let goal = conn.component(to.0, to.1)?;
    // The point `k` samples along the walk.
    let point = |k: usize| ((path[k] % ground.w) as f32 * ground.step, (path[k] / ground.w) as f32 * ground.step);
    let off_edge = |c: f32| (0.75..TILE - 0.75).contains(&(c % TILE));
    // Points a click can aim at: off tile edges and on the goal's walkmesh.
    let aim = |k: usize| {
        let (x, y) = point(k);
        off_edge(x) && off_edge(y) && conn.component(x, y) == Some(goal)
    };
    let arrives = |a: (f32, f32), b: (f32, f32)| {
        let r = sim::area::simulate(area, &sim::Query::new(a, b), false);
        ((r.end.0 - b.0).powi(2) + (r.end.1 - b.1).powi(2)).sqrt() < 1.5
    };
    let last = path.len() - 1;
    let (mut at, mut here, mut clicks) = (0usize, from, 0usize);
    while at < last {
        if clicks == MAX_HOPS {
            return None;
        }
        let ahead = |m: f32| (at + (m / ground.step) as usize).min(last);
        // The goal itself first when it is within the farthest try, then points along the walk, each moved back to one a
        // click can aim at.
        let mut tries: Vec<usize> = Vec::new();
        if ahead(TRIES[0]) == last {
            tries.push(last);
        }
        for m in TRIES {
            let mut k = ahead(m);
            while k > at && !aim(k) {
                k -= 1;
            }
            if k > at && k != last && !tries.contains(&k) {
                tries.push(k);
            }
        }
        let (k, p) = tries.into_iter().map(|k| (k, if k == last { to } else { point(k) })).find(|&(_, p)| arrives(here, p))?;
        (at, here, clicks) = (k, p, clicks + 1);
    }
    Some(clicks)
}

#[cfg(test)]
mod tests {
    use super::walk;
    use crate::oracle::Reach;

    /// A U: two 10 m halls joined at their east end by a 4 m passage, a wall between them.
    fn u_bend() -> Reach {
        Reach::new(20.0, 10.0, 0.5, 1.0, |_, _| 0.0, |x, y| (3.0..7.0).contains(&y) && x < 16.0)
    }

    #[test]
    fn the_walk_goes_round_a_wall_and_stays_on_the_ground() {
        let r = u_bend();
        let at = |x: f32, y: f32| (y / 0.5) as usize * r.w + (x / 0.5) as usize;
        let path = walk(&r, at(2.0, 1.0), at(2.0, 9.0)).unwrap();
        assert_eq!((path[0], *path.last().unwrap()), (at(2.0, 1.0), at(2.0, 9.0)));
        assert!(path.iter().all(|&k| r.comp[k] >= 0));
        // Round the end of the wall at x 16 and back: well over the 8 m straight across.
        assert!(path.iter().any(|&k| (k % r.w) as f32 * 0.5 >= 16.0));
        assert!(path.len() as f32 * 0.5 > 30.0);
        // Each step is to a neighbour.
        assert!(path.windows(2).all(|s| s[0].abs_diff(s[1]) == 1 || s[0].abs_diff(s[1]) == r.w));
    }

    #[test]
    fn ground_apart_has_no_walk() {
        let r = Reach::new(20.0, 10.0, 0.5, 1.0, |_, _| 0.0, |_, y| (3.0..7.0).contains(&y));
        let at = |x: f32, y: f32| (y / 0.5) as usize * r.w + (x / 0.5) as usize;
        assert!(walk(&r, at(2.0, 1.0), at(2.0, 9.0)).is_none());
        assert_eq!(walk(&r, at(2.0, 1.0), at(12.0, 2.0)).map(|p| p.len() > 1), Some(true));
    }
}
