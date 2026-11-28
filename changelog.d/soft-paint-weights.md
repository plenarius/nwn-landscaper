<!-- markdownlint-disable-file MD041 -->
- **Soft paint's weights (stage 2).** An area may carry a second texture and its weight for each sample (`blend`: two bytes a
  sample, in the area file, the export's body, the exported header and canvas blocks). Where a sample has one, the bake draws
  the edge where the weights are half and half, on the brush's own line, in place of the smoothed paint; elsewhere it is as
  before, and an area without weights exports byte for byte as before. An engine older than this ignores the layer in areas and
  exports (a canvas block holding one is refused as damaged by it). See [the design](../docs/DESIGN-soft-paint.md).
- **A texture's `edge` may be `soft`:** where it meets another texture the two blend by their weights, as wide as the brush
  faded. Where two styles meet the softer wins: soft, then broken, then crisp.
