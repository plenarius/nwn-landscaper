# Showcase

Areas made in the Landscaper, filmed in the game. Each card is a short clip of one area, with a still of it. **Open in
Landscaper** opens the area in [the Landscaper in your browser](handbook/try.md); **Download** saves its file, for File → Open
area from a file there, or to keep.

<!-- showcase -->

## Adding an area

Add an entry to `docs/showcase.yml` (its first lines say what each field takes) and put the still under
`docs/assets/showcase/`. The clip itself stays out of the repository: upload it to the `showcase` release and give its path as
`assets/showcase/clips/<file>.mp4`; the docs workflow copies the release's clips into the site when it builds it. The area
itself, as the Landscaper saved it (Download area), goes under `docs/assets/showcase/areas/`, named in the entry's `area`.
