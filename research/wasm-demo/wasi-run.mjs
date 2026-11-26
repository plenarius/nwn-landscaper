// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

// As wasi-run.mjs, with the game install preopened read-only as /game.
import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';
const [wasmPath, dir, game, ...args] = process.argv.slice(2);
const wasi = new WASI({ version: 'preview1', args: ['landscaper', ...args], env: { HOME: '/work', LANDSCAPER_JOBS: '1' }, preopens: { '/work': dir, '/game': game }, returnOnExit: true });
const mod = await WebAssembly.compile(await readFile(wasmPath));
const t1 = performance.now();
const code = wasi.start(await WebAssembly.instantiate(mod, wasi.getImportObject()));
console.error(`[run ${(performance.now() - t1).toFixed(0)} ms, exit ${code}]`);
