<!-- markdownlint-disable-file MD041 -->
- **Canvases can be stored on the engine** (step 3 of [the canvas design](../docs/DESIGN-canvas-cut.md)): a canvas of up to 128
  by 128 tiles is a folder, `canvases/<name>/`, with a small header and its ground in fixed blocks of 16 by 16 tiles, each one
  file with its heights, paint and chasm mask. A save sends only the blocks that changed, with the version of each it started
  from, and is refused whole when another tab saved any of them since; a delete names the versions of everything the tab
  holds. A block or header that is damaged or can't be read stops whatever needs it, names the file, and is never written
  over; only a block that has no file is flat ground. On a team server a canvas is one item of the builder's workspace, and
  needs an assignment for every existing area it is built from, as an area does. The editor's side comes in a later step.
