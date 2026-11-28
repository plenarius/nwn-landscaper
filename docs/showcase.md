# Showcase

Areas made in the Landscaper, filmed in the game. Each card is a short clip of one area, with a still of it. **Open in
Landscaper** opens the area in [the Landscaper in your browser](handbook/try.md); **Download** saves its file, for File → Open
area from a file there, or to keep.

<!-- showcase -->

## Adding an area

Add an entry to `docs/assets/showcase/index.json`, the gallery's list in its order, and put the still beside it. An entry
has a `title` (the area's name), a `text` (one line: what it shows), a `still` (a WebP or PNG picture, 1280 wide at most), a
`clip` and an `area`; paths are from that folder. The clip itself stays out of the repository: upload it to the `showcase`
release and give its path as `clips/<file>.mp4` (or a video's address, or a page that plays it); the docs workflow copies
the release's clips into the site when it builds it. The area itself, as the Landscaper saved it (Download area), goes under
`areas/`, named in the entry's `area`. An entry with no still and no clip shows "Clip to come".

The editor carries the same folder (but the clips) as its `showcase/`, for **File → Open a showcase**:
`scripts/pack-showcase.sh` copies it wherever the editor is packed.
