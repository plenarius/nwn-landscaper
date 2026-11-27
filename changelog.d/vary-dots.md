<!-- markdownlint-disable-file MD041 -->
- **Exported natural ground no longer has dark dots every metre or so.** Since 0.7.3 the tiles' shader drew a small dark
  triangle in the middle of each of the cells that hide a natural texture's repeat. The next export rebuilds the shared
  `ls_textures` hak with the fix, for every area.
