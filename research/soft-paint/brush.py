# brush.py FEATHER OUT: per-texture weights as a soft round brush would paint the trail: gravel along the trail's centreline
# (smoothed from today's paint), radius its half width, falling off over FEATHER m; under it, the other textures as painted
# (gravel's own samples taken by their nearest other texture).
import sys, numpy as np
from scipy.ndimage import gaussian_filter, distance_transform_edt
S = "/srv/burst-out/soft-paint"; SP = 0.625; W, H, P = 257, 225, 6; G = 4
feather = float(sys.argv[1]); out = sys.argv[2]; sig = float(sys.argv[3]) if len(sys.argv) > 3 else 0.0
m = np.fromfile(f"{S}/areas/spa.materials", np.uint8).reshape(H, W)
# the trail: gravel in rows 96..130 (the standing stones' rock rings are another texture)
cx, cy, wd = [], [], []
for i in range(W):
    js = np.where(m[96:131, i] == G)[0] + 96
    if len(js): cx.append(i * SP); cy.append(js.mean() * SP); wd.append(len(js) * SP)
cx, cy, wd = np.array(cx), np.array(cy), np.array(wd)
k = np.ones(9) / 9  # ~5.6 m moving average: the stroke's path, not the grid's steps
cys = np.convolve(np.pad(cy, 4, mode="edge"), k, "valid")
r = (np.median(wd) - SP / 2) / 2
print("trail half width", r, "points", len(cx))
X, Y = np.meshgrid(np.arange(W) * SP, np.arange(H) * SP)
# distance to the polyline
pts = np.stack([cx, cys], 1); d = np.full(X.shape, 1e9)
for (ax, ay), (bx, by) in zip(pts[:-1], pts[1:]):
    vx, vy = bx - ax, by - ay; L2 = vx * vx + vy * vy
    u = np.clip(((X - ax) * vx + (Y - ay) * vy) / L2, 0, 1)
    d = np.minimum(d, np.hypot(X - (ax + u * vx), Y - (ay + u * vy)))
t = np.clip((r + feather / 2 - d) / feather, 0, 1); c = t * t * (3 - 2 * t)
c[(m == G) & (np.arange(H)[:, None] < 96)] = 0; c[(m == G) & (np.arange(H)[:, None] > 130)] = 0
# the ground under the trail: each gravel sample takes its nearest other texture
idx = distance_transform_edt(m == G, return_distances=False, return_indices=True)
under = m[idx[0], idx[1]]
w = np.zeros((P, H, W), np.float32)
oth = np.stack([(under == p).astype(np.float32) for p in range(P)])
if sig > 0:
    oth = np.stack([gaussian_filter(o, sig / SP, mode="nearest") for o in oth])
    oth /= np.maximum(oth.sum(0), 1e-6)
for p in range(P):
    if p != G: w[p] = oth[p] * (1 - c)
w[G] = c
# gravel elsewhere (outside the band) kept as painted
far = (m == G) & ((np.arange(H)[:, None] < 96) | (np.arange(H)[:, None] > 130))
w[:, far] = 0; w[G][far] = 1
w.astype("<f4").tofile(out); print(out, w.shape, w.sum(0).min(), w.sum(0).max())
