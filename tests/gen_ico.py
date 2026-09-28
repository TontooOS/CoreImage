"""Generate committed ICO fixtures + Pillow RGBA dumps (oracle for ico_codec.rs).

16px files use packed AND rows (Pillow writer behavior); 32px files are
unambiguous (padded == packed stride). Run from this directory:
python3 gen_ico.py
"""
import os
from PIL import Image, ImageDraw

D = os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
os.makedirs(D, exist_ok=True)


def pattern(mode, w, h):
    img = Image.new(mode, (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            if w == 16:
                if mode == "1":
                    px[x, y] = (x + y) % 2
                elif mode == "P":
                    px[x, y] = (x * 5 + y * 3) % 8
                elif mode == "RGB":
                    px[x, y] = ((x * 40) % 256, (y * 60) % 256, ((x + y) * 30) % 256)
                elif mode == "RGBA":
                    px[x, y] = ((x * 40) % 256, (y * 60) % 256, ((x + y) * 30) % 256, 100 + ((x * 7 + y) % 156))
            else:
                if mode == "1":
                    px[x, y] = (x * 3 + y) % 2
                elif mode == "P":
                    px[x, y] = (x * 5 + y * 3) % 8
                elif mode == "RGB":
                    px[x, y] = ((x * 8) % 256, (y * 8) % 256, ((x + y) * 5) % 256)
                elif mode == "RGBA":
                    px[x, y] = ((x * 8) % 256, (y * 8) % 256, ((x + y) * 5) % 256, 60 + ((x * 3 + y * 2) % 196))
    if mode == "P":
        pal = []
        for i in range(8):
            pal += [(i * 30) % 256, (i * 20) % 256, (i * 40) % 256]
        pal += [0] * (256 * 3 - len(pal))
        img.putpalette(pal)
    return img


def dump(img, name, w, h):
    open(os.path.join(D, f"{name}.ico.{w}x{h}.rgba"), "wb").write(img.convert("RGBA").tobytes())


# 16px set (packed AND rows).
for mode, name in [("1", "ref_1bit"), ("P", "ref_8bit"), ("RGB", "ref_24bit"), ("RGBA", "ref_32bit")]:
    img = pattern(mode, 16, 16)
    img.save(os.path.join(D, name + ".ico"), format="ICO", bitmap_format="bmp", sizes=[(16, 16)])
    dump(img, name, 16, 16)
    print("saved", name)

multi = pattern("RGB", 32, 32)
multi.save(os.path.join(D, "ref_multi_bmp.ico"), format="ICO", bitmap_format="bmp", sizes=[(16, 16), (32, 32)])
dump(multi, "ref_multi_bmp", 32, 32)
print("saved ref_multi_bmp")

# 32px set (unambiguous AND stride) with punched transparency.
for mode, name in [("1", "ref32_1bit"), ("P", "ref32_8bit"), ("RGB", "ref32_24bit"), ("RGBA", "ref32_32bit")]:
    img = pattern(mode, 32, 32).convert("RGBA")
    px = img.load()
    for y in range(0, 32, 4):
        for x in range(0, 32, 4):
            px[x, y] = (px[x, y][0], px[x, y][1], px[x, y][2], 0)
    img.save(os.path.join(D, name + ".ico"), format="ICO", bitmap_format="bmp", sizes=[(32, 32)])
    dump(img, name, 32, 32)
    print("saved", name)

# 32px native depths (opaque).
for mode, name in [("1", "ref32n_1bit"), ("P", "ref32n_8bit"), ("RGB", "ref32n_24bit")]:
    img = pattern(mode, 32, 32)
    img.save(os.path.join(D, name + ".ico"), format="ICO", bitmap_format="bmp", sizes=[(32, 32)])
    dump(img, name, 32, 32)
    print("saved", name)
