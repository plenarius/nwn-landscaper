- **The editor picks skies.** `/api/skies` lists the installed Landscaper skies (with the preset each suits) and the module's own
  `skyboxes.2da` rows; `/api/sky-picture/<sky>` draws a sky as the game's camera sees it, a strip of the horizon or the whole
  panorama around, made from its model and textures once and kept in the sky library's `pictures` folder. An area's light and
  fog can pick a game row too (`"skyRow": 3`), which the export writes as its `SkyBox`; an imported area keeps its sky that way
  (or by its id, when it is an installed sky's row).
