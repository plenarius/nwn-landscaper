# NWN Landscaper

Draw terrain in your browser and get a Neverwinter Nights: Enhanced Edition area. For builders of a persistent world, on their own
machine or on a team server.

- **Terrain to area:** turns a sculpted and painted heightmap into a tileset and an area, with the paths creatures need to walk it.
- **Objects:** places creatures, doors, triggers and the rest.
- **Module and haks:** builds the module and the haks it needs.
- **Import:** imports existing areas to rework.
- **Blueprints:** edits creature and item blueprints, conversations and scripts, in the browser.

Two programs: this repository is the engine (Rust; it reads your NWN install, builds the areas and serves the editor) and
[nwn-landscaper-web](https://github.com/plenarius/nwn-landscaper-web) is the editor (three.js). It runs on your own machine, or as a team
server on your own domain where each builder has a workspace.

Made for [Layonara](https://layonara.com)'s builders. The code is configurable for any module ([docs/CONFIGURATION.md](docs/CONFIGURATION.md)).

## Install

Download the archive for your system from the [releases](https://github.com/plenarius/nwn-landscaper/releases): Windows
(x86-64), macOS (Apple silicon and Intel) or Linux (x86-64). Unpack it anywhere and start the program in it: `landscaper.exe`
on Windows, `landscaper` on macOS and Linux. A window shows the game and user folder it found, and your browser opens the editor
at <http://127.0.0.1:8765/> (or the next free port, if another program has 8765). Close the window to stop it. `QUICKSTART.txt`
in the archive says the same, and what to set when it can't find the game.

The archive holds the engine and the editor of the same release, so you need nothing else but the game. Check a download against
`SHA256SUMS` on the release page.

The first start:

- **macOS:** the program is signed with a Developer ID and notarized by Apple, so the first start only asks whether to open a
  program downloaded from the internet. (Releases up to 0.4.0 aren't signed: open System Settings, Privacy & Security, and click
  **Open Anyway**.)
- **Windows:** the program isn't signed yet, so SmartScreen may say it protected your PC. Click **More info**, then **Run anyway**.

To build it from source instead, see [docs/SELF-HOSTING.md](docs/SELF-HOSTING.md) and "Development" below.

## You need

- **Neverwinter Nights: Enhanced Edition**, the game itself (not only the dedicated server's download, which lacks the shaders the
  Landscaper builds on). The Landscaper reads the game's files and ships none of them.
- **A user folder whose `hak/` and `modules/` it may write in:** exports go there. The game makes it when it first starts.
- **`nwn_gff`** from [neverwinter.nim](https://github.com/niv/neverwinter.nim) on your `PATH`, only for **Package for your
  module…**.
- **Linux, Windows or macOS.** It is developed and tested on Linux. The release builds are checked on all three systems, but
  exports on Windows and macOS have had less use.
- To build it from source: **Rust 1.98 or newer**, a C++ compiler and cmake (for glslang, which nwn-bake builds with), and the
  editor's files.

[docs/SELF-HOSTING.md](docs/SELF-HOSTING.md) lists everything.

## Run

[docs/SELF-HOSTING.md](docs/SELF-HOSTING.md): on your own machine, or as a team server with Docker. The engine and the editor
change together: use both from the same release (the same `vX.Y.Z` tag in each), or the engine from its `master` branch and the
editor from its `main` branch, and update them together (SELF-HOSTING.md, "Updating").

## Use

`landscaper --help` lists the commands. `--root`, `--user`, `--tlk` and `--jobs` work before or after any command (the same names
as nwn-bake's options; [docs/CONFIGURATION.md](docs/CONFIGURATION.md)).

### serve

The editor, at the address it prints. At start it prints the game install, the user folder and the module it found.

```sh
landscaper serve --web ../nwn-landscaper-web
```

`--port` (8765) and `--bind` (127.0.0.1) say where it listens; `--team` runs a team server behind a sign-in proxy
([docs/TEAM-SERVER.md](docs/TEAM-SERVER.md)). Without `--team`, an address other machines can reach needs `--allow-remote`.

The editor talks to it over an HTTP API under `/api/`, which changes with the editor (CHANGELOG.md). A script may call it too, with
the header `X-Landscaper: 1` (docs/TEAM-SERVER.md, "Requests from other sites"). For example,
`GET /api/blueprint-fields?kind=placeable&resref=plc_chest1` answers a blueprint's own settings
(`{ "kind", "resref", "fields": { "Useable": 1, "Locked": 0, ... } }`; on/off settings as `1` or `0`, a setting the blueprint
doesn't have left out), and 404 when there is no such blueprint.

### import

An existing area into the editor, from its files or from a module, under the name the editor shows it by.

```sh
landscaper import ~/nwn-module/src/elmwood.are.json elmwood
landscaper import mymodule.mod:elmwood elmwood
```

A third argument says where the import goes (default: the exports folder in the data folder). An area more than 32 tiles a side,
or one whose tile list doesn't match its size, is refused before the game is opened. A re-import that fails keeps the earlier
import as it was.

### export

A saved area (the `.json` the editor writes) into the game: its hak into the user folder's `hak/`, its module into `modules/`.
A module already there only takes the new tiles. It prints a summary; `report.json` beside the module has the whole report.
The module is replaced in one step and the one before is kept as `<module>.mod.bak`; a failed export leaves the module and the
hak as they were.

Each entrance's door is tagged `<area>_ent<id>`, from the entrance's own id, so taking out another entrance renames nothing. On a
re-export, the door of an entrance taken out is removed from the module and listed in the report (`updated.doors_removed`);
doors placed by hand stay.

```sh
landscaper export ~/.local/share/landscaper/exports/elmwood.json
```

### check

The export's analysis of a saved area, without building anything: a summary line, then each warning and where it is. `--json`
prints the whole report. An object, thing or entrance more than 10 m outside the area, an object scaled outside 0.1 to 5, or a
value that isn't a number stops the check and the export with a message naming it.

```sh
landscaper check ~/.local/share/landscaper/exports/elmwood.json
```

### textures

The texture library the editor paints with. The first start builds the starter set from your game's files; `textures` adds more.

```sh
landscaper textures init                          # the starter set again, adding what is missing
landscaper textures add my-textures.json          # the game's or your haks' textures, by name
landscaper textures build ~/pictures/ground --category Grass --surface 3
landscaper textures list
landscaper textures list-packs                    # texture packs: installed, and on offer
landscaper textures get some-pack                 # install one (its id, a URL or a .zip)
landscaper textures remove-pack some-pack
```

Texture packs are described in [docs/TEXTURE-PACKS.md](docs/TEXTURE-PACKS.md), with how to make one (`textures pack`).

### marker

Whose export a hidden mark belongs to (a team server's `ls_ref` variable, or `ref:<id>` in an area's comments).

```sh
landscaper marker 3f9a0c12b7de
```

### dev-pack

A test module for a dev server, made from the production module and a builder's export, entered from a waypoint of an existing
area; how it reaches the dev server is up to you. The editor doesn't use it: its Try on dev goes through the review service.

```sh
landscaper dev-pack prod.mod elmwood.mod ~/nwn/hak town01 WP_Gate out/
```

## Documents

- [docs/SELF-HOSTING.md](docs/SELF-HOSTING.md): running it on your machine, or as a team server with Docker.
- [docs/CONFIGURATION.md](docs/CONFIGURATION.md): every setting.
- [docs/BUILDER-WORKFLOW.md](docs/BUILDER-WORKFLOW.md): building an area.
- [docs/TEAM-SERVER.md](docs/TEAM-SERVER.md): the team server.
- [docs/REVIEW-SERVICE.md](docs/REVIEW-SERVICE.md): the calls a review service answers, to write one.
- [docs/DEBUGGING.md](docs/DEBUGGING.md): the `LS_*` debugging switches.
- [docs/TEXTURE-PACKS.md](docs/TEXTURE-PACKS.md): texture packs: installing them, their format, the index, making one.
- [MOONGLOW.md](MOONGLOW.md): the Moonglow version the Landscaper is pinned to, and how to move it.
- [CHANGELOG.md](CHANGELOG.md): what changed in each version.
- [SECURITY.md](SECURITY.md): reporting a security problem.

## Development

`cargo build --release` builds the engine, and `cargo test --workspace --locked` runs its tests. `scripts/check.sh` runs every
check of the code and documentation standard, [STYLE.md](STYLE.md); run it before you push.

The tests that need the game's files are ignored by default, as CI has no game: the export, re-export and import cycle
(`tests/game_cycle.rs`), an import into a folder of its own (`tests/import_dir.rs`), entrance doors (`tests/entrance_doors.rs`),
packages in both module layouts (`tests/package_layouts.rs`), a conversation and a blueprint's fields. Where there is an install,
run them too with `NWN_ROOT=<the game> cargo test --workspace --locked -- --include-ignored` (about ten minutes in a debug
build; the package test also needs `nwn_gff` on the `PATH`).

[nwn-bake](https://github.com/plenarius/nwn-bake), the library that reads the game's files and draws the pictures, is a
repository of its own; `Cargo.toml` takes it as a git dependency at a release tag. Changes to it go through pull requests there,
and reach the Landscaper with the next release. To work on both at once, point Cargo at a local checkout of nwn-bake with a
`[patch]` section that you do not commit:

```toml
[patch."https://github.com/plenarius/nwn-bake"]
nwn-bake = { path = "../nwn-bake/nwn-bake" }
```

`tests/nwn_bake_split.rs` checks that the Landscaper pins the same Moonglow as that release ([MOONGLOW.md](MOONGLOW.md)), and that
the files every repository following STYLE.md shares word for word are the same as nwn-bake's at that tag.

### Releases

The engine and the editor (nwn-landscaper-web) change together, so each release tags **both repositories with the same
`vX.Y.Z`**, the version in this repository's `Cargo.toml`. Tag the editor first: the tag here starts
`.github/workflows/release.yml`, which packs the editor of the same tag with the engine for each system and makes a draft release
to check and publish by hand. The self-host image (`deploy/self-host/Dockerfile`) builds the editor
of that tag by default (`WEB_REF`), and `tests/release.rs` fails when its default and `Cargo.toml`'s version disagree: bump both
in the release's pull request, then tag the two repositories.

When the engine moves to a new nwn-bake release (`Cargo.toml`'s tag), also run `scripts/shared-files.sh --pin vX.Y.Z` in
nwn-landscaper-web with that tag, so its shared files (STYLE.md and the check configurations) stay pinned to the same nwn-bake
release as the engine's (`tests/nwn_bake_split.rs` checks the engine's own copies).

## Credits

- [Moonglow](https://github.com/jadzziaa/moonglow-toolset): the crates that read and write the game's files (GPL-3.0).
- [neverwinter.nim](https://github.com/niv/neverwinter.nim): `nwn_gff`, run as a separate program (MIT).
- [tiny_http_dh](https://github.com/dheijl/tiny-http-dh) 0.12.4, the maintained fork of
  [tiny_http](https://github.com/tiny-http/tiny-http), the HTTP server: in `vendor/tiny_http` with a patch of ours that limits
  how long a connection may wait and how large a request's head may be
  ([vendor/tiny_http/LANDSCAPER.md](vendor/tiny_http/LANDSCAPER.md)). MIT or Apache-2.0.
- The shadow shaders in the hak the Landscaper builds are the game's stock lit shaders (`fslit` and `vslit`), read from your
  install and changed at run time (`src/shared.rs`); no copy of them is in this repository or its releases.

## Licence

Copyright (C) 2026 plenarius

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as
published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License along with this program. If not, see
<https://www.gnu.org/licenses/>.

- The Landscaper's own source code is licensed under the **GNU General Public License, version 3 or (at your option) any later
  version** (`GPL-3.0-or-later`). The full text is in `LICENSE`.
- Moonglow is licensed under **GPL-3.0 only**. The Landscaper's binaries include Moonglow, so **its binaries are under GPL-3.0**
  (version 3 only).
- `src/pathnodes.json` is measured data: where each tile layout puts its path nodes and exit points, measured on a real game
  install by watching the server route creatures ([research/FINDINGS.md](research/FINDINGS.md)). It holds no file or code from
  the game, and is under **CC0-1.0** (`LICENSES/CC0-1.0.txt`).
