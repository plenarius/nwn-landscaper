# Design: normal maps for the ground

Status: design, not built. Written 2026-11-27 for the maintainer's review before any code. The spike behind it is on the branch
[`spike/normal-maps`](https://github.com/plenarius/nwn-landscaper/tree/spike/normal-maps) (not for merge).

Pull request [#96](https://github.com/plenarius/nwn-landscaper/pull/96) gave the ground relief from each texture's brightness,
worked out in the shader. Its ceiling was low: it hardly showed under the midday sun, it read as an emboss filter when made
stronger, and pale stains bulged like stones. It was closed in favour of real normal maps for the textures that need them. This
design says where those normal maps come from, how the export carries them, what they cost in the haks players download, and how
they look in the game client.

Words used here:

- **Normal map:** a picture whose pixels are directions, not colours: which way the surface faces at that point. The game's light
  reads it in place of the flat ground's own direction, so stones get a lit side and a shaded side.
- **Library texture:** a texture of the Landscaper's library (the starter set, a pack's), written once into the shared
  `ls_textures.hak` as a 512 px picture covering 4 m.
- **Baked tile:** a tile painted with more than one texture. It gets its own 512 px picture over its 10 m (2 cm a pixel) in the
  area's hak. A tile in one texture draws the library texture itself, from three sides by the slope (triplanar).
- **Shadow map:** each tile's baked shade (`texture7` of its material), a small picture over the tile.

## The short version

- **The game supports it, but our tiles need our own shader code.** A material's `texture1` is its normal map in the game's stock
  shaders, and the client computes tangents for it from the mesh's first texture coordinates. Our tiles don't use those
  coordinates for their look (triplanar library textures, world-aligned baked pictures), so the Landscaper's own shaders read
  `texture1` themselves, as they already read `texture7`, and tilt the normal in world space. No stock tile has a normal map: the
  base game ships 7 materials and none has a `texture1`.
- **A normal picture per baked tile, and one per library texture for single-texture tiles** (decided). A baked tile gets its own
  normal picture beside its colour picture, exact wherever textures meet. It adds 0 to 78 MB to each showcase area's hak (table
  below); the maintainer chose that over a shared normal map per texture, which would add nothing to an area's hak.
- **Where they come from.** The starter set's stock pictures have no normal maps, so they are made once from a height made from
  the picture, band-passed at the scale of a stone (not texel by texel, as #96 did), and, for laid stone, from the joints, then
  tuned by hand and shipped with the Landscaper (decided; STYLE.md's rule on the game's files is relaxed to allow it). The demo
  pack's photographs (Poly Haven, ambientCG) all ship real normal maps: take those.
- **Cost:** 0.35 MB on the shared texture hak per texture given one (512 px, DXT5 with the normal in alpha and green). Pebbles,
  Cobblestones and Rock: 1.05 MB. Every stony texture (27): 9.4 MB. All 51: 17.8 MB. A texture that repeats its picture twice can
  store one repeat: a quarter of that.
- **How it looks:** with the demo pack's measured normal maps, fit to show at the default setting, by day and by torchlight. With
  normal maps generated from the stock pictures, a real but modest gain on Pebbles, little on Cobblestones and Rock until their
  heights get better. The shader setting High Quality looks the same as the default.
- **Lit per vertex** (the game's "Minimal" shader setting) nothing changes, as with #96.

## What the game does

The game's shaders are text in its data (`fslit.shd`, `fslit_nm.shd`, the `inc_*.shd` includes); the Landscaper already makes its
own from `fslit` and `vslit` at run time (`src/shared.rs`, `shaders`).

- **Material slots.** In `inc_common.shd`, a shader compiled with `NORMAL_MAP 1` reads `texture1` as a tangent-space normal map,
  `SPECULAR_MAP` `texture2`, `ROUGHNESS_MAP` `texture3`, `HEIGHT_MAP` `texture4` (parallax, `inc_displacement.shd`) and
  `SELF_ILLUMINATION_MAP` `texture5`. The stock pair for that is `vslit_nm` / `fslit_nm`.
- **Tangents.** The vertex shader takes a tangent and a handedness per vertex (`inc_transform.shd`). The client computes them at
  load for a mesh whose material binds a normal map, from the mesh's first texture coordinates (nwn-mdl-spec `MDL-MESH-042` to
  `045`).
- **The stock tiles.** None uses a normal map. The base game's materials are seven: two heads, a GUI button and four ice tiles
  (`tti01_*`, which set specularity only). Its only `_n` pictures are three NUI widgets.
- **The shader setting** (Options, Graphics: Minimal, Performance, High Quality) sets `SHADER_QUALITY_MODE` 0, 1 or 2
  (`inc_config.shd`).
  - **Minimal (0)** forces lighting per vertex (`FRAGMENT_LIGHTING 0`). A stock normal-mapped material still lights per fragment
    there (`NORMAL_MAP` turns on `FRAGMENT_NORMAL`); ours, built on `fslit`, does not.
  - **Performance (1)**, the default, lights per fragment: a tilted normal changes the light. Specular Fresnel is simplified.
  - **High Quality (2)** adds light softening, Schlick's geometric shadowing, full Fresnel and, for stock normal-mapped surfaces,
    a fade of each light by the surface's own (untilted) facing, so a bump can't light up on a face turned away from the light
    (`inc_lighting.shd`, line 336). Ours, built on `fslit`, doesn't have that term.
- **Formats.** The client reads TGA and BioWare's DDS: DXT1 (3 channels, 0.5 byte a pixel) or DXT5 (4 channels, 1 byte a pixel).
  Whether it reads a standard DDS in BC5 (ATI2, the two-channel format made for normal maps) wasn't checked; it isn't needed: a
  DXT5 with the normal's x in alpha and y in green ("DXT5nm") has the same size and nearly BC5's quality, and our shader decodes
  it (z from x and y). The client loaded the spike's DXT5nm BioWare DDS files as they are.

### Why our own shader and not `fslit_nm`

- A tile in a library texture is drawn triplanar: its colour comes from three projections of the world position, not from the
  mesh's texture coordinates (`ls_tp_fs`). The client's tangents follow those coordinates (the box projection for viewers without
  shaders, `src/triplanar.rs`), not the three projections: on slopes the tilt would point the wrong way.
- A baked tile's picture is world-aligned too, but its normal comes from a shared library picture repeating every 4 m, not from
  the tile's own coordinates.
- So the spike reads `texture1` in `ls_tp_fs` and in a baked tile's shader, decodes it, and tilts the fragment's normal in world
  space: on the projection from above the normal map's x is world east and its y north; the two side projections swap axes
  (whiteout blend). The normal is turned back to view space before the stock lighting reads it, exactly where #96 put its tilt. No
  tangent stream, no mesh change.

## Where the normal maps come from

### The starter set: generated on the builder's machine

The stock game has none, and no stock tileset texture is a photograph with a height to recover. Three ways were weighed:

1. **Generated from the picture's own height** (chosen). The spike's generator (`spike/normal-maps`,
   `research/normal-maps/normals.py`) takes the picture's brightness, removes everything larger than an eighth of the picture
   (lighting gradients and stains), evens its contrast so a dark stone bulges as much as a pale one, flattens extremes (glints),
   and keeps mostly the bands a stone wide (blurred 2.5 to 7 px), little of the texel-scale grain that made #96 look embossed.
   Normals are its slope, wrapped at the picture's edges so it still tiles. Strength is set per texture.
2. **Joints for laid stone** (cobbles, flagstones, bricks, fieldstone walls; not in the spike). Their joints are dark lines:
   threshold them, and a stone's height is its distance from the nearest joint, capped. Every stone is then a dome or a flat top
   with rounded edges, whatever its stains, which is where brightness fails most.
3. **Drawn by hand** for the one or two textures where both fail, with a desktop tool (Materialize, AwesomeBump, Laigter) doing
   the same as 1 with sliders. Not for the starter set: see the next point.

**Made on the builder's machine, never checked in.** A normal map made from a stock picture is made from the game's file, and
`STYLE.md` ("Never in these repositories") keeps those out of this repository, as it keeps the starter pictures themselves out:
`textures init` reads them from the builder's game. So the generator runs there too, in Rust, when the starter library is built,
with each texture's settings (method, strength, scales) in `src/starter_textures.json`. Its output goes in the library beside the
picture (`<id>_n.png`), like the picture's swatch. The settings are ours; the pictures stay the game's.

How good is a height made from brightness? Picture 1 runs the same generator on the demo pack's photographs, whose sources also
give their real normal maps, and lights both from a low western sun. (The stock pictures themselves can't be shown here, for the
same reason; the client stills below show them in the game.)

![Picture 1: three demo pack photographs (Pebbles, Cobblestones, Rock), flat; lit by a normal map the spike's generator made from
the photograph; lit by the photograph's source's own normal map.](normal-maps-1-sources.webp)

The generated pebbles take shape: lit tops, shaded undersides. The cobbles' joints sink, but each stone stays almost flat: that is
what method 2 is for. Rock gains little: its picture is colour and streaks, not shapes. Every real normal map is stronger and
sharper: the generator is a stand-in for a height that was measured, and it shows.

### The demo pack: the sources' own

Every photograph in the demo pack (`packs/demo`) comes from Poly Haven or ambientCG, and all 49 of them ship a normal map at the
same resolution (Poly Haven's `nor_gl`, ambientCG's `NormalGL`): measured on their APIs. Two of the pack's 51 pictures are made
from others: Dry grass takes Medium grass's normal map, Ice none. `make.py` fetches the normal map beside the colour, repeats and
fits it the same way, and writes `pictures/<id>_n.webp`.

- **Orientation.** Both are "OpenGL" normal maps: green points up the picture. The game stores pictures rows bottom first, so a
  normal map written the way the colour is written keeps its sense; one flipped must have its green negated. A test checks one
  known slope.
- **Size.** The three spike textures' normal maps are 297 KB as WebP at the pack's quality 82, against 186 KB for their colours.
  All 49 would be about 4.8 MB more, past the pack's 3 MB budget (now 2.7 MB): a decision below.
- **Credit.** The pack's `CREDITS.md` already names each source; the line says the normal map comes from the same.

### Texture packs

A pack's list (`docs/TEXTURE-PACKS.md`) gets an optional `normal` beside `picture`: a PNG, TGA or DDS normal map, same rules (512
px at most). An install checks it as it checks the picture. A texture without one has none, as now.

## How baked tiles carry them

### Chosen: the library's normal map, shared, with a share per pixel

A baked tile's material gets `texture1` naming the normal map of the library texture with the most relief in the tile, sampled at
the world position as the bake drew that texture's colour (from above, every 4 m), so the stones in the normal map lie under the
stones in the baked picture. The shadow map's blue channel says how much of that texture each pixel holds, as #96 already writes
it (`relief_shares`; the shade moved to green). A pebble path across grass then has relief on the pebbles only, fading with the
paint's edge.

- **Cost per area: nothing.** The normal maps are in `ls_textures.hak` once.
- **Two textures a tile:** the shadow map's red is spare too (#96 reads its green only), so a second relief texture can have
  `texture2` and red. Three relief textures in one tile keep the two that cover most.
- **The shadow map is DXT1**, which packs red, green and blue together: unrelated data in each channel costs some precision in the
  shade. #96's stills showed no banding with blue; a second share in red needs checking in the client.

### Not chosen: a normal picture per baked tile

The bake would draw each texture's normal as it draws its colour, blended by the paint, into a second 512 px picture beside the
colour one: exact for any number of textures and for soft edges. Its cost was measured on the five showcase areas (their copies in
the director-line session), exported with master, counting each baked tile and the textures its samples hold:

| Area | Tiles | Baked | Area hak now | 512 px DXT5, every baked tile | Only tiles with a stony texture | Only Pebbles, Cobblestones, Rock |
|---|---|---|---|---|---|---|
| Red canyon | 192 | 74 | 55.6 MB | +25.9 MB | +24.8 MB | +15.0 MB |
| The descent (cave) | 120 | 120 | 37.7 MB | +41.9 MB | +41.9 MB | +41.9 MB |
| Dunes | 256 | 171 | 64.8 MB | +59.8 MB | 0 | 0 |
| Snowy pass | 224 | 224 | 72.6 MB | +78.3 MB | +40.2 MB | 0 |
| Lake valley | 256 | 154 | 111.3 MB | +53.8 MB | +7.3 MB | 0 |

One normal picture is 349,572 bytes at 512 px in DXT5 with its mip levels, 174,796 in DXT1, 87,428 at 256 px in DXT5 and 43,724 at
256 px in DXT1: halve or quarter the columns accordingly. Even at 256 px DXT1 and only on stony tiles, the cave adds 5.2 MB and
the pass 5.0 MB, and 256 px over 10 m is 4 cm a pixel, coarser than a pebble. The earlier estimate (21 to 42 MB an area) is the
cave's row at DXT1 and DXT5.

### Sizes and formats

- **512 px, DXT5nm**, mip levels down to 1 px as the colour has (the mip levels average the normals, which flattens the relief
  with distance by itself: the spike has no fade by distance, as #96 needed, and no far-off blotches showed). 0.35 MB a texture.
- **One repeat only for textures that repeat** (`repeats: 2` in the starter list: Pebbles, Cobblestones, Brick floor, Plank floor,
  Old planks, Soft sand, Dune sand and both fieldstone walls). Their library picture is the stock picture twice each way; the
  normal map can be one repeat at 256 px, sampled at twice the frequency, with the same detail: 0.09 MB. The colour could do the
  same later.
- **DXT1** (half the size) was not chosen: x and y would share one pair of 5-6-5 colour endpoints and four levels a block, so a
  smooth dome steps. DXT5 gives x its own 8-bit alpha block.
- **256 px for everything** would quarter the cost but loses pebble detail (1.6 cm a pixel against 0.8).

## What the client showed

**How it was shot.** The game client off screen on Mesa's software renderer, as for #96 and the soft paint design: the cave sample
(`lsdescent`, pebbles and rock, torchlight from lamps) and the test hillside (`lspath`, a pebble path and a cobbled square across
grass, the module's sun at 13:00 and at 18:00). Both exported with #96's branch, which already writes the materials' relief and
the shadow maps' shares; then `research/normal-maps/post.py` adds the three normal maps to `ls_textures.hak`, puts `texture1` on
each material of Pebbles, Cobblestones or Rock, and swaps #96's brightness relief in the two shaders for the normal map.
**Before** is the same export with the `texture1` lines taken out, so the two differ by the normal maps alone. Every still is at
full size (crops, no scaling), at the default shader setting (Performance) and at High Quality.

**Midday (picture 2).** Each pebble gets a lit and a shaded side under the high sun, which #96 couldn't do ("the path and cobbles
look as before"). The path comes out 2% darker (mean brightness of the crop 83.7 against 85.3; 2.5% at High Quality): a tilted
normal faces the sun a little less on average. The cobbles hardly change (61.7 against 62.3 in their close-up): their generated
normal map is weak (picture 1). High Quality looks the same as Performance here.

![Picture 2: the pebble path on the test hillside at 13:00, flat and with the generated normal map, at Performance and at High
Quality. Crops of the client's 1280 by 720 stills, not scaled.](normal-maps-2-midday.webp)

**Torchlight (picture 3).** In the cave, by the lamps, the pebbles get shaded undersides and lose the look of flat discs. It also
takes some contrast out of the brightest stones, and in places reads soft rather than sharper: the generated height smooths each
stone into a dome. The crop is 3% darker at Performance, 4% at High Quality.

![Picture 3: the cave's pebble floor by lamplight, flat and with the generated normal map, at Performance and at High
Quality.](normal-maps-3-torch.webp)

**Dusk and strength (picture 4).** At 18:00 the light is low and weak, and the path changes little (2.5% darker); the cliffs in
the hillside are Cliff rock, which has no normal map here, and didn't change. Made 2.5 times as strong, the generated normal map
darkens the cave's floor by 11.5% and turns it gritty: the domes it guessed become ridges. Strength doesn't make up for a guessed
height.

![Picture 4: top, the pebble path at 18:00, flat and with the generated normal map; bottom, the cave by lamplight with the
generated normal map as made and 2.5 times as strong.](normal-maps-4-dusk-strength.webp)

**The demo pack's photographs with their own normal maps (picture 5).** The same cave and hillside exported with the demo pack as
the texture library (`textures init --pictures`), the three photographs' sources' normal maps wired in the same way. This is the
ceiling: a height that was measured, not guessed. In the cave the pebbles are round, each with its lit top and its shadowed foot,
and the floor reads as a bed of stones, not a picture of one; nothing looks embossed. On the path at midday the gaps between
stones darken and the stones stand out. Darker by 3% on the path and 4.5% in the cave.

![Picture 5: the demo pack's photographs, flat and with their sources' own normal maps: the pebble path at 13:00 and the cave by
lamplight, at Performance.](normal-maps-5-photos.webp)

**Verdict.**

- **Fit to show with measured heights** (picture 5): at the default setting, by day and by torchlight. That's the demo pack today,
  and any pack that ships its normal maps.
- **Generated from the stock pictures: an improvement, not a showpiece.** Pebbles gain real shading at midday and by torchlight
  with nothing embossed at the strength made; Cobblestones barely change until method 2 gives their stones shape; Rock is not
  worth its bytes yet.
- **Performance and High Quality look alike.** The stock High Quality term that fades a bump on a face turned from the light
  doesn't apply to our shader; nothing in the stills calls for it. Minimal shows none of it.
- **The ground gets 2 to 4.5% darker** where it has a normal map, because the normal is made unit length again after the tilt. #96
  left it unnormalised to keep the brightness; the build should do that, or scale by the texture's mean facing, so painting a
  normal map on doesn't dim the path.

## The editor's view

The editor ([nwn-landscaper-web](https://github.com/plenarius/nwn-landscaper-web)) draws the ground with three.js (`js/view.js`,
`splatMaterial`): a Lambert material whose fragment shader takes each library texture from a `sampler2DArray` (`uTex`, one layer a
palette index), blended by the soft paint and drawn from three sides (`js/triplanar.js`). It lights per fragment, by the area's
sun and the tiles' lights (`LIGHTS_FRAGMENT`, from the normal `vNormalArea`).

- **A second array, `uNorm`**, one layer a palette index (a flat layer for a texture without a normal map), filled from the
  library's normal pictures, which the engine already serves beside the colour ones.
- **In the fragment shader**, after the colour: the same triplanar sampling of `uNorm`, decoded and blended into the normal as the
  game's shader does, then written into three.js's `normal` before its lights and into the normal the tile lights read.
- **The same choice as the export:** on walkable ground the view uses the one or two relief textures the bake would pick for the
  tile and their shares, so the view never shows relief the game won't. On steep ground, the face's one texture.
- **A toggle** in the view settings, off on slow machines: one more texture read per projection.
- **Parity:** a test renders a flat patch of pebbles under a set light in the view and compares its brightness at fixed points
  with the same patch shaded by the engine's formula.

## Build order

Each step is one pull request in each repository it touches. Agent time is wall-clock for a coding agent; the maintainer's time is
review and testing.

| # | Step | Repository | Agent | Maintainer | Client |
|---|---|---|---|---|---|
| 0 | Spike: generated normal maps for three textures, wired by hand into an export, the client stills above. **Done** (`spike/normal-maps`) | engine | 2 h | 20 min (this design) | no |
| 1 | The library's normal maps: a texture's optional normal picture (starter list, pack lists, the catalog), `ln_<id>` DDS in `ls_textures.hak`, `texture1` on library tiles, the triplanar normal in `ls_tp_fs`, tests | engine | 6 h | 45 min | 15 min: a pebble path, a cobbled square |
| 2 | Baked tiles: a normal picture per baked tile beside its colour picture (blended from its textures' normal maps as the colour is), `texture1` on the tile, the baked shader, the bake cache's version | engine | 5 h | 30 min | 15 min: the cave |
| 3 | The starter set's normal maps, made once (the generator: height band-passed; joints for laid stone), tuned by hand with stills and shipped in the repository; libraries made before get theirs on the next start. Needs STYLE.md relaxed first (nwn-bake, then each repository's pin) | engine | 6 h | 1 h: judging the stills | 20 min |
| 4 | The demo pack's: `make.py` fetches and fits them, the budget, the web demo installs them | engine | 2 h | 15 min | no |
| 5 | The editor's view: `uNorm`, the shader, the toggle, the parity test | web | 5 h | 30 min | no |
| 6 | Docs: handbook (Paint, Light), `docs/TEXTURE-PACKS.md` (`normal`), changelog | both | 1.5 h | 15 min | no |

About 25.5 hours of agent time beyond the spike and 3.5 hours of the maintainer's, about 1 hour of it in the client. Steps 1 and 2
need each other to look right (a path's library tiles and its baked tiles side by side) and ship in one release with 5. Step 3 can
start now; 4 is independent.

## Decisions

Each gives the recommendation first and the options not chosen after it.

### 1. Shared normal maps, none per baked tile

**Decided (2026-11-27): a normal picture per baked tile**, the option below, for exact relief wherever textures meet; the area
hak cost is accepted.

**Recommended: one normal map per library texture in `ls_textures.hak`, and a share per pixel in each baked tile's shadow map**
(two relief textures a tile). Nothing more in any area's hak.

Considered and not chosen:

- **A normal picture per baked tile:** exact everywhere, but 0 to 78 MB more per showcase area at 512 px (table above), 5 MB even
  at 256 px DXT1 on stony tiles only.
- **Per baked tile at a lower size** (256 px DXT1): 4 cm a pixel, coarser than the pebbles it is meant to shape.

### 2. Which textures get one (the download size)

**Decided (2026-11-27): as recommended.**

**Recommended: the ten gravel and laid-stone textures first** (Pebbles, Grey gravel, Muddy gravel, Cobblestones, Flagstones, Stone
floor, Floor slabs, Brick floor, both fieldstone walls): **2.2 MB** on `ls_textures.hak`, once for every area, with the five that
repeat stored as one repeat. Rock, cliffs, cracked earth and the rest wait until their heights are better than brightness gives
(picture 1, Rock).

Considered and not chosen:

- **Pebbles, Cobblestones and Rock only:** 0.5 MB, but Rock barely changes and the other gravels and floors stay flat beside the
  pebbles.
- **Every stony texture (27):** 8.1 MB with the repeat saving, for many that change little.
- **All 51:** 15.5 MB; grass, sand and snow gain nothing a player would see.

### 3. The starter set's source

**Decided (2026-11-27): made once, tuned (cobble joints drawn in) and shipped** with the Landscaper. STYLE.md's "never the
game's files" rule is relaxed for it, in nwn-bake's master copy and every repository that pins it.

**Recommended: generated on the builder's machine** at `textures init`, by method 1 for gravel and method 2 (joints) for laid
stone, with settings per texture in the starter list. Nothing from the game goes in the repository.

Considered and not chosen:

- **Photographs with real normal maps in place of the stock pictures** (the demo pack's): the strongest relief, but the starter
  set would stop looking like the game's own tiles. Builders who want that look can install a texture pack of photographs that
  ships its normal maps (a pack list's `normal`).
- **Hand-made normal maps:** can't be shipped (made from the game's pictures) and can't be redone on each machine.

### 4. The demo pack's budget

**Decided (2026-11-27): as recommended.**

**Recommended: normal maps for the same ten textures, and the budget from 3 MB to 4 MB** (about 1 MB more as WebP).

Considered and not chosen:

- **All 49:** about 4.8 MB more, nearly tripling the pictures a first visit downloads.
- **None:** the web demo would show flat ground where the game has relief.

### 5. How strong

**Decided (2026-11-27): as recommended.**

**Recommended: as made, with the brightness kept** (see the verdict), and no strength slider for builders at first: a strength per
texture in the starter list, tuned once in the client.

Considered and not chosen:

- **Stronger generated normal maps** (picture 4): darker and gritty, the emboss look #96 hit.
- **A strength per area or per paint stroke:** another setting to learn for a change most builders won't see.

## What this design could not check

- **A real GPU.** The stills come from the client on Mesa's software renderer, with its own choice of mip level. A GPU picks mip
  levels with anisotropic filtering, which keeps the relief sharper at grazing angles.
- **BC5.** Whether the client reads a standard DDS in BC5 wasn't tried; DXT5nm needs no support from it.
- **The second share in red.** The spike carries one relief texture a baked tile.
- **The editor.** Not built; its cost is estimated from its shader.
- **Joint-based heights** (method 2): not built.
- **Other textures and areas in the client:** only Pebbles, Cobblestones and Rock, in the cave and the test hillside. High Quality
  at dusk wasn't shot.
