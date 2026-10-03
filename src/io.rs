//! Loading, saving, byte buffers and metadata.
//!
//! Supported formats: PNG, JPEG, ICO, GIF, BMP, WebP and AVIF. All of them use
//! the pure-Rust codecs in `crate::codecs` (no third-party code). AVIF
//! decodes the ISOBMFF container and the AV1 bitstream headers natively; the
//! AV1 tile level entropy decoder is not part of this crate yet, so AVIF pixel
//! data currently returns a localized `Unsupported` error.

use std::io::Cursor;
use std::path::Path;

use image::ImageFormat as ImgFmt;

use crate::{ImageError, RgbaImage, TiImage, lang::tr};

/// File format selector with quality-aware saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
    WebP,
    Ico,
    Avif,
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
            "ico" => Some(Self::Ico),
            "avif" | "avis" => Some(Self::Avif),
            _ => None,
        }
    }

    /// Detect from magic bytes (PNG/JPEG/GIF/BMP/WebP RIFF/ICO/AVIF ftyp).
    pub fn from_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0u8, 0, 1, 0]) {
            return Some(Self::Ico);
        }
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
        } else if crate::codecs::avif::is_avif(bytes) {
            Some(Self::Avif)
        } else {
            None
        }
    }

    fn to_image_format(self) -> Option<ImgFmt> {
        match self {
            Self::Png => Some(ImgFmt::Png),
            Self::Jpeg => Some(ImgFmt::Jpeg),
            Self::Gif => Some(ImgFmt::Gif),
            Self::Bmp => Some(ImgFmt::Bmp),
            // WebP uses the native `crate::codecs::webp` codec, never the
            // `image` crate.
            Self::WebP => None,
            Self::Ico => Some(ImgFmt::Ico),
            // AVIF uses the native `crate::codecs::avif` codec.
            Self::Avif => None,
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

/// 3. Save a buffer to file with format + quality (quality 1-100 for JPEG).
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

/// Decode GIF bytes with the pure-Rust codec (first frame).
fn decode_gif(bytes: &[u8]) -> Result<TiImage, ImageError> {
    let d = crate::codecs::gif::decode(bytes).map_err(|e| match e {
        crate::codecs::gif::GifError::Unsupported(m) => ImageError::Unsupported(m),
        other => ImageError::Decode(other.to_string()),
    })?;
    let buf = crate::RgbaImage::from_raw(d.width, d.height, d.pixels)
        .ok_or_else(|| ImageError::Decode(tr("invalid_pixel_len")))?;
    let mut out = TiImage::from_rgba(buf);
    out.set_format(ImageFormat::Gif);
    Ok(out)
}

/// Decode BMP bytes with the pure-Rust codec.
fn decode_bmp(bytes: &[u8]) -> Result<TiImage, ImageError> {
    let d = crate::codecs::bmp::decode(bytes).map_err(|e| match e {
        crate::codecs::bmp::BmpError::Unsupported(m) => ImageError::Unsupported(m),
        other => ImageError::Decode(other.to_string()),
    })?;
    let buf = crate::RgbaImage::from_raw(d.width, d.height, d.pixels)
        .ok_or_else(|| ImageError::Decode(tr("invalid_pixel_len")))?;
    let mut out = TiImage::from_rgba(buf);
    out.set_format(ImageFormat::Bmp);
    Ok(out)
}

/// Decode ICO bytes with the pure-Rust codec (largest entry).
fn decode_ico(bytes: &[u8]) -> Result<TiImage, ImageError> {
    let d = crate::codecs::ico::decode(bytes).map_err(|e| match e {
        crate::codecs::ico::IcoError::Unsupported(m) => ImageError::Unsupported(m),
        other => ImageError::Decode(other.to_string()),
    })?;
    let img = d.largest().ok_or_else(|| ImageError::Decode(tr("empty_image")))?;
    let buf = crate::RgbaImage::from_raw(img.width, img.height, img.pixels.clone())
        .ok_or_else(|| ImageError::Decode(tr("invalid_pixel_len")))?;
    let mut out = TiImage::from_rgba(buf);
    out.set_format(ImageFormat::Ico);
    Ok(out)
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

/// Decode WebP bytes with the pure-Rust codec (lossy + lossless,
/// first frame wins for animation).
fn decode_webp(bytes: &[u8]) -> Result<TiImage, ImageError> {
    let d = crate::codecs::webp::decode(bytes).map_err(|e| match e {
        crate::codecs::webp::WebpError::Unsupported(m) => ImageError::Unsupported(m),
        other => ImageError::Decode(other.to_string()),
    })?;
    let buf = crate::RgbaImage::from_raw(d.width, d.height, d.pixels)
        .ok_or_else(|| ImageError::Decode(tr("invalid_pixel_len")))?;
    let mut img = TiImage::from_rgba(buf);
    img.set_format(ImageFormat::WebP);
    Ok(img)
}

/// Decode AVIF bytes with the native container and AV1 header parsers.
fn decode_avif(bytes: &[u8]) -> Result<TiImage, ImageError> {
    let d = crate::codecs::avif::decode(bytes).map_err(|e| match e {
        crate::codecs::avif::AvifError::Unsupported(m) => ImageError::Unsupported(m),
        other => ImageError::Decode(other.to_string()),
    })?;
    let buf = crate::RgbaImage::from_raw(d.width, d.height, d.pixels)
        .ok_or_else(|| ImageError::Decode(tr("invalid_pixel_len")))?;
    let mut img = TiImage::from_rgba(buf);
    img.set_format(ImageFormat::Avif);
    Ok(img)
}

/// 4. Decode from a byte buffer (format auto-detected).
/// PNG, JPEG, GIF, BMP, ICO, WebP and AVIF input use the pure-Rust codecs in
/// `crate::codecs`. Animated GIFs and WebP decode to their first frame.
pub fn from_bytes(bytes: &[u8]) -> Result<TiImage, ImageError> {
    if crate::codecs::png::is_png(bytes) {
        return decode_png(bytes);
    }
    if crate::codecs::jpeg::is_jpeg(bytes) {
        return decode_jpeg(bytes);
    }
    if crate::codecs::gif::is_gif(bytes) {
        return decode_gif(bytes);
    }
    if crate::codecs::bmp::is_bmp(bytes) {
        return decode_bmp(bytes);
    }
    if crate::codecs::ico::is_ico(bytes) {
        return decode_ico(bytes);
    }
    if crate::codecs::webp::is_webp(bytes) {
        return decode_webp(bytes);
    }
    if crate::codecs::avif::is_avif(bytes) {
        return decode_avif(bytes);
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
            ImgFmt::Ico => ImageFormat::Ico,
            _ => ImageFormat::Png,
        });
    }
    Ok(img)
}

/// 5. Decode from a byte buffer with explicit format.
/// `ImageFormat::Png`, `ImageFormat::Jpeg`, `ImageFormat::Gif`,
/// `ImageFormat::Bmp`, `ImageFormat::Ico`, `ImageFormat::WebP` and
/// `ImageFormat::Avif` use the pure-Rust codecs in `crate::codecs`.
pub fn from_bytes_with_format(bytes: &[u8], format: ImageFormat) -> Result<TiImage, ImageError> {
    if format == ImageFormat::Png {
        return decode_png(bytes);
    }
    if format == ImageFormat::Jpeg {
        return decode_jpeg(bytes);
    }
    if format == ImageFormat::Gif {
        return decode_gif(bytes);
    }
    if format == ImageFormat::Bmp {
        return decode_bmp(bytes);
    }
    if format == ImageFormat::Ico {
        return decode_ico(bytes);
    }
    if format == ImageFormat::WebP {
        return decode_webp(bytes);
    }
    if format == ImageFormat::Avif {
        return decode_avif(bytes);
    }
    let fmt = format
        .to_image_format()
        .ok_or_else(|| ImageError::Unsupported(tr("unknown_format")))?;
    let dynimg = image::load_from_memory_with_format(bytes, fmt)
        .map_err(|e| ImageError::Decode(e.to_string()))?;
    let mut img = TiImage::from_rgba(dynimg.to_rgba8());
    img.set_format(format);
    Ok(img)
}

/// 6. Encode a buffer into bytes (quality 1-100, used for JPEG;
/// PNG, GIF, BMP, ICO and WebP are lossless so quality is validated
/// but has no effect). All formats use the pure-Rust codecs in
/// `crate::codecs`. ICO stores the largest fitting entry
/// (max 256 px per side).
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
    if format == ImageFormat::Ico {
        if buf.width() > 256 || buf.height() > 256 {
            return Err(ImageError::InvalidInput(tr("invalid_size")));
        }
        return crate::codecs::ico::encode(buf.width(), buf.height(), buf.as_raw())
            .map_err(|e| ImageError::Encode(e.to_string()));
    }
    if format == ImageFormat::Gif {
        return crate::codecs::gif::encode(buf.width(), buf.height(), buf.as_raw())
            .map_err(|e| ImageError::Encode(e.to_string()));
    }
    if format == ImageFormat::Bmp {
        return crate::codecs::bmp::encode(buf.width(), buf.height(), buf.as_raw())
            .map_err(|e| ImageError::Encode(e.to_string()));
    }
    if format == ImageFormat::WebP {
        return crate::codecs::webp::encode(buf.width(), buf.height(), buf.as_raw(), quality)
            .map_err(|e| ImageError::Encode(e.to_string()));
    }
    if format == ImageFormat::Avif {
        return Err(ImageError::Unsupported(tr("avif_encode_unsupported")));
    }
    // Unreachable: every ImageFormat variant is routed above.
    Err(ImageError::Unsupported(tr("unknown_format")))
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

/// Human readable chroma layout for the metadata `color_type` field.
fn chroma_label(format: crate::codecs::av1::ChromaFormat) -> &'static str {
    use crate::codecs::av1::ChromaFormat;
    match format {
        ChromaFormat::Cs420 => "420",
        ChromaFormat::Cs422 => "422",
        ChromaFormat::Cs444 => "444",
        ChromaFormat::Monochrome => "400",
    }
}

/// 10. Read metadata of a byte buffer.
/// PNG, JPEG, ICO, BMP, WebP and AVIF input use the pure-Rust probes (no full
/// decode); EXIF is read best-effort from the container bytes.
pub fn metadata_from_bytes(bytes: &[u8]) -> Result<ImageMetadata, ImageError> {
    if crate::codecs::bmp::is_bmp(bytes) {
        let (w, h) = crate::codecs::bmp::dimensions(bytes)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        return Ok(ImageMetadata {
            width: w,
            height: h,
            format: Some(ImageFormat::Bmp),
            color_type: "RGBA8".to_string(),
            exif_orientation: None,
            exif_date_taken: None,
            exif_camera: None,
            exif_exposure: None,
        });
    }
    if crate::codecs::ico::is_ico(bytes) {
        let (w, h) = crate::codecs::ico::dimensions(bytes)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        return Ok(ImageMetadata {
            width: w,
            height: h,
            format: Some(ImageFormat::Ico),
            color_type: "RGBA8".to_string(),
            exif_orientation: None,
            exif_date_taken: None,
            exif_camera: None,
            exif_exposure: None,
        });
    }
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
    if crate::codecs::webp::is_webp(bytes) {
        let (w, h) = crate::codecs::webp::dimensions(bytes)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        return Ok(ImageMetadata {
            width: w,
            height: h,
            format: Some(ImageFormat::WebP),
            color_type: "RGBA8".to_string(),
            exif_orientation: None,
            exif_date_taken: None,
            exif_camera: None,
            exif_exposure: None,
        });
    }
    if crate::codecs::avif::is_avif(bytes) {
        let info = crate::codecs::avif::probe(bytes)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let depth = info.depths.first().copied().unwrap_or(8);
        return Ok(ImageMetadata {
            width: info.width,
            height: info.height,
            format: Some(ImageFormat::Avif),
            color_type: format!("AV1-{:02}{}", depth * 4, chroma_label(info.chroma_format)),
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
