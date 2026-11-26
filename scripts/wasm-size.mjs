// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

// The browser build's size, raw, with gzip and with brotli (what a visitor downloads), and a failure over the limit: 3 MB with
// brotli (docs/DESIGN-web-demo.md, "Measured": 2.0 MB at the start).
import { readFileSync } from 'node:fs';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const LIMIT_MB = 3;
const file = process.argv[2];
const raw = readFileSync(file);
const brotli = brotliCompressSync(raw, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: raw.length } });
const mb = n => (n / 1e6).toFixed(2);
console.log(`${file}: ${mb(raw.length)} MB, ${mb(gzipSync(raw, { level: 9 }).length)} MB with gzip, ${mb(brotli.length)} MB with brotli`);
if (brotli.length > LIMIT_MB * 1e6) {
    console.error(`over the limit of ${LIMIT_MB} MB with brotli: what grew it? (docs/DESIGN-web-demo.md)`);
    process.exit(1);
}
