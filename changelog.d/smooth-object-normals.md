<!-- markdownlint-disable-file MD041 -->
- **Baked objects shade as their models do.** A tree, fern or rock baked into the tiles kept only its triangles and was lit flat,
  one normal a face; it now keeps its model's own normals, cut and clipped with it, so it shades as the model placed on its own
  does. Corners that share a place, texture coordinates and normal are one vertex: the forest showcase's tile 0000 has 37,797
  vertices instead of 140,963, and its hak is 465 MB instead of 660 MB. Imported things, which carry no normals, stay flat.
