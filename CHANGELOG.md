# Changelog

All notable changes to the Landscaper engine; the editor's are in
[nwn-landscaper-web's CHANGELOG.md](https://github.com/plenarius/nwn-landscaper-web/blob/main/CHANGELOG.md). Versions follow
[Semantic Versioning](https://semver.org/); before 1.0, a minor version may change the commands, the HTTP API between the engine
and the editor, or the files it writes. [nwn-bake](https://github.com/plenarius/nwn-bake) has its own changelog. Changes not
released yet wait in `changelog.d/`, a file each, until the release puts them in its section.

## 0.7.1

- **Ambience: no particle cap.** The export bakes every placement, however many particles the area keeps alive; its report and
  summary still give the day and night counts. 1,500 is now only the editor's guide to a heavy area.
- **No line along tile borders.** A tile's baked picture now reaches a third of a metre past the tile on each side. Seen from
  afar the game reads a picture's smaller copies, and at a picture's edge it could only repeat its last pixel, so a faint line
  ran down every tile border painted with several textures (clearest along a cave floor at eye level).
- **Cliffs turned from the sun are rock in shade, not black.** The game lights every area from the north-east, so a cliff
  facing south or west gets only the area's ambient light, and the baked shadow map then darkened that again for the shadow
  the face casts on itself. The shadow map now takes a cast shadow only from ground that faces the sun, and the tiles'
  shaders give steep ground some of the sun or moon bounced off the ground below it (none on flat ground). See
  [Light](docs/handbook/light.md#shadows).
- **An area with placeables this install lacks checks and exports.** The path check and the export stopped at the first model they
  couldn't read, so an area from another install or a team showed "Live: error". Such a placeable now blocks nothing and is left
  out of the tiles, and the report names its models (`missing_models`); one in object mode is still written to the module.
- **The browser page carries the game's stock placeables.** `/try/` has every stock placeable model with its walkmesh and
  textures, and the three shaders an export needs, fetched a model at a time: stock trees and rocks draw, block paths and export as
  on the desktop, Place and Scatter work, and Export no longer asks for the game folder.
- **Design: the director's line** ([DESIGN-director-line.md](docs/DESIGN-director-line.md)). A Film tool to draw
  nwn-director's camera flights on the map, preview them on the path the director flies (its planner ported, checked equal on
  five plans), check clearance as they are drawn, and hand them to the director: as a download, on the builder's machine, or
  through a filming queue.
- **The handbook covers the app and the band.** Getting started has an install page for
  [Windows](docs/handbook/install-windows.md), [macOS](docs/handbook/install-macos.md) and
  [Linux](docs/handbook/install-linux.md), one for [the first start and the Help menu](docs/handbook/first-start.md), and
  one for [the program alone](docs/handbook/program-alone.md); README and QUICKSTART put the installers first. Terrain
  describes the band round an area, and Export what the game shows past the edge. A pull request that adds a `changelog.d/`
  entry but changes nothing under `docs/` gets a warning in CI (not a failure).
- **Missing textures:** `/api/textures/starter` lists the starter set and lets an admin add starter textures to the library
  from the game's files, for an area painted with ones the library lacks. Export names every missing texture at once instead of
  the first ([Paint](docs/handbook/paint.md#missing-textures)).
- **Painted edges without the sawtooth.** Where two textures meet in a baked tile, the bake drew the paint's 0.625 m grid, so a
  trail showed square teeth along both sides. It now smooths each texture's paint over about 1.5 m and draws the edge where the
  smoothed paint changes hands: sharp on a smooth line for laid surfaces (cobbles, flagstones, floors, planks), ragged by noise
  and by the textures themselves for natural ground (gravel poking into snow). Every area gets it on its next export, with no
  repainting; specks a sample or two across, like the mix brush's soft edge, are smoothed away. The texture catalog's new
  `edge` (`crisp` or `broken`; unset, the category decides) picks the style, broken winning where the two differ. Tiles
  beside a change of texture are baked too now (about 3% more hak), the minimap follows the smoothed paint, and every tile
  bakes again once. The design: [#88](https://github.com/plenarius/nwn-landscaper/pull/88); how it works, for the editor to
  match: `src/generator/soft.rs`.
- **Design: soft paint** ([DESIGN-soft-paint.md](docs/DESIGN-soft-paint.md)). Why painted edges show the 0.625 m grid as a
  sawtooth in the game, and the fix measured in the client: smoothed weights at bake time for existing areas, then a second
  texture and weight per sample painted by a soft brush.
- **Pebbles, cobbles, bricks and planks come out a believable size.** The starter set's close-ups repeat their picture inside the
  4 m a texture covers (`repeats` in `src/starter_textures.json`): pebbles and cobblestones, brick and plank floors, old
  planks, fieldstone walls and rippled sand twice each way. Drawn once across 4 m, a pebble was a foot wide and the floor read
  as a photograph of rocks. A library made before is updated when the Landscaper starts, except a texture you changed. See
  [Paint](docs/handbook/paint.md#how-large-a-texture-is-drawn).
- **Walls meet the floor without stripes.** A texture's three projections are blended by the ground's own slope (the gentler
  side's on walkable ground, the steeper side's on a steep face), in the baked tiles and in the game's shader through the
  tile's third texture coordinates. They were blended by the light's normal, which is smoothed over several metres on steep
  ground, so flat floor at a cave wall's foot took the projection from the side and smeared into striped bands, and the
  wall's foot took the one from above and streaked. See [Paint](docs/handbook/paint.md#walls-and-steep-ground).
- **The area list says which files it leaves out.** `/api/areas` has `unlisted` (`{ file, why }`) beside `areas`: each `.json`
  of the areas folder that can't be opened, with the reason (a bad name, broken JSON, no header, or a `name` that isn't its
  file's name), and the server's log says each once. A file copied in by hand used to vanish from Open without a word. A file
  whose `name` differs from its file name is no longer listed under that name, which opened another file or none.
- **A lake's shore is smooth in the game, not a sawtooth.** Since water under 15 cm became dry ground (0.7.0), the water's surface
  stopped at the 0.625 m cells where the shallows began, so its edge stepped along the grid. The surface now runs on over the
  shallows and under the bank, and the ground hides it: the shore is where the two meet. The shallows stay dry to walk on, and
  the halo's and the edge scenery's water follow the same surface.

## 0.7.0

- **Export and Check take the editor's band as the area's halo.** `halo` in the query gives the band's samples a side; the
  heights and paint then cover it, and the water's seeds and the chasm's runs, which the editor gives in the area's own grid,
  move into the larger one. The last export's heights, which objects follow when the ground changes, are cut out of a haloed
  file too.
- **The AppImage builds from pinned tools.** Tauri's AppImage tools and the AppImage runtime that appimagetool puts at the start
  of the file come at fixed versions with checked checksums (`deploy/release/appimage-tools.sh`); before, one tool and the
  runtime came from releases that move with every upstream change. The release checks that Tauri fetched nothing of its own
  and that the AppImage starts with the pinned runtime.
- **Particles baked into the tiles.** An area's `ambience` list (`{ "kind": "mist", "x", "y", "z", "scale", "seed" }`, and
  `bearing` for the kinds that blow or lean) exports as the particle pack's emitters in the tiles that hold them, under the
  tile's `<tile>a` dummy, so night-only kinds come on at dusk and day-only ones go off, and mist and ground fog loop
  (`SetTileAnimationLoops` clears them a tile at a time). A placement outside the area, of a kind the pack lacks, or past 1,500
  live particles an area is skipped with a warning; the report gives the live count (`ambience`). The pack is built into the
  program, so a release needs nothing beside it; `LANDSCAPER_PARTICLES` names another pack's folder.
- **Baked shade doesn't darken local light.** A tile's shadow map multiplied all its light, so a torch, a tile light or a
  glowing mesh in a tree's shade was dimmed with it. It now shades only the area's sun or moon (ambient and diffuse), through
  `ls_lighting`, made at run time from the game's `inc_lighting` as the tile shaders are made from `fslit` and `vslit`. Ground
  nothing else lights looks as before. The shared texture hak is rebuilt once, for the new shaders.
- **No dark blotches on steep banks.** A steep face slanting across the samples' grid, like a river's bank, took each
  sample's slope between its neighbours, which swung from one sample to the next with how the face crossed them. The game's
  sun drew that as soft dark blotches along the face. Steep ground now takes its slope over 1.25 m each way, blended in
  between 15 and 30 degrees; gentler ground keeps its normals as they were. The editor still lights steep ground with each
  sample's own slope.
- **An area can carry a halo: the ground beyond its edge, shown past the edge in the game.** The area format gains `halo`
  (samples of ground around the area, whole tiles, at most 5) and `tagPrefix` (what entrance doors' and object-mode placeables'
  tags start with, for the pieces of a canvas). With a halo, the area's normals, baked shade, chasm rims and cave ceilings read
  the ground across its edge, and its boundary tiles draw the halo's real ground out to 50 m: a vertex every 2.5 m with a
  128 px texture a halo tile, the samples' own detail where a shore or a road's groove runs, the halo's water and its trees,
  fading to the fog colour from 25 m out. Trees 25 m or more out are crossed cards showing pictures of the tree's own model, one
  atlas per tree model. The edge models the client repeats 5 deep are left empty. An area without a halo exports as before.
  The canvas-cut design's trial area (10 by 10 tiles) draws 27,900 triangles past its edge (the trial's T, 32,500).
- **Canvases can be stored on the engine** (step 3 of [the canvas design](docs/DESIGN-canvas-cut.md)): a canvas of up to 128
  by 128 tiles is a folder, `canvases/<name>/`, with a small header and its ground in fixed blocks of 16 by 16 tiles, each one
  file with its heights, paint and chasm mask. A save sends only the blocks that changed, with the version of each it started
  from, and is refused whole when another tab saved any of them since; a delete names the versions of everything the tab
  holds. A block or header that is damaged or can't be read stops whatever needs it, names the file, and is never written
  over; only a block that has no file is flat ground. On a team server a canvas is one item of the builder's workspace, and
  needs an assignment for every existing area it is built from, as an area does. The editor's side comes in a later step.
- **Light shafts cross.** The light shafts' second quad leaned along the model's +y while the first leaned along +x, so a placement
  drew two beams splaying apart. Both now lean along +x around one axis and draw one beam, as the design's two crossed quads
  meant; the pack's test checks that every quad of a kind has the same foot and top.
- **A texture pack for the web demo** (`packs/demo`, with its README and credits). One CC0 picture (ambientCG, Poly Haven)
  for each of the 51 starter textures, under the same ids, so the browser demo paints with no game files: 2.7 MB of WebP,
  under a 3 MB budget a test keeps. `packs/demo/make.py` fetches and fits them, and writes the credits.
- **`landscaper textures init --pictures <folder>`** builds the starter set from pictures named by id instead of the game's
  textures, with the starter list's names, groups and footsteps.
- **`/api/me` says whether the engine has the game** (`"game"`): false when `LANDSCAPER_GAME=none`, as the web demo sets it,
  so the editor's panels that need the game say so.
- **Design: the Landscaper as a desktop app.** `docs/DESIGN-desktop-app.md` plans one signed installer per system that runs the
  engine and shows the editor in a window of its own, with no browser: the engine inside a Tauri shell, the installers, signing
  with today's identities, updates, finding the game, and team servers staying in the browser. A prototype drew the editor's 3D
  view in the Linux, Windows and macOS webviews.
- **The desktop app's installers in each release** (step 4 of `docs/DESIGN-desktop-app.md`): an NSIS installer for Windows
  (for one user, no administrator), a `.dmg` with one universal app for macOS 11 and newer, and an AppImage and a `.deb` for
  Linux, built by Tauri with the editor of the same tag inside. On a version tag Tauri signs them with the identities the program
  already uses (Azure Artifact Signing on Windows; a Developer ID on macOS, notarized, with Apple's ticket stapled to the app and
  the `.dmg`). The release checks each installer by installing it and starting the app (`scripts/app-smoke.sh`): its window must
  load the editor at `/?app` from the engine inside it, a second start must only bring it forward, and the engine must stop with
  the app. The archives with the program alone stay beside the installers. The app's log names each page its window loads.
- **The desktop app** (`app/`, step 2 of `docs/DESIGN-desktop-app.md`): the editor in a window of its own, with the engine inside
  it on 127.0.0.1:8766 (else the next free port up to 8785), opened at `/?app` so the editor knows it is in the app. The engine
  in the app also answers `POST /api/app/open` (`{"what": "data" | "log" | "folders"}`: the data folder, the log, or the
  game-folder page again); `landscaper serve` and team servers never do. Only one copy runs; a second start brings its window
  forward. When it can't find the game, its first page asks for the game's folder and the user folder and keeps them in
  `settings.json`. What it prints goes to `landscaper.log` in the data folder. Links to other sites open in the default
  browser, and downloads go to the Downloads folder. On Linux it turns off WebKitGTK's DMA-BUF renderer under NVIDIA's driver,
  and `LANDSCAPER_WEBVIEW_SAFE=1` turns off compositing as well. Not in the release yet: the installers come with step 4.
- **The engine is also a library** (`src/lib.rs`), for the coming desktop app (`docs/DESIGN-desktop-app.md`, step 1): the
  program is its command line, and `serve_local` serves the editor on this machine as `landscaper serve` does. The command line
  and `serve` work as before.
- **A settings file.** Every setting can also be in `settings.json` in the data folder, for a program started without a shell's
  environment (the Dock, a desktop menu); the environment wins over it. `docs/CONFIGURATION.md` says how ("The settings file").
- **The activity log counts an export's routes as the editor does.** It said "332 routes work", the routes that work in one
  click, for an export whose report and editor said all 400 work, 68 of them in shorter clicks. It now says "400 of 400 routes
  work (68 of them in shorter clicks)".
- **A handbook site.** The documents are now also a site, built with MkDocs and published to GitHub Pages at
  <https://plenarius.github.io/nwn-landscaper/>: a step-by-step handbook for builders (terrain, paint and the mix brush, caves,
  scatter, things, light, ambience, skies, texture packs, export, publish), running a server, the design documents, and a
  showcase page for filmed areas. Pull requests build it with `mkdocs build --strict`, so a dead link fails the check.
- **An import whose note can't be read no longer passes as the builder's own area.** Each import keeps a note of the existing
  area it came from (`source.json`), which is what ties it to the builder's assignment. A note that couldn't be read, or
  didn't parse, was taken as "not an import", so the assignment check was skipped: a builder whose assignment had ended could
  still delete or export the area, and its publish went out as a new area instead of a change to the existing one. Now only
  a missing note means "not an import"; a damaged one stops the action with a message naming the file, and nothing changes.
- **Design: why the ambience trial's keyed emitters never switched.** A tile's keyed emitters switch with its `day`, `night`
  and loop animations only under a dummy named after the tile plus `a`, as in stock tiles. `docs/DESIGN-ambience.md` gives the
  rule baked emitters must follow and when the game switches them.
- **Tile lights.** An export takes `lights` from the editor, two `lightcolor.2da` rows a tile, and writes them as the area's
  `Tile_MainLight1` and `Tile_MainLight2`; without them every tile's lights stay off, as before. The editor's Light tool
  paints them.
- **Baked objects keep their glow.** A baked placeable's meshes keep their self-illumination colour (`selfillumcolor`) in the
  tile, so a glowing crystal or mushroom still glows. Before, baking dropped it.
- **Routes too long for one click are no longer called blocked.** The path check now tries shorter clicks along each route
  that stops short. When they get there, the route goes in the report's new `long_routes` list (with how many clicks) and
  `routes.long` counts it, instead of a `failed_route` warning. NWN's route search gives up when the way is about 20 tiles
  longer than going straight there, so a new 12 by 12 *Descending cave* showed 68 blocked routes though every one is walkable
  in two or three clicks; the game client stops and arrives exactly where the check says. `check` and `export` print the
  count after the routes.
- **The particle pack, first cut** (`packs/particles`, with its README). Eleven kinds for the coming
  Ambience tool: water mist, waterfall and rapids spray, fireflies, dust motes, falling leaves, snow gusts, blowing sand, embers,
  light shafts and ground fog. Each is an ASCII model with a day and a darker night look in `pack.json`, keeps at most 56
  particles alive, and doubles as a placeable that a script can switch off, with a sample `placeables.2da` row. Its four own
  textures are placeholders for an artist. The pack is under CC0-1.0, so any team can ship it in its haks.
- **`/api/particles`** gives the editor the particle pack's `pack.json` (its kinds, their colours and live counts) for the
  Ambience tool: the pack the export bakes from, built into the program, or the folder `LANDSCAPER_PARTICLES` names.
- **Preset skies, the first sky pack.** `cargo run --release --example preset-skies` makes it from your own Real Skies
  download, then `landscaper skies pack` zips it: seven skies, each named for the Sky & fog preset it suits. Golden haze, Dusk,
  Overcast, Starry night and Dawn haze put Real Skies' pictures in other times of day (Overcast's greyed); Thick mist and Swamp
  are gradients painted from their presets' fog and light colours, on a dome of their own. The repository ships none of Real
  Skies' pictures ([docs/PRESET-SKIES.md](docs/PRESET-SKIES.md)).
- **Real Skies for modules.** `cargo run --release --example real-skies` turns the EE hak version of Savant1974's Real Skies,
  which you download yourself, into one hak NWSync accepts: each sky texture a DXT1 DDS of 8,192 by 1,024 (5.6 MB, where the
  originals are 77 MB and NWSync refuses files over 15 MB), and Icy's dawn, dusk and night from Winter_Clear. A step outside
  the Landscaper ([docs/REAL-SKIES.md](docs/REAL-SKIES.md)); the repository ships none of Real Skies' pictures.
- **A strip of shore at a tile corner stays reachable.** Where a lake's edge crossed near a tile corner, the tile was water but
  for a strip about a metre wide, and no path node layout fit it: the strip was cut off, and Check paths showed a blocked route
  into it in about 1 in 20 new 16 by 16 Valleys with a lake. Such a tile, and a tile its layout left an island, now takes a
  one-node layout whose crossings (the points 1 to 9 m along an edge the game tries) all land on the strip; the game puts all
  of a one-node tile in one region, so routes reach the strip. Over 64 new 16 by 16 valleys: 0 blocked routes and 0 closed
  passages (were 3 and 41); other samples' routes are unchanged. `LS_NO_LONE` turns it off.
- **An area with a Landscaper sky publishes with its sky.** Publish, a module's publish and Try on dev send `ls_skies.hak`'s
  files as `skies_files` (a check names them in `skies_manifest`), the package for a module has them in `haks/ls_skies/`, and
  `dev-pack` copies the hak and lists it: the area's sky row is in that hak's `skyboxes.2da`. The service routes give it as
  `GET /api/service/skies`. Areas without one send what they did before.
- **The editor picks skies.** `/api/skies` lists the installed Landscaper skies (with the preset each suits) and the module's own
  `skyboxes.2da` rows; `/api/sky-picture/<sky>` draws a sky as the game's camera sees it, a strip of the horizon or the whole
  panorama around, made from its model and textures once and kept in the sky library's `pictures` folder. An area's light and
  fog can pick a game row too (`"skyRow": 3`), which the export writes as its `SkyBox`; an imported area keeps its sky that way
  (or by its id, when it is an installed sky's row).
- **Dirt is brown, not black.** The starter library's Dirt, the floor Dig and Room paint by default, was the game's
  `tcm02_dirt02`, a near-black stone; it is now `tni01_dirt02`, a brown earth. A library that still has the old Dirt as it
  was made gets the new one when the engine starts, and areas painted with it take it at their next export; a Dirt you
  replaced or changed is left alone.
- **Try the Landscaper in your browser**, nothing to install: `/try/` on the handbook site, with a handbook page,
  [Try it in your browser](docs/handbook/try.md). The docs workflow builds it (`scripts/try-page.sh`) from the newest
  release's `landscaper-wasm.zip` and the editor at the same tag, or builds the engine itself until a release has the zip, and
  a release's run puts the new engine on the site.
- **A texture or sky catalog that can't be read is never written over.** A damaged or unreadable `catalog.json` was read as
  an empty library, and the next texture added or pack installed saved a catalog with only that in it: the library lost
  every other texture or sky. Only a missing catalog starts an empty library now; any other stops the change with a message
  naming the file, and lists of what is installed say so instead. A saved area that can't be read is no longer saved over
  without a backup, and an export no longer starts its entrance ids again when the last export's header doesn't read.
- **The engine builds for the browser.** `scripts/build-wasm.sh` builds it for WebAssembly (WASI) with the new `web` feature, for
  the handbook site's coming "try it" page (`docs/DESIGN-web-demo.md`): no threads, the game's archives read without memory maps
  (Moonglow's patched copy on the maintainer's fork), and `landscaper wasi-request` answering one of the editor's requests. Its
  export is byte for byte the native one. Pull requests build it and check its size (1.8 MB with brotli, at most 3), and a
  release carries it as `landscaper-wasm.zip`. Importing an area isn't in it yet.
- **Design: the Landscaper in a browser.** `docs/DESIGN-web-demo.md` plans a public "try it" page with the editor and its engine
  both in the visitor's browser, and no game files served: what of the engine builds for WebAssembly, measured speed, size and
  memory, what needs the game and what stands in for it, saving, hosting on the handbook site, and the stages to build it.
- **Design: the page in the browser, as built.** `docs/DESIGN-web-demo.md` describes step 4 (in nwn-landscaper-web): how the
  page starts the engine and passes it the editor's requests, why the data folder is copied in and out of OPFS rather than
  opened there, the texture library built on the first visit only, the export's zip, the measured start and export times, and
  what to try in Firefox and Safari.
- **On Windows, a file being replaced no longer fails its reader.** Windows can refuse to open a file ("Access is denied") at
  the moment the Landscaper replaces it, for example the texture catalog while a pack installs, or an area while it saves, so
  the request reading it failed once. Reads of the files the Landscaper replaces now try again for up to a second, and so
  does the replace itself while another program has the file open. Publishing a pack no longer starts a new pack list when
  the old one couldn't be read, which dropped every other pack from it.
- **River banks meet the water cleanly.** 0.6.1 cut steep faces where their paint changes on the lines halfway between two
  samples, so the paint still changed in steps of a sample. On a bank that climbs 1.6 m in one sample, each step stood up the
  face: the bed's mud reached up to the water and the bank's sand down into it, a sawtooth seen through the water. Steep ground
  now takes the texture that weighs most in the same blend as walkable ground, cut where two weigh the same. Along a bank that
  line keeps to one height, under the water. In a 10 by 10 test area with a river, no vertex of the mud is above the water
  (150 before, up to 0.23 m), and the tile models are 3% smaller. The editor still draws steep ground with the nearest sample.

- **Sky packs.** `landscaper skies get`, `list`, `remove-pack` and `pack` install skies the way texture packs install textures
  (docs/SKY-PACKS.md). Each sky takes the first free rows of the module's `skyboxes.2da` from 32 up; an area that names one
  (`"atmosphere": { "sky": "<id>" }`) exports with that row, and with `ls_skies.hak`: the skies' files and the module's table
  with their rows. The export refuses a module whose hak list puts another `skyboxes.2da` above `ls_skies`. Texture and sky
  packs now share one download, check and install.
- **Water too shallow to see is dry ground.** The game draws water under about 15 cm almost clear, so a bank a few
  centimetres under the water looked dry until a footstep rippled it. Ground under less than 15 cm of water now exports dry:
  no water surface, the ground's own footsteps and walkmesh surface. It still carries the flood, so a shallow ford doesn't
  split a river. The editor of the same release draws it the same way.
- **Small bakes don't glitter.** A baked texture under 512 px (`LS_BAKE=128`) took 2 by 2 samples a pixel of textures with
  detail at 1 cm, so the ground speckled. It now reads the library textures halved to its samples' spacing. Bakes of 512 px
  and more are unchanged.
- **An imported area's doors and placeables say their model.** `/api/originals` gives each door and placeable its own
  appearance's model as `model` (null when it names none), so the editor draws it: the toolset often gives a door another look
  than its blueprint's.
- **The editor shows which version is running.** `/api/me` gives the engine's version as `version`, and `landscaper
  --version` and the start line say the same: Cargo.toml's, with `+dev` after it in a build from master after that release
  (when CHANGELOG.md starts with `## Unreleased`).

## 0.6.1

- **Released with the editor's 0.6.1**, which is unchanged. The self-host image builds it (`WEB_REF`).
- **The Windows build is signed.** The release's Windows program has an Authenticode signature from Azure Artifact Signing, so
  SmartScreen and antivirus see a verified publisher.
- **No dark teeth along river banks.** Steep ground took one texture a face, the paint nearest the face's middle, so where the
  paint changed on a slope whole faces of one paint stood in the other: along a river, faces of the bed's mud rose above the
  water out of the bank's sand. Steep faces are now cut where the paint changes, halfway between two samples, so each piece
  takes the nearest sample's texture as the editor draws it. In a 20 by 20 test area with a river, the mud's vertices above the
  water went from 291 (up to 0.47 m) to 130 (up to 0.24 m), for 3.5% more triangles.

## 0.6.0

- **Released with the editor's 0.6.0**: painting a mix of textures, quick buttons that match whole words, and lighter undo,
  browser copy and brush. The self-host image builds it (`WEB_REF`).
- **Release notes from the changelogs.** A release's draft notes are this version's section of the engine's and the editor's
  CHANGELOG.md, then how to get started; a tag without its section here fails the release.
- **An entrance undone and redone between path checks keeps its id.** A check judged entrance ids by the last check's header in
  its scratch folder, not the last export's, so an entrance taken out by an undo and brought back by a redo looked like a
  taken-out entrance's id given to another: the check was refused and Live showed an error. A check now reads the last export's
  header, as the export does. An id an export took out still can't go to a new entrance.

## 0.5.0

- **Released with the editor's 0.5.0**: Get more textures, folding texture categories, the module's hak placeables with Small
  and Large plants, textures that keep their size on slopes, and objects that stand on their base. The self-host image builds it
  (`WEB_REF`). The shared texture hak is rebuilt once, for the new `ls_tp_fs` shader.
- **The module's haks' placeables in the editor.** The editor reads the game with the module's haks: on a team server the hak list the
  review service gives (`GET /api/v1/areas/module`), elsewhere `LANDSCAPER_HAKS` (or `--haks`): hak names in the module's order, or
  `module` for the checkout's `module.ifo.json`. Unset, it reads the game alone, as before. Placeables come from the haks' placeables.2da,
  and their models, textures and blueprints from the haks: a module of 57 haks gives 6,259 models where the game alone gives 1,304.
  `/api/placeables` gives each row's `source` (the hak its model is in, `game`, or null for a model that is nowhere). An export lists
  those haks in the module it builds, and adds the ones a module lacks when it installs an area in it: a placed or baked object from a
  hak, its row, model and textures, is there in the game only with them. With that module's haks the first `/api/placeables` takes 0.16 s
  against 0.10 s (the game opened and the table read) and the server holds 108 MB against 76 MB; the haks are mapped, not read.
- **Plant groups.** `/api/placeable-groups`: the plants among the placeables (by the words of their names), each with its model's height
  and width, and whether it is large (1.5 m tall or more), measured once on first request (0.17 s for that module's 367: 274 small, 93
  large).
- **Textures with a hyphen.** `/api/texture/` and `/api/model/` take a hyphen in a name (`des_cactus-01`), as the game does.
- **Texture packs.** Optional sets of textures a user downloads into the library: `landscaper textures get <id | URL | zip>`,
  `list-packs` and `remove-pack`, and `/api/textures/packs` for the editor's Get more textures dialog (install in the background
  with its progress, remove). A pack is one zip of `pack.json` and the library's own pictures; an install checks its size, the
  index's sha256, every file's sha256, every picture and every path, and writes nothing to the catalog until all of it passed.
  Ids are namespaced by each pack's prefix, and any clash with the library or the starter set is refused. A removal is refused
  while an area paints with the pack's textures. The list of packs is `LANDSCAPER_PACKS_URL` (default: an asset of the
  `texture-packs` release); `landscaper textures pack` makes a pack and its index entry from a list of pictures. On a team
  server only admins install or remove. [docs/TEXTURE-PACKS.md](docs/TEXTURE-PACKS.md).
- **Textures keep their size on every slope.** A library texture on the ground is drawn from three sides at once (from above,
  along x and along y), blended by which way the ground faces: triplanar texturing, in a third shader made from the game's
  `fslit` at run time, `ls_tp_fs`. Steep ground took its texture from the side along x + y, so a slope facing north-east or
  south-west smeared it into streaks; walkable slopes took it from above, 1.6 times stretched at 50 degrees. A tile painted with
  several textures bakes the same blend into its picture. Library meshes now always have a material (MTR), shadowed or not, and
  the edge scenery's too; their own texture coordinates, for viewers without the game's shaders (Moonglow, nwn-bake), give each
  face the one side of eight it faces most. Tiles carry the ground's own normals, so light no longer steps at tile and mesh edges.
  Needs the editor of the same release to show it the same way.
- **The macOS build is signed and notarized.** The release's macOS program is signed with a Developer ID and notarized by Apple,
  so it starts without Privacy & Security's Open Anyway. The Windows build is not signed yet.
- **Running out of open files no longer stops the server for good.** When `landscaper serve` had no file descriptor left for a
  new connection (EMFILE: the Forge's team server under Docker's default limit of 1,024, as every open editor tab reconnected after
  an update), its thread that accepts connections ended. The process ran on and answered no one, and the proxy in front answered
  502, until a restart. Now the server drops the connections it can't take, tries again after a pause of 5 ms, doubled up to 1 s,
  and answers again once connections close. The log says so once a burst ("dropping connections: ..."), at most every 10 s while
  it lasts, and once when it ends ("taking connections again: ..."), where it printed a panic for every connection before.
- **A connection is one open file, not four.** The vendored tiny_http copied each connection's socket three times, and a copy that
  failed for want of a file descriptor panicked that connection's thread. Its halves now share the one socket.
- **The server raises its own limit on open files** (`RLIMIT_NOFILE`) to the hard limit at its start, on Linux and macOS, as
  servers commonly do: the soft limit is often 1,024 (Docker) or 256 (macOS). Under 2,048 it prints a warning that says how to
  raise the hard limit (Docker: `--ulimit nofile=65536:65536`); the self-host `compose.example.yml` sets 65,536. Windows is
  unchanged.

## 0.4.0

- **Chasms.** The editor's chasm (its 0.4.0) comes in the export as runs of samples, the header's new, optional `chasm`.
  Each chasm sample drops 20 m below its rim, the nearest ground that isn't chasm. Its walls fade to black and its floor
  is black, so it reads as bottomless from any camera angle; the edge scenery and the minimap draw it black. Nothing walks
  in a chasm or on its rim (the walkmesh, the path nodes and the path check), and water never fills one. A tile's height
  level leaves its chasm out, so ground beside one keeps the level NWN paths on. An older engine ignores the field and
  exports the ground there as it is. The shared texture hak is rebuilt once, for the new fade texture. The self-host image
  builds the editor's 0.4.0 (`WEB_REF`).

## 0.3.1

- **Exports of imported areas are quicker again.** 0.3.0 shipped an early draft of its skip for the ground no one sees under
  imported cliff tops: it found little of that ground and cost more than it saved. The finished test is here: on a large
  imported area it skips about a third of the shadow maps' texels, and their drawing takes about a tenth less time. What the
  game shows is the same. Released with the editor's 0.3.1, which is unchanged (`WEB_REF`).

## 0.3.0

- **Released with the editor's 0.3.0**: four sample landscapes in New area (a canyon, a snowy mountain pass, a dune desert, a
  valley with a lake), Import heightmap, and the underlay, which needs this engine's `/api/underlay`. The self-host image
  builds that editor (`WEB_REF`).
- **Underlays.** `/api/underlay?name=<area>` keeps the editor's underlay, a picture laid under an area to trace over (a
  battlemat, a scanned map), as `areas/<name>.underlay` beside the working copy: `PUT` (PNG, JPEG, WebP, GIF or BMP, up to
  16 MB, told by its first bytes; anything else is refused with 415), `GET` (answered as the type its bytes are) and `DELETE`.
  It counts towards a team workspace's 5 GB. Deleting the area sets it aside with the working copy; a new area that replaces
  one by its name doesn't take its underlay over. Nothing exports it.
- **Baked objects cast their own shadows.** A tree's shadow was its crown filled down to the ground as a column, cast in
  blocky steps, with no trunk: now trunks, fronds and leaves fall on the ground as they stand, through the gaps in cut-out
  textures, sharper near the ground and softer higher up, with a little shade under and around them. Tiles objects shade get a
  shadow map twice as fine (8 cm a pixel); the others are as before.
  Underground, where there is no sun, objects cast no sun shadows, only the shade under and around them.

## 0.2.4

- **No change to the engine.** Released with the editor's 0.2.4, whose Place list pictures check which way up the browser
  hands them back; the self-host image builds that editor (`WEB_REF`).

## 0.2.3

- **No change to the engine.** Released with the editor's 0.2.3, whose Place list pictures are the right way up in every
  browser; the self-host image builds that editor (`WEB_REF`).

## 0.2.2

- **Export works on Windows.** Every export there stopped with "Access is denied (os error 5)" on the module's `.work` copy:
  the copy was flushed to disk through a read-only handle, which Windows refuses. The tests now run on Windows too.
- **A double-click passes over another program on port 8765.** It used to open whatever was listening there (another local
  tool's page) and say it was the Landscaper. Now it opens a Landscaper already running only when `/healthz` says it is one
  (an `X-Landscaper` header), and otherwise starts on the next free port up to 8784, and says which.
- **Export is faster, and an export again much faster.** Each tile's files (its baked texture, shadow map, model, walkmesh and
  minimap) are made on every core at once, the in-tile crossings check spreads single crossings instead of whole tiles, and an
  import's pictures load side by side. `landscaper serve` keeps the last exports' baked textures, shadow maps and simulated routes by
  what they are made from: exported again, only the tiles whose paint or ground changed are baked again, and the route checks
  run again only when the walkmeshes changed. What an export writes is the same, byte for byte.
- **Export's check for winding tiles is faster.** Each way across a tile stops being searched once it is over the limit
  (200,000 units of work), so the step takes about a second where one bad tile made it 8 (a 13x12 area). The warning says
  "over 200,000" instead of the exact cost (the report's `units` is now `over`, the limit), and its marker is on one of the
  tile's ways over the limit, not always the worst. What an export writes is the same, byte for byte.

- **A trusted service can list and fetch a builder's exports.** With `LANDSCAPER_SERVICE_KEY` set, a team server answers
  read-only `/api/service/*` routes: a builder's exports, one export's area files and new blueprints (as Publish packs them), its
  tile hak and the texture library, streamed with an `ETag`. Each is checked as Publish checks it (the name, the assignment) for
  the builder named, and recorded in the activity log. docs/REVIEW-SERVICE.md, "Fetching exports".
- **Publish sends the export's id** (`export`, and in each area of `publish-module`): a hash of the export's header and its hak,
  the same id the service routes list, so a service can tell whether the tiles it fetched are the ones published.
- **Publish's check is much faster.** It sends the review service the tile and library files' names, git blob ids and sizes
  instead of the files (a review service that wants the files gets them, as before), and a hak's files are read and packed once
  per export: the check, the pull requests and Try on dev reuse them, and a check that passes packs the zips in the background
  for what usually follows. docs/REVIEW-SERVICE.md, "Publishing".
- **The minimap shows what is built on the area.** An export draws its area straight down with nwn-bake, from the hak and module
  it has written (the tiles, the placeables and doors, their own textures), and cuts the picture into each tile's minimap:
  houses, walls, towers and platforms are on it now, not only the ground and water. Where nothing is drawn (a cave's rock) the
  terrain's minimap shows, as it does if the drawing fails. `LS_MINIMAP=terrain` keeps the terrain's alone.
  It is lit by the game's sun, from the north-east as the terrain's baked shadows are, so shadows fall south-west; they are
  drawn half as long, so a wall's or a house's shadow marks it and a 10 m tower's (4.3 m) stays off the next tile. Needs
  nwn-bake 0.7.0 (`--sun`, `--shadow-length`).

## 0.2.1

- **The dev server's full test is gone** from the team server: `/api/dev-test` (status, start and release) and the review
  service's `/api/v1/dev/test` calls it made. Try on dev replaces it; new scripts can't be tried on dev and go through Publish.
  `landscaper dev-pack` stays, as a command of its own. An existing area's waypoints, for a transition to lead to, are now
  `/api/waypoints` (was `/api/dev-test/waypoints`), still from the service's `/api/v1/areas/<resref>/waypoints`.

## 0.2.0

- **Try on dev.** A team server whose review service runs a dev game server can put an area, or its whole module, on dev with
  no restart (`/api/dev-trial`): it sends what Publish sends, checked the same way, always with the shared texture library. The
  builder walks in with `;forge <area>`, keeps the trial longer or releases it. docs/REVIEW-SERVICE.md, "Trying areas on a dev
  server".

## 0.1.2

- **Publishing sends only the files the Landscaper made.** An import copies the textures its cliffs and objects use into
  `<name>.import/tex/` (sometimes an EE material's texture under the plain name), and an export puts them in the area's hak so
  it looks right on its own. Those are the game's and the module's haks' own textures, so a publish and a package now leave
  them out: shipped in a hak under the same name, they would replace the originals in every area.
- `url` 2.5.9, for RUSTSEC-2026-0592.

## 0.1.1

- **Downloads to unzip and start.** Each release has an archive for Windows (x86-64), macOS (one program for Apple silicon and
  Intel) and Linux (x86-64), holding the program, the editor of the same release in `web/`, the licences and a QUICKSTART. No
  Rust, compiler or second repository needed. They are made by `.github/workflows/release.yml` on a version tag, as a draft
  release, with `SHA256SUMS` and the complete source. The builds are not signed (README, "Install").
- **Started with no arguments** (a double-click), the program runs `serve` on 127.0.0.1:8765, opens the editor in the default
  browser and keeps its window open, showing the game and user folder it found and how to stop it. Without a game install it says
  what to set (`NWN_ROOT`, and how on that system) and waits for Enter instead of closing. If the editor is already running, it
  opens the browser at it. On Windows it also finds the user folder in OneDrive's `Documents`. Every command, and `serve` with
  any option, works as before.
- **`serve` says what to set when it finds no game or user folder**, and why a `NWN_ROOT` that is set isn't the game's folder.
- **Windows: the data folder is `%LOCALAPPDATA%\landscaper`** (`XDG_DATA_HOME` still wins). Without a `HOME` it was a
  `.local/share/landscaper` folder under wherever the program was started.
- **The engine builds on Windows.** `--reload-on-rebuild` and the team server's disk quota used Unix-only calls; on Windows the
  first starts the new build in the same window, and the second counts a file with two names twice.

## 0.1.0

The first public release: the editor's engine (`serve`, on your own machine or as a team server), area import and export, the
route check, the texture library with a starter set read from the game's files, and the hidden mark on a team server's exports.
The editor, [nwn-landscaper-web](https://github.com/plenarius/nwn-landscaper-web), is released with it under the same version tag
(README, "Releases"), and records its own changes.

It uses nwn-bake 0.6.1 for reading the game's files and drawing pictures.

### Since the private builds

**If you ran a private build, check these first:**

- **The module layout's default is now `nasher`** (nasher's own layout: everything in `src/`). A module laid out a folder per
  kind (`areas/<group>/`, `core/<ext>/`, `core/nss/`, `module.ifo.json` at the root), which was the default, now needs
  `LANDSCAPER_LAYOUT=folders`. A team server refuses to start with a `LANDSCAPER_LAYOUT` that is neither.
- **`serve` without `--team` refuses an address other machines can reach** (`--bind 0.0.0.0`) unless you add `--allow-remote`.
  On your own machine every request is you, with no sign-in. Team servers are not affected.
- **`X-Forwarded-For` is trusted only on requests that carry the proxy secret.** The activity log records the address a request
  came from otherwise, and no user name from its headers.
- **`LANDSCAPER_PROXY_SECRET` is trimmed**, so an env file with a trailing space or Windows line endings works.
- **Uploads have limits on a team server:** 150 MB a file, 20 files and 500 MB in a workspace, one upload at a time per builder
  and four at once in all. More answers 413 or 429 with the reason.
- **Imports are limited to 32 tiles a side.** A bigger area, or one whose tile list doesn't match its size, is refused before
  the game is opened.
- **Check and export refuse what an area can't hold:** an object, thing or entrance more than 10 m outside the area, an object
  scaled outside 0.1 to 5, a value that isn't a number, or an entrance wider or deeper than the area. The message names it.
  (The editor never saves such an area; this stops a hand-made request using all of the server's memory.)
- **A team server refuses to start when `assignments.json` or `markers.json` doesn't parse**, and names the file. Before, it
  read the file as empty, and the next grant or export wrote it back with one row. While it runs, a damaged file makes the
  routes that need it answer an error instead.
- **`--jobs` (`LANDSCAPER_JOBS`) now limits exports and checks too, not only imports.** When it isn't given, an export works on
  every core (it used to bake textures on up to 8 threads and check routes on every core), and an import or a check on half the
  cores (a check's routes used every core). A team server started with `LANDSCAPER_JOBS` or `--jobs` keeps that limit for its
  exports too. `LS_THREADS` is gone.
- **Imports no longer write `<name>.import.png`** (the whole area at full size: 144 MB for 16 by 15 tiles), **and exports no
  longer write each tile's model as text** (`<tile>.mdl.txt` beside the module: 58 MB for 32 by 32). Nothing read either;
  `LS_DUMP` writes the models' text again. A re-import, or removing the import, clears an old `.import.png` away.
- **A team server's workspaces have a disk quota:** 100 areas and 5 GB each, uploads aside. A save, import, export or check
  that would go over is answered 413, and only the newest 3 `<name>.json.replaced-<seconds>` copies of an area are kept. A
  builder makes room by deleting an area in the editor (below).
- **A slow request is cut off.** Reads and writes on a connection wait 30 s at most; a request's line and headers must come
  within 30 s and 1 MB (414, 431 or 408 otherwise), checked as they arrive; a body must average 16 KB/s after its first 20 s
  (408), and one person's requests may be sending at most 8 bodies at once (429). A connection idle for 30 s is closed. A
  body refused unread is thrown away by its connection, not the thread that answered it. The HTTP server is now tiny_http_dh
  0.12.4, the maintained fork of tiny_http, with a patch of ours, in `vendor/tiny_http`.
- **A request whose body is framed two ways is refused** (400, and the connection closed): a `Transfer-Encoding` other than
  `chunked` alone, or one with a `Content-Length`. tiny_http took any `Transfer-Encoding` for chunked, so behind a proxy that
  framed the body by its length a request could be hidden in another's body and answered without the proxy seeing it (request
  smuggling, CVE-2026-66752; fixed in tiny_http_dh).
- **`days` in a grant is 1 to 36,525** (`POST /api/assignments`); more, or a value that isn't a whole number, is refused.
  Before, a huge value overflowed into a grant that ended within a day, and a value that wasn't a number gave the area for good.
- **Answers on a team server name files relative to the builder's workspace:** the check and export reports' `module` and
  `installed_module` read `exports/...` or `nwn/modules/...`, and errors have the server's folders taken out. An upload import's
  `source` is `<file>:<area>` (on your own machine too).
- **`/api/preview-file` and `/api/preview-icon` answer 502 when the review service fails**, and 404 only when it has no such
  file. A preview is served as the type its name says, never the one the service sent.
- **The self-host image builds the editor of the engine's own version** (`WEB_REF`, default `v` and the version).

**New:**

- **Stable entrance ids.** Each entrance keeps its own id, and its door's tag is `<area>_ent<id>`, so taking one entrance out no
  longer renames the doors of the others. Areas saved before keep every door's tag. The export's header keeps the next id
  (`entranceNextId`), and an export that gives a new entrance the id of one taken out is refused.
- **An entrance taken out takes its door with it.** The next export removes that door from the installed module and lists its
  tag in the report (`updated.doors_removed`). Doors placed by hand in the toolset stay.
- **A blueprint's own settings for the editor:** `GET /api/blueprint-fields?kind=<kind>&resref=<resref>` answers the settings
  the blueprint has (on/off as `1` or `0`; one it doesn't have is left out), or 404.
- **Security headers on every answer:** the editor's Content-Security-Policy (read from its `index.html`) with
  `frame-ancestors 'none'`, `X-Frame-Options: DENY` and `X-Content-Type-Options: nosniff`. `Caddyfile.example` gives Caddy's
  own answers the same without replacing the engine's (docs/TEAM-SERVER.md, "Requests from other sites").
- **A new area never replaces another without asking.** The first save of a new area (`/api/save?new=1`) is answered 409 when
  the workspace already has an area of that name, unless `&replace=1`, which keeps the old copy as
  `<name>.json.replaced-<seconds>`.
- **Two tabs on one area no longer save over each other.** `GET /api/area` answers the working copy's version
  (`X-Landscaper-Version`), a save answers the new one, and a save that sends the version it started from (`&base=`) is answered
  409 (`"moved": true`, with when the other copy was saved) when the area was saved from elsewhere since. Areas saved before
  `created` ids are covered too. `&replace=1` keeps the other copy as `<name>.json.replaced-<seconds>`.
- **The read-only view is held by the engine:** while the `ls_as` cookie names another workspace, everything but reads, `POST
  /api/view-as` and `POST /api/check` is answered 403, whatever sends it. Before, such a request worked in the admin's own
  workspace.
- **A builder can delete an area** (`DELETE /api/area?name=<name>&base=<version>`, the editor's File, Delete area). Its working
  copy and import are set aside in `deleted/<name>-<seconds>/` for an admin to bring back (docs/TEAM-SERVER.md, "Deleted
  areas"); its last export, its build folder and the copy before its last save are removed; the installed module, its hak and
  any assignment stay. The name is free again and no longer counts towards the 100 areas. It takes the area's lock as a save
  does, and is answered 409 when the area was saved since the version the editor holds (`"moved": true`, as a save), or while
  another area uses its import (`used_by`); 404 when there is no such area. On a team server it works in the builder's own
  workspace only, never in the read-only view, refuses an area whose assignment has ended (403), and keeps the newest 3
  deleted copies of a name, which count towards the 5 GB.
- **The export's `palette`, `water`, `entrances` and `atmosphere` go in its body's JSON**, beside the objects, not in its query
  string, which keeps the area's name and its simple settings (`tiles`, `tilesY`, `shadow`, `module`, `kind`, `roof`,
  `clearance`). An export or check that still sends one in the query string (an editor older than this engine) is answered 400
  and told to reload the page. The editor of the same release sends the new form.
- **A re-import takes the area's lock** as a save does: another tab's save at that moment was moved aside unseen, or put the old
  area back over the import.
- **The review service's last area list is kept on disk** (`review-areas.json` in the data folder) and read at start, so a
  restart while the service is down no longer stops builders creating, importing and exporting areas (docs/TEAM-SERVER.md,
  "While the review service is down").
- **Releases** tag the engine and the editor with the same `vX.Y.Z`, and a test keeps the self-host image's `WEB_REF` in step.

**Quicker and lighter:**

- Exports draw the tiles' shadow maps on several threads, as the baked textures are: a 32 by 32 export took 38 s and now
  takes 15 s on 12 threads, with the same hak.
- A painted export holds each baked texture once, not twice (about 170 MB less at 32 by 32), and an import no longer holds the
  whole area's picture (1.45 GB at its peak for 16 by 15 tiles, now 0.84 GB, and half the time).
- An export no longer reads the area's hak into memory to put it back if it fails: it keeps it under a second name instead.
- The area list and the links (`/api/areas`, `/api/links`) read only a few fields of each saved area, not its heights: about
  seven times quicker with 40 areas of 32 by 32 tiles.

**Fixed:**

- An import is refused (409, the same answer as a save's) when the workspace has a working copy of that name. Before, the
  editor then opened the old copy, and its next export took in the import's objects.
- Revoking an area removes only the imports no other assignment of the builder still covers: revoking `bay1` from someone who
  also holds `*` keeps their import of it, and a revoke that ends nothing removes nothing. Before, `area=*` removed every
  import of a module area in the workspace, whatever the builder still held.
- A slow or hung review service no longer makes requests wait in turn: one request fetches the area list, references or hak
  list (15 s timeout, was 30 s) while the others use the copy there was, and a failure is kept 30 seconds instead of being
  retried by every request. With no area list at all, a new area's name is refused ("the module's area list isn't available")
  instead of let through unchecked, and `/api/import-sources` answers 503 instead of an empty list.
- Whether a refusal is logged as `denied`, and whether a missing blueprint or script counts as "none", no longer depend on the
  wording of a message.
- `serve` kept every area plan it had made, so a team server grew by one or more megabytes with each Check and export as people
  edited. It now keeps only the current area's.
- A failed export changes nothing: the module is replaced in one step (the one before is kept as `<module>.mod.bak`), and the
  export's own files and hak are put back. Before, the next export could lose changes to light, fog and settings.
- A failed re-import keeps the earlier import, instead of leaving the area with no kept original.
- One request could stop the engine, without signing in: a `Content-Length` of 100 TB on any request whose body wasn't read
  (one the gate refused, say) made it abort, unable to allocate a buffer of that size to throw the body away.
- A builder could read another builder's blueprints and conversations: the `from` of `POST /api/new-blueprint` and
  `POST /api/dialog`, and the `resref` of `GET /api/dialog?any=1`, took a path. They take a resref only now.
- 32 requests whose bodies stopped arriving held every answer thread of a team server, and a request line of hundreds of MB was
  kept in memory before the gate refused it. (See "A slow request is cut off" above.)
- A preview whose type the review service sent wrong could open as a page on the editor's origin; a type no header can hold
  answered 500, and a file over 64 MB was cut short and kept by the browser for a year. All three are refused now (502).
- Saves of one area take turns, and the area's file is replaced in one step: overlapping saves on a team server could leave only
  the `.bak`.
- The area's hak, the shared hak, the texture library's catalogue and the installed module are written in one step, so a crash
  or a full disk can't leave a cut-off file (a cut-off catalogue used to fail every export on a team server).
- A builder's own blueprint is never shared through the cache with another builder who asks for the same resref.
- An area imported into a folder of its own (`landscaper import <source> <name> <dir>`) now exports from there.
- Uploads are written to disk as they arrive instead of being held in memory, each in a part file of its own.
- `publish-module` sends the group the area's files go in as `kind`, instead of always `core`.
- Error answers and log lines no longer show a service's address or a Discord webhook's token. Discord notices are at most one
  of each kind per person every ten minutes, successful imports included.
- `%XX` in a request is decoded only when both characters are hex digits.
- `textures add` and `textures build` explain their arguments in `--help`.
- An admin's activity log (`GET /api/audit`) skipped February's events from 1 to 3 March: "last month" was taken as 31 days
  back, which is January then. It now reads this calendar month and the one before.

**Other:**

- `src/pathnodes.json`, the measured path node layouts, is under CC0-1.0.
- `GET /api/scripts` no longer answers `canWrite`; the editor stopped reading it.
- The self-host image checks the neverwinter.nim download (`NWN_NIM_SHA256`) before unpacking `nwn_gff`.
- New tests, some needing the game's files: the export, re-export and import cycle, an import into a folder of its own,
  entrance doors, and packages in both module layouts (README, "Development").
