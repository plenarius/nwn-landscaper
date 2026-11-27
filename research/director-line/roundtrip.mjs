// A hand-written plan into the proposed area-file form and back out as the export, and each key into the 3D view's orbit camera
// (position and target, as "Key from view" would read it) and back.
import fs from 'node:fs';
import { flightPosition, ground } from './flight.js';
const toLine = (plan, id) => { const { area, ...rest } = plan; return { id, ...rest }; };
const toPlan = (areaName, line) => { const { id, ...rest } = line; return { area: areaName, ...rest }; };
const rad = d => d * Math.PI / 180, deg = r => r * 180 / Math.PI, mod = (a, n) => ((a % n) + n) % n;
const out = {};
for (const [f, a] of [['vale', 'lsvale'], ['canyon', 'lscanyon'], ['dunes', 'lsdunes'], ['pass', 'lspass'], ['cave', 'lsdescent']]) {
    const text = fs.readFileSync(`../input/${f}.json`, 'utf8');
    const plan = JSON.parse(text);
    const areaFile = JSON.parse(fs.readFileSync(`../input/${a}.json`));
    areaFile.flights = [toLine(plan, 1)];
    const saved = JSON.parse(JSON.stringify(areaFile));              // saved and opened again
    const back = toPlan(saved.name, saved.flights[0]);
    const planSame = JSON.stringify(back) === JSON.stringify(plan);
    // key -> orbit camera -> key
    const h = ground(areaFile), pivot = plan.pivot ?? 2, total = plan.keys.at(-1).t;
    let e = { at: 0, height: 0, pitch: 0, heading: 0 };
    for (const k of plan.keys) {
        const p = flightPosition(plan.keys, k.t);   // the carrier is on the curve through the keys: at the key itself
        let heading = k.heading;
        if (k.lookAt) heading = mod(deg(Math.atan2(k.lookAt[1] - p[1], k.lookAt[0] - p[0])), 360);
        if (heading === undefined) { const q = flightPosition(plan.keys, Math.min(total, k.t + 0.5)); heading = mod(deg(Math.atan2(q[1] - p[1], q[0] - p[0])), 360); }
        const dist = k.distance ?? (k.height - pivot) / Math.cos(rad(k.pitch));
        const target = [k.at[0], k.at[1], h(...k.at) + pivot];
        const horiz = dist * Math.sin(rad(k.pitch));
        const position = [target[0] - Math.cos(rad(heading)) * horiz, target[1] - Math.sin(rad(heading)) * horiz, target[2] + dist * Math.cos(rad(k.pitch))];
        // back, from the orbit camera alone
        const d = Math.hypot(position[0] - target[0], position[1] - target[1], position[2] - target[2]);
        const at = [target[0], target[1]];
        const pv = target[2] - h(...at);
        const pitch = deg(Math.acos((position[2] - target[2]) / d));
        const hd = mod(deg(Math.atan2(target[1] - position[1], target[0] - position[0])), 360);
        const height = pv + d * Math.cos(rad(pitch));
        e.at = Math.max(e.at, Math.hypot(at[0] - k.at[0], at[1] - k.at[1]));
        if (k.height !== undefined) e.height = Math.max(e.height, Math.abs(height - k.height));
        e.pitch = Math.max(e.pitch, Math.abs(pitch - k.pitch));
        e.heading = Math.max(e.heading, Math.abs(((hd - heading + 540) % 360) - 180));
    }
    out[f] = { planSame, bytesInAreaFile: JSON.stringify(areaFile.flights).length, keyViewKeyMaxError: e };
}
console.log(JSON.stringify(out, null, 1));
