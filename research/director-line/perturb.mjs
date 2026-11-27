import fs from 'node:fs';
import { planFlight, checkFlight } from './flight.js';
const dims = JSON.parse(fs.readFileSync('../input/model_dims.json'));
for (const [f, a, cave] of [['vale', 'lsvale'], ['dunes', 'lsdunes'], ['cave', 'lsdescent', true]]) {
    const area = JSON.parse(fs.readFileSync(`../input/${a}.json`));
    const spec = JSON.parse(fs.readFileSync(`bad-${f}.json`));
    const t0 = performance.now();
    let r;
    for (let i = 0; i < 20; i++) r = checkFlight(area, planFlight(spec), dims, cave);
    const ms = (performance.now() - t0) / 20;
    console.log(f, 'js issues', r.issues.length, 'first', r.issues[0]?.t, r.issues[0]?.notes.join('; '), `plan+check ${ms.toFixed(1)} ms`);
}
