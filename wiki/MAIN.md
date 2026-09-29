# CoreImage – Wiki

CoreImage is the image loading, transform, filter, composite and analysis library for TontooOS.
Text watermarks resolve fonts through CoreText. SF Symbol overlays are composed by CoreIcon
on top of this library (`load`, `resize`/`fit`, `tint`, `overlay`), so CoreIcon can
depend on CoreImage without a dependency cycle.

- Repository: https://github.com/TontooOS/CoreImage
- License: TCL v27.0
- Version: 27.0.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| LoadingSaving | [LoadingSaving.md](LoadingSaving.md) | File load/save, byte buffers, metadata |
| Codecs | [Codecs.md](Codecs.md) | Pure-Rust codecs, PNG 100 percent without third-party code |
| Transform | [Transform.md](Transform.md) | Resize, thumbnails, crop, fit, rotation |
| Filters | [Filters.md](Filters.md) | Adjustments and combinable filter chains |
| Composite | [Composite.md](Composite.md) | Layering, tints, CoreText text, masks |
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

- 2026-09-28: Removed the CoreIcon dependency and `TiImage::overlay_sf_symbol`; symbol
  overlays now compose in CoreIcon from `load` + `resize`/`fit` + `tint` + `overlay`,
  so CoreIcon can depend on CoreImage without a cycle. Added `Rgba` and `FilterType`
  re-exports for buffer-level users. See [Composite.md](Composite.md).
- 2026-09-28: Pure-Rust BMP codec in `src/codecs/` (DIB reuse, RLE4/8, 16-bit); only WebP still via `image` crate.
- 2026-09-28: Pure-Rust GIF codec in `src/codecs/` (single frame, LZW, median-cut).
- 2026-09-28: Pure-Rust ICO codec in `src/codecs/` (PNG + BMP entries, AND masks).
- 2026-09-28: Pure-Rust JPEG codec in `src/codecs/` (baseline sequential decode/encode).
- 2026-09-27: Pure-Rust PNG codec in `src/codecs/` (100 percent spec decode, own encoder).
- 2026-09-26: Initial CoreImage wiki and 40+ function API.
