# Atmosphere and skies

An area's look is its light, its fog and its sky. The light and fog are set in the **Sky & fog** panel; the sky is a row of the
game's `skyboxes.2da`, set in **Area settings**.

## Pick a look

**Area > Sky & fog** (`L`), **Looks**: each sets the sun, the moon, the fog and the time of day at once.

| Look | For |
| --- | --- |
| Clear day | open ground in daylight, day and night cycling |
| Golden haze | a low sun, warm light |
| Dusk | evening light |
| Overcast | grey, even light |
| Swamp | green fog, close in |
| Thick mist | thick white fog |
| Cave, Torchlit cave | under ground, without and with warm light |
| Indoors | interiors |
| Dark night | a night with little moon |

Change any colour after picking one; [Light](light.md) says what each setting does. **View distance** sets how far the fog
lets the camera see.

![The Sky & fog panel with its looks](../assets/handbook/atmosphere.webp)

## Pick a sky

**Area > Area settings**, **Sky box**: the sky the game draws over the area, from the module's `skyboxes.2da`. The sample
landscapes in *New area* pick one that fits their light.

## More skies

- **Sky packs** add skies: new rows of `skyboxes.2da` with their models and textures, installed as texture packs are. The export
  puts them in their own hak, `ls_skies.hak`. Setting one up is a job for whoever runs the Landscaper:
  [Sky packs](../SKY-PACKS.md).
- **Real Skies** replaces the game's five stock skies with photographed panoramas. It is a hak for the module, made by a script
  from your own download: [Real Skies for modules](../REAL-SKIES.md).

## Weather

Rain, snow and wind, like the toolset's Area Properties, are in **Area > Area settings**, with the area's name, tag, scripts,
music and ambient sound.
