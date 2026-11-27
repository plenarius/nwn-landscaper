# Light

An area has one sun by day and one moon by night, set in the **Sky & fog** panel (`L`). On top of that, the **Light** tool (`b`)
gives each tile its own two lights: warm pools in a village at night, a cold glow in a cave. Sky & fog lies over the tools'
panels: picking a tool closes it, and `L` opens it again.

## Set the area's light

1. Open **Area > Sky & fog** (`L`).
2. Pick a look from **Looks** (see [Atmosphere and skies](atmosphere.md)), or set the colours yourself.
3. Switch between **Day** and **Night** to set the sun and the moon apart. For each:
   - **Shade (ambient):** the light in shadow.
   - **Light (diffuse):** the light from the sun or the moon.
   - **Fog colour** and **Fog:** its colour and how thick it is.
4. **Time:** *Day and night* cycles with the game's clock; *Always day* and *Always night* hold one.

**Show in the view** draws the area by day or by night, so you see the light the game will use.

![The Sky & fog panel by night: the view shows the night sky, the moon's light and the fireflies](../assets/handbook/light.webp)

## Shadows

- **Shadows** (for the sun and for the moon apart): whether the game casts shadows from that light.
- **Baked shadows:** how dark the shade is that the export bakes into the tiles under scenery (trees, rocks, what you placed or
  scattered). It shades only the sun's or the moon's light: a tile light, a torch or a glowing mesh in a tree's shade keeps its
  full light.

## Relief on the ground

The game's light falls on the ground's texture as if it had relief: the texture's lighter parts stand a little higher, so the
stones of gravel, cobbles and rock catch a torch or the sun on one side and fall into shade on the other. It costs nothing in
the hak. It needs the game's shaders set above **Minimal**; at Minimal the ground is lit flat, as before.

## Paint the tiles' lights

![The Light tool: Torchlight on every tile of a cave](../assets/handbook/light-tool.webp)

Each tile has two lights, over its middle, reaching 14 m. A tile is 10 m, so this is a broad wash of colour, not a lamp: for a
lamp or a torch, place a placeable that gives light ([Things](things.md)).

1. Pick **Light** (`b`).
2. Pick a look from **Looks**, or set **First light** and **Second light** yourself from the game's 32 tile light colours
   (`lightcolor.2da`). The looks that suit the open area (by day or night, outdoors, under ground, inside) are outlined.
3. Drag over the ground: the tiles under the brush get the two lights. Shift-drag turns them off.

- **Whole area** gives every tile the two lights; **All off** turns every tile's lights off. Each is one Undo.
- The panel counts the tiles lit.
- The view draws the lights on the ground, under the area's light: press `L`, switch **Sky & fog** to **Night** and tick **Show
  in the view** to see them as they will look after dark. Picking a tool closes the panel; the view stays by night.

| Look | Lights |
| --- | --- |
| None | off: outdoors by day the sun is enough |
| Warm pools | lamplight over a village or camp at night |
| Torchlight | firelight: a hall, a lit cave |
| Cold blue | a cold glow deep in a cave |
| Moonlit | a faint blue for open ground at night |
| Indoors | plain light in rooms |
| Eerie | a sickly green: a crypt, a swamp |
| Arcane | purple and aqua: a mage's den |

!!! warning "Day and night"
    The game lights with the tile lights by day too. Outdoors by day, leave them off.

The export writes the colours into the area (the toolset's main lights of each tile). It needs an engine of the same release:
an older one leaves the lights off.

## Light from what you bake

A placeable baked into the tiles (with [Scatter or Place](scatter.md)) keeps its glow: a glowing crystal or mushroom still glows
in the tile.
