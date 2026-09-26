# ColorAnalysis

Colorspace conversion and image analysis: dominant color, average color,
histograms, luminance and palettes.

## Functions

### `convert_colorspace`

```rust
pub fn convert_colorspace(&self, space: ColorSpace) -> Self
```

| Variant | Behavior |
|---|---|
| `ColorSpace::Srgb` | Identity |
| `ColorSpace::DisplayP3` | Wide-gamut approximation (gamma lift + saturation) |
| `ColorSpace::Grayscale` | Luma conversion |

### `dominant_color`

```rust
pub fn dominant_color(&self) -> Result<Rgba8, ImageError>
```

4-bit quantization plus frequency vote. Skips near-transparent pixels.
Returns `Err` when the image is empty or fully transparent. Used for
wallpaper light/dark split previews.

### `average_color`

```rust
pub fn average_color(&self) -> Result<Rgba8, ImageError>
```

Alpha-weighted mean. Returns `Err` on empty or fully transparent images.

### `histogram`

```rust
pub fn histogram(&self) -> Histogram
```

Per-channel 256-bin counts in `Histogram { r, g, b }`. Built for photo apps.

### `luminance`

```rust
pub fn luminance(&self) -> f32
```

Mean Rec. 709 luminance in `0.0..=1.0`.

### `is_dark` / `is_light`

```rust
pub fn is_dark(&self) -> bool
pub fn is_light(&self) -> bool
```

`is_dark` is `luminance() < 0.4`; `is_light` is `luminance() >= 0.6`.

### `palette`

```rust
pub fn palette(&self, k: usize) -> Vec<Rgba8>
```

Top-`k` quantized colors sorted by frequency. `k` clamps to at least 1.

## Usage / Example

```rust
use coreimage::TiImage;

let img = TiImage::load("wallpaper.png")?;
let dom = img.dominant_color()?;
let theme = if img.is_dark() { "dark" } else { "light" };
let bins = img.histogram();
println!("{theme} {:?} red[255]={}", dom, bins.r[255]);
```

## Cross References

- [Composite.md](Composite.md) – tint overlays from dominant colors
- [Filters.md](Filters.md) – grade before analysis
