# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
# measure.py: per showcase area, the hak's baked tile pictures and what normal maps would add under each option. (throwaway)
import json, os, re, glob, sys
M = "/srv/burst-out/normal-maps/measure"
# Bytes of one BioWare DDS with its mips: DXT1 0.5 B/px, DXT5 1 B/px.
def dds(size, bpp):
    s, n = size, 0
    while True:
        n += max(4, s) ** 2 * bpp
        if s == 1: break
        s //= 2
    return int(n) + 20
SMALL = {"gravelPebbles", "floorCobble", "rock"}
WIDE = SMALL | {"gravelGrey", "gravelMuddy", "floorFlag", "floorStone", "floorSlabs", "floorBrick", "earthCracked", "mudCracked",
                "rockCliff", "swampRock", "canyonRock", "winterRock", "rockGrey", "rockRed", "rockPale", "rockMossy", "rockCave",
                "rockSlate", "rockBrown", "rockFrosted", "wallFieldstone", "wallMossy", "wallField", "wallWeathered"}
rows = []
for a in ["lscanyon", "lsdescent", "lsdunes", "lspass", "lsvale"]:
    d = f"{M}/x-{a}-hak"
    if not os.path.isdir(d):
        os.makedirs(d); os.system(f"cd {d} && nwn_erf -x -f {M}/h-{a}/hak/{a}.hak >/dev/null 2>&1")
    j = json.load(open(f"{M}/{a}.json"))
    pal, w, h = j["palette"], j["width"], j["height"]
    mat = open(f"{M}/{a}.materials", "rb").read()
    hak = os.path.getsize(f"{M}/h-{a}/hak/{a}.hak")
    baked = sorted(re.match(rf"{a}_(\d\d)(\d\d)t\.dds", os.path.basename(f)).groups() for f in glob.glob(f"{d}/{a}_*t.dds"))
    pic = sum(os.path.getsize(f) for f in glob.glob(f"{d}/{a}_*t.dds"))
    one = os.path.getsize(glob.glob(f"{d}/{a}_*t.dds")[0]) if baked else 0
    def has(tx, ty, want):
        for jy in range(ty * 16, ty * 16 + 17):
            for ix in range(tx * 16, tx * 16 + 17):
                if 0 <= ix < w and 0 <= jy < h and pal[mat[jy * w + ix]] in want: return True
        return False
    # (Tile names are column then row, two digits each: the engine's tile_name.)
    small = sum(has(int(x), int(y), SMALL) for x, y in baked)
    wide = sum(has(int(x), int(y), WIDE) for x, y in baked)
    used = sorted({pal[m] for m in set(mat)})
    rows.append(dict(area=a, tiles=j["tiles"] * j["tilesY"], baked=len(baked), baked_pic=one, hak=hak, pictures=pic,
                     small=small, wide=wide, palette=used, relief_small=sorted(set(used) & SMALL), relief_wide=sorted(set(used) & WIDE)))
opts = {"512 DXT5": dds(512, 1), "512 DXT1": dds(512, 0.5), "256 DXT5": dds(256, 1), "256 DXT1": dds(256, 0.5)}
print("bytes per normal picture:", opts)
for r in rows:
    print(f"{r['area']}: {r['tiles']} tiles, {r['baked']} baked ({r['baked_pic']} B each), hak {r['hak']/1e6:.1f} MB, baked pictures {r['pictures']/1e6:.1f} MB")
    print(f"   baked tiles with pebbles/cobbles/rock {r['small']}, with any stony texture {r['wide']}; palette {r['palette']}")
    for k, v in opts.items():
        print(f"   per baked tile {k}: all {r['baked']*v/1e6:.1f} MB, stony {r['wide']*v/1e6:.1f} MB, three {r['small']*v/1e6:.1f} MB")
json.dump(dict(rows=rows, opts=opts), open(f"{M}/measure.json", "w"), indent=1)
