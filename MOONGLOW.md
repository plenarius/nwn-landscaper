# Moonglow, the toolset the Landscaper builds on

The Landscaper uses crates from <https://github.com/jadzziaa/moonglow-toolset> (GPL-3): `mg-core`, `mg-mdl` (model compiler),
`mg-erf`, `mg-gff`, `mg-set`, `mg-resman`, `mg-rules`, `mg-module`, `mg-tiles`, `mg-area`, `mg-script` and `mg-image`.
`Cargo.toml` pins them all to one commit (`rev = ...`), so every build, here and in CI, uses the same Moonglow.

**Now:** `ac17ade` (Moonglow 4.0.0, the commit the tag `v4.0.0` points at).

[nwn-bake](https://github.com/plenarius/nwn-bake), which the Landscaper takes as a git dependency at a release tag, pins Moonglow
once in its own `Cargo.toml`, and the Landscaper must pin the same `rev`: Cargo treats one git URL at two revisions as two crates,
and the build fails. nwn-bake's `MOONGLOW.md` is its own note.

## Bumping it

1. The bump happens in nwn-bake first, by a pull request on <https://github.com/plenarius/nwn-bake>, and goes out in a release.
   Then change nwn-bake's `tag` in `Cargo.toml` to that release and every Moonglow `rev` to the commit it pins. A
   `cargo build --release` then updates `Cargo.lock` for those alone; a plain `cargo update` would move every other dependency
   too. `tests/nwn_bake_split.rs` fails while `Cargo.toml`, `Cargo.lock` and the release's manifest do not name one commit.
2. `cargo test --release --workspace`.
3. Export the same area with the old and the new build and compare every file in the hak (hash each entry). Anything that
   differs has to be understood. A change in the compiled models (`.mdl`) means looking at the area in the game before merging.
   Give each export its own NWN user folder (`NWN_HOME`, with links to the real haks and talk table, leaving out the Landscaper's
   own `ls_*.hak`) and data folder (`XDG_DATA_HOME`): an export writes its hak and `ls_textures.hak` into the user folder's
   `hak/`.
4. Compare nwn-bake's output too, old build against new: `area-iso` (all five views), `area-scene` digests, creature bakes,
   placeable thumbnails and the placeables index, with the real haks.
5. Change **Now** above to the new commit, and say in the pull request what changed and how it was checked.
