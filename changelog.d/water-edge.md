<!-- markdownlint-disable-file MD041 -->
- **A lake's shore is smooth in the game, not a sawtooth.** Since water under 15 cm became dry ground (0.7.0), the water's surface
  stopped at the 0.625 m cells where the shallows began, so its edge stepped along the grid. The surface now runs on over the
  shallows and under the bank, and the ground hides it: the shore is where the two meet. The shallows stay dry to walk on, and
  the halo's and the edge scenery's water follow the same surface.
