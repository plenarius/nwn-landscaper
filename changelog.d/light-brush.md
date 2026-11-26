- **Tile lights.** An export takes `lights` from the editor, two `lightcolor.2da` rows a tile, and writes them as the area's
  `Tile_MainLight1` and `Tile_MainLight2`; without them every tile's lights stay off, as before. The editor's Light tool
  paints them.
- **Baked objects keep their glow.** A baked placeable's meshes keep their self-illumination colour (`selfillumcolor`) in the
  tile, so a glowing crystal or mushroom still glows. Before, baking dropped it.
