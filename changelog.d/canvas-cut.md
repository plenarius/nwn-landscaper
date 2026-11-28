<!-- markdownlint-disable-file MD041 -->
- **A canvas exports as one module, an area a piece** (`POST /api/canvas-export`, `landscaper canvas-export`): each piece is cut
  out with 5 tiles of its neighbours' ground round it, its share of the water, and what stands in it; a trigger or a cave
  across a cut is refused by name. Crossings are made along every cut where open ground meets open ground: a transition
  trigger each way, at most 2 tiles long, leading to a waypoint 4 m into the next piece, tagged by its place on the canvas.
  See [the design](../docs/DESIGN-canvas-cut.md), step 4.
- **The canvas route check** (`POST /api/canvas-check`) checks each piece, then walks the crossings between them: pieces no
  crossing leads into, pieces a player can't reach from the start piece, and arrivals no route leads on from.
- **Every route check now routes between waypoints, transitions and cave entrances** too, beside its random pairs: the report's
  `fixed` lists them and which pairs a player can walk between.
- **The edge scenery's first ring is as sharp as the area's tiles:** the halo tiles touching an area are baked at its own size
  (512 px), so where a player stands at the edge (a canvas's cut, or a single area's band) the ground past it no longer
  turns soft. The rest of the halo stays at 128 px.
