# Paint and the mix brush

Paint the ground with one texture, or with a mix of several in patches, so a meadow isn't one flat green.

## Paint with one texture

1. Pick **Paint** (`5`). The panel lists the texture library by category.
2. Keep **One texture** on and click a texture.
3. Drag over the ground. **Strength** sets how much each stroke covers.

Click a category's heading to fold it away, or use **Collapse all** and **Expand all**. A folded category still shows the
texture you paint with.

![The Paint tool with the texture library](../assets/handbook/paint.webp)

## Pick a texture up from the ground

Press `I` (or click **Pick from the ground**, or Alt-click) and click the ground. On ground you painted it picks that texture.
On an imported area it picks the tileset's own texture under the click and adds it to the library, under **Tileset**, the first
time. It then paints like any other texture and goes into your texture hak at export.

## Paint a mix

The mix brush paints a blend of textures in patches: grass with dry patches and dirt, sand with pebbles.

1. Switch the panel to **Mix**, or Shift-click a texture to start one.
2. Click textures to put them in the mix or take them out. Shift-pick from the ground adds the texture under the cursor.
3. Each texture in the mix has a slider: its share of the mix. Set **Patch size**: how large each texture's patches are.
4. Drag over the ground. The strength slider becomes **Cover**: how much of the ground under the brush the mix paints.

- **Soft edge** thins the mix out toward the brush's rim, so a stroke has no hard edge.
- **Reshuffle** lays the patches out anew for the next strokes.
- **Clear** takes every texture out of the mix.

![The Paint tool painting a mix](../assets/handbook/mix.webp)

## Edges in the game

Where two textures meet, the export doesn't draw the paint's grid (a point every 0.625 m): it smooths each texture's paint over
about 1.5 m and draws the edge along that. Natural ground gets a ragged edge, gravel poking into snow; laid surfaces such as
cobbles, flagstones, floors and planks get a sharp one. Where a laid surface meets natural ground, the ragged edge wins.

- A line of paint one point wide, or a patch of up to about two points by two (the mix brush's soft edge makes them), is
  smoothed away. Paint lines at least two points wide.
- The editor draws the same edges, so what you paint is what the game shows.

## More textures

**Get more textures** lists the texture packs on offer. See [Texture packs](texture-packs.md).
