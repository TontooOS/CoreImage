# Transform

Scaling, thumbnails, cropping, aspect-fit modes, rotation and mirroring.

## Functions

### `resize`

```rust
pub fn resize(&self, width: u32, height: u32, filter: FilterType) -> Self
```

Exact resize. Pass `FilterType::Lanczos3` for quality or `Triangle` for speed.

### `thumbnail`

```rust
pub fn thumbnail(&self, max_size: u32) -> Result<Self, ImageError>
```

Downscales so the longer edge equals `max_size`. Returns `Err` when `max_size` is 0.
Images smaller than `max_size` are cloned unchanged.

### `crop`

```rust
pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Result<Self, ImageError>
```

Returns `Err` with `invalid_crop` when the rectangle leaves the image.

### `crop_aspect`

```rust
pub fn crop_aspect(&self, aspect_w: u32, aspect_h: u32) -> Result<Self, ImageError>
```

Center-crops to a ratio such as `16, 9`.

### `fit`

```rust
pub fn fit(&self, width: u32, height: u32, mode: FitMode) -> Result<Self, ImageError>
```

| Mode | Behavior |
|---|---|
| `FitMode::Fill` | Cover then center-crop to exact size |
| `FitMode::Fit` | Contain with transparent letterbox padding |
| `FitMode::Stretch` | Exact size, aspect ignored |

### `rotate_90`

```rust
pub fn rotate_90(&self, step: Rotate90) -> Self
```

Lossless 90-degree steps (`Deg90`, `Deg180`, `Deg270`).

### `rotate_free`

```rust
pub fn rotate_free(&self, angle_degrees: f32, background: Rgba8) -> Self
```

Bilinear free rotation. The canvas expands and empty areas use `background`.

### `flip_horizontal` / `flip_vertical`

```rust
pub fn flip_horizontal(&self) -> Self
pub fn flip_vertical(&self) -> Self
```

Mirror without re-encoding loss beyond the pixel copy.

### `perspective_skew`

```rust
pub fn perspective_skew(&self, skew_x: f32) -> Self
```

Preview of the v2 quad warp: a mild horizontal skew in `-1.0..=1.0`. Full
4-point perspective correction is tracked for v2.

## Usage / Example

```rust
use coreimage::{FitMode, Rotate90, TiImage};

let img = TiImage::load("wallpaper.png")?;
let cover = img.fit(1920, 1080, FitMode::Fill)?;
let upright = cover.rotate_90(Rotate90::Deg90)?;
let detail = upright.crop_aspect(1, 1)?;
```

## Cross References

- [LoadingSaving.md](LoadingSaving.md) – fast thumbnails for large wallpapers
- [Composite.md](Composite.md) – rounded corners after transforms
