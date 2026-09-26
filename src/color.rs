//! Colorspace conversion and image analysis
//! (dominant color, histogram, palette).

use std::collections::HashMap;

use crate::{ImageError, Rgba8, TiImage, lang::tr};

/// Target colorspace for [`TiImage::convert_colorspace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    DisplayP3,
    Grayscale,
}

/// 256-bin per-channel histogram (each vec always holds 256 bins).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
}

impl TiImage {
    /// 41. Convert colorspace (sRGB identity, Display P3 wide-gamut
    /// boost approximation, Grayscale luma).
    pub fn convert_colorspace(&self, space: ColorSpace) -> Self {
        match space {
            ColorSpace::Srgb => self.clone(),
            ColorSpace::Grayscale => self.grayscale(),
            ColorSpace::DisplayP3 => {
                // Approximation: mild saturation + gamma lift to use
                // the wider P3 primaries on capable displays.
                let mut buf = self.as_rgba().clone();
                for px in buf.pixels_mut() {
                    for c in px.0.iter_mut().take(3) {
                        let v = (*c as f32 / 255.0).powf(1.0 / 1.1);
                        *c = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
                Self::from_rgba(buf).saturate(1.08)
            }
        }
    }

    /// 42. Dominant color via 4-bit quantization + frequency vote.
    /// Used e.g. for wallpaper light/dark split previews.
    pub fn dominant_color(&self) -> Result<Rgba8, ImageError> {
        if self.width() == 0 || self.height() == 0 {
            return Err(ImageError::InvalidInput(tr("empty_image")));
        }
        let mut votes: HashMap<(u8, u8, u8), (u32, u32, u32, u32)> = HashMap::new();
        for px in self.as_rgba().pixels() {
            if px[3] < 16 {
                continue;
            }
            let key = (px[0] >> 4, px[1] >> 4, px[2] >> 4);
            let e = votes.entry(key).or_insert((0, 0, 0, 0));
            e.0 += 1;
            e.1 += px[0] as u32;
            e.2 += px[1] as u32;
            e.3 += px[2] as u32;
        }
        let best = votes
            .values()
            .max_by_key(|v| v.0)
            .ok_or_else(|| ImageError::InvalidInput(tr("empty_image")))?;
        Ok(Rgba8::new(
            (best.1 / best.0) as u8,
            (best.2 / best.0) as u8,
            (best.3 / best.0) as u8,
            255,
        ))
    }

    /// 43. Average color (alpha-weighted).
    pub fn average_color(&self) -> Result<Rgba8, ImageError> {
        let mut sr = 0u64;
        let mut sg = 0u64;
        let mut sb = 0u64;
        let mut sa = 0u64;
        let mut n = 0u64;
        for px in self.as_rgba().pixels() {
            let a = px[3] as u64;
            sr += px[0] as u64 * a;
            sg += px[1] as u64 * a;
            sb += px[2] as u64 * a;
            sa += a;
            n += 1;
        }
        if n == 0 || sa == 0 {
            return Err(ImageError::InvalidInput(tr("empty_image")));
        }
        Ok(Rgba8::new(
            (sr / sa) as u8,
            (sg / sa) as u8,
            (sb / sa) as u8,
            255,
        ))
    }

    /// 44. Per-channel 256-bin histogram (for photo apps).
    pub fn histogram(&self) -> Histogram {
        let mut h = Histogram { r: vec![0; 256], g: vec![0; 256], b: vec![0; 256] };
        for px in self.as_rgba().pixels() {
            h.r[px[0] as usize] += 1;
            h.g[px[1] as usize] += 1;
            h.b[px[2] as usize] += 1;
        }
        h
    }

    /// 45. Mean luminance 0.0..=1.0 (Rec. 709).
    pub fn luminance(&self) -> f32 {
        let mut sum = 0u64;
        let n = (self.width() as u64 * self.height() as u64).max(1);
        for px in self.as_rgba().pixels() {
            sum += (0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32) as u64;
        }
        sum as f32 / n as f32 / 255.0
    }

    /// 46. `true` when the image reads as dark (luminance < 0.4).
    pub fn is_dark(&self) -> bool {
        self.luminance() < 0.4
    }

    /// 47. `true` when the image reads as light (luminance >= 0.6).
    pub fn is_light(&self) -> bool {
        self.luminance() >= 0.6
    }

    /// 48. Top-`k` palette colors (quantized frequency vote).
    pub fn palette(&self, k: usize) -> Vec<Rgba8> {
        let mut votes: HashMap<(u8, u8, u8), (u32, u32, u32, u32)> = HashMap::new();
        for px in self.as_rgba().pixels() {
            if px[3] < 16 {
                continue;
            }
            let key = (px[0] >> 4, px[1] >> 4, px[2] >> 4);
            let e = votes.entry(key).or_insert((0, 0, 0, 0));
            e.0 += 1;
            e.1 += px[0] as u32;
            e.2 += px[1] as u32;
            e.3 += px[2] as u32;
        }
        let mut entries: Vec<_> = votes.into_values().collect();
        entries.sort_by_key(|v| std::cmp::Reverse(v.0));
        entries
            .into_iter()
            .take(k.max(1))
            .map(|v| Rgba8::new((v.1 / v.0) as u8, (v.2 / v.0) as u8, (v.3 / v.0) as u8, 255))
            .collect()
    }
}
