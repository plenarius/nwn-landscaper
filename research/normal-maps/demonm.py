# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
# demonm.py: the demo pack's own normal maps for three textures, fitted as make.py fits the colour, and their WebP size. (throwaway)
import io, json, urllib.request, zipfile, sys
from PIL import Image
A = {'User-Agent': 'landscaper-research'}
D = '/srv/burst-out/normal-maps/demo-dl'
def get(url): return urllib.request.urlopen(urllib.request.Request(url, headers=A), timeout=120).read()
def ph(asset):
    info = json.loads(get(f'https://api.polyhaven.com/info/{asset}')); files = json.loads(get(f'https://api.polyhaven.com/files/{asset}'))
    return get(files['nor_gl']['1k']['jpg']['url']), (info.get('dimensions') or [2000])[0] / 1000, get(files['Diffuse']['1k']['jpg']['url'])
def acg(asset):
    z = zipfile.ZipFile(io.BytesIO(get(f'https://ambientcg.com/get?file={asset}_1K-JPG.zip')))
    return z.read(next(n for n in z.namelist() if n.endswith('_NormalGL.jpg'))), None, z.read(next(n for n in z.namelist() if n.endswith('_Color.jpg')))
tot = 0
for tid, src in [('gravelPebbles', 'polyhaven:ganges_river_pebbles'), ('floorCobble', 'polyhaven:cobblestone_floor_001'), ('rock', 'ambientcg:Rock030')]:
    site, asset = src.split(':')
    pic, metres, col = (ph if site == 'polyhaven' else acg)(asset)
    for what, data in (('_c', col), ('_n', pic)):
      img = Image.open(io.BytesIO(data)).convert('RGB')
      n = max(1, round(4.0 / metres)) if metres else 1
      if n > 1:
        t = Image.new('RGB', (img.width * n, img.height * n))
        for y in range(n):
            for x in range(n): t.paste(img, (x * img.width, y * img.height))
        img = t
      img = img.resize((512, 512), Image.Resampling.LANCZOS)
      img.save(f'{D}/out/{tid}{what}.png')
    buf = io.BytesIO(); img.save(buf, 'WEBP', quality=82, method=6); tot += len(buf.getvalue())
    open(f'{D}/out/{tid}_n.webp', 'wb').write(buf.getvalue())
    print(tid, src, 'repeats', n, 'webp', len(buf.getvalue()))
print('total', tot)
