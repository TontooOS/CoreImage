//! Colour conversion from decoded AV1 planes to RGBA8.

use super::Frame;

/// Matrix coefficients that appear in AVIF files.
const MC_IDENTITY: u32 = 0;
const MC_BT_709: u32 = 1;
const MC_UNSPECIFIED: u32 = 2;
const MC_BT_470_MC: u32 = 4;
const MC_BT_470_BG: u32 = 5;
const MC_BT_601: u32 = 6;
const MC_SMPTE_240: u32 = 7;
const MC_BT_2020_NCL: u32 = 9;

/// Convert a decoded frame to RGBA8.
///
/// `colour` carries the container `colr` values. When present they win over the
/// sequence header, matching how players treat AVIF files.
pub fn frame_to_rgba(
    frame: &Frame,
    colour: Option<crate::codecs::avif::ColourInfo>,
) -> Vec<u8> {
    let (matrix, full_range) = match colour {
        Some(c) => {
            let matrix = if c.matrix as u32 == MC_UNSPECIFIED {
                frame.matrix_coefficients
            } else {
                c.matrix as u32
            };
            (matrix, c.full_range)
        }
        None => (frame.matrix_coefficients, frame.color_range),
    };
    let (kr, kb) = luma_coefficients(matrix);
    let width = frame.width as usize;
    let height = frame.height as usize;
    let mut out = vec![0u8; width * height * 4];
    let y_plane = &frame.planes[0];
    let has_chroma = frame.planes.len() >= 3;
    let cw = if has_chroma { frame.planes[1].width as usize } else { 0 };
    let ch = if has_chroma { frame.planes[1].height as usize } else { 0 };
    for y in 0..height {
        let row = y_plane.row(y as u32);
        for x in 0..width {
            let luma = row[x] as i32;
            let (r, g, b) = if has_chroma && cw > 0 {
                let (cb, cr) = sample_chroma(frame, x, y, cw, ch);
                yuv_to_rgb(luma, cb, cr, kr, kb, full_range, frame.bit_depth)
            } else {
                let v = luma_to_rgb(luma, full_range, frame.bit_depth);
                (v, v, v)
            };
            let i = (y * width + x) * 4;
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
            out[i + 3] = 255;
        }
    }
    out
}

/// Nearest neighbour chroma sample, matching the simple upsample of players.
fn sample_chroma(frame: &Frame, x: usize, y: usize, cw: usize, ch: usize) -> (i32, i32) {
    let sx = x * cw / frame.width as usize;
    let sy = y * ch / frame.height as usize;
    let cb = frame.planes[1].get(sx.min(cw - 1) as u32, sy.min(ch - 1) as u32) as i32;
    let cr = frame.planes[2].get(sx.min(cw - 1) as u32, sy.min(ch - 1) as u32) as i32;
    (cb, cr)
}

/// Luma and chroma coefficients `kr`, `kb` of the matrix.
fn luma_coefficients(matrix: u32) -> (f64, f64) {
    match matrix {
        MC_BT_709 => (0.2126, 0.0722),
        MC_BT_470_MC | MC_BT_470_BG | MC_BT_601 | MC_SMPTE_240 => (0.299, 0.114),
        MC_BT_2020_NCL => (0.2627, 0.0593),
        MC_UNSPECIFIED => (0.301, 0.114),
        _ => (0.2126, 0.0722),
    }
}

fn scale_sample(value: i32, bit_depth: u32, full_range: bool, is_chroma: bool) -> f64 {
    let max = ((1u32 << bit_depth) - 1) as f64;
    let v = value as f64;
    if full_range {
        if is_chroma {
            (v - max / 2.0) / (max / 2.0)
        } else {
            v / max
        }
    } else if is_chroma {
        (v - max / 2.0) / (max * 0.5 / 255.0 * 224.0 / 255.0)
    } else {
        (v - 16.0 / 255.0 * max) / (224.0 / 255.0 * max)
    }
}

fn yuv_to_rgb(
    luma: i32,
    cb: i32,
    cr: i32,
    kr: f64,
    kb: f64,
    full_range: bool,
    bit_depth: u32,
) -> (u8, u8, u8) {
    let y = scale_sample(luma, bit_depth, full_range, false);
    let u = scale_sample(cb, bit_depth, full_range, true);
    let v = scale_sample(cr, bit_depth, full_range, true);
    let kg = 1.0 - kr - kb;
    let r = y + 2.0 * (1.0 - kr) * v;
    let b = y + 2.0 * (1.0 - kb) * u;
    let g = y - (2.0 * (1.0 - kr) * kr / kg) * v - (2.0 * (1.0 - kb) * kb / kg) * u;
    (to_byte(r, full_range), to_byte(g, full_range), to_byte(b, full_range))
}

fn luma_to_rgb(luma: i32, full_range: bool, bit_depth: u32) -> u8 {
    let y = scale_sample(luma, bit_depth, full_range, false);
    to_byte(y, full_range)
}

fn to_byte(value: f64, full_range: bool) -> u8 {
    let scaled = if full_range {
        value * 255.0
    } else {
        (value * 224.0 + 16.0)
    };
    let rounded = scaled.round();
    if rounded <= 0.0 {
        0
    } else if rounded >= 255.0 {
        255
    } else {
        rounded as u8
    }
}

/// True when the matrix is the identity (GBR planes).
pub fn is_identity(matrix: u32) -> bool {
    matrix == MC_IDENTITY
}