//! CoreImage for TontooOS: load, transform, filter, composite and analyze images.
//!
//! Backends:
//! - Raster ops on the `image` crate (PNG, JPEG, GIF, WebP, BMP).
//! - SF Symbols via `coreicon` (see [`composite::overlay_sf_symbol`]).
//! - Text rendering via `coretext` font resolution + `ab_glyph`
//!   rasterization with the SF Pro system font
//!   (see [`composite::watermark_text`] and [`composite::watermark_text_coretext`]).
//! - EXIF metadata via `kamadak-exif`.
//!
//! All public strings and error messages resolve through `lang/en_us.json`
//! and `lang/de_de.json` (see [`lang`]).

/// Pure-Rust image codecs (PNG built-in, more formats to come).
pub mod codecs;
pub mod color;
pub mod composite;
pub mod ffi;
pub mod filter;
pub mod io;
pub mod lang;
pub mod transform;

pub use color::{ColorSpace, Histogram};
pub use composite::BlendMode;
pub use filter::{Filter, FilterChain};
pub use io::{ImageFormat, ImageMetadata};
pub use lang::{LangCode, tr};
pub use transform::{FitMode, Rotate90};

use image::{ImageBuffer, Rgba};

/// RGBA8 pixel buffer used across the whole library.
pub type RgbaImage = ImageBuffer<Rgba<u8>, Vec<u8>>;

/// Solid RGBA color (0-255 per channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
    pub const WHITE: Self = Self::new(255, 255, 255, 255);
    pub const BLACK: Self = Self::new(0, 0, 0, 255);
    pub const TRANSPARENT: Self = Self::new(0, 0, 0, 0);
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        match hex.len() {
            6 => Some(Self::new(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
                255,
            )),
            8 => Some(Self::new(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
                u8::from_str_radix(&hex[6..8], 16).ok()?,
            )),
            _ => None,
        }
    }
    pub fn into_image_pixel(self) -> Rgba<u8> {
        Rgba([self.r, self.g, self.b, self.a])
    }
    pub fn from_image_pixel(px: Rgba<u8>) -> Self {
        Self::new(px[0], px[1], px[2], px[3])
    }
}

/// Errors returned by every CoreImage function.
#[derive(Debug)]
pub enum ImageError {
    Io(String),
    Decode(String),
    Encode(String),
    InvalidInput(String),
    Unsupported(String),
    Font(String),
    Icon(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(m) => write!(f, "{}: {}", tr("err_io"), m),
            Self::Decode(m) => write!(f, "{}: {}", tr("err_decode"), m),
            Self::Encode(m) => write!(f, "{}: {}", tr("err_encode"), m),
            Self::InvalidInput(m) => write!(f, "{}: {}", tr("err_invalid"), m),
            Self::Unsupported(m) => write!(f, "{}: {}", tr("err_unsupported"), m),
            Self::Font(m) => write!(f, "{}: {}", tr("err_font"), m),
            Self::Icon(m) => write!(f, "{}: {}", tr("err_icon"), m),
        }
    }
}

impl std::error::Error for ImageError {}

/// Main image handle: owns an RGBA8 buffer plus its source format.
#[derive(Debug, Clone)]
pub struct TiImage {
    buf: RgbaImage,
    format: Option<ImageFormat>,
}

impl TiImage {
    /// Wrap an existing RGBA buffer.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        if pixels.len() != (width as usize) * (height as usize) * 4 {
            return Err(ImageError::InvalidInput(tr("invalid_pixel_len")));
        }
        let buf = RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| ImageError::InvalidInput(tr("invalid_pixel_len")))?;
        Ok(Self { buf, format: None })
    }

    /// Solid-color image.
    pub fn solid(width: u32, height: u32, color: Rgba8) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::InvalidInput(tr("invalid_size")));
        }
        Ok(Self {
            buf: RgbaImage::from_pixel(width, height, color.into_image_pixel()),
            format: None,
        })
    }

    /// Load from file (format auto-detected).
    pub fn load(path: &str) -> Result<Self, ImageError> {
        io::load(path)
    }

    /// Load from file with an explicit format hint.
    pub fn load_with_format(path: &str, format: ImageFormat) -> Result<Self, ImageError> {
        io::load_with_format(path, format)
    }

    /// Decode from a memory buffer (format auto-detected).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ImageError> {
        io::from_bytes(bytes)
    }

    /// Decode from a memory buffer with explicit format.
    pub fn from_bytes_with_format(bytes: &[u8], format: ImageFormat) -> Result<Self, ImageError> {
        io::from_bytes_with_format(bytes, format)
    }

    /// Save to file with format + quality (quality 1-100, used for JPEG/WebP).
    pub fn save(&self, path: &str, format: ImageFormat, quality: u8) -> Result<(), ImageError> {
        io::save(&self.buf, path, format, quality)
    }

    /// Encode into a memory buffer.
    pub fn to_bytes(&self, format: ImageFormat, quality: u8) -> Result<Vec<u8>, ImageError> {
        io::to_bytes(&self.buf, format, quality)
    }

    pub fn width(&self) -> u32 {
        self.buf.width()
    }
    pub fn height(&self) -> u32 {
        self.buf.height()
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.buf.width(), self.buf.height())
    }
    pub fn source_format(&self) -> Option<ImageFormat> {
        self.format
    }
    pub fn as_rgba(&self) -> &RgbaImage {
        &self.buf
    }
    pub fn into_rgba(self) -> RgbaImage {
        self.buf
    }
    pub fn from_rgba(buf: RgbaImage) -> Self {
        Self { buf, format: None }
    }
    pub(crate) fn set_format(&mut self, format: ImageFormat) {
        self.format = Some(format);
    }
}

/// Library version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
