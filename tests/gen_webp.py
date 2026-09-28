"""Generate committed WebP fixtures for CoreImage codec tests (PIL as reference encoder)."""
import os
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
os.makedirs(OUT, exist_ok=True)

# Photo-like RGB pattern (gradients + shapes + noise-ish detail).
def photo(w, h):
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            r = (x * 4 + y * 2) % 256
            g = (y * 5 + x) % 256
            b = ((x * x + y * y) // 7) % 256
            px[x, y] = (r, g, b)
    d = ImageDraw.Draw(img)
    d.ellipse([w // 4, h // 4, 3 * w // 4, 3 * h // 4], fill=(200, 40, 90))
    d.rectangle([w // 8, h // 8, w // 3, h // 2], fill=(30, 220, 160))
    return img

def photo_rgba(w, h):
    img = photo(w, h).convert("RGBA")
    px = img.load()
    for y in range(h):
        for x in range(w):
            r, g, b, _ = px[x, y]
            px[x, y] = (r, g, b, (x * 7 + y * 3) % 256)
    return img

def save(img, name, **kw):
    p = os.path.join(OUT, name)
    img.save(p, format="WEBP", **kw)
    print(name, os.path.getsize(p), "bytes")

def save_ref(img, name):
    p = os.path.join(OUT, name)
    img.save(p, format="PNG")
    print(name, os.path.getsize(p), "bytes")

img = photo(64, 48)
save(img, "webp_lossy_q80.webp", quality=80, method=4)
save_ref(img, "webp_lossy_q80.png")

rgba = photo_rgba(64, 48)
save(rgba, "webp_lossy_alpha_q80.webp", quality=80, method=4)
save_ref(rgba, "webp_lossy_alpha_q80.png")

save(rgba, "webp_lossless_rgba.webp", lossless=True, method=4)
save_ref(rgba, "webp_lossless_rgba.png")

odd = photo(37, 23)
save(odd, "webp_odd_37x23_lossless.webp", lossless=True, method=4)
save_ref(odd, "webp_odd_37x23_lossless.png")

red = Image.new("RGB", (16, 16), (255, 0, 0))
blue = Image.new("RGB", (16, 16), (0, 0, 255))
red.save(os.path.join(OUT, "webp_animated.webp"), format="WEBP",
         save_all=True, append_images=[blue], duration=100, loop=0)
print("webp_animated.webp", os.path.getsize(os.path.join(OUT, "webp_animated.webp")), "bytes")
save_ref(red, "webp_animated_first.png")
print("done")
