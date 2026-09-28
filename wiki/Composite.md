# Composite

Layering, blend modes, CoreText watermarks, tints, masks and
rounded corners.

## Functions

### `overlay`

```rust
pub fn overlay(&self, other: &TiImage, x: i64, y: i64) -> Self
```

Normal alpha compositing of `other` at (`x`, `y`). Off-canvas pixels clip.

### `blend`

```rust
pub fn blend(&self, other: &TiImage, mode: BlendMode, opacity: f32, x: i64, y: i64) -> Self
```

| Mode | Behavior |
|---|---|
| `Normal` | Alpha composite |
| `Multiply` | Darken blend |
| `Screen` | Lighten blend |
| `Overlay` | Contrast blend |
| `Add` | Linear dodge |

`opacity` clamps to `0.0..=1.0`.

### `watermark_text`

```rust
pub fn watermark_text(&self, text: &str, x: u32, y: u32, size: f32, color: Rgba8) -> Result<Self, ImageError>
```

SF Pro watermark. Font resolution follows CoreText (`coretext::FontRegistry`
plus `system_font_dirs`: `/usr/share/fonts/OTF`, `/usr/share/fonts/TTF`,
`~/.fonts`). Glyphs rasterize with `ab_glyph`. Returns `Err` when `text` is empty.

### `watermark_text_coretext`

```rust
pub fn watermark_text_coretext(&self, text: &str, family: &str, x: u32, y: u32, size: f32, color: Rgba8) -> Result<Self, ImageError>
```

Same as `watermark_text` with an explicit CoreText family. When no font file
resolves (e.g. CI without system fonts), a bitmap placeholder bar is painted so
callers still get a visible mark instead of an error.

### `tint`

```rust
pub fn tint(&self, tint: Rgba8) -> Self
```

RGB multiply preserving alpha. Recolors glyph-style artwork (black
symbols with alpha become the tint color); transparent pixels stay
transparent. CoreIcon builds SF Symbol overlays on top of this
primitive (`load` the symbol asset, `resize`/`fit` it, `tint` it,
`overlay` it).

### `alpha_mask`

```rust
pub fn alpha_mask(&self, mask: &TiImage) -> Result<Self, ImageError>
```

White keeps opaque. Returns `Err` when mask dimensions differ.

### `rounded_corners`

```rust
pub fn rounded_corners(&self, radius: u32) -> Self
```

Clears corner pixels outside the radius. `radius` clamps to half the short edge.

### `circular_mask`

```rust
pub fn circular_mask(&self) -> Self
```

Center-crops to a square, then applies a full circular alpha mask.

## Usage / Example

```rust
use coreimage::{ImageFormat, Rgba8, TiImage};

let img = TiImage::load("wallpaper.png")?;
let badge = TiImage::solid(64, 64, Rgba8::WHITE)?;
let out = img.overlay(&badge, 24, 24)
  .watermark_text("TontooOS", 24, 104, 24.0, Rgba8::WHITE)?
  .rounded_corners(48);
```

> **Note:** `overlay` takes `&TiImage`; stamping an SF Symbol asset
> works the same way: `TiImage::load` the symbol file resolved by
> CoreIcon, then `resize`/`fit`, `tint` and `overlay`. Symbol
> resolution lives in CoreIcon so CoreIcon can depend on CoreImage
> without a dependency cycle.

## Cross References

- [Transform.md](Transform.md) – sizing layers before compositing
- [ColorAnalysis.md](ColorAnalysis.md) – dominant color for tint selection
- [Ffi.md](Ffi.md) – C-visible blur helper
