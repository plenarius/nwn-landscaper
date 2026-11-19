# Debugging switches

Environment variables named `LS_*` turn a step of the Landscaper off, print what a step did, or try a variant. The research
behind the Landscaper used them, and working on it still does; none is needed to use it. **Some change what an export writes**
(*differs* in the Export column), so the program says at start which switches are set, those first, and names any `LS_*` variable
it doesn't know.

`src/switches.rs` holds the same list; a test fails when the code reads a switch that is missing from it or from this file.

A switch is on when it is set to anything (`LS_TIMING=1`); one that takes a value says so.

## The test terrain (research commands, and an export with no heightmap)

| Switch | What it does | Export |
|---|---|---|
| `LS_HEIGHTMAP` | A heightmap file (the editor's JSON) the research commands work on, instead of the built-in test terrain | the same |
| `LS_CLIFF` | The test terrain's cliff height in metres | *differs* |
| `LS_FLAT` | The test terrain without height: its cliff becomes a strip of non-walkable ground | *differs* |
| `LS_RUGGED` | The rugged test terrain: terraced noise over 8 x 8 tiles | *differs* |
| `LS_CONTROL` | `generate` adds a control area made of the game's own rural tiles (ttr01) | *differs* |

## Walkmesh and path nodes

| Switch | What it does | Export |
|---|---|---|
| `LS_SAMPLES` | Height samples along a tile's side for its walkmesh (2^k + 1; default 17) | *differs* |
| `LS_WALK_ERROR` | How far the walkmesh may stray from the terrain, in metres | *differs* |
| `LS_NOLEVEL` | No tile height levels: every tile at level 0 | *differs* |
| `LS_NO_OPENING` | Keep thin walkable slivers that the walkmesh normally closes | *differs* |
| `LS_OLD_SEAMS` | The older way of matching tile edges and choosing path nodes | *differs* |
| `LS_UNTANGLE` | An experimental pass that untangles path node choices (slow) | *differs* |
| `LS_NO_PRUNE` | Keep walkable faces that no path node layout reaches | *differs* |
| `LS_KEEP_UNREPRESENTED` | Keep walkable faces at tile edges that the chosen path node layouts don't represent | *differs* |
| `LS_KEEP_HOLES` | Keep small holes in tile walkmeshes instead of filling them | *differs* |
| `LS_UPPER` | Choose only from the upper case path node layouts | *differs* |
| `LS_LOOSE_EXITS` | Match tile exits to walkable regions loosely | *differs* |
| `LS_IGNORE_POS` | Accept a path node layout even when a node lies off its region | *differs* |
| `LS_NO_SPREAD` | Score the path node layouts without the penalty for crossings that break | *differs* |
| `LS_NO_UNBREAK` | Skip the pass that removes crossings that break | *differs* |
| `LS_NO_SURFACES` | Walkmesh surfaces as grass and non-walkable only, not the painted surfaces | *differs* |

## The tiles' files

| Switch | What it does | Export |
|---|---|---|
| `LS_BAKE` | Pixels a side of each tile's baked texture (a power of two; default 512) | *differs* |
| `LS_SHADOW` | How dark baked shadows are, 0 to 1, over the editor's setting | *differs* |
| `LS_SHADOW_DEBUG` | `half` or `corner`: test patterns in the shadow maps, to see how the game lays them | *differs* |
| `LS_REUSE` | Every tile uses one of the game's models and minimaps (ttr01_o01_01): to test the tileset alone | *differs* |
| `LS_NO_EDGE` | No scenery beyond the area's edge | *differs* |
| `LS_NO_ORIGINAL` | An imported area without the original's identity, scripts and objects: to test pathing alone | *differs* |
| `LS_ONLOAD` | The test module runs its route test from OnModuleLoad (for tracing under a debugger) | *differs* |
| `LS_CASE` | With LS_ONLOAD: the one route the test runs (default `detour`) | *differs* |
| `LS_DUMP` | Also write each tile's baked texture as a PNG and each tile's model as text (`.mdl.txt`) beside the export | the same |

## Imports

| Switch | What it does | Export |
|---|---|---|
| `LS_IMPORT_TIMES` | Print how long each stage of an import took | the same |
| `LS_IMPORT_WATER` | Print the water faces an import finds | the same |
| `LS_IMPORT_MATS` | Print the walkable area of each surface an import finds | the same |

## Printing what a step did

| Switch | What it does | Export |
|---|---|---|
| `LS_TIMING` | Print how long each step of an export took | the same |
| `LS_DEBUG_PLAN` | Print the path node plan, round by round | the same |
| `LS_DEBUG_SEAM` | `a,b`: print the walkmesh faces of tiles a and b along their shared edge | the same |
| `LS_DEBUG_TILE` | A tile number: print its path node regions | the same |
| `LS_DEBUG_BREAKS` | Print the crossings that break, before and after they are removed | the same |
| `LS_DEBUG_ROUTES` | `export` and `check`: print each route the report tries | the same |
| `LS_CHECK_MAP` | `export` and `check`, `x0,y0,x1,y1`: print the ground in that box every 0.5 m (walkable, joined, lost) | the same |
| `LS_CHECK_PAIR` | `export` and `check`, `x0,y0,x1,y1`: print which rules join the two points | the same |

## The route simulator and the oracle (research commands only)

| Switch | What it does | Export |
|---|---|---|
| `LS_QUIET` | Don't print the path nodes each plan chose (less printed) | the same |
| `LS_SIM_ALL` | `sim-check`: print every route, not only the ones that differ | the same |
| `LS_SIM_DEBUG` | The simulator prints its in-tile search | the same |
| `LS_SIM_SUB` | The simulator smooths the sub-tile path first | the same |
| `LS_SIM_LINE_US` | The simulator's cost of a line test, in microseconds (default 0.7) | the same |
| `LS_SIM_GRID_US` | The simulator's cost of a grid line test, in microseconds (default 0.7) | the same |
| `LS_VARIANTS` | `sim-predict`: only these terrain variants, comma separated (cliff3.0 ... cliff6.0, flat, rugged) | the same |
| `LS_VERBOSE` | `sim-predict`: print every route | the same |
| `LS_PAIRS` | `oracle`, `x0,y0,x1,y1;...`: these routes instead of random ones | the same |
| `LS_ORACLE_SECONDS` | `oracle`: how long the server walks each route (default 40) | the same |
| `LS_CW` | `validate-pathnodes`: read tile orientations clockwise | the same |
| `LS_ROTATED` | `validate-pathnodes`: only tiles turned a quarter or three quarters | the same |
