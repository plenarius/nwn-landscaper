<!-- markdownlint-disable-file MD041 -->
- **A texture pack for the web demo** (`packs/demo`, with its README and credits). One CC0 picture (ambientCG, Poly Haven)
  for each of the 51 starter textures, under the same ids, so the browser demo paints with no game files: 2.7 MB of WebP,
  under a 3 MB budget a test keeps. `packs/demo/make.py` fetches and fits them, and writes the credits.
- **`landscaper textures init --pictures <folder>`** builds the starter set from pictures named by id instead of the game's
  textures, with the starter list's names, groups and footsteps.
- **`/api/me` says whether the engine has the game** (`"game"`): false when `LANDSCAPER_GAME=none`, as the web demo sets it,
  so the editor's panels that need the game say so.
