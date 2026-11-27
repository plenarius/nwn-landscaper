import fs from 'node:fs';
import { planFlight, checkFlight } from './flight.js';
const dims = JSON.parse(fs.readFileSync('model_dims.json'));
for (const [f, a, cave] of [['vale','lsvale'],['canyon','lscanyon'],['dunes','lsdunes'],['pass','lspass'],['cave','lsdescent',true]]) {
  const area = JSON.parse(fs.readFileSync(`${a}.json`)), spec = JSON.parse(fs.readFileSync(`${f}.json`));
  for (let i = 0; i < 5; i++) checkFlight(area, planFlight(spec), dims, cave);
  const t0 = performance.now(); for (let i = 0; i < 20; i++) checkFlight(area, planFlight(spec), dims, cave);
  console.log(f, ((performance.now() - t0) / 20).toFixed(1), 'ms');
}
