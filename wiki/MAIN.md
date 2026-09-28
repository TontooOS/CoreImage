# CoreImage – Wiki

CoreImage is the image loading, transform, filter, composite and analysis library for TontooOS.
SF Symbols resolve through CoreIcon and text watermarks resolve fonts through CoreText.

- Repository: https://github.com/TontooOS/CoreImage
- License: TCL v26.1
- Version: 26.1.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| LoadingSaving | [LoadingSaving.md](LoadingSaving.md) | File load/save, byte buffers, metadata |
| Codecs | [Codecs.md](Codecs.md) | Pure-Rust codecs, PNG 100 percent without third-party code |
| Transform | [Transform.md](Transform.md) | Resize, thumbnails, crop, fit, rotation |
| Filters | [Filters.md](Filters.md) | Adjustments and combinable filter chains |
| Composite | [Composite.md](Composite.md) | Layering, CoreIcon symbols, CoreText text, masks |
| ColorAnalysis | [ColorAnalysis.md](ColorAnalysis.md) | Colorspaces, dominant color, histogram |
| Ffi | [Ffi.md](Ffi.md) | C ABI in `Headers/coreimage.h` |

## Quick Start

```rust
use coreimage::{ImageFormat, TiImage};

let img = TiImage::load("/usr/share/wallpapers/tontoo.png")?;
let thumb = img.thumbnail(512)?;
let warm = thumb.white_balance(12.0).gaussian_blur(0.8);
warm.save("/tmp/thumb.png", ImageFormat::Png, 100)?;
```

```c
#include "coreimage.h"

uint32_t w = 0, h = 0;
coreimage_dimensions("/usr/share/wallpapers/tontoo.png", &w, &h);
```

See [LoadingSaving.md](LoadingSaving.md) for details.

## Changelog

- 2026-09-28: Pure-Rust GIF codec in `src/codecs/` (single frame, LZW, median-cut); WebP/BMP still via `image` crate.
- 2026-09-28: Pure-Rust ICO codec in `src/codecs/` (PNG + BMP entries, AND masks).
- 2026-09-28: Pure-Rust JPEG codec in `src/codecs/` (baseline sequential decode/encode).
- 2026-09-27: Pure-Rust PNG codec in `src/codecs/` (100 percent spec decode, own encoder).
- 2026-09-26: Initial CoreImage wiki and 40+ function API.
