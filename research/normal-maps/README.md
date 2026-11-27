# Normal maps spike (not for merge)

Throwaway tools behind [docs/DESIGN-normal-maps.md](../../docs/DESIGN-normal-maps.md), run on a burst server on 2026-11-27.
Paths in them are that session's (`/srv/burst-out/normal-maps`).

- `normals.py IN OUT STRENGTH [SIGMAS]`: a normal map from a picture's height (band-passed brightness), as a BioWare DXT5 DDS
  with x in alpha and y in green, plus PNG previews. Run on the starter set's library pictures (not committed: they are the
  game's) and on the demo pack's photographs.
- `photonm.py`: the demo pack's photographs' own normal maps (Poly Haven, ambientCG) as the same DDS, turned the way the
  exported colour picture is.
- `post.py HAKDIR AREA...`: turns an export of #96's branch (`ground-relief`) into the spike: adds the normal maps to
  `ls_textures.hak`, swaps #96's brightness relief in `ls_tp_fs` and `ls_bk_fs` for the normal map (`texture1`), and gives each
  material with `lsRelief` its texture's `texture1`. The two shader blocks in it are the spike's shader code.
- `measure.py`, `demonm.py`: the hak sizes and the demo pack's normal map sizes in the design.
- `fig.py`, `fig_sources.py`: the design's pictures.
