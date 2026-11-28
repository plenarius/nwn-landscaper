# Texture packs

A texture pack is a set of ready-made textures a user downloads into their texture library, on top of the starter set the
Landscaper builds from the game's own files. The editor's Paint panel lists the packs on offer under **Get more textures…**;
the command line has `landscaper textures get`, `list-packs` and `remove-pack`. A pack's textures go into `ls_textures.hak`
with the rest of the library, so a pack only offers art its makers allow in a hak.

## Install, list and remove

```sh
landscaper textures list-packs                 # installed, then the ones the index offers
landscaper textures get some-pack              # by its id in the index
landscaper textures get ./some-pack-1.0.zip    # or a zip on this machine
landscaper textures get https://example.org/some-pack-1.0.zip
landscaper textures remove-pack some-pack      # refused while an area paints with one of its textures
```

`--index URL` (on `get` and `list-packs`) reads another index than `LANDSCAPER_PACKS_URL`'s. `remove-pack --force` removes a pack
that areas still paint with; those areas don't export until it is installed again, or their paint is replaced. On a team
server, run these inside the container; in the editor only admins install or remove, because the library is the whole team's.

Installing a different version of an installed pack replaces it. A new version that drops a texture an area paints with is
refused, as a removal would be.

## What an install checks

Nothing from a pack reaches the library until all of it has been read and checked, in a folder of its own inside the library
(`.incoming/`, which the server never serves):

- **Size:** a download stops at `LANDSCAPER_PACK_MAX_MB` (default 512 MB), and so does one that says it is bigger. A download
  that stops before the length it said is refused. A picture over 16 MB, a `pack.json` over 4 MB and a pack of more than 4,096
  textures are refused.
- **The index's hash:** a pack installed from the index must have the size and sha256 the index gives.
- **Every file's sha256** against `pack.json`, and every picture decoded: a PNG at most 512 px a side, its swatch at most 128 px.
- **Paths:** every entry of the zip is a plain file name that `pack.json` lists (no folders, no `..`, nothing hidden, no links).
  Anything else in the zip refuses the whole pack.
- **Ids:** see [Ids never clash](#ids-never-clash).

Then the pictures are moved in and the catalog is written in one step: the catalog is the commit point. A pack that fails a
check leaves the library as it was, and a crash in the middle leaves at most pictures no entry names. The shared hak is built
again on the next export, as for any change to the library.

## Ids never clash

A saved area names its textures by id, so an id must mean one picture for good. Nothing is renamed on install; a clash is
refused, with what clashes:

- Every texture id of a pack starts with the pack's `prefix` (2 to 4 lower-case letters and digits, such as `ly`), and no two
  installed packs share a prefix.
- An id is letters and digits, at most 13: with the hak's `ls_` it is the texture's resref, which is 16 characters at most.
- An id already in the library (the starter set's, a team's own, a picked tileset texture) is refused, by id and by its resref
  in the hak (`ls_` and its first 13 letters in lower case). So is any id of the starter set, installed or not.
- `landscaper textures add` doesn't replace a pack's texture either.

## The pack's zip

One zip per pack, holding `pack.json` and, for each texture, the two pictures the library keeps: `<id>.png` (at most 512 px a
side, opaque) and `<id>_128.png` (the palette's swatch, at most 128 px). All at the zip's top level.

```json
{
  "format": 1,
  "id": "example-ground",
  "version": "1.0.0",
  "name": "Example ground",
  "description": "Grass, earth and rock.",
  "licence": "CC-BY-4.0",
  "licence_text": "Optional: the licence's text, when it isn't a well-known one",
  "credits": ["Alice", "Bob"],
  "homepage": "https://example.org/ground",
  "prefix": "eg",
  "textures": [
    {
      "id": "egMossRoad",
      "name": "Mossy road",
      "category": "Earth",
      "surface": 1,
      "files": {
        "render": { "path": "egMossRoad.png", "sha256": "…64 hex digits…" },
        "swatch": { "path": "egMossRoad_128.png", "sha256": "…" }
      }
    }
  ]
}
```

- **`id`:** 1 to 40 lower-case letters, digits and `-`, starting with a letter.
- **`version`:** letters, digits and `.+-`.
- **`licence`:** an SPDX id (`CC-BY-4.0`, `CC0-1.0`) or the licence's short name; the editor shows it with the credits.
- **`category`:** the palette group; a new name makes a new group, after the starter set's.
- **`surface`:** the `surfacemat.2da` row ground painted with it walks as (footsteps): 1 dirt, 3 grass, 4 stone, 5 wood,
  13 mud, 19 snow, 20 sand.
- **`edge`** (optional): how its edge meets another texture's in the game, `crisp` (a sharp line: laid surfaces such as cobbles,
  flagstones, floors and planks), `broken` (a sharp line ragged by noise and by the textures themselves: natural ground) or
  `soft` (a gradient as wide as the brush's soft rim: a wash). Left out, a category naming a floor, wall, cobbles, flagstones,
  paving, planks, wood, bricks, tiles, slabs, carpet, rugs or metal is `crisp` and any other `broken`. Where two textures meet,
  the softer wins (`soft`, then `broken`).

The catalog keeps a record of each installed pack (`"packs"` in `catalog.json`: its fields, the ids it installed, when, and
from where), and each of its textures says `"pack": "<id>"`.

## The index

The list of packs on offer is a small JSON file at `LANDSCAPER_PACKS_URL` (a URL or a file). Unset, it is
`https://github.com/plenarius/nwn-landscaper/releases/download/texture-packs/index.json`, an asset of the `texture-packs` release;
pointing the setting at another server (a CDN) moves everything, since each pack's `url` may be relative to the index.

```json
{
  "format": 1,
  "packs": [
    {
      "id": "example-ground",
      "version": "1.0.0",
      "name": "Example ground",
      "description": "Grass, earth and rock.",
      "licence": "CC-BY-4.0",
      "credits": ["Alice", "Bob"],
      "url": "example-ground-1.0.0.zip",
      "size": 3612345,
      "sha256": "…the zip's sha256…",
      "textures": 40,
      "categories": ["Earth", "Grass", "Rock"],
      "swatches": ["data:image/png;base64,…"]
    }
  ]
}
```

[Sky packs](SKY-PACKS.md) are in the same index, in a list of their own (`skies`).

`swatches` are a few 64 px pictures, one a category, carried in the index as `data:` URIs so the dialog shows them without
loading anything from another site. An entry without a `url` or a valid `sha256` is skipped. The server keeps the index it read
for 5 minutes; the editor's dialog asks for a fresh one when it opens.

## Making a pack

`landscaper textures pack` makes the zip from a list of pictures, fitting and flattening each one as `landscaper textures add`
does, and adds the pack to an index file:

```sh
landscaper textures pack ground.json --out dist --index dist/index.json
```

```json
{
  "id": "example-ground", "version": "1.0.0", "name": "Example ground", "description": "Grass, earth and rock.",
  "licence": "CC-BY-4.0", "credits": ["Alice", "Bob"], "prefix": "eg",
  "textures": [
    { "id": "egMossRoad", "name": "Mossy road", "category": "Earth", "surface": 1, "picture": "pictures/moss_road.tga" }
  ]
}
```

`picture` is a PNG, TGA or DDS file, relative to the list; `edge` is as in `pack.json`; `surface` may be left out (the category's
name suggests one, as for `textures build`). The list is checked as an install checks `pack.json`, so a pack that builds also
installs. The zip is `<out>/<id>-<version>.zip`, and the same list makes the same bytes. With `--index`, the pack's entry is
added to that file (made if missing) or replaces the entry of the same id, its `url` the zip's file name beside the index.

To publish: upload the zip and `index.json` side by side (release assets of one release, or one folder of a CDN), then point
`LANDSCAPER_PACKS_URL` at the index if it isn't the default.

## The HTTP API

The editor's calls (with the `X-Landscaper: 1` header, like every API call):

- **`GET /api/textures/packs`** (`?fresh=1` reads the index again): `{ installed, available, index_error, manage, job }`.
  `installed` are the catalog's records; `available` the index's entries without their `url`, each with `installed` (the
  installed version, or null); `manage` whether this user may install and remove.
- **`POST /api/textures/packs/install?id=<id>`** (admins): starts installing the index's pack `id` and answers 202 with the job;
  409 while another install runs.
- **`GET /api/textures/packs/job`**: the install under way or last finished: `{ job, pack, step, done, total, running, error,
  result }`, `step` being `downloading` (bytes), `checking` and `installing` (files), then `done` or `failed`.
- **`POST /api/textures/packs/remove?id=<id>`** (admins): `{ removed, name, textures }`, or 409 with `{ error, areas }` while
  areas paint with its textures (`user: area` on a team server).

- **`GET /api/textures/starter`**: `{ textures, add }`, the starter set (`[{ id, name, category, resref }]`) and whether this user
  may add from it (an admin, with the game's files). The editor uses it for an area painted with textures the library lacks.
- **`POST /api/textures/starter?ids=<id>,<id>`** (admins): makes those starter textures from the game's files, as `textures add`
  would with their lines of `src/starter_textures.json`, and answers `{ added, kept, problems }`. A library texture of that id
  is kept as it is.

On a team server, installs and removals go into the activity log (`texture_pack`), and starter textures added (`texture_starter`).
