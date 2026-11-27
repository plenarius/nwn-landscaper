# sheet.py OUT SHOT V... : one view of each variant side by side (2 a row), the game's picture cropped to its 3D view, labelled.
import sys
from PIL import Image, ImageDraw, ImageFont
S = "/srv/burst-out/soft-paint"; out, shot, run = sys.argv[1], sys.argv[2], sys.argv[3]; vs = sys.argv[4:]
LBL = {"a": "A today", "b": "B bake only: soft blend", "c": "C bake only: crisp smooth edge", "d": "D bake only: broken edge",
       "e": "E weights from the brush: soft", "f": "F weights from the brush: crisp", "g": "G weights from the brush: broken",
       "h": "H bake only: soft blend, narrower"}
ims = []
for v in vs:
    p = None
    for r in run.split(","):
        try: p = Image.open(f"{S}/stills/{r}/{v}-{shot}.png").convert("RGB"); break
        except FileNotFoundError: pass
    im = p.crop((128, 0, 1072, 610)).resize((628, 406))
    d = ImageDraw.Draw(im); f = ImageFont.load_default(size=22)
    d.rectangle((0, 0, 628, 32), fill=(0, 0, 0)); d.text((8, 4), LBL.get(v, v), fill=(255, 255, 255), font=f)
    ims.append(im)
cols = 2; rows = (len(ims) + 1) // 2
M = Image.new("RGB", (cols * 628 + 6, rows * 406 + 6 * (rows - 1)), (255, 255, 255))
for k, im in enumerate(ims): M.paste(im, ((k % cols) * 634, (k // cols) * 412))
M.save(out); print(out, M.size)
