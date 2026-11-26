- **The engine builds for the browser.** `scripts/build-wasm.sh` builds it for WebAssembly (WASI) with the new `web` feature, for
  the handbook site's coming "try it" page (`docs/DESIGN-web-demo.md`): no threads, the game's archives read without memory maps
  (Moonglow's patched copy on the maintainer's fork), and `landscaper wasi-request` answering one of the editor's requests. Its
  export is byte for byte the native one. Pull requests build it and check its size (1.8 MB with brotli, at most 3), and a
  release carries it as `landscaper-wasm.zip`. Importing an area isn't in it yet.
