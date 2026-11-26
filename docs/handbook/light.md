# Light

An area has one sun by day and one moon by night. You set their colours, how strong the shade is and the fog in the
**Sky & fog** panel (`L`), and the export bakes shadows from them into the tiles.

## Set the area's light

1. Open **Area > Sky & fog** (`L`).
2. Pick a look from **Looks** (see [Atmosphere and skies](atmosphere.md)), or set the colours yourself.
3. Switch between **Day** and **Night** to set the sun and the moon apart. For each:
   - **Shade (ambient):** the light in shadow.
   - **Light (diffuse):** the light from the sun or the moon.
   - **Fog colour** and **Fog:** its colour and how thick it is.
4. **Time:** *Day and night* cycles with the game's clock; *Always day* and *Always night* hold one.

**Show in the view** draws the area by day or by night, so you see the light the game will use.

![The Sky & fog panel by night](../assets/handbook/light.webp)

## Shadows

- **Shadows** (for the sun and for the moon apart): whether the game casts shadows from that light.
- **Baked shadows:** how dark the shade is that the export bakes into the tiles under scenery (trees, rocks, what you placed or
  scattered). It shades only the sun's or the moon's light.

## The Light tool

!!! note "Not in a release yet"
    Every Landscaper tile carries the game's two tile lights, exported dark today. A **Light** tool that paints their colour
    tile by tile is in review
    ([nwn-landscaper-web#122](https://github.com/plenarius/nwn-landscaper-web/pull/122),
    [nwn-landscaper#45](https://github.com/plenarius/nwn-landscaper/pull/45)). This section gets its steps when it is merged.
