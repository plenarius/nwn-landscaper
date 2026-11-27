# mosaic.py X0 Y0 X1 Y1 OUT V... : the dumped baked textures of each variant over area metres (x0,y0)-(x1,y1), north up, stacked.
import sys
from PIL import Image, ImageDraw
S = "/srv/burst-out/soft-paint"
x0, y0, x1, y1 = map(float, sys.argv[1:5]); out = sys.argv[5]; vs = sys.argv[6:]
PX = 51.2  # px a metre
rows = []
for v in vs:
    W, H = int((x1 - x0) * PX), int((y1 - y0) * PX)
    im = Image.new("RGB", (W, H))
    for tx in range(int(x0 // 10), int((x1 - 1e-6) // 10) + 1):
        for ty in range(int(y0 // 10), int((y1 - 1e-6) // 10) + 1):
            t = Image.open(f"{S}/areas/sp{v}_nwn/sp{v}_{tx:02}{ty:02}t.png").convert("RGB")
            # tile's top-left in the mosaic (north up)
            im.paste(t, (int((tx * 10 - x0) * PX), int((y1 - (ty + 1) * 10) * PX)))
    ImageDraw.Draw(im).text((8, 8), v, fill=(255, 0, 0))
    rows.append(im)
M = Image.new("RGB", (rows[0].width, sum(r.height + 6 for r in rows)), (255, 255, 255))
y = 0
for r in rows: M.paste(r, (0, y)); y += r.height + 6
M.save(out); print(out, M.size)
