# Terrain

Shape the ground with the sculpting tools, then add water, chasms and cave mouths.

![A valley with a lake; the orange marks are the live path check](../assets/handbook/terrain.webp)

## Shape the ground

Pick a tool, set **Brush size** (`[` `]`) and **Strength**, and left-drag over the ground.

- **Raise** (`1`) builds the ground up; **Lower** (`2`) digs it down.
- **Smooth** (`3`) softens bumps and edges.
- **Flatten** (`4`) levels the ground to the height where you first click: drag from a spot at the height you want.

Keep **Walkable** (`W`) on while you work. Red ground is too steep for creatures; the export uses the same rule, so what is red
in the editor is closed to them in the game.

## Start from a picture

- **Area > Import heightmap** replaces the ground with a greyscale image, its top to the north. Set the heights black and white
  stand for, then stretch the image over the area or keep its shape. A 16-bit PNG is read at full precision; for an 8-bit image,
  tick *Smooth out steps*. Paint, water and things stay where they are, and Undo puts the old ground back.
- **Area > Underlay** lays a picture under the ground to trace over: a battlemat or a map. **Scale by 2 points**: click two
  points of the picture a known distance apart and type the distance (`5 ft` works). `U` shows or hides it. It is never
  exported.

## Water

**Water** (`6`): click in a hollow and drag up to fill it. Drag on water to change its level; Shift-click water to drain it.
Creatures don't walk into deep water, and the path check treats it as closed.

## Chasms

**Chasm** (`c`) opens a bottomless drop under the brush: the ground falls 20 m under the rim into the dark, and nobody walks into
it or across it. Shift-drag fills it in again, the ground as it was.

- *Along the whole edge* opens one the width of the brush all round the area, so a plateau or an island seems to stand over
  nothing.
- To cross a chasm, leave a strip of ground at least 6 m wide over the middle of a tile's edge: the game's pathfinding crosses
  from tile to tile only there.
- Outdoor areas only.

## Cave mouths

**Entrance** (`7`): click a slope or a cave wall to carve an entrance, as wide as the brush; Shift-click one to remove it. Each
entrance has a door. Pick where it leads (an entrance door in the same module), or link it in the toolset. See
[Caves and interiors](caves.md) for the door's tag.
