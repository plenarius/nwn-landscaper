<!-- markdownlint-disable-file MD041 -->
- **The installers and archives start the editor again.** Releases 0.7.0 to 0.7.2 packed the editor without its `try/`
  folder, which it loads while it starts, so the page stopped with an empty toolbar and no tools in every browser. The release
  now packs it, and fails if the packed editor names a file it doesn't have.
