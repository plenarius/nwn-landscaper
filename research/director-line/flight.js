// Spike (not for merge): nwn-director's flight planner (director.py plan_flight, at main d5129b5) and the camera simulation of
// flightcheck.py, ported to JavaScript so the editor can preview and check a director's line exactly as the director plans it.

export const FLIGHT_WALK = 5.5;
export const FLIGHT_RUN = 11.0;
export const FLIGHT_MAX_SPEED = FLIGHT_RUN * 1.99;
export const TURN_RATE = 0.23;
export const PITCH_RATE = 0.2;
export const DIST_RATE = 2.0;
export const FLIGHT_MAX_DIST = 40.0;
const RAMP_STEPS = 3;

// Python's round(): half to even.
function pyRound(x, nd = 0) {
    const m = 10 ** nd;
    const y = x * m;
    let r = Math.round(y);
    if (Math.abs(y - Math.trunc(y)) === 0.5) r = 2 * Math.round(y / 2);
    return r / m;
}
const dist2 = (p, q) => Math.hypot(p[0] - q[0], p[1] - q[1]);
const deg = r => (r * 180) / Math.PI;
const rad = d => (d * Math.PI) / 180;
const mod = (a, n) => ((a % n) + n) % n;

function easedTime(t, total, ease) {
    if (ease <= 0) return t;
    let w;
    if (t < ease) w = (t * t) / (2 * ease);
    else if (t > total - ease) w = total - ease - (total - t) ** 2 / (2 * ease);
    else w = t - ease / 2;
    return (w * total) / (total - ease);
}

function catmull(p0, p1, p2, p3, u) {
    return [0, 1].map(c => {
        const [a, b, cc, d] = [p0[c], p1[c], p2[c], p3[c]];
        return 0.5 * (2 * b + (-a + cc) * u + (2 * a - 5 * b + 4 * cc - d) * u * u + (-a + 3 * b - 3 * cc + d) * u ** 3);
    });
}

export function flightPosition(keys, t) {
    const pts = keys.map(k => k.at);
    const times = keys.map(k => k.t);
    if (t >= times[times.length - 1]) return pts[pts.length - 1];
    let i = 0;
    for (let j = 0; j < times.length; j++) if (times[j] <= t) i = j;
    const u = (t - times[i]) / (times[i + 1] - times[i]);
    const p0 = i > 0 ? pts[i - 1] : [2 * pts[0][0] - pts[1][0], 2 * pts[0][1] - pts[1][1]];
    const n = pts.length;
    const p3 = i + 2 < n ? pts[i + 2] : [2 * pts[n - 1][0] - pts[n - 2][0], 2 * pts[n - 1][1] - pts[n - 2][1]];
    return catmull(p0, pts[i], pts[i + 1], p3, u);
}

function flightCurve(keys, per = 60) {
    const pts = [];
    for (let i = 0; i < keys.length - 1; i++) {
        const a = keys[i].t,
            b = keys[i + 1].t;
        for (let j = 0; j < per; j++) pts.push(flightPosition(keys, a + ((b - a) * j) / per));
    }
    pts.push(keys[keys.length - 1].at);
    const arc = [0];
    for (let i = 1; i < pts.length; i++) arc.push(arc[i - 1] + dist2(pts[i - 1], pts[i]));
    const atKeys = keys.map((_, i) => arc[i * per]);
    return { pts, arc, atKeys };
}

function along(pts, arc, s) {
    if (s <= 0) return pts[0];
    if (s >= arc[arc.length - 1]) return pts[pts.length - 1];
    let lo = 0,
        hi = arc.length - 1;
    while (hi - lo > 1) {
        const mid = (lo + hi) >> 1;
        if (arc[mid] <= s) lo = mid;
        else hi = mid;
    }
    const u = (s - arc[lo]) / Math.max(arc[hi] - arc[lo], 1e-9);
    return [pts[lo][0] + (pts[hi][0] - pts[lo][0]) * u, pts[lo][1] + (pts[hi][1] - pts[lo][1]) * u];
}

function flightDistance(keys, ease, blend = 4.0, dt = 0.05) {
    const { pts, arc, atKeys } = flightCurve(keys);
    const total = keys[keys.length - 1].t;
    const n = Math.round(total / dt);
    const times = Array.from({ length: n + 1 }, (_, i) => i * dt);
    let speeds = [];
    for (const t of times.slice(0, -1)) {
        let i = keys.length - 2;
        if (t < total) for (let j = 0; j < keys.length - 1; j++) if (keys[j].t <= t + 1e-9) i = j;
        speeds.push((atKeys[i + 1] - atKeys[i]) / (keys[i + 1].t - keys[i].t));
    }
    const w = Math.max(1, Math.round(blend / dt));
    for (let pass = 0; pass < 2; pass++) {
        const sm = [];
        for (let i = 0; i < speeds.length; i++) {
            const lo = Math.max(0, i - (w >> 1)),
                hi = Math.min(speeds.length, i + (w >> 1) + 1);
            let s = 0;
            for (let j = lo; j < hi; j++) s += speeds[j];
            sm.push(s / (hi - lo));
        }
        speeds = sm;
    }
    const dist = [0];
    for (const v of speeds) dist.push(dist[dist.length - 1] + v * dt);
    const scale = dist[dist.length - 1] > 0 ? arc[arc.length - 1] / dist[dist.length - 1] : 0;
    const d2 = dist.map(x => x * scale);
    const eased = times.map(t => {
        const te = easedTime(t, total, ease);
        const k = Math.min(Math.trunc(te / dt), n - 1);
        const u = (te - k * dt) / dt;
        return d2[k] + (d2[k + 1] - d2[k]) * u;
    });
    return { times, eased, pts, arc };
}

function pace(v) {
    const base = v <= FLIGHT_WALK ? FLIGHT_WALK : FLIGHT_RUN;
    const pct = pyRound((v / base - 1) * 100);
    return [base === FLIGHT_RUN, Math.max(-99, Math.min(99, pct))];
}

export const turns = (a, b) => mod(b - a + 180, 360) - 180;

function cameraSpeed(turn, seconds, tilt = 0, move = 0) {
    if (seconds <= 0) return 0;
    const n = Math.max(
        (Math.abs(turn) / seconds / TURN_RATE) ** (2 / 3),
        Math.abs(tilt) / seconds / PITCH_RATE,
        20 * (Math.abs(move) / seconds / DIST_RATE) ** (1 / 0.63),
    );
    return Math.max(1, Math.min(100, pyRound(n)));
}

/** The flight as the director films it: the carrier's legs and the camera's moves (director.py plan_flight). Throws where the
 *  director would stop, with its reason. */
export function planFlight(spec) {
    const keys = spec.keys;
    const total = keys[keys.length - 1].t;
    const ease = spec.ease ?? 3.0;
    const step = spec.step ?? 1.0;
    const pivot = spec.pivot ?? 2.0;
    const ramp = spec.ramp ?? 2.0;
    const name = spec.name || `${spec.area}-flight`;
    const start = flightPosition(keys, 0);
    const { times, eased: dist, pts, arc } = flightDistance(keys, ease);
    const gap = times[1] - times[0];
    const legs = [];
    let here = start,
        t = 0;
    while (t < total - 1e-6) {
        const dt = Math.min(step, total - t);
        const there = along(pts, arc, dist[Math.min(dist.length - 1, pyRound((t + dt) / gap))]);
        const v = dist2(here, there) / dt;
        if (v > FLIGHT_MAX_SPEED)
            throw new Error(`the carrier would have to move ${v.toFixed(1)} m/s around ${t.toFixed(0)} s: give its keys more time`);
        if (v < 0.06) legs.push({ wait: pyRound(dt, 2) });
        else {
            const [run, pct] = pace(v);
            legs.push({ to: there.map(c => pyRound(c, 2)), run: run ? 1 : 0, pct, secs: pyRound(dt, 2) });
        }
        here = there;
        t += dt;
    }
    const camera = k => {
        const p = flightPosition(keys, k.t);
        let heading;
        if (k.lookAt) heading = mod(deg(Math.atan2(k.lookAt[1] - p[1], k.lookAt[0] - p[0])), 360);
        else if (k.heading !== undefined) heading = mod(k.heading, 360);
        else if (k.t < total) {
            const q = flightPosition(keys, Math.min(total, k.t + 0.5));
            heading = mod(deg(Math.atan2(q[1] - p[1], q[0] - p[0])), 360);
        } else {
            const q = flightPosition(keys, total - 0.5);
            heading = mod(deg(Math.atan2(p[1] - q[1], p[0] - q[0])), 360);
        }
        const d = k.distance !== undefined ? k.distance : (k.height - pivot) / Math.cos(rad(k.pitch));
        if (!(d >= 1 && d <= FLIGHT_MAX_DIST))
            throw new Error(`key at ${k.t} s puts the camera ${d.toFixed(1)} m from the carrier (1 to 40)`);
        return { dir: pyRound(heading, 1), dist: pyRound(d, 2), pitch: k.pitch };
    };
    const cams = [{ ...camera(keys[0]), t: 0, speed: 0 }];
    let prev = cams[0];
    for (let i = 0; i < keys.length - 1; i++) {
        const a = keys[i],
            b = keys[i + 1];
        const target = camera(b);
        const span = b.t - a.t;
        const lead = Math.min(ramp, span / 4);
        const speed = cameraSpeed(turns(prev.dir, target.dir), span - lead / 2, target.pitch - prev.pitch, target.dist - prev.dist);
        if (lead > 0 && speed > 2) {
            for (let j = 0; j < RAMP_STEPS; j++)
                cams.push({
                    t: pyRound(a.t + (lead * j) / RAMP_STEPS, 2),
                    ...target,
                    speed: Math.max(1, pyRound((speed * (j + 1)) / (RAMP_STEPS + 1))),
                });
            cams.push({ t: pyRound(a.t + lead, 2), ...target, speed });
        } else cams.push({ t: a.t, ...target, speed });
        prev = cams[cams.length - 1];
    }
    return { name, area: spec.area, at: start.map(c => pyRound(c, 2)), seconds: total, pivot, legs, cams };
}

/** Where the carrier is at time t: straight between its legs' ends, as the creature walks them (flightcheck.py carrier). */
export function carrierTrack(shot) {
    const track = [[0, shot.at]];
    for (const leg of shot.legs) {
        const [t0, p] = track[track.length - 1];
        track.push(leg.wait !== undefined ? [t0 + leg.wait, p] : [t0 + leg.secs, leg.to]);
    }
    return t => {
        for (let i = 0; i < track.length - 1; i++) {
            const [ta, pa] = track[i],
                [tb, pb] = track[i + 1];
            if (ta <= t && t <= tb) {
                const u = (t - ta) / Math.max(tb - ta, 1e-9);
                return [pa[0] + (pb[0] - pa[0]) * u, pa[1] + (pb[1] - pa[1]) * u];
            }
        }
        return track[track.length - 1][1];
    };
}

/** The camera's direction, distance and pitch at time t: each move runs every part at its measured rate, then holds
 *  (flightcheck.py camera_at). */
export function cameraAt(cams, t) {
    let val = { dir: cams[0].dir, dist: cams[0].dist, pitch: cams[0].pitch };
    for (let i = 0; i < cams.length - 1; i++) {
        const c = cams[i + 1];
        if (t <= c.t) break;
        const n = Math.max(c.speed, 1);
        const nxt = i + 2 < cams.length ? cams[i + 2].t : 1e9;
        const el = Math.min(t - c.t, nxt - c.t);
        const rates = { dir: TURN_RATE * n ** 1.5, pitch: PITCH_RATE * n, dist: DIST_RATE * (n / 20) ** 0.63 };
        const delta = { dir: turns(val.dir, c.dir), pitch: c.pitch - val.pitch, dist: c.dist - val.dist };
        const out = {};
        for (const k of ['dir', 'pitch', 'dist']) {
            const s = Math.min(Math.abs(delta[k]), rates[k] * el);
            out[k] = val[k] + Math.sign(delta[k]) * s;
        }
        val = out;
    }
    return val;
}

/** The flight's pose at time t over ground `h(x, y)`: the carrier, the pivot the camera turns about, and the camera. */
export function poser(shot, h) {
    const carrier = carrierTrack(shot);
    return t => {
        const [cx, cy] = carrier(t);
        const g = h(cx, cy);
        const c = cameraAt(shot.cams, t);
        const r = rad(c.dir);
        const horiz = c.dist * Math.sin(rad(c.pitch));
        const camera = [cx - Math.cos(r) * horiz, cy - Math.sin(r) * horiz, g + shot.pivot + c.dist * Math.cos(rad(c.pitch))];
        return { t, carrier: [cx, cy, g], pivot: [cx, cy, g + shot.pivot], camera, ...c };
    };
}

/** An area file's ground (heights on the 0.625 m grid), bilinear, as flightcheck.py reads it. */
export function ground(area) {
    const bin = typeof Buffer !== 'undefined' ? Buffer.from(area.heights, 'base64') : Uint8Array.from(atob(area.heights), c => c.charCodeAt(0));
    const H = new Float32Array(bin.buffer.slice(bin.byteOffset, bin.byteOffset + bin.length));
    const NX = area.tiles * 16 + 1;
    const SX = area.tiles * 10,
        SY = area.tilesY * 10,
        SP = 0.625;
    const h = (x, y) => {
        x = Math.min(Math.max(x, 0), SX - 1e-3);
        y = Math.min(Math.max(y, 0), SY - 1e-3);
        const i = x / SP,
            j = y / SP;
        const i0 = Math.trunc(i),
            j0 = Math.trunc(j);
        const fx = i - i0,
            fy = j - j0;
        const g = (a, b) => H[b * NX + a];
        return (
            g(i0, j0) * (1 - fx) * (1 - fy) + g(i0 + 1, j0) * fx * (1 - fy) + g(i0, j0 + 1) * (1 - fx) * fy + g(i0 + 1, j0 + 1) * fx * fy
        );
    };
    h.size = [SX, SY];
    h.slope = (x, y) => Math.max(Math.abs(h(x + SP, y) - h(x, y)), Math.abs(h(x, y + SP) - h(x, y)));
    return h;
}

/** flightcheck.py's checks every half second: the camera in the ground, an object or a wall, outside the area; the carrier
 *  into an object, on ground too steep to walk, in water. */
export function checkFlight(area, shot, dims, cave = false) {
    const h = ground(area);
    const [SX, SY] = h.size;
    const water = (area.water ?? []).map(w => w.level);
    const top = water.length ? Math.max(...water) : null;
    const objs = area.objects.map(o => {
        const m = o.model.toLowerCase();
        const [tall, r0] = dims[m] ?? [13.0, 3.2];
        const r = r0 * 0.8 + 0.5;
        const s = o.scale ?? 1;
        return { x: o.x, y: o.y, z: o.z ?? h(o.x, o.y), tall: tall * s + 0.5, rad: r * s, m };
    });
    const pose = poser(shot, h);
    const out = [];
    let minClear = 1e9;
    for (let i = 0; i * 0.5 <= shot.seconds + 1e-6; i++) {
        const t = i * 0.5;
        const p = pose(t);
        const [cx, cy, g] = p.carrier;
        const [px, py, pz] = p.camera;
        let gr = h(px, py);
        if (top !== null && !cave) gr = Math.max(gr, gr < top ? top : gr);
        const clear = pz - gr;
        minClear = Math.min(minClear, clear);
        const note = [];
        if (!(px >= 0 && px <= SX && py >= 0 && py <= SY)) note.push('camera outside the area');
        if (clear < 1.0) note.push(`camera ${clear.toFixed(1)} m over the ground`);
        if (cave && (gr > g + 3.5 || pz > Math.max(g, gr) + 5.0)) note.push('camera in or over a wall');
        for (const o of objs) {
            if (Math.hypot(px - o.x, py - o.y) < o.rad && o.z - 0.5 < pz && pz < o.z + o.tall) note.push(`camera in ${o.m}`);
            const block = o.m.includes('tree') || o.m.includes('_pn') || o.m.includes('dtr') ? 1.2 : o.rad * 0.6 + 0.3;
            if (Math.hypot(cx - o.x, cy - o.y) < block) note.push(`carrier runs into ${o.m}`);
        }
        if (h.slope(cx, cy) > (0.62 * 0.62) / 0.64) note.push('carrier on ground too steep to walk');
        if (top !== null && h(cx, cy) < top - 0.3 && !cave) note.push('carrier in water');
        p.clear = clear;
        p.notes = [...new Set(note)].sort();
        out.push(p);
    }
    return { samples: out, minClear, issues: out.filter(p => p.notes.length) };
}
