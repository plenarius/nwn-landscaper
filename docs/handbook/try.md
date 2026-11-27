# Try it in your browser

**[Open the Landscaper in your browser](https://plenarius.github.io/nwn-landscaper/try/)**: the editor and its engine both
run in the page, with nothing to install and nothing sent to a server. It has none of the game's files, so it paints with
textures of its own; for the rest, it uses your copy of the game when you point it there.

![The Landscaper in the browser: a hill sculpted, a dirt road painted with the demo's textures, and the path check](../assets/handbook/try.webp)

## What works

- **Sculpt, paint, water, chasms, caves, light, ambience** and the rest of the editor, as in [the handbook](index.md). The
  textures are the demo's own: CC0 photographs under the names of the game's starter set, credited in
  [packs/demo/CREDITS.md](../../packs/demo/CREDITS.md).
- **Check paths**, after each edit or on demand, as on the desktop.
- **Export** downloads a zip of the module and its haks: put the files of `hak/` in your Neverwinter Nights user folder's
  `hak` folder and the module in its `modules` folder (the zip's README says where those are).

Place, Scatter, Things, Scripts and Import read the game's own models, blueprints and tilesets, which the page doesn't have:
they say so. Use the Landscaper on your computer for them ([Getting started](getting-started.md)).

## Your game folder, at Export

An area's tiles are drawn by shaders the Landscaper makes from three of the game's own files. At Export the page asks for the
folder the game is installed in (the one with `data` and `lang` in it) and reads only those three files from it, about 20 KB.
They stay in the page until you close it; nothing is uploaded anywhere.

- **Chrome and Edge** open a folder picker, and remember the folder for your next visit.
- **Firefox and Safari** open an upload field, and ask again each visit. They say the folder is uploaded: it isn't, the page
  only reads it.

Without the folder the export still downloads, but the area doesn't draw right in the game, and the report says so. Export
again and choose the folder.

## Where your areas are

In your browser's storage for this site: they are there when you come back, in the same browser. That storage is no backup:
clearing the site's data, or a browser short of space, removes it. **File, Download area** saves the area open as a file;
**File, Open area from a file** brings it back, in this browser or another.

The page works in one tab at a time: a second tab waits until the first is closed.

## Size

Areas up to 16 by 16 tiles are made at once. A larger one asks first: exporting it needs more memory in the page (about 0.4
GB at 16 by 16, 1.2 GB at 32 by 32), and a phone or a small laptop may not have it. An export that runs out says the area is
too big for this browser; the Landscaper on your computer has no such limit.

## Browsers

A current Chrome, Edge, Firefox or Safari, with WebGL. The first visit takes a few seconds more than the next ones: it
prepares the textures once and keeps them.

## Licences

The engine in the page is the Landscaper's, GPL-3.0-or-later, built for WebAssembly: its source is
[nwn-landscaper](https://github.com/plenarius/nwn-landscaper), and the page serves its
[licence](https://plenarius.github.io/nwn-landscaper/try/try/LICENSE-engine.txt) and a
[note](https://plenarius.github.io/nwn-landscaper/try/try/NOTICE-engine.md) of the C and C++ libraries built into it. The
editor is [nwn-landscaper-web](https://github.com/plenarius/nwn-landscaper-web) (MIT).

## How the page is built

The docs workflow makes `/try/` with `scripts/try-page.sh`: the newest release's `landscaper-wasm.zip`, the editor at the same
tag, and `packs/demo`, put together by the editor's `scripts/try-site.sh` from listed files only. Until a release has the zip,
the workflow builds the engine itself, with the editor's main branch. A release's own run of the workflow puts the new engine
on the site.

The page needs no header that GitHub Pages can't send. It runs the engine on one thread, so it needs no cross-origin isolation
(that comes with threads, [DESIGN-web-demo.md](../DESIGN-web-demo.md) question 7). The Service Worker and the engine's Worker
are the site's own files, and Pages serves the engine as `application/wasm`. The whole page is about 14 MB, far under Pages'
limits. [DESIGN-web-demo.md](../DESIGN-web-demo.md) has the rest: how the page runs the engine, and what was measured.
