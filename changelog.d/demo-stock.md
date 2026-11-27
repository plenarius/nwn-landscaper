<!-- markdownlint-disable-file MD041 -->
- **An area with placeables this install lacks checks and exports.** The path check and the export stopped at the first model they
  couldn't read, so an area from another install or a team showed "Live: error". Such a placeable now blocks nothing and is left
  out of the tiles, and the report names its models (`missing_models`); one in object mode is still written to the module.
- **The browser page carries the game's stock placeables.** `/try/` has every stock placeable model with its walkmesh and
  textures, and the three shaders an export needs, fetched a model at a time: stock trees and rocks draw, block paths and export as
  on the desktop, Place and Scatter work, and Export no longer asks for the game folder.
