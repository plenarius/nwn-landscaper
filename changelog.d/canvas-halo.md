<!-- markdownlint-disable-file MD041 -->
- **An area can carry a halo: the ground beyond its edge, shown past the edge in the game.** The area format gains `halo`
  (samples of ground around the area, whole tiles, at most 5) and `tagPrefix` (what entrance doors' and object-mode placeables'
  tags start with, for the pieces of a canvas). With a halo, the area's normals, baked shade, chasm rims and cave ceilings read
  the ground across its edge, and its boundary tiles draw the halo's real ground out to 50 m: a vertex every 2.5 m with a
  128 px texture a halo tile, the samples' own detail where a shore or a road's groove runs, the halo's water and its trees,
  fading to the fog colour from 25 m out. Trees 25 m or more out are crossed cards showing pictures of the tree's own model, one
  atlas per tree model. The edge models the client repeats 5 deep are left empty. An area without a halo exports as before.
  The canvas-cut design's trial area (10 by 10 tiles) draws 27,900 triangles past its edge (the trial's T, 32,500).
