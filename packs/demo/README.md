# The web demo's texture pack

The browser demo (`docs/DESIGN-web-demo.md`, "Art for visitors without the game") has none of the game's files, so it can't build
the starter texture library from them. This pack stands in: one picture for each of the 51 starter textures
(`src/starter_textures.json`), under the same ids, so the editor's defaults, the palette and the sample landscapes look right
with no game.

- `sources.json`: where each picture comes from (ambientCG or Poly Haven, both CC0), or how it is made from another of the
  pack's (`standin`: tinted and softened).
- `make.py`: fetches the pictures, repeats each to cover the 4 m the Landscaper repeats a texture over (so a pebble is a
  pebble's size), fits it to 512 px and writes `pictures/<id>.webp`, `pack.json` (each picture's sha256) and `CREDITS.md`.
  Needs Pillow; `--cache DIR` keeps the downloads.
- `pictures/`: what the page downloads. WebP, because the photographs are four times smaller than as PNG.
- `CREDITS.md`: every picture's source, authors and licence.

To change a texture, change its line in `sources.json` and run `make.py`; a test checks that every starter id has its picture,
that `pack.json` matches the files, and the budget.

## Size

**Budget: 3 MB** for all the pictures (now 2.7 MB, 51 pictures of 512 px), about the size of the rest of a first visit (the
engine and the editor, 3.1 MB). The test fails above it.

## How the page installs it

1. Fetch `pack.json`, then each picture, and decode it to PNG in the page (`createImageBitmap`, then an `OffscreenCanvas`'s
   `convertToBlob({ type: 'image/png' })`): the engine reads PNG, TGA and DDS only.
2. Put the PNGs in one folder of the engine's files (`<id>.png`) and run `landscaper textures init --pictures <folder>`: the
   starter set from them, with the starter list's names, groups and footsteps.
3. Run the engine with `LANDSCAPER_GAME=none`: `/api/me` then says there is no game, and the editor's Place, Scatter,
   Things, Scripts and Import say they need it.

In the spike's page this takes about 3.4 s in headless Chromium (2.5 s of it decoding to PNG), once per start; a page that
keeps the library (OPFS) does it once.

## The art

CC0 photographs for now. The maintainer's own pictures, and the NWN community's art shared for the community, can replace
any of them: each is credited in `CREDITS.md`, and anyone who asks to have their picture taken out gets it taken out.
Nothing here is the game's: the pack is made from the sources above by `make.py`, never copied from an install.
