# Filters

Brightness, contrast, color and blur adjustments plus combinable pipelines
in the style of Apple CIFilter chains.

## Functions

### `brightness`

```rust
pub fn brightness(&self, value: i32) -> Self
```

Additive shift clamped to `-255..=255` per RGB channel. Alpha is preserved.

### `contrast`

```rust
pub fn contrast(&self, factor: f32) -> Self
```

`1.0` is unchanged, `0.0` is flat gray. Negative factors clamp to `0.0`.

### `saturate`

```rust
pub fn saturate(&self, factor: f32) -> Self
```

`1.0` is unchanged, `0.0` is grayscale. Used for vivid wallpaper grades.

### `sharpen`

```rust
pub fn sharpen(&self, sigma: f32, threshold: i32) -> Self
```

Unsharp mask. `sigma` clamps to `0.3..=10.0`, `threshold` to `0..=255`.

### `white_balance`

```rust
pub fn white_balance(&self, temperature: f32) -> Self
```

`-100` is cold, `+100` is warm. Shifts red/blue channels in opposite directions.

### `grayscale`

```rust
pub fn grayscale(&self) -> Self
```

Luminosity conversion. Alpha is preserved.

### `sepia`

```rust
pub fn sepia(&self) -> Self
```

Classic sepia tone matrix.

### `invert`

```rust
pub fn invert(&self) -> Self
```

Inverts RGB and restores the original alpha channel.

### `gaussian_blur`

```rust
pub fn gaussian_blur(&self, sigma: f32) -> Self
```

`sigma` clamps to `0.1..=50.0`. Used for dock backgrounds and UI blur effects.

### `hue_rotate`

```rust
pub fn hue_rotate(&self, degrees: i32) -> Self
```

Hue rotation clamped to `-180..=180`.

### `FilterChain`

```rust
pub fn push(self, filter: Filter) -> Self
pub fn apply(&self, img: &TiImage) -> Result<TiImage, ImageError>
```

Ordered pipeline. `apply` runs every step in order and never fails on current
filter variants (returns `Result` for forward-compatible fallible filters).

## Usage / Example

```rust
use coreimage::filter::{Filter, FilterChain};
use coreimage::TiImage;

let img = TiImage::load("photo.jpg")?;
let chain = FilterChain::new()
  .push(Filter::WhiteBalance { temperature: 10.0 })
  .push(Filter::Contrast(1.08))
  .push(Filter::GaussianBlur(0.6));
let out = chain.apply(&img)?;
```

## Cross References

- [Composite.md](Composite.md) – blur-then-overlay dock backgrounds
- [ColorAnalysis.md](ColorAnalysis.md) – analysis after grading
