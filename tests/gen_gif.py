"""Generate committed GIF fixtures with Pillow (oracle for gif_codec.rs).

GIF is lossless: our decode must match the `image` crate bit for bit.
Run from this directory: python3 gen_gif.py
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
    img.save(p, format="GIF", **kw)
    print(name, os.path.getsize(p), "bytes")


rgb = photo(64, 48)
save(rgb.convert("P", palette=Image.ADAPTIVE, colors=256), "gif_8bit.gif")
save(rgb.convert("P", palette=Image.ADAPTIVE, colors=16), "gif_4bit.gif")
save(rgb.convert("P", palette=Image.ADAPTIVE, colors=2), "gif_2bit.gif")
save(photo(64, 48).convert("L"), "gif_gray.gif")
save(photo(37, 23).convert("P", palette=Image.ADAPTIVE, colors=64), "gif_odd_37x23.gif")
save(rgb.convert("P", palette=Image.ADAPTIVE, colors=128), "gif_interlaced.gif", interlace=True)

# Transparency: circle pixels forced to palette index 255 (transparent).
pal = photo(48, 32).convert("P", palette=Image.ADAPTIVE, colors=255)
px = pal.load()
for y in range(32):
    for x in range(48):
        if (x - 24) ** 2 + (y - 16) ** 2 < 100:
            px[x, y] = 255
save(pal, "gif_transparent.gif", transparency=255)

# Animation: 3 solid frames (red/green/blue); decoders read the first.
frames = []
for c in ((200, 10, 10), (10, 200, 10), (10, 10, 200)):
    f = Image.new("P", (24, 18), 0)
    f.putpalette([c[0], c[1], c[2]] + [0] * 765)
    frames.append(f)
frames[0].save(
    os.path.join(D, "gif_animated.gif"),
    format="GIF", save_all=True, append_images=frames[1:],
    duration=100, loop=0,
)
print("gif_animated.gif", os.path.getsize(os.path.join(D, "gif_animated.gif")), "bytes")
