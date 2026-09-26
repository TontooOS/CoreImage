//! Scaling, cropping, fit modes, rotation and mirroring.

use image::imageops::FilterType;

use crate::{ImageError, TiImage, lang::tr};

/// Aspect-fit behavior for [`TiImage::fit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    /// Scale to fill, then center-crop to the exact size.
    Fill,
    /// Scale to fit inside, letterboxed with transparent padding.
    Fit,
    /// Stretch to the exact size, ignoring aspect ratio.
    Stretch,
}

/// 90-degree rotation steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotate90 {
    Deg90,
    Deg180,
    Deg270,
}

impl TiImage {
    /// 12. Resize to exact dimensions (Lanczos3 default recommended).
    pub fn resize(&self, width: u32, height: u32, filter: FilterType) -> Self {
        let buf = image::imageops::resize(self.as_rgba(), width.max(1), height.max(1), filter);
        Self::from_rgba(buf)
    }

    /// 13. Downscale so the longer edge equals `max_size` (keeps aspect).
    pub fn thumbnail(&self, max_size: u32) -> Result<Self, ImageError> {
        if max_size == 0 {
            return Err(ImageError::InvalidInput(tr("invalid_size")));
        }
        let (w, h) = self.dimensions();
        let scale = (max_size as f32 / w.max(h) as f32).min(1.0);
        if scale >= 1.0 {
            return Ok(self.clone());
        }
        Ok(self.resize(
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
            FilterType::Lanczos3,
        ))
    }

    /// 14. Crop a rectangle; errors when out of bounds.
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Result<Self, ImageError> {
        let (w, h) = self.dimensions();
        if width == 0 || height == 0 || x + width > w || y + height > h {
            return Err(ImageError::InvalidInput(tr("invalid_crop")));
        }
        let mut dynimg = image::DynamicImage::ImageRgba8(self.as_rgba().clone());
        let sub = image::imageops::crop(&mut dynimg, x, y, width, height).to_image();
        Ok(Self::from_rgba(sub))
    }

    /// 15. Center-crop to an aspect ratio (`aspect_w`:`aspect_h`).
    pub fn crop_aspect(&self, aspect_w: u32, aspect_h: u32) -> Result<Self, ImageError> {
        if aspect_w == 0 || aspect_h == 0 {
            return Err(ImageError::InvalidInput(tr("invalid_size")));
        }
        let (w, h) = self.dimensions();
        let target = aspect_w as f32 / aspect_h as f32;
        let current = w as f32 / h as f32;
        let (cw, ch) = if current > target {
            ((h as f32 * target).round() as u32, h)
        } else {
            (w, (w as f32 / target).round() as u32)
        };
        let (cw, ch) = (cw.max(1).min(w), ch.max(1).min(h));
        self.crop((w - cw) / 2, (h - ch) / 2, cw, ch)
    }

    /// 16. Fit into `width` x `height` with a [`FitMode`].
    pub fn fit(&self, width: u32, height: u32, mode: FitMode) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::InvalidInput(tr("invalid_size")));
        }
        let (w, h) = self.dimensions();
        match mode {
            FitMode::Stretch => Ok(self.resize(width, height, FilterType::Lanczos3)),
            FitMode::Fit => {
                let scale = (width as f32 / w as f32).min(height as f32 / h as f32);
                let (nw, nh) = (
                    ((w as f32 * scale).round() as u32).max(1),
                    ((h as f32 * scale).round() as u32).max(1),
                );
                let small = self.resize(nw, nh, FilterType::Lanczos3);
                let mut canvas =
                    crate::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 0]));
                image::imageops::overlay(
                    &mut canvas,
                    small.as_rgba(),
                    ((width - nw) / 2) as i64,
                    ((height - nh) / 2) as i64,
                );
                Ok(Self::from_rgba(canvas))
            }
            FitMode::Fill => {
                let scale = (width as f32 / w as f32).max(height as f32 / h as f32);
                let (nw, nh) = (
                    ((w as f32 * scale).round() as u32).max(1),
                    ((h as f32 * scale).round() as u32).max(1),
                );
                let big = self.resize(nw, nh, FilterType::Lanczos3);
                big.crop((nw - width) / 2, (nh - height) / 2, width, height)
            }
        }
    }

    /// 17. Rotate by 90-degree steps.
    pub fn rotate_90(&self, step: Rotate90) -> Self {
        let buf = match step {
            Rotate90::Deg90 => image::imageops::rotate90(self.as_rgba()),
            Rotate90::Deg180 => image::imageops::rotate180(self.as_rgba()),
            Rotate90::Deg270 => image::imageops::rotate270(self.as_rgba()),
        };
        Self::from_rgba(buf)
    }

    /// 18. Free rotation by `angle_degrees` (expands canvas, fills with `background`).
    pub fn rotate_free(&self, angle_degrees: f32, background: crate::Rgba8) -> Self {
        use imageproc::geometric_transformations::{Interpolation, rotate_about_center};
        let bg = background.into_image_pixel();
        let rad = angle_degrees.to_radians();
        let buf = rotate_about_center(
            self.as_rgba(),
            rad,
            Interpolation::Bilinear,
            bg,
        );
        Self::from_rgba(buf)
    }

    /// 19. Mirror horizontally.
    pub fn flip_horizontal(&self) -> Self {
        Self::from_rgba(image::imageops::flip_horizontal(self.as_rgba()))
    }

    /// 20. Mirror vertically.
    pub fn flip_vertical(&self) -> Self {
        Self::from_rgba(image::imageops::flip_vertical(self.as_rgba()))
    }

    /// 21. Perspective correction placeholder (v2): applies a mild
    /// horizontal skew as a preview of the full quad warp API.
    /// Full 4-point perspective warp is tracked for v2.
    pub fn perspective_skew(&self, skew_x: f32) -> Self {
        use imageproc::geometric_transformations::{Interpolation, Projection, warp};
        let skew = skew_x.clamp(-1.0, 1.0);
        let proj = Projection::from_matrix([1.0, skew, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
            .unwrap_or_else(|| Projection::from_matrix([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).unwrap());
        let buf: crate::RgbaImage = warp(
            self.as_rgba(),
            &proj,
            Interpolation::Bilinear,
            image::Rgba([0, 0, 0, 0]),
        );
        Self::from_rgba(buf)
    }
}
