# Landscaper: how the game server routes creatures (findings, 2026-10-19)

All observed on a real NWN:EE server (nwserver-linux 8193.37-17), running test areas with Moonglow's `mg test` and recording
where creatures walk. Every rule below is behaviour seen from outside: what the server accepts, where it crashes, where a
creature goes.

## Works

- Generated tiles (mdl + wok + .set, all unique per area) load and render paths; creatures walk across tile seams, follow height, climb
  ramps, are blocked by non-walk (surfacemat 7) faces.
- Flat terrain with a non-walk strip: creatures route around it, with PathNodes chosen automatically by pathnode.rs.
- Cliffs up to ~3.5 m: routing works in most cases.

## Server rules learned

- `.set` must be CRLF: with LF-only line endings the server crashes while it reads the file.
- `.set` tile `Orientation` is in DEGREES (0/90/180/270); layout rotation index = degrees / 90. Writing 1/2/3 crashes pathing.
- PathNode letter = built-in layout (src/pathnodes.json): nodes, each with exit points at 1/2.5/5/7.5/9 m along the tile edges. Frame:
  x east, y north, 0..10.
- Each layout node must map to a distinct walkable region of the walkmesh, or the server crashes when it looks up the region of a point.
- An exit only links to the neighbour tile if the neighbour's layout has the same edge point.
- `validate-pathnodes`: BioWare's own tiles are 74-84% consistent with this model (exits snapped up to ~1.25 m); the rest look like loose
  hand assignments.
- Scripted moves for creatures with no player in the area run at a low AI level and only walk straight; tests must SetAILevel(o, AI_LEVEL_HIGH).
- A straight line between two points comes out one of four ways: clear, blocked by a tile's walkmesh, blocked by the detailed
  non-walk polygons of placeables and doors, or blocked by objects; the server routes differently for each.

## How the server routes (measured on oracle runs)

- Only walkable faces (surfacemat.2da `Walk`) count: they alone decide which faces touch, which region a point is in, and which
  face a point stands on.
- Two faces touch only if they share vertex indices; an edge on the tile border leads to the next tile. A T-junction (a vertex
  in the middle of a neighbour's edge) breaks the connection.
- The region of a point: if the layout has one node, region 0. Otherwise the region of the first node reached by walking from
  the point's face across touching faces; if no node is reached, the nearest node by distance.
- Exits and regions between tiles come only from the layout table: a crossing has to match one of its exit points almost exactly.
- Tile to tile: an iterative-deepening depth-first search over (tile x, tile y, region), at most 6 regions a tile; the route is
  kept as tiles and exit coordinates.
- Inside a tile: an iterative-deepening search over triangles whose bound grows by 5 m a round and that gives up past 40 m (four
  tile widths). It runs in time slices of at least 500 us. On dense walkmeshes it never finishes: the request stays "in
  progress" and the creature walks straight.
- The crossing point on a tile edge: the server aims at the route's next exit (turned with the tile's `Tile_Orientation`) and
  tries candidate points around it, testing for each a straight line across the tile's walkmesh, past non-walk polygons and past
  creatures.
- BioWare walkmeshes: median ~45 faces a tile, p90 ~78, max ~250 (wok-stats).

## Generator state

- Walkmesh = RTIN (src/rtin.rs) over 9x9 samples, split on height error > 0.25 m or walkability change. Pathing results improve sharply with
  fewer faces.
- With RTIN 9x9: 3.0 m and 6.0 m cliffs route on all three test walks; 4.0 m fails (the route crosses back into the cliff tile on the wrong
  side of the cliff).

## Simulator (src/sim, 2026-10-20)

Our own reimplementation of how the server plans an NPC's walk inside an area, written from the behaviour above and fitted
to the oracle runs below until it reproduced them: the straight-line test, the route search, the short-range grid fallback and
where a new creature is placed.

- `landscaper sim <x0> <y0> <x1> <y1>`: one request with a trace (spawn placement, tile routes, each
  tile's goal box and result, denials, line tests, outcome, waypoints).
- `landscaper sim-check <oracle.json>...`: regenerates the area (`cliff<h>-` / `flat-` in the file name
  set LS_CLIFF / LS_FLAT) and compares arrival and end point with the server for every pair. An oracle
  run comes from `landscaper oracle --out <dir> [N] [SEED]` (same LS_CLIFF / LS_FLAT / LS_RUGGED): it
  generates the test area with a script that walks a creature between N random pairs, runs it on a real
  server through Moonglow's `mg test` (MG names it), and writes `<dir>/results.json`; rename it after
  its variant (`cliff4.0-seed31.json`). The corpora used below are not in this repository.
- `landscaper sim-line <x0> <y0> <x1> <y1>`: walkmesh coverage along a segment ('#' walkable).
- Agreement on the corpus as it was on 2026-10-20 (13 runs of 40 pairs, later called v1): arrival
  520/520, end point within 0.5 m 519/520 (the one miss is a partial end of the grid fallback, 3 m short along the
  cliff foot; arrival is the same). `sim-check` regenerates the area with the code of the day, so a run
  recorded against an older generator stops matching when the generator changes. On 2026-11-11 the 25 runs
  gave: v1 (16 runs, the first generator) 546/640 arrivals; v2 (5 runs) 198/200; v3 (4 runs, the current
  generator) 160/160 arrivals and 159/160 end points within 0.5 m, every server failure reproduced.
- What the oracle's walks ask for (fitted to the runs): a creature radius of 0.4 m (so every in-tile line
  test uses the 0.1 m clamp), height 1.0, range 0, a budget of 1000 us a call and 100 calls.
  `CreateObject` moves a creature that starts within ~0.4 m of non-walkable ground (two corpus pairs);
  the simulator's spawn step does the same.
- Time model: ~0.7 us per line test (fitted to the server's timings). A request whose route search needs more
  than ~145k line tests runs out of calls and takes the partial straight line. Fine-mode (start
  within 15 m) expansions cost ~19 line tests each; coarse ones almost none.
- Within 20 m, a failed route search falls back to a greedy 0.1 m grid walk toward the goal with at
  most ~3.6 m of detour, which on our cliff stops at the cliff foot (the "stopped at x = 22.3" ends).

## Why the cliff-4.0 routes fail (all 50 failures of seeds 1-5, simulator and server agree)

Every failure dies in the ramp tile (2,0) (layout I3: one node, exits W (0,5), N (5,10), E (10,5)).
The ramp's walkable band across the x = 20 seam is y 5..7.75 on tile (2,0)'s side, but y 5..10 on
tile (1,0)'s side: the seam samples are shared, but each side's faces also use that tile's interior
samples, and the ramp's north flank is steeper east of x = 20, so the faces touching the seam are
walkable on (1,0)'s side and not on (2,0)'s (`landscaper sim-line 19.99 0 19.99 10` vs `20.01`).

1. Eastbound (41/50): the route enters (2,0) from (1,0) at exit (10,5). (1,0)'s east edge has one exit,
   so the crossing point's candidates are y = 1, 3, 5, 7, 9 (exit +/-4 m). The route arrives in
   (1,0) from the north (y = 10), so candidate 9 gets the +1 "nearest the entry" weight, and its
   straight line is clear *on tile (1,0)*: it wins. Tile (2,0) then starts at (0,9), off its walkmesh;
   its search fails at once, the server gives up on crossing (2,0) W->E, there is no other way onto the plateau, and
   the request ends in the partial straight line (or, within 20 m, the grid fallback) to the cliff foot.
   With a 3.0 m cliff (0,9) is walkable on both sides; with 6.0 m it is not walkable on (1,0)'s side
   either, so candidate 9's line is blocked there and 7 (in the band) wins: both succeed.
2. Westbound (9/50): from the plateau no west-edge candidate has a clear straight line, so they are
   scored by free space next to the edge only; y = 1 wins, in the low ground south of the ramp, a
   walkable component of tile (2,0) not connected to the plateau. The in-tile IDA* then exhausts every
   bound (up to 2.6M work units, 80k line tests, 55 of the 100 calls) and fails; the denial again
   leaves no route.
The common cause: the server picks the crossing point on a tile edge from the CURRENT tile's walkmesh
only, anywhere within the candidate spread, and one failed crossing removes that exit for the request.

What the generator should do (not implemented here):

- Make seam walkability agree: ground at a point of a tile edge should be walkable on one side iff it
  is on the other (classify the faces touching a seam from both sides, e.g. walkable only if both
  sides' faces are).
- For every exit a route can use, keep the whole candidate spread walkable on both sides and in the
  exit's own walkable component: exit +/-4 m on an edge with one exit, +/-2 m with two, +/-2.4 m
  (middle) / +/-0.8 m (sides) with three. Or pick layouts with more exits on such edges, so the
  spread is narrower.
- Give each walkable component that touches an edge within a candidate spread its own node/exit, or
  keep it out of the spread (here: the low ground under the ramp next to exit (0,5)).
- Avoid single points of failure: one denied crossing kills every request that needs it.
- Run `sim-check`-style random pairs through the simulator for every generated area before export.

## Open

- The grid fallback's end point depends on its time slices; one corpus pair ends 3 m off (arrival agrees).
- The time model's constants are fitted to three timed requests; only heavy searches depend on them.
- Not modelled: placeables, doors, other non-ghost creatures (the line tests against non-walk polygons and
  creatures), PCs (no coarse mode, other call caps), rotated tiles (the generator uses orientation 0),
  and a smoothing choice that varies from run to run (waypoint shape only).

## Generator fixes (2026-10-20), guided by the simulator

`landscaper sim-predict <pairs> <seeds>`: oracle-style random pairs over cliff 3/3.5/4/5/6 m and flat,
scored by the simulator alone (seconds), with reachability judged on the final walkmeshes.

1. **Shared tile edges** (rtin.rs `full_border`): the border is split at every sample, so neighbouring
   tiles have identical edge vertices; a face on the border is walkable iff both its border vertices
   are. Cliff 4.0 went 299 -> 396 of 400.
2. **Reconciliation** (seams.rs): walkable pieces under 2 m^2 (slivers along edges) are pruned, and a
   border segment stays walkable only if both sides agree, repeated until stable.
3. **Unrepresentable regions** (generator.rs `area_plan`, pathnode.rs `unowned_border_faces`): a walkable
   region that touches a tile edge but that no node of the chosen layout owns (e.g. the low pocket
   under the ramp: the game's layout table has no 2-node layout fitting it) is made non-walkable,
   then edges are reconciled and layouts rechosen, until nothing changes. The crossing-spread check
   (pathnode.rs `crossing_breaks`) scores layouts whose exit spreads stay in the right regions.

Result: sim-predict 2297/2301 (was 2121/2400 before, with sample-based truth). Server, fresh seeds:
cliff 4.0/3.5/5.0 -> 120/120 arrived (cliff 4.0 used to be ~30/40). On seeds where the simulator
still predicted a failure, the server failed exactly those routes at the predicted spots.

Remaining: (a) a few long detours run out of the NPC call budget ("out of calls");
(b) with 9 samples a tile the walkmesh's no-walk band around a cliff is ~3.75 m wide (the real
slope is 0.6 m), so ~5% of points near cliffs are off the walkmesh.

Oracle corpora: v1 (generator before these fixes: check with LS_OLD_SEAMS=1), v2 (current generator).

## Cliff band and time budget (2026-10-20, continued)

`sim-predict` now also reports walkable coverage (share of the truly walkable 0.5 m grid on walkable
walkmesh faces) and planner call use (p50/p95/max of the NPC's 100 calls). New variant `rugged`
(LS_RUGGED=1): 8x8 tiles of terraced fractal noise, the stress test. `check-tiles` lists the most
expensive in-tile crossings (exit to exit of one node) as the simulator costs them.

Changes:

1. **17 samples a tile** (0.625 m, was 9): coverage near cliffs 93% -> 96%, no pairs lost.
2. **Bad crossings removed without losing connectivity** (pathnode.rs `remove_bad_crossings`): after the
   usual choice, layouts are swapped to drop crossings whose spread can land in the wrong region, but
   only if every pair of regions connected through exits stays connected. A plain negative score
   dropped essential crossings (cliffs fell to ~210/400).
3. **Strict exits for our tiles** (`Regions::strict`): an exit counts only if the ground 0.3 m inside the
   edge is walkable (the lenient 1.25 m inward snap is kept for judging BioWare tiles). Rugged had
   exits matched across fully blocked edges.
4. **Unrepresentable crossings pruned** (`unrepresented_crossings`): a walkable edge segment whose two
   sides aren't joined through any exit (a pass between the fixed exit positions) is made
   non-walkable, so the walkmesh only offers what the server can route.
5. **Small interior holes filled** (< 4 m^2, seams.rs) and a **morphological opening** of sample
   walkability (bridges/spurs under ~2 samples removed).

Results (sim-predict 40 pairs x 10 seeds): cliffs 3-6 m and flat 2400/2400 routes, calls max 3/100;
rugged 349/363, calls p95 2, max 102 (out of calls). Server (oracle v3, current generator): cliff
4.0/6.0 80/80; rugged 72/80; simulator agrees on all 160 incl. every failure.

Time budget: with clean crossings, triangle count is not the problem (cliff tiles ~320 faces, 1
call). What exhausts it is an in-tile search across an irregular region: the in-tile search is IDA*
with a bound starting near the straight-line distance, so detours inside one
tile re-explore exponentially many paths each round. BioWare tiles never have such regions; rugged
noise does. `untangle` (LS_UNTANGLE=1, opt-in) tries min-cuts (mincut.rs) to split such regions but
doesn't work yet: its 1-2 face cuts are undone by hole filling, and a run takes ~10 minutes.
Next ideas: cut at the region's widest-detour point rather than the minimum cut (and exempt cut
faces from hole filling); or limit region shapes per tile up front (e.g. reject tiles whose
exit-to-exit geodesic is much longer than the straight line); and have the painter warn builders
about such spots. Painted terrain is likely much smoother than this noise.

Oracle corpora: v1 (old generator, LS_OLD_SEAMS=1), v2 (9 samples, before this section: check with
LS_SAMPLES=9 LS_NO_OPENING=1 LS_KEEP_HOLES=1), v3 (current default).

Untangle follow-up (2026-10-20): cut faces are now locked against hole filling, and crossing costs
are cached per tile (a rugged run takes ~4 min instead of ~10). The cuts now do separate the
regions (crossing cost drops to 0), but on rugged every one of them disconnects the two points:
each expensive U-shaped crossing is the only connection between its parts, so nothing can be cut
safely. Conclusion: automatic cutting can't fix this; the terrain has to change. Untangle stays
opt-in (LS_UNTANGLE=1); the painter should warn builders about such spots (check-tiles) instead.
