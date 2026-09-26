//! Brightness, contrast, color and blur filters plus combinable pipelines.

use crate::{ImageError, TiImage};

/// One step of a [`FilterChain`]. Mirrors Apple CIFilter chaining.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Filter {
    Brightness(i32),
    Contrast(f32),
    Saturate(f32),
    Sharpen { sigma: f32, threshold: i32 },
    WhiteBalance { temperature: f32 },
    Grayscale,
    Sepia,
    Invert,
    GaussianBlur(f32),
    HueRotate(i32),
}

impl Filter {
    fn apply_to(&self, img: &TiImage) -> Result<TiImage, ImageError> {
        Ok(match *self {
            Self::Brightness(v) => img.brightness(v),
            Self::Contrast(v) => img.contrast(v),
            Self::Saturate(v) => img.saturate(v),
            Self::Sharpen { sigma, threshold } => img.sharpen(sigma, threshold),
            Self::WhiteBalance { temperature } => img.white_balance(temperature),
            Self::Grayscale => img.grayscale(),
            Self::Sepia => img.sepia(),
            Self::Invert => img.invert(),
            Self::GaussianBlur(s) => img.gaussian_blur(s),
            Self::HueRotate(d) => img.hue_rotate(d),
        })
    }
}

/// Ordered, combinable filter pipeline.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FilterChain {
    steps: Vec<Filter>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self::default()
    }
    /// 31. Push one filter onto the chain (builder style).
    pub fn push(mut self, filter: Filter) -> Self {
        self.steps.push(filter);
        self
    }
    /// 32. Apply the whole chain in order.
    pub fn apply(&self, img: &TiImage) -> Result<TiImage, ImageError> {
        let mut out = img.clone();
        for step in &self.steps {
            out = step.apply_to(&out)?;
        }
        Ok(out)
    }
    pub fn len(&self) -> usize {
        self.steps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl TiImage {
    /// 22. Brightness shift (-255..=255, additive per channel).
    pub fn brightness(&self, value: i32) -> Self {
        let v = value.clamp(-255, 255);
        let mut buf = self.as_rgba().clone();
        for px in buf.pixels_mut() {
            for c in px.0.iter_mut().take(3) {
                *c = (*c as i32 + v).clamp(0, 255) as u8;
            }
        }
        Self::from_rgba(buf)
    }

    /// 23. Contrast factor (1.0 = unchanged, 0.0 = flat gray).
    pub fn contrast(&self, factor: f32) -> Self {
        let f = factor.max(0.0);
        let mut buf = self.as_rgba().clone();
        for px in buf.pixels_mut() {
            for c in px.0.iter_mut().take(3) {
                let v = (*c as f32 - 128.0) * f + 128.0;
                *c = v.round().clamp(0.0, 255.0) as u8;
            }
        }
        Self::from_rgba(buf)
    }

    /// 24. Saturation factor (1.0 = unchanged, 0.0 = grayscale).
    pub fn saturate(&self, factor: f32) -> Self {
        let f = factor.max(0.0);
        let mut buf = self.as_rgba().clone();
        for px in buf.pixels_mut() {
            let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            px[0] = (luma + (r - luma) * f).round().clamp(0.0, 255.0) as u8;
            px[1] = (luma + (g - luma) * f).round().clamp(0.0, 255.0) as u8;
            px[2] = (luma + (b - luma) * f).round().clamp(0.0, 255.0) as u8;
        }
        Self::from_rgba(buf)
    }

    /// 25. Sharpen via unsharp mask (`sigma` > 0, `threshold` 0..=255).
    pub fn sharpen(&self, sigma: f32, threshold: i32) -> Self {
        let sigma = sigma.clamp(0.3, 10.0);
        let threshold = threshold.clamp(0, 255) as f32;
        let blurred = image::imageops::blur(self.as_rgba(), sigma);
        let mut buf = self.as_rgba().clone();
        for ((x, y, dst), blr) in buf.enumerate_pixels_mut().zip(blurred.pixels()) {
            let _ = (x, y);
            for c in 0..3 {
                let diff = dst[c] as f32 - blr[c] as f32;
                if diff.abs() >= threshold {
                    dst[c] = (dst[c] as f32 + diff).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Self::from_rgba(buf)
    }

    /// 26. White balance / color temperature (-100 cold .. +100 warm).
    pub fn white_balance(&self, temperature: f32) -> Self {
        let t = (temperature.clamp(-100.0, 100.0) / 100.0) * 30.0;
        let mut buf = self.as_rgba().clone();
        for px in buf.pixels_mut() {
            px[0] = (px[0] as f32 + t).round().clamp(0.0, 255.0) as u8;
            px[2] = (px[2] as f32 - t).round().clamp(0.0, 255.0) as u8;
        }
        Self::from_rgba(buf)
    }

    /// 27. Grayscale conversion (luminosity method).
    pub fn grayscale(&self) -> Self {
        let gray = image::DynamicImage::ImageRgba8(self.as_rgba().clone()).to_luma8();
        Self::from_rgba(image::DynamicImage::ImageLuma8(gray).to_rgba8())
    }

    /// 28. Sepia tone.
    pub fn sepia(&self) -> Self {
        let mut buf = self.as_rgba().clone();
        for px in buf.pixels_mut() {
            let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
            px[0] = (0.393 * r + 0.769 * g + 0.189 * b).round().clamp(0.0, 255.0) as u8;
            px[1] = (0.349 * r + 0.686 * g + 0.168 * b).round().clamp(0.0, 255.0) as u8;
            px[2] = (0.272 * r + 0.534 * g + 0.131 * b).round().clamp(0.0, 255.0) as u8;
        }
        Self::from_rgba(buf)
    }

    /// 29. Invert colors (alpha preserved).
    pub fn invert(&self) -> Self {
        let mut buf = self.as_rgba().clone();
        image::imageops::invert(&mut buf);
        // `invert` flips alpha too; restore original alpha.
        for (dst, src) in buf.pixels_mut().zip(self.as_rgba().pixels()) {
            dst[3] = src[3];
        }
        Self::from_rgba(buf)
    }

    /// 30. Gaussian blur (`sigma` > 0, e.g. for dock backgrounds).
    pub fn gaussian_blur(&self, sigma: f32) -> Self {
        let sigma = sigma.clamp(0.1, 50.0);
        Self::from_rgba(image::imageops::blur(self.as_rgba(), sigma))
    }

    /// Hue rotation in degrees (-180..=180).
    pub fn hue_rotate(&self, degrees: i32) -> Self {
        Self::from_rgba(image::imageops::huerotate(
            self.as_rgba(),
            degrees.clamp(-180, 180),
        ))
    }
}
