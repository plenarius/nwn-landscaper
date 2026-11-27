// The JS port against nwn-director's planner and flightcheck.py: the same legs and camera moves, and the same poses every 0.5 s.
import fs from 'node:fs';
import { planFlight, checkFlight } from './flight.js';
const [inp] = process.argv.slice(2);
const ref = JSON.parse(fs.readFileSync('ref.json'));
const dims = JSON.parse(fs.readFileSync(`${inp}/model_dims.json`));
let worst = 0;
const report = {};
for (const [plan, r] of Object.entries(ref)) {
    const spec = JSON.parse(fs.readFileSync(`${inp}/${plan}.json`));
    const area = JSON.parse(fs.readFileSync(`${inp}/${r.area}.json`));
    const shot = planFlight(spec);
    const same = JSON.stringify({ at: shot.at, legs: shot.legs, cams: shot.cams }) ===
        JSON.stringify({ at: r.shot.at, legs: r.shot.legs, cams: r.shot.cams });
    const chk = checkFlight(area, shot, dims, plan === 'cave');
    let dPos = 0, dAng = 0;
    chk.samples.forEach((p, i) => {
        const q = r.rows[i];
        for (const k of ['carrier', 'camera']) for (let c = 0; c < 3; c++) dPos = Math.max(dPos, Math.abs(p[k][c] - q[k][c]));
        for (const k of ['dir', 'pitch']) dAng = Math.max(dAng, Math.abs(p[k] - q[k]));
    });
    worst = Math.max(worst, dPos);
    report[plan] = { legsAndCamsIdentical: same, samples: chk.samples.length, maxPosDiff_m: dPos, maxAngleDiff_deg: dAng,
        issues: chk.issues.length, refIssues: r.issues.length, minClear: +chk.minClear.toFixed(2), refMinClear: +r.minclear.toFixed(2) };
}
console.log(JSON.stringify(report, null, 1));
