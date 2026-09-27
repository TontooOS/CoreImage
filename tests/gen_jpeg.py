"""Generate committed JPEG fixtures for CoreImage codec tests (PIL as reference encoder)."""
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

def save(img, name, **kw):
    p = os.path.join(OUT, name)
    img.save(p, format="JPEG", **kw)
    print(name, os.path.getsize(p), "bytes")

img = photo(64, 48)
ex = Image.Exif()
ex[271] = "Tontoo"          # Make
ex[272] = "TestCam 1"       # Model
ex[274] = 6                 # Orientation
ex[306] = "2026:09:27 12:00:00"  # DateTime
ex[33434] = (1, 60)         # ExposureTime
save(img, "baseline_420_q75_exif.jpg", quality=75, subsampling=2, exif=ex)
save(img, "baseline_422_q80.jpg", quality=80, subsampling=1)
save(img, "baseline_444_q90.jpg", quality=90, subsampling=0)
save(img.convert("L"), "gray_q75.jpg", quality=75)
save(photo(37, 23), "odd_37x23_q75.jpg", quality=75, subsampling=2)
save(img, "progressive_q75.jpg", quality=75, progressive=True)
save(img.convert("CMYK"), "cmyk_q75.jpg", quality=75)
print("done")
