# Showcase

Areas made in the Landscaper, filmed in the game. Each card is a short clip of one area, with a still of it.

<!-- showcase -->

## Adding an area

Add an entry to `docs/showcase.yml` (its first lines say what each field takes) and put the still under
`docs/assets/showcase/`. The clip itself stays out of the repository: upload it to the `showcase` release and give its path as
`assets/showcase/clips/<file>.mp4`; the docs workflow copies the release's clips into the site when it builds it.
