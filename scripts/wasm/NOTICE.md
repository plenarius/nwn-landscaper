# The Landscaper's engine for the browser

`landscaper.wasm` is the Landscaper's engine built for WebAssembly (WASI, `wasm32-wasip1`) by `scripts/build-wasm.sh`, for a page
that runs it in the visitor's browser (`docs/DESIGN-web-demo.md`). It is GPL-3.0-or-later (`LICENSE`), with its source at the
same tag on <https://github.com/plenarius/nwn-landscaper>.

Built into it, besides the Rust crates listed in the release's `THIRD-PARTY-LICENSES.html`:

- Moonglow's crates from the patched copy on <https://github.com/plenarius/moonglow-toolset> (branch `wasi/v4.0.0`), GPL-3.0;
- from wasi-sdk (<https://github.com/WebAssembly/wasi-sdk>): wasi-libc (Apache-2.0 WITH LLVM-exception, with parts under MIT and
  other permissive licences) and libc++ and libc++abi (Apache-2.0 WITH LLVM-exception).

It reads no file of the game unless the page hands it the visitor's own game folder.
