# Building an area

From an empty landscape or an existing area to something you can test and hand over. Everything here is in the editor; nothing
needs the toolset.

## The path

1. **Start.** *New area* (size, a start from flat, hills, a cave or an interior, or a sample descending crypt, dungeon or cave to
   look at and change) or *Import* an existing area to rework, or upload your own `.mod` or `.erf` and import areas from it.
   Several areas can share a module. Areas are 3 to 32 tiles (10 m each) a side; a bigger one can't be imported. A new area
   never replaces one of the same name without asking.
2. **Shape and paint the ground.** Raise, Lower, Smooth, Flatten, Paint, Water, Chasm (a bottomless drop, see *Chasms* below),
   Entrance (cave mouths), Scatter and Place (props and baked scenery), and for caves and interiors Dig and Room (passages and
   chambers cut out of the rock), Ramp (a ramp or stairs between two levels) and Descent (a whole switchback descent at once); see
   *Caves, interiors and descents* below. The editor shows what will be walkable and flags ground creatures would struggle with.
3. **Put things in it** (the Things tool, below).
4. **Set the area up:** its name, tag, scripts, flags, weather, lighting, music and ambient sound, load screen and variables.
5. **Check** (live, and again when you publish).
6. **Export to NWN.** This builds the area into a module and hak in your NWN user folder (on a team server, your workspace) that
   you can open in the toolset or the game.
7. **Hand it over:** *Publish* (pull requests, with a review service) or *Package for your module* (a zip to commit yourself).

## Chasms

The toolset's outdoor tilesets block the way only with what rises: trees, mountains, walls, water. A chasm drops away: a canyon
rim nobody climbs down, a gap between two ledges, a plateau or an island standing over nothing.

- **Chasm** opens one under the brush; Shift-drag fills it in, the ground as it was. *Along the whole edge* opens one the brush
  size wide all round the area. Chasms don't reach into the band round the area yet (*The band round the area*, below): lower
  the band past the edge to keep the drop going there.
- The ground there drops 20 m under the rim (the nearest ground that isn't chasm), its walls fade to black from the rim down and
  its floor is black, so no camera angle shows a bottom; the game's fog takes the rest. Nothing walks in it or on its edge, and
  the path check sends no route into it. Water stops at it.
- **To cross one**, leave a strip of ground across it at least 6 m wide, over the middle of a tile's edge, as for a cave passage.
- For outdoor areas only.

## The band round the area

Every outdoor area has a band of ground 50 m wide round it, which the game shows past the area's edge. Nobody walks on it and it
is never built into tiles.

- Raise, Lower, Smooth, Flatten and Paint shape and paint it where the brush reaches past the edge, with undo. The area's water
  runs on into it where its ground is lower.
- A new area's band, and an old area's when it is first opened, carries the area's edge straight out until you shape it.
- The texture mix, chasms, Dig and objects don't reach into it yet.
- The export draws it past the edge in the game out to 50 m, fading into the fog colour from 25 m, with trees farther than 25 m
  out as crossed pictures.
- Caves and interiors have none.

## Caves, interiors and descents

A cave or an interior starts as solid rock. You cut rooms and passages out of it, and what is left is the wall.

- **Room** cuts a rectangle (snapped to 1.25 m, at least 5 m a side) at the floor where you start; rooms that overlap join. **Dig**
  cuts passages and chambers with a round brush (3 m radius at least).
- **Passages need to be about 6 m wide at least, and run along the middle of a tile** (5 m from its edges). NWN's pathfinding only
  crosses from one tile to the next through the middle of an edge; narrower or off-centre passages are closed to it.
- **One ground height per spot.** An area has a single floor at each point, so floors can't sit on top of each other. A level that
  drops inside one area has to wind sideways as it drops; for stacked chambers use one area per level and link them with
  transitions.
- **Ramp** lays a ramp (or stairs, with a step height) between two points, rising to where you let go or by an amount you type.
  Dig or make a room first and lay the ramp inside it, a little wider than the passage: in a cave or interior it only changes floor,
  all of it, so a ramp that is narrower than the passage leaves a ledge along each wall. A slope steeper than creatures can walk is
  eased (the tool says so).
- **Ceiling height** (Area menu, caves and interiors): how many metres of headroom to keep over the floor everywhere. Without it
  the whole area has one flat roof, so a passage that descends gets taller walls the lower it goes. With it the rock is cut down to
  that height over the nearest floor, so a descent keeps the same tight ceiling. It applies to the whole area.
- **Descent** fills a box with a switchback descent in one drag: halls side by side, each a floor lower than the last, a ramp at
  alternating ends. Drag over whole tiles (the box snaps outward to them); each floor takes two tiles across the box, and the halls
  run along its longer side, at least four tiles. Floors, the drop per floor (5 m is a 14 degree ramp over 20 m), the ceiling and
  which side to start from are in its panel; the status line says why a box can't be used (too shallow, too short, too steep, not a
  cave or interior). It sets the ceiling height too, and one Undo takes it back.
- **Sample starts** in *New area* (a descending crypt, dungeon or cave) are a descent already laid at a size they pick, to change
  with the tools above.
- **Entrance doors.** Each entrance is a door, tagged `<area>_ent<n>`. The number belongs to the entrance and stays with it, so
  taking another entrance out renames nothing, and transitions and scripts can name these doors. When you take an entrance out,
  the next export removes its door from the module and the export's report lists it; doors you placed yourself in the toolset
  are never removed. A transition elsewhere that led to the removed door still names it and leads nowhere: change it yourself.
- **Long descents and click-to-move.** The game's route search gives up when the way is about 20 tiles (200 m) longer than going
  straight across the tiles between its ends, so a click from one end of a long descent to the other walks a few metres and
  stops. Shorter clicks along the way get there, and so does walking with the keys. The path check tries those shorter clicks
  for each route that stops: when they get there, it counts the route as too long for one click (the report's `long_routes`,
  with how many clicks) instead of blocked, and the editor says the paths are OK. The sample descents have such routes.

## Things

The Things tool places creatures, waypoints, transitions, triggers, sounds, encounters, stores, items, doors and placeables, each
made from a blueprint (the module's, the game's, or your own, below). On each one you can set:

- its tag, name, scripts and local variables;
- what a creature carries and wears, a container's contents, a store's stock (and which items never run out), the creatures an
  encounter spawns;
- locks, keys and traps, hit points, sound timing, and for triggers and encounters the outline (corners can be dragged, added and
  removed after drawing).

An imported area's own objects are shown greyed: click one to move it, turn it, take it out, or change its properties. Click a
marker to select it from any tool (which markers show is the **Show** menu's). Right-click a marker for its menu. In an imported
building, **Show > Cut away roofs and upper floors** hides what is above a height you choose, to see inside.

**Painting with the tileset's own textures:** in the Paint tool, press `I` (or click **Pick from the ground**, or Alt-click) and click the
ground. On ground you have painted it picks that texture; on an imported area's ground it picks the tileset texture that is under the
click (the grass, dirt or stone the original tiles were drawn with) and adds it to the library, under **Tileset**, the first time. It
then paints like any other texture, and goes into your `ls_textures.hak` when you export. An area imported before this was added has no
record of its textures: import it again (a re-import keeps the original) to pick from it.

Previews show what creatures, placeables, doors and items look like in the lists (creatures and items only with a review service
that draws them).

## Your own content

- **Creatures, items, placeables, doors and stores:** make one as a copy of any existing one with its own name and resref, then
  change its settings (a placeable's or door's appearance, a store's stock and which items never run out). Items get properties
  (what the base item can have, with the value worked out as the game does it); creatures, placeables and stores get their own
  contents.
- **Conversations:** write one, or copy one, as a tree of NPC lines and player replies with conditions, actions and links. Opened
  from a creature's Conversation setting.
- **Scripts** (admins only, because a script runs in the game): an editor that compiles with the game's own compiler as you type and
  shows the error's line. Every script-name box has a picker, and **View script** opens the module's or the game's script read only,
  with a button to make your own copy.

These are published with the areas that use them.

## Checks before it is handed over

Errors stop a publish; warnings are listed. Errors: an area with no name or tag, or a tag another area has; a transition with no
destination; a door or trigger leading to a tag that isn't a door or waypoint in the module; a script or conversation that isn't in
the module or the game (your own count); a script of yours that doesn't compile. An object, thing or entrance more than 10 m
outside the area, an object scaled outside 0.1 to 5, or an entrance wider or deeper than the area stops the check and the export
at once, naming it. Warnings: objects in the module's copy of the area that stand outside it (up to 10 m out, or the toolset's,
or an imported area's own); duplicate waypoint, door, trigger or store tags; a tag one or two other areas have; a door whose
destination leads elsewhere; an area with more objects than areas usually have; a new area nothing leads into. Tags used in three
or more areas are treated as conventions.

## Connecting a new area to the world

A transition needs both ends, and the existing end is a rework of an existing area:

1. Import the neighbouring area into the same module as the new one.
2. In the new area, place a waypoint where people arrive (tag it, e.g. `bay_arrive`) and a transition leading to a waypoint of the
   neighbour (**Leads to**, then *Pick a waypoint of an existing area*).
3. In the neighbour, place a transition leading to `bay_arrive`, and a waypoint for the way back.
4. Export both, and publish or package **the whole module**, so one hand-over holds the new area and the changed neighbour.

## Testing

Open the exported module in the toolset or the game. With a review service that runs a dev server, **Try on dev** in the
Publish dialog puts the area, or its whole module, on dev with no restart; walk in with `;forge <area>`. New scripts can't be
tried on dev: they go through Publish.

## Handing over

Either way it is what your **last export** built, so export again after any change. The dialog first runs the checks above.

- **Package for your module:** a zip in the layout of your module's source: the area files, tile files, the shared texture library
  (and the skies' hak when the area has a Landscaper sky),
  new blueprints, conversations and scripts, `module.ifo.json` with a new area added, and a `MANIFEST.md`.
- **Publish:** the same files as pull requests for review, with a note for the reviewers. A reworked area's PR says what changed
  against the module's copy. Nothing is merged by the Landscaper.

## Not in the editor yet

Item armour looks and colours, class levels and feats on creatures, your own triggers, waypoints, sounds and encounters (those
are placed and set directly rather than made from a blueprint of your own), journal and factions, copy and paste, multi-select, tile
lighting, and a way to change a script of the module in place (you make your own copy instead).
