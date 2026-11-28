<!-- markdownlint-disable-file MD041 -->
- **An export says how long it will take, and how far it is.** The editor's export runs as a job (`/api/export/job`) that
  reports its step, its tiles done out of the total, and the time left. It estimates the time before the tiles start, from the
  tiles whose ground and shadows the cache already holds, the placed objects' triangles and the threads, at the speed this
  machine measured on its last export (`export-speed.json` in the data folder). The time left is worked out again as each tile
  finishes. Cancelling stops at the next tile and leaves the last export as it was. Exporting lsforest from nothing on 8
  threads took 113 s against 90 s estimated at the start (at lsplains' speed) and 110 s at half the tiles; again, 52 s against
  51 s. The plain `/api/export` is as before; the browser page's engine prints its progress as `LS_PROGRESS` lines.
