// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

// One API request through the engine's browser entry point (wasi-request), under Node's WASI. Prints the raw answer.
import { readFile, writeFile } from 'node:fs/promises';
import { openSync } from 'node:fs';
import { WASI } from 'node:wasi';
const TMP = process.env.TMPDIR ?? '/tmp';
const [wasmPath, dir, game, method, path, bodyFile] = process.argv.slice(2);
const body = bodyFile ? await readFile(bodyFile) : Buffer.alloc(0);
const head = `${method} ${path} HTTP/1.1\r\nHost: localhost\r\nX-Landscaper: 1\r\nContent-Type: application/json\r\nContent-Length: ${body.length}\r\n\r\n`;
await writeFile(`${TMP}/req.bin`, Buffer.concat([Buffer.from(head), body]));
const env = { HOME: '/work', LANDSCAPER_JOBS: '1' };
const preopens = { '/work': dir, '/web': dir };
if (game !== '-') { env.NWN_ROOT = '/game'; preopens['/game'] = game; }
env.NWN_HOME = '/work/user';
const wasi = new WASI({ version: 'preview1', args: ['landscaper', 'wasi-request'], env, preopens, returnOnExit: true,
  stdin: openSync(`${TMP}/req.bin`, 'r'), stdout: openSync(`${TMP}/resp.bin`, 'w') });
const mod = await WebAssembly.compile(await readFile(wasmPath));
const t1 = performance.now();
const code = wasi.start(await WebAssembly.instantiate(mod, wasi.getImportObject()));
const out = await readFile(`${TMP}/resp.bin`);
console.error(`[${method} ${path}: run ${(performance.now() - t1).toFixed(0)} ms, exit ${code}, ${out.length} bytes]`);
process.stdout.write(out.subarray(0, 1500));
