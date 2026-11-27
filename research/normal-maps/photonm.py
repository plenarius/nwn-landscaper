# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
# photonm.py TEXDIR: the demo photos' own normal maps as DXT5nm DDS, oriented as the exported colour picture is. (throwaway)
import os, sys, numpy as np
from PIL import Image
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from normals import read_bioware_dxt1, write_dds
G = '/srv/burst-out/normal-maps'; T = sys.argv[1]
for tid, res in [('gravelPebbles', 'ls_gravelpebbles'), ('floorCobble', 'ls_floorcobble'), ('rock', 'ls_rock')]:
    dds = read_bioware_dxt1(f'{T}/{res}.dds').mean(-1)  # rows as stored: row 0 = v 0
    png = np.asarray(Image.open(f'{G}/demo-pics/{tid}.png').convert('L'), float) / 255  # rows top first
    same = np.corrcoef(dds.ravel(), png.ravel())[0, 1]; flip = np.corrcoef(dds.ravel(), png[::-1].ravel())[0, 1]
    n = np.asarray(Image.open(f'{G}/demo-dl/out/{tid}_n.png').convert('RGB'), float) / 127.5 - 1
    # GL normal: green points up the picture (towards row 0 of the PNG).
    if flip > same:
        n = n[::-1].copy()  # stored bottom first: row index = v = up the picture; green already points +v
    else:
        n[..., 1] *= -1  # stored top first: row index = v = down the picture
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    write_dds(n, f'{G}/nmphoto/ln_{res[3:]}.dds')
    print(tid, 'corr same', round(same, 3), 'flipped', round(flip, 3))
