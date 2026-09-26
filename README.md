# Tontoo CoreImage

Image loading, transform, filter, composite and analysis for TontooOS.
SF Symbols resolve through CoreIcon, text watermarks resolve fonts through CoreText.

40+ functions: PNG/JPEG/GIF/BMP/WebP load and save with quality, byte buffers,
EXIF metadata, thumbnails, crop, fit modes, rotation, mirroring, brightness,
contrast, saturation, sharpen, white balance, grayscale, sepia, invert, blur,
filter chains, blend modes, watermarks, masks, rounded corners, colorspaces,
dominant color and histograms.

## Made for TontooOS

Explore more at https://github.com/TontooOS/Libs

Full docs: [wiki/MAIN.md](wiki/MAIN.md)

## Adding to Your Project

Add to your `Cargo.toml`:

```toml
[dependencies]
coreimage = { path = "/Library/System/coreimage" }
```

Or through the system SDK (once registered):

```toml
[dependencies]
sdk = { path = "/Library/System/sdk", features = ["CoreImage"] }
```

```rust
use coreimage::{ImageFormat, TiImage};

let img = TiImage::load("wallpaper.png")?;
let thumb = img.thumbnail(512)?;
thumb.save("thumb.png", ImageFormat::Png, 100)?;
```

## License

TCL v26.1
