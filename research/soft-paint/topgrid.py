# topgrid.py X0 Y0 X1 Y1 SCALE OUT V... : the dumped bakes over area metres (x0,y0)-(x1,y1), north up, a labelled grid 2 a row.
import sys
from PIL import Image, ImageDraw, ImageFont
S = "/srv/burst-out/soft-paint"
x0, y0, x1, y1, sc = map(float, sys.argv[1:6]); out = sys.argv[6]; vs = sys.argv[7:]
LBL = {"a": "A today", "b": "B bake only: soft blend", "c": "C bake only: crisp", "d": "D bake only: broken",
       "e": "E brush weights: soft", "f": "F brush weights: crisp", "g": "G brush weights: broken", "h": "H bake only: narrow blend"}
PX = 51.2
tiles = []
for v in vs:
    W, H = int((x1 - x0) * PX), int((y1 - y0) * PX)
    im = Image.new("RGB", (W, H))
    for tx in range(int(x0 // 10), int((x1 - 1e-6) // 10) + 1):
        for ty in range(int(y0 // 10), int((y1 - 1e-6) // 10) + 1):
            t = Image.open(f"{S}/areas/sp{v}_nwn/sp{v}_{tx:02}{ty:02}t.png").convert("RGB")
            im.paste(t, (int((tx * 10 - x0) * PX), int((y1 - (ty + 1) * 10) * PX)))
    im = im.resize((int(W * sc), int(H * sc)), Image.LANCZOS)
    d = ImageDraw.Draw(im); f = ImageFont.load_default(size=20)
    d.rectangle((0, 0, im.width, 28), fill=(0, 0, 0)); d.text((6, 3), LBL.get(v, v), fill=(255, 255, 255), font=f)
    tiles.append(im)
w, h = tiles[0].size; cols = 2 if len(tiles) > 1 else 1; rows = (len(tiles) + cols - 1) // cols
M = Image.new("RGB", (cols * w + 6 * (cols - 1), rows * h + 6 * (rows - 1)), (255, 255, 255))
for k, im in enumerate(tiles): M.paste(im, ((k % cols) * (w + 6), (k // cols) * (h + 6)))
M.save(out, optimize=True); print(out, M.size)
