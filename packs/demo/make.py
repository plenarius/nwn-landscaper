#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
"""Makes the web demo's texture pack: a picture for each starter texture id, none of them the game's.

Reads sources.json, fetches each CC0 picture (ambientCG, Poly Haven) into a cache, repeats it to cover the 4 m the Landscaper
repeats a texture over, fits it to SIZE px and writes pictures/<id>.webp. A `standin` is made here from another picture of the
pack. Writes CREDITS.md and pack.json (what the page installs) from the same list. Needs Pillow; run with --cache to keep the
downloads between runs. The same sources make the same pictures.

    python3 packs/demo/make.py [--cache DIR] [--quality Q]
"""

import argparse
import hashlib
import io
import json
import os
import urllib.request
import zipfile

from PIL import Image, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
# The ground one texture repeat covers in the Landscaper (src/paint.rs, TEXTURE_METRES).
REPEAT_METRES = 4.0
SIZE = 512
AGENT = {'User-Agent': 'texture-pack-maker/1.0'}
LICENCES = {
    'ambientcg': ('CC0-1.0', 'ambientCG', 'https://ambientcg.com/view?id={}'),
    'polyhaven': ('CC0-1.0', 'Poly Haven', 'https://polyhaven.com/a/{}'),
}


def fetch(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers=AGENT), timeout=120) as r:
        return r.read()


def cached(cache, name, url):
    path = os.path.join(cache, name)
    if not os.path.exists(path):
        data = fetch(url)
        with open(path + '.part', 'wb') as f:
            f.write(data)
        os.replace(path + '.part', path)
    with open(path, 'rb') as f:
        return f.read()


def source(cache, site, asset):
    """The asset's colour picture, its size on the ground in metres and its authors."""
    if site == 'polyhaven':
        info = json.loads(cached(cache, f'ph-{asset}.json', f'https://api.polyhaven.com/info/{asset}'))
        files = json.loads(cached(cache, f'ph-{asset}-files.json', f'https://api.polyhaven.com/files/{asset}'))
        url = files['Diffuse']['1k']['jpg']['url']
        picture = cached(cache, f'ph-{asset}.jpg', url)
        metres = (info.get('dimensions') or [2000])[0] / 1000
        return picture, metres, sorted(info.get('authors', {}))
    if site == 'ambientcg':
        archive = cached(cache, f'acg-{asset}.zip', f'https://ambientcg.com/get?file={asset}_1K-JPG.zip')
        with zipfile.ZipFile(io.BytesIO(archive)) as z:
            name = next(n for n in z.namelist() if n.endswith('_Color.jpg'))
            picture = z.read(name)
        return picture, None, ['ambientCG']
    raise SystemExit(f'unknown source {site}:{asset}')


def fitted(picture, metres):
    """`picture` repeated to cover REPEAT_METRES (whole repeats only, so it still tiles), at SIZE px."""
    img = Image.open(io.BytesIO(picture)).convert('RGB')
    n = max(1, round(REPEAT_METRES / metres))
    if n > 1:
        tiled = Image.new('RGB', (img.width * n, img.height * n))
        for y in range(n):
            for x in range(n):
                tiled.paste(img, (x * img.width, y * img.height))
        img = tiled
    return img.resize((SIZE, SIZE), Image.Resampling.LANCZOS)


def standin(base, spec):
    """A picture made from another of the pack's: tinted (per channel) and softened."""
    img = base
    if spec.get('smooth'):
        # Blurred with the picture's own edges wrapped round, so it still tiles.
        w, h = img.size
        wrap = Image.new('RGB', (w * 3, h * 3))
        for y in range(3):
            for x in range(3):
                wrap.paste(img, (x * w, y * h))
        img = wrap.filter(ImageFilter.GaussianBlur(spec['smooth'])).crop((w, h, 2 * w, 2 * h))
    if spec.get('tint'):
        r, g, b = img.split()
        k = spec['tint']
        img = Image.merge('RGB', [c.point(lambda v, m=m: min(255, round(v * m))) for c, m in zip((r, g, b), k)])
    return img


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--cache', default=os.path.join(HERE, '.cache'))
    ap.add_argument('--quality', type=int, default=82)
    args = ap.parse_args()
    os.makedirs(args.cache, exist_ok=True)
    out = os.path.join(HERE, 'pictures')
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(HERE, 'sources.json')) as f:
        sources = json.load(f)['textures']
    with open(os.path.join(HERE, '..', '..', 'src', 'starter_textures.json')) as f:
        starter = {t['id']: t for t in json.load(f)['textures']}
    missing = sorted(set(starter) - {t['id'] for t in sources})
    if missing:
        raise SystemExit(f'sources.json has no picture for the starter ids {", ".join(missing)}')

    made, entries, credits = {}, [], []
    for t in sorted(sources, key=lambda t: 'standin' in t):
        if 'standin' in t:
            img = standin(made[t['standin']['of']], t['standin'])
            credit = {'id': t['id'], 'source': f'made by make.py from {t["standin"]["of"]}', 'licence': 'as that picture'}
        else:
            site, asset = t['from'].split(':', 1)
            picture, metres, authors = source(args.cache, site, asset)
            metres = t.get('metres', metres) or 2.0
            img = fitted(picture, metres)
            licence, name, page = LICENCES[site]
            credit = {'id': t['id'], 'source': f'{name}: {asset}', 'url': page.format(asset), 'authors': authors, 'licence': licence}
        made[t['id']] = img
        buf = io.BytesIO()
        img.save(buf, 'WEBP', quality=args.quality, method=6)
        data = buf.getvalue()
        file = f'{t["id"]}.webp'
        with open(os.path.join(out, file), 'wb') as f:
            f.write(data)
        entries.append({'id': t['id'], 'file': file, 'sha256': hashlib.sha256(data).hexdigest()})
        credits.append(credit)

    order = list(starter)
    entries.sort(key=lambda e: order.index(e['id']))
    credits.sort(key=lambda c: order.index(c['id']))
    total = sum(os.path.getsize(os.path.join(out, e['file'])) for e in entries)
    with open(os.path.join(HERE, 'pack.json'), 'w') as f:
        json.dump({'format': 1, 'size': SIZE, 'bytes': total, 'textures': entries}, f, indent=1)
        f.write('\n')
    write_credits(credits, starter)
    print(f'{len(entries)} pictures, {total / 1024:.0f} KB')


def write_credits(credits, starter):
    lines = [
        '# Credits: the web demo\'s textures',
        '',
        'Every picture in `pictures/` and where it comes from. None is from Neverwinter Nights. The photographs are CC0 (public',
        'domain): no credit is required, and it is given anyway. Ask and a picture is taken out.',
        '',
        '| Texture | Name in the editor | Source | Authors | Licence |',
        '|---|---|---|---|---|',
    ]
    for c in credits:
        src = f'[{c["source"]}]({c["url"]})' if c.get('url') else c['source']
        lines.append(f'| `{c["id"]}` | {starter[c["id"]]["name"]} | {src} | {", ".join(c.get("authors", [])) or "-"} | {c["licence"]} |')
    with open(os.path.join(HERE, 'CREDITS.md'), 'w') as f:
        f.write('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
