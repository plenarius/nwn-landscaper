- **Walls meet the floor without stripes.** A texture's three projections are blended by the ground's own slope (the gentler
  side's on walkable ground, the steeper side's on a steep face), in the baked tiles and in the game's shader through the
  tile's third texture coordinates. They were blended by the light's normal, which is smoothed over several metres on steep
  ground, so flat floor at a cave wall's foot took the projection from the side and smeared into striped bands, and the
  wall's foot took the one from above and streaked. See [Paint](../docs/handbook/paint.md#walls-and-steep-ground).
