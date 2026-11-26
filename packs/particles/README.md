# The particle pack

Eleven particle kinds for the Ambience tool ([DESIGN-ambience.md, "The particle pack"](../../docs/DESIGN-ambience.md#the-particle-pack)):
mist, spray, fireflies, motes, leaves, snow, sand, embers, light shafts and ground fog. This is the first cut: written as ASCII
models, tuned against stills from the game client, and left for an artist to tune by eye in NeverBlender.

Each kind is used two ways:

- **Baked into the tiles** (the default, step 5 of the design): the export copies the kind's emitter nodes from its model into the
  tile under each placement, and writes the day and night values below into the tile's `day` and `night` animations.
- **As a placeable,** for a kind a script must switch off: the same model, with a sample `placeables.2da` row for a team to merge
  into its own table.

## Files

- `models/<model>.mdl`: one ASCII model a kind, written for this pack. The model's root is the placement's point; emitters sit
  above it. Its static values are the kind's look as a placeable: the night look for fireflies, the day look for the others.
- `pack.json`: the kinds, their day and night values, their live particle counts and budgets, and the textures.
- `placeables.2da`: the sample rows, one a kind.
- `textures/`: the pack's own textures, **placeholders** (below), and `placeholders.py`, which writes them.

`tests/particle_pack.rs` keeps these in step: every model compiles, carries the look `pack.json` names, stays within its budget
by day and by night, and switches off as a placeable.

## The kinds

*Live* is the number of particles one placement keeps alive: each emitter's birth rate times its particles' life, added up. Every
kind stays within the design's budget and under 60 a placement.

| Kind | Model | Shows | Live by day | Live at night | Budget | Derived from | Texture |
|---|---|---|---|---|---|---|---|
| `mist` | `lsp_mist` | always, thicker at night | 18 | 18 | 18 | `tnp_gmist`, `dag_fog_med` | `fxpa_mist` |
| `waterfall` | `lsp_fallspray` | always | 34 | 34 | 34 | `plc_shrine`'s spray and splash | `fxpa_splash`, `fxpa_mist` |
| `rapids` | `lsp_rapids` | always | 6 | 6 | 6 | `plc_j02`'s small fountain | `fxpa_splash` |
| `fireflies` | `lsp_fireflies` | night only | 0 | 18 | 18 | `c_fairy`'s fairy dust | `lspt_firefly` |
| `motes` | `lsp_motes` | day only | 30 | 0 | 30 | `c_drgfaerie`'s fairy dust, slowed | `lspt_motes` |
| `leaves` | `lsp_leaves` | always | 4 | 4 | 4 a tree | `fx_leaf` | `fxpa_leaf` |
| `snow` | `lsp_snowgust` | always | 56 | 56 | 60 | `fx_snow`, blown sideways | `fxpa_dot01` |
| `sand` | `lsp_sandblow` | always | 18 | 18 | 18 | `fx_step_dust`'s tinted smoke | `lspt_sandstrk` |
| `embers` | `lsp_embers` | always | 15 | 15 | 15 | `c_fire`'s sparks | `fxpa_flare` |
| `shafts` | `lsp_lightshaft` | day only | 24 | 0 | 30 | `tnp_gsol1`, as two crossed quads | `lspt_shaft`, `lspt_motes` |
| `groundfog` | `lsp_groundfog` | always, thicker at night | 10 | 10 | 10 | `silm_fog_50_lo`, flat and fewer | `silm_fog_cloud` |

The models are written from scratch: no stock model is copied. *Derived from* names the stock model whose emitter settings
(update, render, blend, texture, sizes, speeds) each kind started from. The stock textures are the game's own, used by name.

- **Direction:** snow and sand blow along the model's +x, and the light shafts lean towards it. A placement's bearing turns them
  downwind, or away from the sun.
- **Mist and spray** float at the placement's height: place them on the water's surface.
- **Leaves** fall from 4 m, under a tree's crown: started higher, they spent their life inside it. **Embers** start 0.6 m up, over
  a fire's flames. **Ground fog** lies flat 1 m up: lower, rolling ground hid most of each 10 m sprite.
- **Birth rates are whole numbers:** in the client, an emitter at 0.5 a second drew nothing. A kind gets a small count from a short
  life instead.

## Day and night

Particles don't take the area's light: the trial's white mist was as bright at midnight as at noon. So each emitter has a day
and a night look in `pack.json`, which the export writes as keys of the tile's `day` and `night` animations:

```json
"emitters": {
  "mist": {
    "day":   { "birthrate": 1.5, "colorStart": [0.92, 0.94, 0.96], "colorEnd": [0.85, 0.87, 0.9], "alphaStart": 0.35, "alphaEnd": 0 },
    "night": { "birthrate": 1.5, "colorStart": [0.3, 0.34, 0.43], "colorEnd": [0.26, 0.3, 0.38], "alphaStart": 0.45, "alphaEnd": 0 }
  }
}
```

- A kind that shows only by day or only at night has a birth rate of 0 in the other. The light shafts' quads also have an
  `alpha` a mesh, 0 at night.
- The names are the ASCII model's controller names. Each look gives the same five values, so a key in one animation always has
  its partner in the other.
- **Glowing kinds keep their colour at night.** Embers and fireflies make their own light, so their night colour isn't darker.
  Every other kind's is.
- On a baked tile the emitters switch by the rule spike 0 found (the design's "Spike 0" section,
  [DESIGN-ambience.md](../../docs/DESIGN-ambience.md)): they hang under the tile's `<tile>a` dummy, and the static values are the
  look the tile starts in.

## `pack.json`

The format the export (step 5) and the editor's Ambience tool (step 7) read. `format` is 1; a change that breaks a reader raises it.

| Field | What it is |
|---|---|
| `id`, `version`, `name`, `description` | The pack |
| `prefix` | `lsp`: every model and texture of the pack starts with it |
| `textures[]` | The pack's own textures: `name` (the resref), `file` (under this folder), `placeholder`, `use` |
| `stock_textures[]` | The game's textures the emitters draw, by resref |
| `kinds[].id` | The kind, as an area's `ambience` list names it (`"kind": "mist"`) |
| `kinds[].name`, `where` | For people: the palette's label, and where the kind belongs |
| `kinds[].model` | The model under `models/`, without `.mdl` |
| `kinds[].shows` | `always`, `day` (off at night) or `night` (off by day) |
| `kinds[].loops` | `true`: a script may switch it for a whole tile, so the export keys it on in `animloop01` and off in `tiledefault`, and `SetTileAnimationLoops` switches it. Mist and ground fog, which a quest might clear |
| `kinds[].budget` | The design's live particle budget for one placement |
| `kinds[].live` | `{ "day": n, "night": n }`: the particles one placement keeps alive in each look, for the editor's count |
| `kinds[].stock` | The stock model it was derived from, and how |
| `kinds[].emitters` | By the emitter's node name in the model: `{ "day": look, "night": look }`, each look `birthrate`, `colorStart`, `colorEnd`, `alphaStart`, `alphaEnd` |
| `kinds[].meshes` | The light shafts only: by mesh node name, `{ "day": { "alpha": 1 }, "night": { "alpha": 0 } }` |

Everything else about an emitter (texture, render, sizes, speeds, spawn area, life) is in its model and the same by day and by
night. Node names repeat between kinds (`spray`, `motes`), so the export renames them when it writes several placements into one
tile.

## The placeables

`placeables.2da` holds one sample row a kind. To use them, a team:

1. Appends the rows to its own `placeables.2da`, numbered after its last row. The game numbers rows by their order in the file,
   so the first column is only a label.
2. Puts `models/*.mdl` and `textures/*.tga` in a hak. The game reads ASCII models as they are; compiling them first (cleanmodels
   or the toolset) makes them load faster.
3. Places them as non-static placeables, so a script can switch them.

Each model has `default`, `on` and `off` animations, as the stock torches have. `off` sets every birth rate to 0 (and the light
shafts' alpha to 0), so `PlayAnimation(ANIMATION_PLACEABLE_DEACTIVATE)` on the placeable stops its particles, and
`ANIMATION_PLACEABLE_ACTIVATE` starts them again. A placeable keeps one look, day or night: a script that wants the other places
the other look's model, or switches the kind off out of its hours.

## Placeholder textures

The pack's own four textures are **placeholders for an artist**, written by `textures/placeholders.py`:

- `lspt_firefly`: a soft glow, four frames of brightness, so a firefly flickers as its frames run.
- `lspt_motes`: four frames of three or four soft specks, so one particle reads as several motes.
- `lspt_sandstrk`: a thin, grainy horizontal wisp.
- `lspt_shaft`: the light shafts' quads, bright at the foot and fading upwards and to the edges, drawn additively
  (`lspt_shaft.txi`).

The design listed three new textures. The fourth, `lspt_shaft`, is there because a mesh is drawn additively only when its
texture's TXI says so, and a TXI for the stock shaft texture (`plc_lightshaft`) would change every stock model that draws it.
In the client, an alpha-blended shaft drew as a flat, even band; the additive one reads as light. Each texture has its shape in
its alpha, and in its colour for the additive ones, so the emitter's colours tint it. Replace a file with one of the same name,
size and frame layout.

## Not measured

**Frame time.** Every still came from the game client on the CPU renderer (llvmpipe), which can't tell what a GPU spends on
particles. The budgets above are particle counts, not milliseconds.
