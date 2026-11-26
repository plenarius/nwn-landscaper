- **Particles baked into the tiles.** An area's `ambience` list (`{ "kind": "mist", "x", "y", "z", "scale", "seed" }`, and
  `bearing` for the kinds that blow or lean) exports as the particle pack's emitters in the tiles that hold them, under the
  tile's `<tile>a` dummy, so night-only kinds come on at dusk and day-only ones go off, and mist and ground fog loop
  (`SetTileAnimationLoops` clears them a tile at a time). A placement outside the area, of a kind the pack lacks, or past 1,500
  live particles an area is skipped with a warning; the report gives the live count (`ambience`). The pack is built into the
  program, so a release needs nothing beside it; `LANDSCAPER_PARTICLES` names another pack's folder.
