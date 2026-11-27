<!-- markdownlint-disable-file MD041 -->
- **Cliffs turned from the sun are rock in shade, not black.** The game lights every area from the north-east, so a cliff
  facing south or west gets only the area's ambient light, and the baked shadow map then darkened that again for the shadow
  the face casts on itself. The shadow map now takes a cast shadow only from ground that faces the sun, and the tiles'
  shaders give steep ground some of the sun or moon bounced off the ground below it (none on flat ground). See
  [Light](../docs/handbook/light.md#shadows).
