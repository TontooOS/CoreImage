# Tontoo CoreImage

Image loading, transform, filter, composite and analysis for TontooOS.
Text watermarks resolve fonts through CoreText.

40+ functions: PNG (100 percent pure Rust), JPEG (baseline sequential,
pure Rust), GIF (single frame, pure Rust), BMP (incl. RLE, pure Rust)
and ICO (PNG + BMP entries, pure Rust) plus WebP load
and save with quality, byte buffers,
EXIF metadata, thumbnails, crop, fit modes, rotation, mirroring, brightness,
contrast, saturation, sharpen, white balance, grayscale, sepia, invert, blur,
filter chains, blend modes, watermarks, masks, rounded corners, colorspaces,
dominant color and histograms. More pure-Rust codecs land in `src/codecs/`.

## Made for TontooOS

Explore more at https://github.com/TontooOS/Libs


## Adding to Your Project

Add to your `Cargo.toml`:

```toml
[dependencies]
coreimage = { path = "/Library/System/coreimage" }
```

## License

TCL v26.1
