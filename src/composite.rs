//! Layering, blend modes, text watermarks (CoreText), SF Symbols (CoreIcon),
//! masks and rounded corners.

use crate::{ImageError, Rgba8, TiImage, lang::tr};
use ab_glyph::{Font, ScaleFont};

/// Blend modes for [`TiImage::blend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Add,
}

fn blend_px(base: u8, top: u8, mode: BlendMode) -> u8 {
    let (b, t) = (base as f32 / 255.0, top as f32 / 255.0);
    let v = match mode {
        BlendMode::Normal => t,
        BlendMode::Multiply => b * t,
        BlendMode::Screen => 1.0 - (1.0 - b) * (1.0 - t),
        BlendMode::Overlay => {
            if b < 0.5 {
                2.0 * b * t
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - t)
            }
        }
        BlendMode::Add => (b + t).min(1.0),
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

impl TiImage {
    /// 33. Overlay `other` at (`x`, `y`) with normal alpha compositing.
    pub fn overlay(&self, other: &TiImage, x: i64, y: i64) -> Self {
        let mut base = self.as_rgba().clone();
        image::imageops::overlay(&mut base, other.as_rgba(), x, y);
        Self::from_rgba(base)
    }

    /// 34. Blend `other` at (`x`, `y`) with a blend mode + global opacity.
    pub fn blend(&self, other: &TiImage, mode: BlendMode, opacity: f32, x: i64, y: i64) -> Self {
        let op = opacity.clamp(0.0, 1.0);
        let mut base = self.as_rgba().clone();
        let (bw, bh) = (base.width() as i64, base.height() as i64);
        for (ox, oy, px) in other.as_rgba().enumerate_pixels() {
            let (dx, dy) = (x + ox as i64, y + oy as i64);
            if dx < 0 || dy < 0 || dx >= bw || dy >= bh {
                continue;
            }
            let dst = base.get_pixel_mut(dx as u32, dy as u32);
            let alpha = px[3] as f32 / 255.0 * op;
            for c in 0..3 {
                let blended = blend_px(dst[c], px[c], mode);
                dst[c] = (dst[c] as f32 * (1.0 - alpha) + blended as f32 * alpha)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            if mode == BlendMode::Normal {
                let a = dst[3] as f32 / 255.0;
                dst[3] = ((a + (px[3] as f32 / 255.0) * op * (1.0 - a)) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
        Self::from_rgba(base)
    }

    /// 35. Text watermark rendered with the SF Pro system font.
    ///
    /// Font resolution follows CoreText rules (`coretext::FontRegistry`
    /// + `system_font_dirs`): SF Pro files from
    /// `/usr/share/fonts/OTF`, `/usr/share/fonts/TTF`, `~/.fonts`.
    /// Glyph rasterization uses `ab_glyph` into the RGBA buffer.
    pub fn watermark_text(
        &self,
        text: &str,
        x: u32,
        y: u32,
        size: f32,
        color: Rgba8,
    ) -> Result<Self, ImageError> {
        self.watermark_text_coretext(text, "SF Pro", x, y, size, color)
    }

    /// 36. Text watermark with explicit font family via CoreText.
    ///
    /// `family` is resolved with `coretext::FontRegistry::resolve`
    /// (custom registrations win, then system scan, then SF Pro
    /// fallback). When no font file is found, a built-in bitmap
    /// block fallback paints a placeholder bar so callers still get
    /// a visible mark instead of an error.
    pub fn watermark_text_coretext(
        &self,
        text: &str,
        family: &str,
        x: u32,
        y: u32,
        size: f32,
        color: Rgba8,
    ) -> Result<Self, ImageError> {
        if text.is_empty() {
            return Err(ImageError::InvalidInput(tr("text_empty")));
        }
        let mut out = self.as_rgba().clone();
        let registry = coretext::FontRegistry::new();
        let desc = coretext::CTFontDescriptor::new(family, size.max(6.0));
        let resolved = registry.resolve(&desc);
        let scale_px = size.max(6.0);

        if let Some(path) = resolved.file {
            let data = std::fs::read(&path).map_err(|e| ImageError::Font(e.to_string()))?;
            let font = ab_glyph::FontArc::try_from_vec(data)
                .map_err(|e| ImageError::Font(e.to_string()))?;
            let scale = ab_glyph::PxScale::from(scale_px);
            let scaled = font.as_scaled(scale);
            let mut cx = x as f32;
            let baseline = y as f32 + scaled.ascent();
            for ch in text.chars() {
                let glyph_id = font.glyph_id(ch);
                let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(cx, baseline));
                let outlined = scaled.outline_glyph(glyph);
                if let Some(o) = outlined {
                    let bb = o.px_bounds();
                    o.draw(|gx, gy, v| {
                        let (px, py) = (bb.min.x as i64 + gx as i64, bb.min.y as i64 + gy as i64);
                        if px < 0 || py < 0 || px >= out.width() as i64 || py >= out.height() as i64 {
                            return;
                        }
                        let dst = out.get_pixel_mut(px as u32, py as u32);
                        let a = (v * color.a as f32 / 255.0).clamp(0.0, 1.0);
                        for c in 0..3 {
                            let src = [color.r, color.g, color.b][c] as f32;
                            dst[c] = (dst[c] as f32 * (1.0 - a) + src * a).round() as u8;
                        }
                        let da = dst[3] as f32 / 255.0;
                        dst[3] = ((da + a * (1.0 - da)) * 255.0).round() as u8;
                    });
                    cx += scaled.h_advance(glyph_id);
                } else {
                    cx += scaled.h_advance(glyph_id);
                }
                if cx >= out.width() as f32 {
                    break;
                }
            }
        } else {
            // Bitmap fallback: visible placeholder bar (no font file on system).
            let bar_h = (scale_px / 2.0).round() as u32;
            let bar_w = ((text.chars().count() as f32) * scale_px * 0.6).round() as u32;
            for dy in 0..bar_h {
                for dx in 0..bar_w {
                    let (px, py) = (x + dx, y + dy);
                    if px < out.width() && py < out.height() {
                        let dst = out.get_pixel_mut(px, py);
                        let a = color.a as f32 / 255.0 * 0.85;
                        for c in 0..3 {
                            let src = [color.r, color.g, color.b][c] as f32;
                            dst[c] = (dst[c] as f32 * (1.0 - a) + src * a).round() as u8;
                        }
                    }
                }
            }
        }
        Ok(Self::from_rgba(out))
    }

    /// 37. Overlay an SF Symbol (CoreIcon) by name, e.g. `"star.fill"`.
    ///
    /// The PNG is resolved with `coreicon::resolve_icon_path`,
    /// scaled to (`width`, `height`) and tinted when `tint` is set.
    pub fn overlay_sf_symbol(
        &self,
        symbol_name: &str,
        x: i64,
        y: i64,
        width: u32,
        height: u32,
        tint: Option<Rgba8>,
    ) -> Result<Self, ImageError> {
        let path = coreicon::resolve_icon_path(symbol_name);
        if !path.exists() {
            return Err(ImageError::Icon(format!(
                "{}: {}",
                tr("err_icon"),
                symbol_name
            )));
        }
        let icon = crate::io::load(&path.to_string_lossy())
            .map_err(|_| ImageError::Icon(symbol_name.to_string()))?;
        let icon = icon.resize(width.max(1), height.max(1), image::imageops::FilterType::Lanczos3);
        let icon = match tint {
            Some(t) => icon.tint(t),
            None => icon,
        };
        Ok(self.overlay(&icon, x, y))
    }

    /// Tint helper used by [`TiImage::overlay_sf_symbol`]: multiplies
    /// RGB channels while preserving per-pixel alpha.
    pub fn tint(&self, tint: Rgba8) -> Self {
        let mut buf = self.as_rgba().clone();
        for px in buf.pixels_mut() {
            px[0] = (px[0] as u16 * tint.r as u16 / 255) as u8;
            px[1] = (px[1] as u16 * tint.g as u16 / 255) as u8;
            px[2] = (px[2] as u16 * tint.b as u16 / 255) as u8;
            px[3] = (px[3] as u16 * tint.a as u16 / 255) as u8;
        }
        Self::from_rgba(buf)
    }

    /// 38. Apply a grayscale alpha mask (white = opaque).
    pub fn alpha_mask(&self, mask: &TiImage) -> Result<Self, ImageError> {
        let (w, h) = self.dimensions();
        let (mw, mh) = mask.dimensions();
        if w != mw || h != mh {
            return Err(ImageError::InvalidInput(tr("invalid_size")));
        }
        let mut buf = self.as_rgba().clone();
        for (dst, m) in buf.pixels_mut().zip(mask.as_rgba().pixels()) {
            let luma = (0.299 * m[0] as f32 + 0.587 * m[1] as f32 + 0.114 * m[2] as f32).round() as u8;
            dst[3] = ((dst[3] as u16 * luma as u16) / 255) as u8;
        }
        Ok(Self::from_rgba(buf))
    }

    /// 39. Rounded corners with `radius` px (e.g. for icon previews).
    pub fn rounded_corners(&self, radius: u32) -> Self {
        let (w, h) = self.dimensions();
        let r = radius.min(w / 2).min(h / 2) as f32;
        let mut buf = self.as_rgba().clone();
        for (x, y, px) in buf.enumerate_pixels_mut() {
            let corners = [
                (r - x as f32 - 1.0, r - y as f32 - 1.0),
                (x as f32 - (w as f32 - r), r - y as f32 - 1.0),
                (r - x as f32 - 1.0, y as f32 - (h as f32 - r)),
                (x as f32 - (w as f32 - r), y as f32 - (h as f32 - r)),
            ];
            for (dx, dy) in corners {
                if dx > 0.0 && dy > 0.0 && dx * dx + dy * dy > r * r {
                    px[3] = 0;
                    break;
                }
            }
        }
        Self::from_rgba(buf)
    }

    /// 40. Circular crop (centered, diameter = shorter edge).
    pub fn circular_mask(&self) -> Self {
        let (w, h) = self.dimensions();
        let side = w.min(h);
        let cropped = self
            .crop((w - side) / 2, (h - side) / 2, side, side)
            .unwrap_or_else(|_| self.clone());
        cropped.rounded_corners(side / 2)
    }
}
