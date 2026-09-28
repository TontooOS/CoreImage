"""Generate committed BMP fixtures with Pillow (oracle for bmp_codec.rs).

BMP is lossless: our decode must match the `image` crate bit for bit.
Run from this directory: python3 gen_bmp.py
"""
import os
from PIL import Image, ImageDraw

D = os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
os.makedirs(D, exist_ok=True)


def photo(w, h):
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = ((x * 4 + y * 2) % 256, (y * 5 + x) % 256, ((x * x + y * y) // 7) % 256)
    d = ImageDraw.Draw(img)
    d.ellipse([w // 4, h // 4, 3 * w // 4, 3 * h // 4], fill=(200, 40, 90))
    d.rectangle([w // 8, h // 8, w // 3, h // 2], fill=(30, 220, 160))
    return img


def save(img, name, **kw):
    p = os.path.join(D, name)
    img.save(p, format="BMP", **kw)
    print(name, os.path.getsize(p), "bytes")


save(photo(64, 48), "bmp_24bit.bmp")
save(photo(37, 23), "bmp_odd_37x23.bmp")

rgba = photo(48, 32).convert("RGBA")
px = rgba.load()
for y in range(0, 32, 3):
    for x in range(0, 48, 3):
        px[x, y] = (px[x, y][0], px[x, y][1], px[x, y][2], 80)
save(rgba, "bmp_32bit.bmp")

save(photo(64, 48).convert("P", palette=Image.ADAPTIVE, colors=16), "bmp_8bit.bmp")
save(photo(64, 48).convert("1"), "bmp_1bit.bmp")
