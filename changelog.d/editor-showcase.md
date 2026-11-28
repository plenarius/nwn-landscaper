<!-- markdownlint-disable-file MD041 -->
- **The showcase areas come with the editor.** Every package of the editor (the app, the program alone, the self-host image,
  the forge and the page in the browser) carries the eight showcase areas and their stills in its `showcase/` folder, for the
  editor's **File → Open a showcase**. `scripts/pack-showcase.sh` copies them from `docs/assets/showcase`, still their only
  copy, and `scripts/check-editor-files.py` fails a package without them; `landscaper serve` run from the source serves that
  folder itself. The showcase's list moved from `docs/showcase.yml` to `docs/assets/showcase/index.json`, which the handbook's
  showcase page and the editor both read.
