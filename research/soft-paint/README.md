# Soft paint spike (not for merge)

The throwaway code behind `docs/DESIGN-soft-paint.md`. `src/generator/soft.rs` bakes a tile's ground from smoothed weight fields
when `LS_SOFT=<blend|crisp|broken>:<sigma m>` is set (`LS_SOFT_TAU`, `LS_SOFT_NOISE`, `LS_SOFT_LUM` tune it), or from weights in a
file (`LS_SOFT_W`, f32 per palette entry per sample) as a soft brush would store them (`brush.py`).

- `run.sh AREA LABEL [ENV...]`: one timed export. `LS_DUMP=1` writes each baked texture as a PNG.
- `brush.py FEATHER OUT [SIGMA]`: weights for the snowy pass's trail painted again with a round brush along its own centreline.
- `mosaic.py`, `topgrid.py`: the dumped bakes over a stretch of the area, north up. `sheet.py`: the client stills side by side.
- `mkmod.py ORDER`, `sp_tour.nss`, `stills.py OUT`: a module with each variant's area and a camera tour, and the off-screen client
  run that grabs a still at each `ETSHOT` line (another client on the machine holds port 5121: give this one's
  `settings.tml` `server.net.port` another).
