# Composite

Layering, blend modes, CoreText watermarks, CoreIcon SF Symbols, masks and
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

### `overlay_sf_symbol`

```rust
pub fn overlay_sf_symbol(&self, symbol_name: &str, x: i64, y: i64, width: u32, height: u32, tint: Option<Rgba8>) -> Result<Self, ImageError>
```

Resolves the PNG with `coreicon::resolve_icon_path` (for example `"star.fill"`),
scales it to `width` x `height`, applies the optional RGBA tint multiply and
overlays it. Returns `Err` when the symbol file does not exist.

### `tint`

```rust
pub fn tint(&self, tint: Rgba8) -> Self
```

RGB multiply preserving alpha. Used by `overlay_sf_symbol`.

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
use coreimage::{Rgba8, TiImage};

let img = TiImage::load("wallpaper.png")?;
let out = img
  .overlay_sf_symbol("star.fill", 24, 24, 64, 64, Some(Rgba8::WHITE))?
  .watermark_text("TontooOS", 24, 104, 24.0, Rgba8::WHITE)?
  .rounded_corners(48);
```

## Cross References

- [Transform.md](Transform.md) – sizing layers before compositing
- [ColorAnalysis.md](ColorAnalysis.md) – dominant color for tint selection
- [Ffi.md](Ffi.md) – C-visible blur helper
