# Scatter and Place

Scenery from the game's models: trees, rocks, bushes, ruins. What you put down is baked into the tiles, so it casts baked shadows
and costs the game no objects.

## Scatter

**Scatter** (`9`) paints models over the ground.

1. Click models in the panel to add them to the mix; click one again to take it out.
2. Set **Density**, and the **Smallest** and **Largest** size.
3. Drag over the ground to scatter. Shift-drag erases.

- **Blocks walking:** creatures walk round what you scatter.
- **Not on steep ground or water:** keeps trees off cliffs and out of lakes.

![Trees scattered round a lake](../assets/handbook/scatter.webp)

## Place

**Place** (`8`) puts one model down where you click; drag to turn it. Shift-click one to remove it. **Size**, **Random turn** and
**Blocks walking** apply to the next one, and so does the mode:

- **Bake into terrain:** scenery, part of the tiles, with baked shadows.
- **Placeable:** a real placeable from the model, to use, open or script in the toolset.

Click a placed model to select it, then:

| Key | Does |
| --- | --- |
| `G` | move (then `X` or `Y` locks an axis) |
| `R` | turn |
| `S` | size |
| `Q`, `E` | turn 15 degrees |
| `Delete` | remove |
| `Esc` | deselect, or cancel a move |

After `G`, `R` or `S`, move the mouse and click, or type a number for an exact value. Ctrl snaps. You can also drag the handles
on the model: the arrows move it, the ring turns it, the corner box sizes it.

!!! tip "Scenery or a placeable"
    Place puts down scenery from a model. For a placeable with settings (a chest that holds items, a lever with a script), use
    [Things](things.md), which places one from its blueprint.

## In the toolset

Baked trees fade their crowns, as the stock forest tiles' trees do: on a model taller than 4 m, what stands more than 4 m above its
base fades, cut flat, and the trunk stays. In the game the crowns fade out about the player. In the toolset, turn on Fade Geometry
to hide them and work on the ground underneath, between the trunks. Bushes, boulders and rocks stay solid.
