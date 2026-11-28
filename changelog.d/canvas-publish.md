<!-- markdownlint-disable-file MD041 -->
- **Which canvas pieces changed since the last publish, and why** (`GET /api/canvas-changes`): each piece's own ground, what
  stands in it, its edge (a neighbour's ground or objects showing in its halo, with that neighbour named), its crossings, its
  settings, or a corner that moved; new pieces, renamed ones and pieces no longer on the canvas. Pieces whose export is older
  than the canvas are marked, so the editor can say "export again" first. See [the design](../docs/DESIGN-canvas-cut.md), step 7.
- **A canvas publishes as one review request** (`POST /api/canvas-publish`): the changed pieces by default, or those named,
  through the review service's `publish-module`. What was sent is recorded with the canvas, so the next publish knows what
  changed. A neighbour whose crossings with the sent pieces aren't live yet is named.
- **Package and Try on dev take a canvas** (`/api/canvas-package`, `/api/canvas-package-download`, `POST /api/canvas-trial`):
  every piece by default, together, so their crossings lead to each other's waypoints.
- **Exporting some pieces of a canvas** checks that the neighbours they cross into were exported with the same crossings, and
  names the ones to export first (409 with `exportFirst`) instead of leaving a crossing that leads nowhere. Its route check
  no longer says a piece has no way in when the pieces leading into it weren't part of that export.
- **A canvas exports as an export job** (`POST /api/canvas-export/job`), followed like an area's (`GET /api/export/job`,
  cancel with `/api/export/job/cancel`): the job's `area` says which piece is under way ("rivl02 (piece 2 of 3)"), and its
  progress is that piece's.
