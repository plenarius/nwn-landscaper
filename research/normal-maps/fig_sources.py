# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
# fig_sources.py: picture 1: CC0 photographs flat, lit by a normal map generated from the photo, lit by the photo's own. (throwaway)
import numpy as np
from PIL import Image, ImageDraw
D = '/srv/burst-out/normal-maps'
L = np.array([-0.6, 0.45, 0.45]); L /= np.linalg.norm(L)
def lit(rgb, n): return np.clip(rgb * (0.25 + 0.75 * np.clip(n @ L, 0, 1) / L[2])[..., None], 0, 1)
def nrm(png):
    a = np.asarray(Image.open(png).convert('RGB'), float) / 127.5 - 1
    return a / np.linalg.norm(a, axis=-1, keepdims=True)
T = 256
rows = []
for t in ['gravelPebbles', 'floorCobble', 'rock']:
    rgb = np.asarray(Image.open(f'{D}/demo-dl/out/{t}_c.png').convert('RGB'), float)[:T, :T] / 255
    # Both PNGs run rows top first; the generated one's green follows the rows (down), the GL one's points up.
    g = nrm(f'{D}/nmdemo/{t}_n.png')[:T, :T]
    s = nrm(f'{D}/demo-dl/out/{t}_n.png')[:T, :T]; s[..., 1] *= -1
    rows.append([rgb, lit(rgb, g), lit(rgb, s)])
W = Image.new('RGB', (3 * T + 2 * 6, 3 * T + 2 * 6 + 24), (255, 255, 255))
d = ImageDraw.Draw(W)
for c, txt in enumerate(['photograph, flat', 'normal map generated from it', "the source's own normal map"]):
    d.text((c * (T + 6) + 4, 6), txt, fill=(0, 0, 0))
for r, row in enumerate(rows):
    for c, img in enumerate(row):
        W.paste(Image.fromarray((img * 255).astype(np.uint8)), (c * (T + 6), 24 + r * (T + 6)))
W.save(f'{D}/cmp/sources.png')
