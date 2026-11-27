# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
# fig.py OUT.webp SPEC.json: the design's comparison pictures: rows of [label, before still, after still, crop box], side by side,
# at full size (no scaling), labelled. (throwaway)
import json, sys
from PIL import Image, ImageDraw
out, spec = sys.argv[1], json.load(open(sys.argv[2]))
G = "/srv/burst-out/normal-maps/stills/"
rows = []
for label_a, a, label_b, b, box in spec:
    A, B = Image.open(G + a).convert("RGB").crop(box), Image.open(G + b).convert("RGB").crop(box)
    rows.append((label_a, A, label_b, B))
w = max(r[1].width for r in rows) * 2 + 8
h = sum(r[1].height for r in rows) + 8 * (len(rows) - 1)
W = Image.new("RGB", (w, h), (255, 255, 255))
d = ImageDraw.Draw(W)
y = 0
for la, A, lb, B in rows:
    for x, img, t in ((0, A, la), (A.width + 8, B, lb)):
        W.paste(img, (x, y))
        d.rectangle((x + 4, y + 4, x + 12 + 6 * len(t), y + 20), fill=(0, 0, 0))
        d.text((x + 8, y + 7), t, fill=(255, 255, 255))
    y += A.height + 8
W.save(out, "WEBP", quality=88, method=6)
print(out, W.size)
