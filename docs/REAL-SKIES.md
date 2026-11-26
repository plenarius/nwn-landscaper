# Real Skies for modules

[Real Skies](https://neverwintervault.org/project/nwnee/hakpak/real-skies-hak-version) is Savant1974's: panoramas made from free
wallpapers, blended and stitched together, for the five stock skies of Neverwinter Nights. Its hak can't go through NWSync: every
texture is a 24-bit TGA of 77 MB or more, and NWSync refuses any file over 15 MB. `scripts/real-skies.rs` turns your download
into one hak that fits. The pictures stay Real Skies': this repository ships none of them, so download it yourself, and keep the
credit with the hak.

This is a step outside the Landscaper: a module uses the hak whether its areas come from the Landscaper or not.

## Making the hak

1. Download the **EE hak version** (`real_skies_1.02.7z`, 382 MB) from the page above and unpack it with 7-Zip. It holds
   `patch/Real_Skies.hak` (1.34 GB).
2. In a checkout of this repository (README, "Development"):

   ```sh
   cargo run --release --example real-skies -- ~/Downloads/real_skies_1.02 realskies_dds.hak
   ```

   The first argument is `Real_Skies.hak` or the folder it was unpacked into; the second is the hak to write (default
   `realskies_dds.hak`). `--jobs` sets how many textures are re-encoded at once (default 4, about 150 MB of memory each).
3. Add the hak to the module's hak list and to its NWSync manifest. It replaces the stock sky models by name, so its place in the
   list matters only against another hak that replaces them (the original `Real_Skies.hak`: take that one out).

The hak is about 95 MB: 17 textures of about 5.6 MB and 20 small models.

## What it changes

| Stock row | Times | Pictures |
|---|---|---|
| Grass_Clear | dawn, day, dusk, night | Real Skies' |
| Grass_Storm | dawn, day, dusk, night | Real Skies' |
| Desert_Clear | dawn, day, dusk, night | Real Skies' |
| Winter_Clear | dawn, day, dusk, night | Real Skies' |
| Icy | day | Real Skies' |
| Icy | dawn, dusk, night | Winter_Clear's from Real Skies (Real Skies leaves them stock) |

- **Textures:** each panorama (14,384 by 1,798 pixels; Grass_Clear's day 16,384 by 2,048) is scaled to 8,192 by 1,024, each
  pixel the average of the area it covers, and stored as DXT1 DDS with its mipmaps: 5,592,476 bytes. 8,192 px is also the
  largest texture some older graphics cards accept.
- **Models:** Real Skies' band mesh as it is, with each model named after its file. Real Skies names 14 of its 17 models
  `Sky_001` inside, whatever their file is called.
- **`skyboxes.2da`:** unchanged. The rows keep their numbers and model names.

## Checked in the game

On 2026-11-26, in the game client off screen on the CPU renderer (llvmpipe), in a small Landscaper area that runs day and night.
A script switched the sky with `SetSkyBox` and moved the camera; I looked at every still.

![The five skies in the game: Grass_Clear, Grass_Storm, Desert_Clear, Winter_Clear and Icy from left to right; dawn, day, dusk
and night from top to bottom. Pictures from Real Skies by Savant1974.](real-skies-1-times.png)

- **Every sky shows its own picture** at dawn, day, dusk and night, and Icy's dawn, dusk and night are Winter_Clear's.
- **Dawn and dusk need the clock to run into them.** Jumping into the dawn or dusk hour with `SetTime` left the night or day
  sky up; the client fades to the dawn sky when the hour turns to dawn, and from the day sky to the dusk sky over the first
  seconds of dusk.
- **No black triangles** at the usual camera, in about 50 stills.

![A high camera over Real Skies (top row) and the stock skies (bottom row): level at 120, 200 and 300 m above the character, then
looking down from 300 m. Pictures from Real Skies by Savant1974.](real-skies-2-high.png)

- **The open top shows from about 300 m up**, as flat grey above the band; at 600 m the whole view is grey. The stock dome's cap
  still covers it at 300 m. Up to 200 m the two look alike. The player's camera never gets that high: it took `SetCameraHeight`.
- **Looking down from 60 m and higher, a black wedge** fills the view beyond the area, under a band of the fog colour. It looks
  much like the reported black triangles, but the stock skies show it the same way, so it comes from the client past the area's
  edge, not from Real Skies.

Not checked: anything on a graphics card. The reported crash loading Desert_Clear can't be reproduced on the CPU renderer, which
accepts textures of 16,384 px; the hak's 8,192 px DDS textures remove the likely causes (a card's texture limit, or memory for
77 MB textures), but that is inferred.
