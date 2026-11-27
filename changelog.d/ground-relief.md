<!-- markdownlint-disable-file MD041 -->
- **Pebbles, cobbles and rock have relief in the game's light.** A library texture's new `relief` says how high its lightest
  parts stand over its darkest; the tiles' shaders tilt the normal the light reads by the slope of the texture's brightness,
  on near-flat ground within about 20 m of the camera, without changing the ground's average brightness. The starter set
  gives it to Pebbles, Cobblestones and Rock (a library made before gets theirs too); everything else stays flat. Nothing
  more in the hak; at the game's Minimal shaders nothing changes. See [Light](../docs/handbook/light.md#relief-on-the-ground).
