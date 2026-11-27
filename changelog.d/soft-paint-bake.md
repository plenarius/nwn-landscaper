- **Painted edges without the sawtooth.** Where two textures meet in a baked tile, the bake drew the paint's 0.625 m grid, so a
  trail showed square teeth along both sides. It now smooths each texture's paint over about 1.5 m and draws the edge where the
  smoothed paint changes hands: sharp on a smooth line for laid surfaces (cobbles, flagstones, floors, planks), ragged by noise
  and by the textures themselves for natural ground (gravel poking into snow). Every area gets it on its next export, with no
  repainting; specks a sample or two across, like the mix brush's soft edge, are smoothed away. The texture catalog's new
  `edge` (`crisp` or `broken`; unset, the category decides) picks the style, broken winning where the two differ. Tiles
  beside a change of texture are baked too now (about 3% more hak), the minimap follows the smoothed paint, and every tile
  bakes again once. The design: [#88](https://github.com/plenarius/nwn-landscaper/pull/88); how it works, for the editor to
  match: `src/generator/soft.rs`.
