#!/usr/bin/env python3
"""Generate the AVIF fixtures in tests/data/.

The fixtures are encoded with libavif's `avifenc` (aom encoder); the reference
PNG files next to them are decoded by libavif through Pillow, which is what the
decoder is compared against once pixel reconstruction lands.

Usage:
    python3 tests/gen_avif.py [output-dir]

Requires `avifenc` from libavif 1.x and Pillow with AVIF support.
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, 'data')


def sources(work):
    """Write the PNG sources the fixtures are encoded from."""
    from PIL import Image

    def base(w=96, h=64):
        im = Image.new('RGB', (w, h))
        px = im.load()
        for y in range(h):
            for x in range(w):
                px[x, y] = ((x * 255) // (w - 1),
                            (y * 255) // (h - 1),
                            ((x ^ y) * 255) // (w + h - 2))
        return im

    base().save(os.path.join(work, 'base.png'))
    # Odd sizes exercise row and column remainder handling.
    base(37, 23).save(os.path.join(work, 'odd.png'))
    # Flat field: only DC_PRED blocks, the simplest intra case.
    Image.new('RGB', (64, 48), (32, 96, 200)).save(os.path.join(work, 'flat.png'))
    # Hard edges and corners for CDEF and deblocking.
    edge = Image.new('RGB', (96, 64), (250, 250, 250))
    px = edge.load()
    for y in range(64):
        for x in range(96):
            if (x // 8 + y // 8) % 2 == 0:
                px[x, y] = (12, 12, 240)
            elif x > 40 and y > 20:
                px[x, y] = (240, 12, 12)
    edge.save(os.path.join(work, 'edge.png'))
    # Flat alpha ramp for the alpha auxiliary item.
    alpha = Image.new('RGBA', (64, 48))
    px = alpha.load()
    for y in range(48):
        for x in range(64):
            v = 255 if x < 16 else (0 if x > 48 else (x - 16) * 255 // 32)
            px[x, y] = (200, 40, 40, v)
    alpha.save(os.path.join(work, 'alpha.png'))


# (name, avifenc arguments) in the order the fixtures are generated.
ENCODINGS = [
    ('avif_420_q60', ['-s', '6', '-q', '60', '--yuv', '420', 'base.png']),
    ('avif_444_q70', ['-s', '6', '-q', '70', '--yuv', '444', 'base.png']),
    ('avif_422_q70', ['-s', '6', '-q', '70', '--yuv', '422', 'base.png']),
    ('avif_mono_q70', ['-s', '6', '-q', '70', '--yuv', '400', 'flat.png']),
    ('avif_lossless', ['-s', '4', '-q', '100', '--lossless', 'base.png']),
    ('avif_10bit_420', ['-s', '6', '-q', '60', '--yuv', '420', '--depth', '10', 'base.png']),
    ('avif_tiles', ['-s', '6', '-q', '60', '--yuv', '420',
                    '--tilerowslog2', '1', '--tilecolslog2', '1', 'base.png']),
    ('avif_odd_37x23', ['-s', '6', '-q', '70', '--yuv', '444', 'odd.png']),
    ('avif_screenc', ['-s', '10', '-q', '60', '--yuv', '420', 'edge.png']),
    ('avif_alpha', ['-s', '6', '-q', '70', '--yuv', '444', 'alpha.png']),
    ('avif_edge_q80', ['-s', '6', '-q', '80', '--yuv', '420', 'edge.png']),
]


def main():
    from PIL import Image

    os.makedirs(OUT, exist_ok=True)
    work = os.path.join(OUT, '_work')
    os.makedirs(work, exist_ok=True)
    sources(work)
    for name, args in ENCODINGS:
        target = os.path.join(OUT, name + '.avif')
        subprocess.run(['avifenc'] + args + [target], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        im = Image.open(target)
        im.load()
        if im.mode == 'L':
            im = im.convert('RGB')
        im.save(os.path.join(OUT, name + '.png'))
        print('%-18s %s %s' % (name, im.mode, im.size))
    for leftover in os.listdir(work):
        os.remove(os.path.join(work, leftover))
    os.rmdir(work)


if __name__ == '__main__':
    main()