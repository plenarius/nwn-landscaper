- **Baked shade doesn't darken local light.** A tile's shadow map multiplied all its light, so a torch, a tile light or a
  glowing mesh in a tree's shade was dimmed with it. It now shades only the area's sun or moon (ambient and diffuse), through
  `ls_lighting`, made at run time from the game's `inc_lighting` as the tile shaders are made from `fslit` and `vslit`. Ground
  nothing else lights looks as before. The shared texture hak is rebuilt once, for the new shaders.
