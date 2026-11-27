import fs from 'node:fs';
import { planFlight, cameraAt, turns, TURN_RATE, PITCH_RATE, DIST_RATE } from './flight.js';
for (const f of ['vale', 'canyon', 'dunes', 'pass', 'cave']) {
    const plan = JSON.parse(fs.readFileSync(`../input/${f}.json`));
    const shot = planFlight(plan);
    // miss at each key time: the simulated camera against the move's target
    let miss = { dir: 0, pitch: 0, dist: 0 };
    const keyT = plan.keys.map(k => k.t);
    for (let i = 1; i < keyT.length; i++) {
        const target = shot.cams.filter(c => c.t < keyT[i]).at(-1);
        const c = cameraAt(shot.cams, keyT[i] - 1e-6);
        miss.dir = Math.max(miss.dir, Math.abs(turns(c.dir, target.dir)));
        miss.pitch = Math.max(miss.pitch, Math.abs(c.pitch - target.pitch));
        miss.dist = Math.max(miss.dist, Math.abs(c.dist - target.dist));
    }
    // holds: stretches where the camera's direction, pitch and distance all stand still while the flight runs
    let still = 0, longest = 0, run = 0;
    for (let t = 0.1; t < shot.seconds - plan.ease; t += 0.1) {
        const a = cameraAt(shot.cams, t - 0.1), b = cameraAt(shot.cams, t);
        const moving = Math.abs(turns(a.dir, b.dir)) > 1e-4 || Math.abs(a.pitch - b.pitch) > 1e-4 || Math.abs(a.dist - b.dist) > 1e-4;
        if (!moving && t > (plan.ease ?? 3)) { still += 0.1; run += 0.1; longest = Math.max(longest, run); } else run = 0;
    }
    console.log(f.padEnd(7), 'miss at keys: dir', miss.dir.toFixed(2), 'deg, pitch', miss.pitch.toFixed(2), 'deg, dist', miss.dist.toFixed(2),
        'm | camera still', still.toFixed(1), 's of', shot.seconds, ', longest', longest.toFixed(1), 's | speeds', shot.cams.map(c => c.speed).join(','));
}
