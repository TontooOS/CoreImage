# LoadingSaving

File loading/saving, byte buffers and metadata. Formats are PNG, JPEG, GIF, BMP and WebP.
AVIF/HEIC input returns an `Unsupported` error when the decoder cannot handle the magic.

## Formats

| Variant | Extension | Backend | Notes |
|---|---|---|---|
| `ImageFormat::Png` | `png` | Pure Rust (`codecs::png`) | Lossless, 100 percent spec decode, no third-party code |
| `ImageFormat::Jpeg` | `jpg`, `jpeg` | Pure Rust (`codecs::jpeg`) | Baseline sequential, `quality` 1-100, no third-party code |
| `ImageFormat::Ico` | `ico` | Pure Rust (`codecs::ico`) | Largest entry wins, max 256 px per side |
| `ImageFormat::Gif` | `gif` | Pure Rust (`codecs::gif`) | Single frame (first wins), lossless, no third-party code |
| `ImageFormat::Bmp` | `bmp` | Pure Rust (`codecs::bmp`) | Uncompressed, RLE, 16-bit, no third-party code |
| `ImageFormat::WebP` | `webp` | `image` crate | Default encoder path, own codec planned |

See [Codecs.md](Codecs.md) for the pure-Rust PNG codec scope and for adding new codecs.

## Functions

### `TiImage::load`

```rust
pub fn load(path: &str) -> Result<Self, ImageError>
```

Loads a file with format auto-detection. Returns `Err` when the file is missing
or undecodable.

### `TiImage::load_with_format`

```rust
pub fn load_with_format(path: &str, format: ImageFormat) -> Result<Self, ImageError>
```

Loads a file with an explicit format hint.

### `TiImage::save`

```rust
pub fn save(&self, path: &str, format: ImageFormat, quality: u8) -> Result<(), ImageError>
```

Saves with format and quality. Returns `Err` when `quality` is outside 1-100.

### `TiImage::from_bytes`

```rust
pub fn from_bytes(bytes: &[u8]) -> Result<Self, ImageError>
```

Decodes network/download or clipboard buffers. Used for wallpaper downloads.

### `TiImage::from_bytes_with_format`

```rust
pub fn from_bytes_with_format(bytes: &[u8], format: ImageFormat) -> Result<Self, ImageError>
```

Decodes buffers with an explicit format.

### `TiImage::to_bytes`

```rust
pub fn to_bytes(&self, format: ImageFormat, quality: u8) -> Result<Vec<u8>, ImageError>
```

Encodes into bytes for upload, clipboard or thumbnails.

### `TiImage::probe_format`

```rust
pub fn probe_format(path: &str) -> Result<ImageFormat, ImageError>
```

Probes extension plus magic bytes without a full decode.

### `probe_format_from_bytes`

```rust
pub fn probe_format_from_bytes(bytes: &[u8]) -> Result<ImageFormat, ImageError>
```

Returns `Err` with `unknown_format` when magic is unrecognized.

### `TiImage::probe_metadata`

```rust
pub fn probe_metadata(path: &str) -> Result<ImageMetadata, ImageError>
```

Resolution is always present. EXIF fields (`exif_orientation`, `exif_date_taken`,
`exif_camera`, `exif_exposure`) are best-effort and `None` when absent.

### `thumbnail_fast`

```rust
pub fn thumbnail_fast(path: &str, max_size: u32) -> Result<Self, ImageError>
```

Reads dimensions first, then downscales with a triangle filter. Full streaming
partial-JPEG decode is a planned optimization for 4K/6K wallpapers.

## Usage / Example

```rust
use coreimage::{ImageFormat, TiImage};

let img = TiImage::load("photo.jpg")?;
let meta = TiImage::probe_metadata("photo.jpg")?;
println!("{}x{} taken {:?}", meta.width, meta.height, meta.exif_date_taken);
let thumb = TiImage::thumbnail_fast("photo.jpg", 512)?;
thumb.save("thumb.jpg", ImageFormat::Jpeg, 82)?;
```

## Cross References

- [Transform.md](Transform.md) – thumbnail, crop and fit on loaded images
- [ColorAnalysis.md](ColorAnalysis.md) – resolution and EXIF consumers
