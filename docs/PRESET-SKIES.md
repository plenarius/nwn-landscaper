# Preset skies

Preset skies is the first [sky pack](SKY-PACKS.md): a sky for each outdoor Sky & fog preset but Clear day (which keeps the game's
Grass Clear), so an area's sky matches its light. The design is [DESIGN-ambience.md](DESIGN-ambience.md), question 2. Five skies
re-use the pictures of Savant1974's [Real Skies](https://neverwintervault.org/project/nwnee/hakpak/real-skies-hak-version) at
other times of day. Real Skies states no licence, so this repository ships none of its pictures and the pack is not in the
Landscaper's list of packs: you make it from your own download. Thick mist and Swamp are painted by the script from their presets'
colours.

## The skies

| Sky | Id | Preset | Dawn | Day | Dusk | Night |
|---|---|---|---|---|---|---|
| Golden haze | `golden-haze` | `golden` | Desert dawn | Desert dawn | Grass dusk | Desert night |
| Dusk | `dusk` | `dusk` | Grass dusk | Grass dusk | Grass dusk | Grass night |
| Overcast | `overcast` | `overcast` | Storm dawn, greyed | Storm day, greyed | Storm dusk, greyed | Storm night, greyed |
| Starry night | `starry-night` | `night` | Desert night | Desert night | Desert night | Desert night |
| Dawn haze | `dawn-haze` | `dawn` | Desert dawn | Winter dawn | Winter dusk | Winter night |
| Thick mist | `thick-mist` | `mist` | painted | painted | painted | painted |
| Swamp | `swamp` | `swamp` | painted | painted | painted | painted |

"Desert dawn" is Real Skies' picture for Desert_Clear's dawn, and so on; Grass is Grass_Clear and Storm is Grass_Storm. The
re-used skies keep Real Skies' band mesh and its pictures at 8,192 by 1,024, as `real-skies` makes them
([REAL-SKIES.md](REAL-SKIES.md)), so the pack's copy of a picture is the same bytes as that hak's, and NWSync stores it once.

- **Overcast** is Grass_Storm's four pictures with their colours a quarter as strong and their contrast two thirds, around
  each picture's mean: grey cloud cover over the preset's flat grey light.
- **Thick mist and Swamp** are gradients, from the preset's fog colour at the horizon, easing into its diffuse colour half
  greyed overhead, the sun's by day and the moon's by night; dawn and dusk take the colours halfway between. Where the diffuse
  colour is darker than the fog (Thick mist by day), the top is the fog colour a little lighter, so the sky is never darker
  overhead than at the horizon. Each is a 16 by 256 px texture on a dome of the script's own: 600 m out, from 300 m below the
  horizon to a cap overhead. The colours are copied from the editor's presets (`js/atmosphere.js` in nwn-landscaper-web) into
  `scripts/preset-skies.rs`: change them together.

The pack has 34 files, 60 MB: 11 pictures of 5.6 MB, 6 small gradients and 17 models.

## Making and installing it

1. Download the **EE hak version** of Real Skies (`real_skies_1.02.7z`) and unpack it with 7-Zip, as for
   [REAL-SKIES.md](REAL-SKIES.md).
2. In a checkout of this repository (README, "Development"), make the pack's files and its list, then the pack:

   ```sh
   cargo run --release --example preset-skies -- ~/Downloads/real_skies_1.02 preset-skies
   landscaper skies pack preset-skies/skies.json --out dist
   ```

   The first argument is `Real_Skies.hak` or the folder it was unpacked into; the second, the folder to write. `--jobs` sets how
   many pictures are re-encoded at once (default 4, about 150 MB of memory each).
3. Install it as any sky pack, with the module's haks set: `landscaper skies get dist/preset-skies-1.0.0.zip`. Its seven skies
   take the first free rows of the module's `skyboxes.2da` from 32 up.

From then on each preset in the editor's Sky & fog panel picks its sky from the pack, and the Sky row lists the seven
([Atmosphere and skies](handbook/atmosphere.md)). Without the pack, a preset picks the game's sky nearest to it.

Preset skies works with or without the `realskies_dds.hak` that `real-skies` makes: it carries its own copies of the pictures
under its own names.

## Checked in the game

On 2026-11-26, in the game client off screen on the CPU renderer (llvmpipe): one module with a small exported area per preset,
each naming its sky. `skies get` gave the seven skies rows 32 to 38, and each area exported with its row. A script toured the
areas at noon, dusk, night and dawn, the clock run into each hour, and shorter runs checked dawn and dusk; I looked at every still (about 190).

![The seven skies in the game: Golden haze, Dusk, Overcast, Starry night, Dawn haze, Thick mist and Swamp from left to right; by
day on top, by night below, each over its preset's light. Pictures from Real Skies by Savant1974, except the last two.](preset-skies-1-presets.png)

- **Each sky matches its light** by day and by night, and shows its own picture at dawn and dusk. Thick mist and Swamp meet
  the fog at the horizon without a line; the camera sees only the lowest part of their dome, so they read as the fog colour
  lightening upwards.
- **No black triangles, seams or upside-down panels** at the level camera or the usual one (the usual one shows only a strip
  of sky).
- **Standing in an area through dawn or dusk**, the sky fades smoothly from one picture to the next, as the stock skies do.
- **Coming back into an area during the first minutes of dawn or dusk** drew a flat sky of the fog colour until the fade ended
  (about 1 minute 20 seconds into the hour, with 10 minutes to the game hour). It happened with the pack's models in a stock
  row as in a new one, and never on a first visit, so it is not the pack's rows or models; whether stock skies do it too wasn't
  tried.

On 2026-11-26 again, with the editor's presets naming their skies (step 4): an area per preset, its light and sky from the
editor's own presets, each exported with its sky's row. The new Dawn preset's warm light suits Dawn haze at noon, and standing in
the area while the clock ran into dawn, its sky faded from the night picture into the dawn one in about 3 minutes.

Not checked: anything on a graphics card.
