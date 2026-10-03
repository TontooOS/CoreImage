# Tontoo CoreImage

Image loading, transform, filter, composite and analysis for TontooOS.
Text watermarks resolve fonts through CoreText.

40+ functions: PNG (100 percent pure Rust), JPEG (baseline sequential,
pure Rust), GIF (single frame, pure Rust), BMP (incl. RLE, pure Rust),
ICO (PNG + BMP entries, pure Rust) and WebP (VP8 lossy plus VP8L
lossless, pure Rust) with quality, byte buffers,
EXIF metadata, thumbnails, crop, fit modes, rotation, mirroring, brightness,
contrast, saturation, sharpen, white balance, grayscale, sepia, invert, blur,
filter chains, blend modes, watermarks, masks, rounded corners, colorspaces,
dominant color and histograms. All file codecs live in `src/codecs/`. AVIF is parsed
natively as well: ISOBMFF container plus AV1 sequence and frame headers, so probe,
metadata and dimensions work without the `image` crate; pixel reconstruction of AV1
tiles is not implemented yet and reports `Unsupported`.

## Made for TontooOS

Explore more at https://github.com/TontooOS/Libs


## Adding to Your Project

Add to your `Cargo.toml`:

```toml
[dependencies]
coreimage = { path = "/Library/System/coreimage" }
```

## License

TCL v27.0
