# Spike: the director's line (not for merge)

For [DESIGN-director-line.md](../../docs/DESIGN-director-line.md). nwn-director's flight planner (`plan_flight`, nwn-director
`main` d5129b5) and flightcheck.py's camera model and checks, ported to JavaScript (`flight.js`), and tested against them.

The inputs are not here: the five hand-written plans (vale, canyon, dunes, pass, cave), their Landscaper area files (`ls*.json`),
`model_dims.json`, `looks.json` and flightcheck.py, from the burst session of 2026-11-27 (`director-line/input/` in its copied
`burst-out` folder). Copy them into a folder with these files, and a clone of nwn-director beside it.

- `ref.py INPUT DIRECTOR > ref.json`: the director's plan and flightcheck.py's poses every half second, for each plan.
- `parity.mjs INPUT`: the port against `ref.json` (`parity-result.json`: identical legs and camera moves, poses within 1e-13 m).
- `perturb.py INPUT`, `perturb.mjs`: three plans broken on purpose; the port's issue counts match flightcheck.py's (36, 12, 23).
- `roundtrip.mjs`: each plan into an area file's `flights` and back, and each key into an orbit camera and back
  (`roundtrip-result.json`).
- `keymiss.mjs`: how far the simulated camera is from each key's pose, and how long it holds still (`keymiss-result.txt`).
- `timing.mjs`: plan and check, per line.
- `preview.html`, `shoot.mjs`: the rough preview (three.js from nwn-landscaper-web's `vendor/three`, linked as `three/`), served
  over HTTP, shot with Playwright's Chromium: `node shoot.mjs map lsvale vale out.png`, `node shoot.mjs frames lsvale vale out 0 20`.
