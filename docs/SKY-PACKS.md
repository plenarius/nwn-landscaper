# Sky packs

A sky pack adds skies to the Landscaper: new rows of `skyboxes.2da`, each with its four models (dawn, day, dusk, night) and
their textures. It is a sibling of [texture packs](TEXTURE-PACKS.md): the same index, download, sha256 checks and install in one
step, with its own checks, its own library (`skies` in the data folder) and its own hak, `ls_skies.hak`. The design is
[DESIGN-ambience.md](DESIGN-ambience.md), question 4.

## Install, list and remove

```sh
landscaper skies list                      # installed skies with their rows, then the packs the index offers
landscaper skies get some-skies            # by its id in the index
landscaper skies get ./some-skies-1.0.zip  # or a zip on this machine, or a URL
landscaper skies remove-pack some-skies    # refused while an area uses one of its skies
```

`--index URL` and `remove-pack --force` work as for texture packs. The install reads the module's `skyboxes.2da`, so run it
with the module's haks set (`--haks`, `LANDSCAPER_HAKS` or a team server's review service), as the editor runs.

## Rows never clash

An area keeps its sky's id in its light and fog, `"atmosphere": { ..., "sky": "dusk" }`, and the export turns the id into a row.

- **At install** each new sky of the pack takes a row of the module's `skyboxes.2da`: the first free run of rows from 32 up, as
  many as the pack has new skies. A row is free when the module's table doesn't reach it or leaves it empty, and no other
  installed sky has it. The catalog records each sky's row. A new version of a pack keeps the rows of the skies it keeps, since
  areas exported before name them.
- **At export** the area's `SkyBox` is its sky's row, and `ls_skies.hak` holds every installed sky's files and a
  `skyboxes.2da`: the module's table with the skies' rows added (label `LS_<id>`, cycling with the time of day). The module
  lists `ls_skies` after the area's hak and `ls_textures`, above the module's own haks.
- **The module's own list must agree.** If the module's hak list (the editor's haks) has `ls_skies`, and a hak above it has its
  own `skyboxes.2da`, the game would read that table, without the skies: the export refuses and names that hak. If the module
  doesn't list `ls_skies` at all, the export says so.
- **A module that grows into a sky's row** is refused at export, naming the row and the sky: remove the pack (`--force`) and
  install it again for new rows, then export its areas again.

Underground areas have no sky, whatever their atmosphere says.

**Where the area goes, the hak goes.** Publish (and Try on dev) sends the hak's files with an area that has a Landscaper sky, the
package for a module has them in `haks/ls_skies/`, and `dev-pack` copies the hak and lists it. An area without one sends nothing
more than before.

## What an install checks

Everything a texture pack's install checks about the download, the index's hash, each file's sha256 and the zip's names, and:

- **Files:** each `<name>.mdl` or `<name>.dds`, the name 1 to 16 lower-case letters, digits and `_`, starting with the pack's
  `prefix`. At most 15 MB each (NWSync's limit) and 512 a pack.
- **Textures:** each DDS reads as the game reads it (BioWare's header or the standard one), at most 8,192 px a side.
- **Models:** each reads (text or compiled), is named as its file, and draws only with textures the pack or the game has.
- **Skies:** at most 64 a pack, each with an id (1 to 32 lower-case letters, digits and `-`), a name and four models the pack
  or the game has.
- **Adding, never replacing:** a file the game or a module hak already has is refused, and so is a sky id or a prefix another
  installed sky pack has. A prefix doesn't start with `ls`.

## The pack's zip

`pack.json` and the files, all at the zip's top level:

```json
{
  "format": 1,
  "kind": "sky",
  "id": "example-skies",
  "version": "1.0.0",
  "name": "Example skies",
  "description": "A dusk and a starry night.",
  "licence": "CC-BY-4.0",
  "credits": ["Alice"],
  "prefix": "ex",
  "files": [
    { "path": "ex_dusk.mdl", "sha256": "…64 hex digits…" },
    { "path": "ex_dusk.dds", "sha256": "…" }
  ],
  "skies": [
    {
      "id": "dusk",
      "name": "Dusk",
      "preset": "dusk",
      "models": { "dawn": "ex_dusk", "day": "ex_dusk", "dusk": "ex_dusk", "night": "skyn_001" }
    }
  ]
}
```

`preset` is the Sky & fog preset the sky suits. A model may be the game's own (`skyn_001` above).

## The index

Sky packs share the texture packs' index (`LANDSCAPER_PACKS_URL`), in its own list, `skies`, which Landscapers before sky packs
don't read:

```json
{
  "format": 1,
  "packs": [],
  "skies": [
    {
      "id": "example-skies", "kind": "sky", "version": "1.0.0", "name": "Example skies", "licence": "CC-BY-4.0",
      "url": "example-skies-1.0.0.zip", "size": 5812345, "sha256": "…", "skies": 1, "names": ["Dusk"]
    }
  ]
}
```

## Making a pack

```sh
landscaper skies pack skies.json --out dist --index dist/index.json
```

The list has the fields of `pack.json`, with `files` the paths of the pack's files, relative to the list. A `.mdl` or `.dds` goes
in as it is; a `.tga` or `.png` picture becomes a DDS (DXT1, with mipmaps) of the same name, halved until it is at most 8,192 px
a side. The pack is checked as an install checks it, with the game when the Landscaper finds it. The same list makes the same
bytes.
