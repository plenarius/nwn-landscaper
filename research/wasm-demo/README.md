# Spike: the engine in a browser (not for merge)

The engine built for WASI (`wasm32-wasip1`) and run behind the editor's own UI in a browser, with none of the game's files. The
design and the numbers are in [DESIGN-web-demo.md](https://github.com/plenarius/nwn-landscaper/blob/master/docs/DESIGN-web-demo.md);
the editor's half is the `spike/wasm-demo` branch of nwn-landscaper-web (`sw.js`, `try/`).

- `build.sh`: clones Moonglow and nwn-bake beside this repository, applies `moonglow-wasi.patch` and `nwn-bake-wasi.patch`, fetches
  wasi-sdk and builds `target/wasm32-wasip1/release/landscaper.wasm`.
- `wasi-run.mjs`: runs a command of the WASI build under Node (`node wasi-run.mjs landscaper.wasm <work> <game> generate /work/out`).
- `wasi-req.mjs`: one API request through `landscaper wasi-request` under Node.
- `wasm-crates.py`: a wasm file's code size by crate.

What changed in the engine, all for WASI only: `landscaper wasi-request` (one request on stdin, its answer on stdout,
`src/serve.rs`); no threads (`in_parallel_on`); no process id (`crate::pid`); importing an area left out (mg-area draws with
wgpu); placeholder tile shaders when there is no game to make them from (`src/shared.rs`); tiny_http's test request takes any
bytes as its body.
