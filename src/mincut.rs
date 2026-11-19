// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimum vertex cut between two sets of faces of a walkmesh: the fewest walkable faces whose removal separates them (max-flow on the
//! face graph, each face split into in/out with capacity 1). Used to break U-shaped regions inside a tile, which the engine's in-tile
//! search cannot cross in time (CNWTileSurfaceMesh::IntraTileDFS explores every path under a bound that starts near the straight-line
//! distance).

use std::collections::{HashMap, VecDeque};

/// `faces`: triangle vertex indices and walkability. Returns the faces to make non-walkable, or None if `sources` and `sinks` are not
/// connected or share a face.
pub fn min_cut(faces: &[([u32; 3], bool)], sources: &[usize], sinks: &[usize]) -> Option<Vec<usize>> {
    if sources.iter().any(|s| sinks.contains(s)) {
        return None;
    }
    let n = faces.len();
    // Nodes: face i in = 2i, out = 2i+1; super source 2n, super sink 2n+1.
    let (src, snk) = (2 * n, 2 * n + 1);
    let mut graph: Vec<Vec<usize>> = vec![Vec::new(); 2 * n + 2];
    let mut to: Vec<usize> = Vec::new();
    let mut cap: Vec<i32> = Vec::new();
    let mut add = |g: &mut Vec<Vec<usize>>, a: usize, b: usize, c: i32| {
        g[a].push(to.len());
        to.push(b);
        cap.push(c);
        g[b].push(to.len());
        to.push(a);
        cap.push(0);
    };
    const INF: i32 = 1 << 20;
    let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, (f, walk)) in faces.iter().enumerate() {
        if !walk {
            continue;
        }
        // Source and sink faces can't be cut.
        let c = if sources.contains(&i) || sinks.contains(&i) { INF } else { 1 };
        add(&mut graph, 2 * i, 2 * i + 1, c);
        for k in 0..3 {
            let (a, b) = (f[k].min(f[(k + 1) % 3]), f[k].max(f[(k + 1) % 3]));
            edges.entry((a, b)).or_default().push(i);
        }
    }
    for fs in edges.values() {
        if let [a, b] = fs[..] {
            add(&mut graph, 2 * a + 1, 2 * b, INF);
            add(&mut graph, 2 * b + 1, 2 * a, INF);
        }
    }
    for &s in sources {
        add(&mut graph, src, 2 * s, INF);
    }
    for &t in sinks {
        add(&mut graph, 2 * t + 1, snk, INF);
    }
    // Edmonds-Karp.
    let mut flow = 0;
    loop {
        let mut prev: Vec<Option<usize>> = vec![None; 2 * n + 2];
        let mut q = VecDeque::from([src]);
        let mut seen = vec![false; 2 * n + 2];
        seen[src] = true;
        while let Some(u) = q.pop_front() {
            for &e in &graph[u] {
                if cap[e] > 0 && !seen[to[e]] {
                    seen[to[e]] = true;
                    prev[to[e]] = Some(e);
                    q.push_back(to[e]);
                }
            }
        }
        if !seen[snk] {
            // Cut: faces whose in-node is reachable and out-node is not.
            if flow == 0 || flow >= INF {
                return None;
            }
            let cut = (0..n).filter(|&i| faces[i].1 && seen[2 * i] && !seen[2 * i + 1]).collect();
            return Some(cut);
        }
        let mut v = snk;
        let mut push = INF;
        while let Some(e) = prev[v] {
            push = push.min(cap[e]);
            v = to[e ^ 1];
        }
        let mut v = snk;
        while let Some(e) = prev[v] {
            cap[e] -= push;
            cap[e ^ 1] += push;
            v = to[e ^ 1];
        }
        flow += push;
        if flow >= INF {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip of quads (two triangles each) joined end to end: cutting it takes the two triangles of one quad, or fewer.
    #[test]
    fn a_strip_is_cut_across_at_most_one_quad() {
        // Vertices along two rows: top i*2, bottom i*2+1, i = 0..5.
        let mut faces = Vec::new();
        for i in 0..5u32 {
            let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            faces.push(([a, b, c], true));
            faces.push(([b, d, c], true));
        }
        let cut = min_cut(&faces, &[0], &[9]).expect("connected");
        assert!(!cut.is_empty() && cut.len() <= 2, "{cut:?}");
        assert!(!cut.contains(&0) && !cut.contains(&9));
    }

    #[test]
    fn disconnected_faces_have_no_cut() {
        let faces = vec![([0, 1, 2], true), ([3, 4, 5], true)];
        assert!(min_cut(&faces, &[0], &[1]).is_none());
    }
}
