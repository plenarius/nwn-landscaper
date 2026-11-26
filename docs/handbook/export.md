# Export

**Export to NWN** builds the area into the game: its hak into your user folder's `hak/`, its module into `modules/`. Open the
module in the toolset or the game.

## Export an area

1. Click **Export to NWN** in the toolbar.
2. The export builds the tiles, the hak and the module, then checks paths: it sends the game's pathfinding along random routes
   over the area.
3. Read the result on the map:
   - **Red markers:** a route stops short. Open up the ground between the two ends.
   - **Orange markers:** a tile whose walkable ground is too winding for the game's search within a tile. Straighten or widen the
     passage.

A module already there takes only the new tiles. The module before is kept as `<module>.mod.bak`, and a failed export leaves the
module and the hak as they were. `report.json`, beside the module, holds the whole report.

![The path check's markers after an export](../assets/handbook/export.webp)

## Check without exporting

- **Check paths** runs the path check with nothing exported.
- **Live** checks paths after each edit.
- **Markers** (`M`) shows or hides the markers; the checks go on.

## What stops an export

An object, thing or entrance more than 10 m outside the area, an object scaled outside 0.1 to 5, or an entrance wider or deeper
than the area stops the export, with a message naming it. Move it or take it out, and export again.

## What goes into the game

- `<area>.hak`: the area's own tiles, with the scenery, shadows and particles baked in.
- `ls_textures.hak`: the texture library's textures the area paints with, shared by every area.
- `ls_skies.hak`: the installed sky packs' skies, when there are any.
- the module, with the area, its doors, its things and the blueprints, conversations and scripts it uses.

Each entrance's door is tagged `<area>_ent<id>`. On a re-export, the door of an entrance you took out is removed and listed in
the report; doors placed by hand stay.
