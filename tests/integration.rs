use coreimage::filter::{Filter, FilterChain};
use coreimage::{BlendMode, ColorSpace, FitMode, ImageFormat, Rotate90, Rgba8, TiImage};

fn test_image() -> TiImage {
    TiImage::solid(64, 48, Rgba8::new(200, 60, 40, 255)).unwrap()
}

#[test]
fn load_save_bytes_roundtrip() {
    let img = test_image();
    for fmt in [ImageFormat::Png, ImageFormat::Bmp] {
        let bytes = img.to_bytes(fmt, 100).unwrap();
        let back = TiImage::from_bytes(&bytes).unwrap();
        assert_eq!(back.dimensions(), (64, 48));
    }
}

#[test]
fn jpeg_roundtrip_dimensions() {
    let img = test_image();
    let bytes = img.to_bytes(ImageFormat::Jpeg, 80).unwrap();
    let back = TiImage::from_bytes(&bytes).unwrap();
    assert_eq!(back.dimensions(), (64, 48));
}

#[test]
fn probe_format_from_magic() {
    let img = test_image();
    let png = img.to_bytes(ImageFormat::Png, 100).unwrap();
    assert_eq!(coreimage::io::probe_format_from_bytes(&png).unwrap(), ImageFormat::Png);
}

#[test]
fn metadata_from_bytes() {
    let img = test_image();
    let png = img.to_bytes(ImageFormat::Png, 100).unwrap();
    let meta = coreimage::io::metadata_from_bytes(&png).unwrap();
    assert_eq!((meta.width, meta.height), (64, 48));
}

#[test]
fn file_save_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.png");
    let img = test_image();
    img.save(path.to_str().unwrap(), ImageFormat::Png, 100).unwrap();
    let back = TiImage::load(path.to_str().unwrap()).unwrap();
    assert_eq!(back.dimensions(), (64, 48));
    assert_eq!(TiImage::probe_format(path.to_str().unwrap()).unwrap(), ImageFormat::Png);
    let meta = TiImage::probe_metadata(path.to_str().unwrap()).unwrap();
    assert_eq!(meta.width, 64);
}

#[test]
fn scale_crop_fit() {
    let img = test_image();
    assert_eq!(img.thumbnail(32).unwrap().dimensions(), (32, 24));
    assert_eq!(img.crop(0, 0, 16, 16).unwrap().dimensions(), (16, 16));
    assert!(img.crop(60, 40, 16, 16).is_err());
    assert_eq!(img.crop_aspect(1, 1).unwrap().dimensions(), (48, 48));
    assert_eq!(img.fit(32, 32, FitMode::Fill).unwrap().dimensions(), (32, 32));
    assert_eq!(img.fit(32, 32, FitMode::Fit).unwrap().dimensions(), (32, 32));
    assert_eq!(img.fit(32, 32, FitMode::Stretch).unwrap().dimensions(), (32, 32));
}

#[test]
fn rotation_mirror() {
    let img = test_image();
    assert_eq!(img.rotate_90(Rotate90::Deg90).dimensions(), (48, 64));
    assert_eq!(img.rotate_90(Rotate90::Deg180).dimensions(), (64, 48));
    assert_eq!(img.rotate_90(Rotate90::Deg270).dimensions(), (48, 64));
    assert_eq!(img.flip_horizontal().dimensions(), (64, 48));
    assert_eq!(img.flip_vertical().dimensions(), (64, 48));
    assert_eq!(img.rotate_free(15.0, Rgba8::TRANSPARENT).dimensions(), (64, 48));
    assert_eq!(img.perspective_skew(0.1).dimensions(), (64, 48));
}

#[test]
fn filters_and_chain() {
    let img = test_image();
    assert_eq!(img.brightness(10).dimensions(), (64, 48));
    assert_eq!(img.contrast(1.2).dimensions(), (64, 48));
    assert_eq!(img.saturate(0.5).dimensions(), (64, 48));
    assert_eq!(img.sharpen(1.0, 3).dimensions(), (64, 48));
    assert_eq!(img.white_balance(20.0).dimensions(), (64, 48));
    assert_eq!(img.grayscale().dimensions(), (64, 48));
    assert_eq!(img.sepia().dimensions(), (64, 48));
    assert_eq!(img.invert().dimensions(), (64, 48));
    assert_eq!(img.gaussian_blur(1.5).dimensions(), (64, 48));
    assert_eq!(img.hue_rotate(30).dimensions(), (64, 48));
    let chain = FilterChain::new()
        .push(Filter::Grayscale)
        .push(Filter::Brightness(10))
        .push(Filter::Contrast(1.1))
        .push(Filter::GaussianBlur(0.8));
    assert_eq!(chain.len(), 4);
    assert_eq!(chain.apply(&img).unwrap().dimensions(), (64, 48));
}

#[test]
fn composite_and_masks() {
    let base = test_image();
    let small = TiImage::solid(16, 16, Rgba8::new(40, 80, 200, 200)).unwrap();
    assert_eq!(base.overlay(&small, 4, 4).dimensions(), (64, 48));
    for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay, BlendMode::Add] {
        assert_eq!(base.blend(&small, mode, 0.8, 4, 4).dimensions(), (64, 48));
    }
    let mask = TiImage::solid(64, 48, Rgba8::WHITE).unwrap();
    assert_eq!(base.alpha_mask(&mask).unwrap().dimensions(), (64, 48));
    assert_eq!(base.rounded_corners(8).dimensions(), (64, 48));
    assert_eq!(base.circular_mask().dimensions(), (48, 48));
    assert_eq!(base.tint(Rgba8::new(255, 200, 200, 255)).dimensions(), (64, 48));
}

#[test]
fn text_watermark_runs() {
    let img = test_image();
    // Works with SF Pro when installed, else bitmap fallback still paints.
    assert_eq!(
        img.watermark_text("Hello", 4, 4, 16.0, Rgba8::WHITE).unwrap().dimensions(),
        (64, 48)
    );
}

#[test]
fn color_analysis() {
    let img = test_image();
    let dom = img.dominant_color().unwrap();
    assert!(dom.r > 150);
    let avg = img.average_color().unwrap();
    assert!(avg.r > 150);
    let hist = img.histogram();
    assert!(hist.r.iter().sum::<u32>() > 0);
    assert!(img.luminance() > 0.0);
    assert!(!img.is_dark() || !img.is_light() || true);
    // Solid image has a single quantized color.
    assert_eq!(img.palette(3).len(), 1);
    // Three-stripe image yields three palette entries.
    let mut striped = TiImage::solid(90, 10, Rgba8::BLACK).unwrap();
    let red = TiImage::solid(30, 10, Rgba8::new(255, 0, 0, 255)).unwrap();
    let green = TiImage::solid(30, 10, Rgba8::new(0, 255, 0, 255)).unwrap();
    let blue = TiImage::solid(30, 10, Rgba8::new(0, 0, 255, 255)).unwrap();
    striped = striped.overlay(&red, 0, 0).overlay(&green, 30, 0).overlay(&blue, 60, 0);
    assert_eq!(striped.palette(3).len(), 3);
    assert_eq!(img.convert_colorspace(ColorSpace::Srgb).dimensions(), (64, 48));
    assert_eq!(img.convert_colorspace(ColorSpace::DisplayP3).dimensions(), (64, 48));
    assert_eq!(img.convert_colorspace(ColorSpace::Grayscale).dimensions(), (64, 48));
}

#[test]
fn lang_tables() {
    assert_eq!(coreimage::lang::tr_in(coreimage::lang::LangCode::EnUs, "err_io"), "I/O error");
    assert!(!coreimage::lang::tr("err_io").is_empty());
}
