# Codecs

Pure-Rust image codecs in `src/codecs/`. Each file extension maps to
one codec module. PNG and JPEG are fully implemented with no
third-party dependencies; GIF, WebP and BMP still route through the
`image` crate until their own modules land (`gif.rs`, `webp.rs`,
`bmp.rs`).

## PNG (`codecs::png`)

100 percent pure Rust: chunk I/O, CRC32, Adler32, inflate/deflate
(stored, fixed and dynamic Huffman blocks) and Adam7 are all
implemented in `src/codecs/png.rs` without third-party code.

### Decode scope

| Capability | Status |
|---|---|
| Color types 0, 2, 3, 4, 6 | Supported |
| Bit depths 1, 2, 4, 8, 16 | Supported, 16-bit scales to 8-bit with rounding |
| Filters None, Sub, Up, Average, Paeth | Supported |
| Adam7 interlacing | Supported |
| PLTE and tRNS transparency | Supported |
| Multi-IDAT streams | Supported, concatenated per spec |
| CRC verification | Strict, mismatch returns `Err` |
| Unknown ancillary chunks | Skipped (`iCCP`, `tEXt`, `sRGB`, ...) |
| Unknown critical chunks | `Unsupported` error |
| Color management (iCCP, sRGB, gAMA) | Ignored in v1, no color conversion applied |

### Encode scope

| Capability | Status |
|---|---|
| Output | Color type 6 (RGBA8), bit depth 8, no interlace |
| Filtering | Per-row adaptive pick of filters 0-4 |
| Compression | LZ77 with hash chains plus dynamic Huffman blocks, stored fallback |
| Provenance | `tEXt` Software chunk identifying the CoreImage codec |

### Functions

```rust
pub fn is_png(bytes: &[u8]) -> bool
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), PngError>
pub fn decode(bytes: &[u8]) -> Result<DecodedPng, PngError>
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, PngError>
```

`decode` always returns RGBA8 pixels in `DecodedPng { width, height, pixels }`.
Returns `Err` when the signature, CRC, dimensions, color combo, filter
bytes, zlib stream or trailing data are invalid.

### `PngError`

```rust
pub enum PngError {
  Decode(String),
  Encode(String),
  Unsupported(String),
}
```

`crate::io` maps `Unsupported` to `ImageError::Unsupported` and the rest
to `ImageError::Decode` / `ImageError::Encode`.

## JPEG (`codecs::jpeg`)

Sequential baseline JPEG (SOF0/SOF1, 8-bit Huffman), pure Rust.

### Decode scope

| Capability | Status |
|---|---|
| Grayscale and YCbCr, sampling factors 1-4 | Supported |
| Standard and custom quantization/Huffman tables | Supported |
| Restart markers (DRI/RSTn) | Supported |
| Multi-scan sequential images | Supported |
| APPn/COM segments | Skipped (EXIF APP1 is read by `crate::io` via `kamadak-exif`) |
| Progressive (SOF2), arithmetic (DAC), 12-bit, CMYK | `Unsupported` with a clear message |
| Fast dimension probe | Works for any SOFn, including progressive |

### Encode scope

| Capability | Status |
|---|---|
| Output | Sequential baseline, JFIF APP0 |
| Quality 1-100 | libjpeg-compatible table scaling |
| Sampling | 4:2:0 default, 4:4:4 via `encode_with_sampling`, gray via `encode_grayscale` |
| Huffman | Standard Annex K tables |

### Functions

```rust
pub fn is_jpeg(bytes: &[u8]) -> bool
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), JpegError>
pub fn decode(bytes: &[u8]) -> Result<DecodedJpeg, JpegError>
pub fn encode(width: u32, height: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, JpegError>
pub fn encode_with_sampling(width: u32, height: u32, rgba: &[u8], quality: u8, sampling: JpegSampling) -> Result<Vec<u8>, JpegError>
pub fn encode_grayscale(width: u32, height: u32, gray: &[u8], quality: u8) -> Result<Vec<u8>, JpegError>
```

`decode` always returns RGBA8 pixels (alpha 255, JPEG has none).
Cross-checked against the `image` crate and Pillow fixtures in
`tests/data/` (grayscale max diff 1, 4:4:4 max diff 2; subsampled
maxima sit on sharp chroma edges by filter design).

## Usage / Example

```rust
use coreimage::codecs::png;

let file = std::fs::read("icon.png")?;
let img = png::decode(&file)?;
assert_eq!((img.width, img.height), png::dimensions(&file)?);
let again = png::encode(img.width, img.height, &img.pixels)?;
```

```rust
use coreimage::codecs::jpeg;

let photo = std::fs::read("photo.jpg")?;
let img = jpeg::decode(&photo)?;
let small = jpeg::encode(800, 600, &img.pixels, 82)?;
```

## Adding a codec

1. Create `src/codecs/<format>.rs` with `is_<format>`, `dimensions`,
   `decode` and `encode` mirroring [`png`](https://github.com/TontooOS/CoreImage).
2. Register `pub mod <format>;` in `src/codecs/mod.rs`.
3. Route the format in `src/io.rs` (`from_bytes`, `from_bytes_with_format`,
   `to_bytes`, `metadata_from_bytes`).
4. Add `tests/<format>_codec.rs` with roundtrips, cross-checks against the
   `image` crate and hand-built vectors.
5. Document the module on this page and link it in `MAIN.md`.

## Cross References

- [LoadingSaving.md](LoadingSaving.md) – routing of formats in `io`
- [Composite.md](Composite.md) – CoreIcon PNGs decode through this codec
