//! Loading, saving, byte buffers and metadata.
//!
//! Supported formats: PNG, JPEG, GIF, BMP, WebP.
//! PNG and JPEG use the pure-Rust codecs in `crate::codecs` (no
//! third-party code); GIF, BMP and WebP still decode/encode
//! through the `image` crate until their own codec modules land.
//! AVIF/HEIC input returns a localized `Unsupported` error.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageFormat as ImgFmt};

use crate::{ImageError, RgbaImage, TiImage, lang::tr};

/// File format selector with quality-aware saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
    WebP,
}

impl ImageFormat {
    /// Detect from a file extension. Returns `None` for unknown extensions.
    pub fn from_extension(path: &str) -> Option<Self> {
        let ext = Path::new(path).extension()?.to_str()?.to_lowercase();
        match ext.as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "bmp" => Some(Self::Bmp),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }

    /// Detect from magic bytes (PNG/JPEG/GIF/BMP/WebP RIFF).
    pub fn from_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 12 {
            // BMP header is only 2 bytes, still detectable on tiny buffers.
            if bytes.starts_with(b"BM") {
                return Some(Self::Bmp);
            }
            return None;
        }
        if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if bytes.starts_with(b"BM") {
            Some(Self::Bmp)
        } else if bytes.starts_with(b"RIFF") && bytes[8..12] == *b"WEBP" {
            Some(Self::WebP)
        } else {
            None
        }
    }

    fn to_image_format(self) -> ImgFmt {
        match self {
            Self::Png => ImgFmt::Png,
            Self::Jpeg => ImgFmt::Jpeg,
            Self::Gif => ImgFmt::Gif,
            Self::Bmp => ImgFmt::Bmp,
            Self::WebP => ImgFmt::WebP,
        }
    }
}

/// Photo / file metadata (resolution always present, EXIF best-effort).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub format: Option<ImageFormat>,
    pub color_type: String,
    pub exif_orientation: Option<u32>,
    pub exif_date_taken: Option<String>,
    pub exif_camera: Option<String>,
    pub exif_exposure: Option<String>,
}

fn check_quality(quality: u8) -> Result<(), ImageError> {
    if quality == 0 || quality > 100 {
        return Err(ImageError::InvalidInput(tr("invalid_quality")));
    }
    Ok(())
}

/// 1. Load an image file (format auto-detected).
pub fn load(path: &str) -> Result<TiImage, ImageError> {
    let bytes = std::fs::read(path).map_err(|e| ImageError::Io(e.to_string()))?;
    let mut img = from_bytes(&bytes)?;
    img.set_format(ImageFormat::from_extension(path).unwrap_or(ImageFormat::Png));
    Ok(img)
}

/// 2. Load an image file with an explicit format hint.
pub fn load_with_format(path: &str, format: ImageFormat) -> Result<TiImage, ImageError> {
    let bytes = std::fs::read(path).map_err(|e| ImageError::Io(e.to_string()))?;
    let mut img = from_bytes_with_format(&bytes, format)?;
    img.set_format(format);
    Ok(img)
}

/// 3. Save a buffer to file with format + quality (quality 1-100 for JPEG/WebP).
pub fn save(buf: &RgbaImage, path: &str, format: ImageFormat, quality: u8) -> Result<(), ImageError> {
    check_quality(quality)?;
    let bytes = to_bytes(buf, format, quality)?;
    std::fs::write(path, bytes).map_err(|e| ImageError::Io(e.to_string()))
}

/// Decode PNG bytes with the pure-Rust codec.
fn decode_png(bytes: &[u8]) -> Result<TiImage, ImageError> {
    let d = crate::codecs::png::decode(bytes).map_err(|e| match e {
        crate::codecs::png::PngError::Unsupported(m) => ImageError::Unsupported(m),
        other => ImageError::Decode(other.to_string()),
    })?;
    let buf = crate::RgbaImage::from_raw(d.width, d.height, d.pixels)
        .ok_or_else(|| ImageError::Decode(tr("invalid_pixel_len")))?;
    let mut img = TiImage::from_rgba(buf);
    img.set_format(ImageFormat::Png);
    Ok(img)
}

/// Decode JPEG bytes with the pure-Rust codec.
fn decode_jpeg(bytes: &[u8]) -> Result<TiImage, ImageError> {
    let d = crate::codecs::jpeg::decode(bytes).map_err(|e| match e {
        crate::codecs::jpeg::JpegError::Unsupported(m) => ImageError::Unsupported(m),
        other => ImageError::Decode(other.to_string()),
    })?;
    let buf = crate::RgbaImage::from_raw(d.width, d.height, d.pixels)
        .ok_or_else(|| ImageError::Decode(tr("invalid_pixel_len")))?;
    let mut img = TiImage::from_rgba(buf);
    img.set_format(ImageFormat::Jpeg);
    Ok(img)
}

/// 4. Decode from a byte buffer (format auto-detected).
/// PNG and JPEG input use the pure-Rust codecs in `crate::codecs`.
pub fn from_bytes(bytes: &[u8]) -> Result<TiImage, ImageError> {
    if crate::codecs::png::is_png(bytes) {
        return decode_png(bytes);
    }
    if crate::codecs::jpeg::is_jpeg(bytes) {
        return decode_jpeg(bytes);
    }
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImageError::Decode(e.to_string()))?;
    let fmt = reader.format();
    let dynimg = reader.decode().map_err(|e| {
        if e.to_string().contains("unsupported") || e.to_string().contains("APNG") {
            ImageError::Unsupported(e.to_string())
        } else {
            ImageError::Decode(e.to_string())
        }
    })?;
    let mut img = TiImage::from_rgba(dynimg.to_rgba8());
    if let Some(f) = fmt {
        img.set_format(match f {
            ImgFmt::Png => ImageFormat::Png,
            ImgFmt::Jpeg => ImageFormat::Jpeg,
            ImgFmt::Gif => ImageFormat::Gif,
            ImgFmt::Bmp => ImageFormat::Bmp,
            ImgFmt::WebP => ImageFormat::WebP,
            _ => ImageFormat::Png,
        });
    }
    Ok(img)
}

/// 5. Decode from a byte buffer with explicit format.
/// `ImageFormat::Png` and `ImageFormat::Jpeg` use the pure-Rust
/// codecs in `crate::codecs`.
pub fn from_bytes_with_format(bytes: &[u8], format: ImageFormat) -> Result<TiImage, ImageError> {
    if format == ImageFormat::Png {
        return decode_png(bytes);
    }
    if format == ImageFormat::Jpeg {
        return decode_jpeg(bytes);
    }
    let dynimg = image::load_from_memory_with_format(bytes, format.to_image_format())
        .map_err(|e| ImageError::Decode(e.to_string()))?;
    let mut img = TiImage::from_rgba(dynimg.to_rgba8());
    img.set_format(format);
    Ok(img)
}

/// 6. Encode a buffer into bytes (quality 1-100, used for JPEG/WebP;
/// PNG is lossless so quality is validated but has no effect).
/// PNG and JPEG output use the pure-Rust codecs in `crate::codecs`.
pub fn to_bytes(buf: &RgbaImage, format: ImageFormat, quality: u8) -> Result<Vec<u8>, ImageError> {
    check_quality(quality)?;
    if format == ImageFormat::Png {
        return crate::codecs::png::encode(buf.width(), buf.height(), buf.as_raw())
            .map_err(|e| ImageError::Encode(e.to_string()));
    }
    if format == ImageFormat::Jpeg {
        return crate::codecs::jpeg::encode(buf.width(), buf.height(), buf.as_raw(), quality)
            .map_err(|e| ImageError::Encode(e.to_string()));
    }
    let dynimg = DynamicImage::ImageRgba8(buf.clone());
    let mut out = Cursor::new(Vec::new());
    match format {
        ImageFormat::Png => unreachable!("handled above"),
        ImageFormat::Gif => {
            dynimg.write_to(&mut out, ImgFmt::Gif).map_err(|e| ImageError::Encode(e.to_string()))?;
        }
        ImageFormat::Bmp => {
            dynimg.write_to(&mut out, ImgFmt::Bmp).map_err(|e| ImageError::Encode(e.to_string()))?;
        }
        ImageFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgba8(buf.clone()).to_rgb8();
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
            #[allow(unused_imports)]
            use image::ImageEncoder as _;
            enc.encode_image(&rgb).map_err(|e| ImageError::Encode(e.to_string()))?;
        }
        ImageFormat::WebP => {
            // `image` WebP encoder is lossless; quality selects lossless vs lossy path
            // is not exposed here, so quality is validated but the default encoder is used.
            dynimg.write_to(&mut out, ImgFmt::WebP).map_err(|e| ImageError::Encode(e.to_string()))?;
        }
    }
    Ok(out.into_inner())
}

/// 7. Probe the format of a file without full decode (extension + magic).
pub fn probe_format(path: &str) -> Result<ImageFormat, ImageError> {
    if let Some(f) = ImageFormat::from_extension(path) {
        // Confirm magic when the file is readable; fall back to extension.
        if let Ok(bytes) = std::fs::read(path) {
            if let Some(m) = ImageFormat::from_magic(&bytes) {
                return Ok(m);
            }
        }
        return Ok(f);
    }
    let bytes = std::fs::read(path).map_err(|e| ImageError::Io(e.to_string()))?;
    ImageFormat::from_magic(&bytes).ok_or_else(|| ImageError::Unsupported(tr("unknown_format")))
}

/// 8. Probe the format of a byte buffer.
pub fn probe_format_from_bytes(bytes: &[u8]) -> Result<ImageFormat, ImageError> {
    ImageFormat::from_magic(bytes).ok_or_else(|| ImageError::Unsupported(tr("unknown_format")))
}

/// 9. Read metadata of a file (resolution + best-effort EXIF).
pub fn metadata(path: &str) -> Result<ImageMetadata, ImageError> {
    let bytes = std::fs::read(path).map_err(|e| ImageError::Io(e.to_string()))?;
    let mut meta = metadata_from_bytes(&bytes)?;
    if meta.format.is_none() {
        meta.format = ImageFormat::from_extension(path);
    }
    Ok(meta)
}

/// 10. Read metadata of a byte buffer.
/// PNG and JPEG input use the pure-Rust probes (no full decode);
/// EXIF is read best-effort from the container bytes.
pub fn metadata_from_bytes(bytes: &[u8]) -> Result<ImageMetadata, ImageError> {
    if crate::codecs::png::is_png(bytes) {
        let (w, h) = crate::codecs::png::dimensions(bytes)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        return Ok(ImageMetadata {
            width: w,
            height: h,
            format: Some(ImageFormat::Png),
            color_type: "RGBA8".to_string(),
            exif_orientation: None,
            exif_date_taken: None,
            exif_camera: None,
            exif_exposure: None,
        });
    }
    let (w, h, format) = if crate::codecs::jpeg::is_jpeg(bytes) {
        let (w, h) = crate::codecs::jpeg::dimensions(bytes)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        (w, h, Some(ImageFormat::Jpeg))
    } else {
        let reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let (w, h) = reader.into_dimensions().map_err(|e| ImageError::Decode(e.to_string()))?;
        (w, h, ImageFormat::from_magic(bytes))
    };
    let mut meta = ImageMetadata {
        width: w,
        height: h,
        format,
        color_type: "RGBA8".to_string(),
        exif_orientation: None,
        exif_date_taken: None,
        exif_camera: None,
        exif_exposure: None,
    };
    // Best-effort EXIF (JPEG/TIFF only); missing EXIF is not an error.
    if let Ok(exif) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) {
        meta.exif_orientation = exif
            .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .and_then(|f| f.value.get_uint(0));
        meta.exif_date_taken = exif
            .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
            .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))
            .map(|f| f.display_value().to_string());
        let make = exif
            .get_field(exif::Tag::Make, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string());
        let model = exif
            .get_field(exif::Tag::Model, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string());
        if make.is_some() || model.is_some() {
            meta.exif_camera = Some(format!(
                "{} {}",
                make.unwrap_or_default(),
                model.unwrap_or_default()
            ));
        }
        meta.exif_exposure = exif
            .get_field(exif::Tag::ExposureTime, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string());
    }
    Ok(meta)
}

/// 11. Fast thumbnail for huge wallpapers: reads dimensions first and
/// picks a cheap decode path (currently full decode + high-quality
/// downscale; streaming partial-JPEG decode is a planned optimization).
pub fn thumbnail_fast(path: &str, max_size: u32) -> Result<TiImage, ImageError> {
    if max_size == 0 {
        return Err(ImageError::InvalidInput(tr("invalid_size")));
    }
    let meta = metadata(path)?;
    let scale = (max_size as f32 / meta.width.max(meta.height) as f32).min(1.0);
    let img = load(path)?;
    if scale >= 1.0 {
        return Ok(img);
    }
    let (w, h) = (
        (meta.width as f32 * scale).round() as u32,
        (meta.height as f32 * scale).round() as u32,
    );
    Ok(img.resize(w.max(1), h.max(1), image::imageops::FilterType::Triangle))
}

impl TiImage {
    /// Convenience wrapper: see [`thumbnail_fast`].
    pub fn thumbnail_fast(path: &str, max_size: u32) -> Result<Self, ImageError> {
        thumbnail_fast(path, max_size)
    }
    /// Convenience wrapper: see [`metadata`].
    pub fn probe_metadata(path: &str) -> Result<ImageMetadata, ImageError> {
        metadata(path)
    }
    /// Convenience wrapper: see [`probe_format`].
    pub fn probe_format(path: &str) -> Result<ImageFormat, ImageError> {
        probe_format(path)
    }
}
