//! Pure-Rust BMP codec (no third-party dependencies).
//!
//! Standalone `BM` files: 14-byte file header plus a DIB parsed with
//! the shared [`crate::codecs::ico`] parser (`parse_bmp_dib`,
//! `xor_pixel`, `xor_stride`). Supported: BITMAPCOREHEADER (12),
//! BITMAPINFOHEADER (40), V4/V5 with BI_RGB at 1/4/8 (palette), 24
//! (BGR) and 32 (BGRA) bits, bottom-up and top-down storage, RLE4/RLE8
//! compression and 16/32-bit BI_BITFIELDS. JPEG/PNG-compressed BMPs,
//! bitmap arrays (`BA`) and OS/2 V2 headers report `Unsupported`.

use crate::codecs::ico::{parse_bmp_dib, xor_pixel, xor_stride};

// ── Public API ────────────────────────────────────────────────

/// Errors of the BMP codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BmpError {
    Decode(String),
    Encode(String),
    Unsupported(String),
}

impl std::fmt::Display for BmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "bmp decode error: {m}"),
            Self::Encode(m) => write!(f, "bmp encode error: {m}"),
            Self::Unsupported(m) => write!(f, "bmp unsupported: {m}"),
        }
    }
}

impl std::error::Error for BmpError {}

/// Decoded BMP image: always RGBA8 pixels (alpha 255, BMP has none
/// except 32-bit BGRA or 16-bit XRGB with zeroed alpha).
#[derive(Debug, Clone)]
pub struct DecodedBmp {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Max dimension / pixel bytes (bounds allocation on corrupt input).
const MAX_DIMENSION: u32 = 32768;
const MAX_PIXEL_BYTES: u64 = 1024 * 1024 * 1024;

/// True for `BM` files (bitmap arrays and other magic rejected).
pub fn is_bmp(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == b'B' && bytes[1] == b'M'
}

/// Fast probe: dimensions from the headers without decoding pixels.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), BmpError> {
    let dib = split_file(bytes)?.1;
    let dib = parse_bmp_dib(dib).map_err(BmpError::Decode)?;
    let (w, h) = (dib.width, dib.height.unsigned_abs());
    if w == 0 || h == 0 {
        return Err(BmpError::Decode("zero image dimension".into()));
    }
    Ok((w, h))
}

/// Decode a BMP file into RGBA8 pixels.
pub fn decode(bytes: &[u8]) -> Result<DecodedBmp, BmpError> {
    let (data_off, dib_bytes) = split_file(bytes)?;
    let dib = parse_bmp_dib(dib_bytes).map_err(BmpError::Decode)?;
    // Pixel data must start after the headers (profile gaps allowed).
    let min_off = 14 + (dib_bytes.len() - dib.row_data.len()) as u32;
    if data_off < min_off {
        return Err(BmpError::Decode("invalid data offset".into()));
    }
    if dib.planes != 1 {
        return Err(BmpError::Decode("invalid BMP planes".into()));
    }
    let width = dib.width;
    let (height, top_down) = if dib.height < 0 {
        (-dib.height as u32, true)
    } else {
        (dib.height as u32, false)
    };
    if width == 0 || width > MAX_DIMENSION || height == 0 || height > MAX_DIMENSION {
        return Err(BmpError::Decode("invalid BMP dimensions".into()));
    }
    if width as u64 * height as u64 * 4 > MAX_PIXEL_BYTES {
        return Err(BmpError::Decode("image too large".into()));
    }

    // Pixel source: direct rows, RLE expansion or bitfield conversion.
    enum Rows {
        Packed { stride: usize, buf: Vec<u8>, bpp: u16 },
       Rgb { stride: usize, buf: Vec<u8> },
    }
    let rows = match dib.compression {
        0 => match dib.bpp {
            1 | 4 | 8 | 24 | 32 => {
                let stride = xor_stride(width, dib.bpp);
                let need = data_off as usize + stride * height as usize;
                if bytes.len() < need {
                    return Err(BmpError::Decode("truncated BMP pixels".into()));
                }
                Rows::Packed {
                    stride,
                    buf: bytes[data_off as usize..data_off as usize + stride * height as usize]
                        .to_vec(),
                    bpp: dib.bpp,
                }
            }
            16 => {
                let stride = xor_stride(width, 16);
                let need = data_off as usize + stride * height as usize;
                if bytes.len() < need {
                    return Err(BmpError::Decode("truncated BMP pixels".into()));
                }
                let raw = &bytes[data_off as usize..data_off as usize + stride * height as usize];
                Rows::Rgb {
                    stride: width as usize * 3,
                    buf: convert_16bit(width, height, stride, raw, dib_bytes)?,
                }
            }
            _ => return Err(BmpError::Decode(format!("bpp {} unsupported", dib.bpp))),
        },
        1 => {
            // BI_RLE8: 8-bit only.
            if dib.bpp != 8 {
                return Err(BmpError::Decode("RLE8 needs 8-bit".into()));
            }
            Rows::Packed {
                stride: width as usize,
                buf: rle8_decode(width, height, &bytes[data_off as usize..])?,
                bpp: 8,
            }
        }
        2 => {
            // BI_RLE4: 4-bit only.
            if dib.bpp != 4 {
                return Err(BmpError::Decode("RLE4 needs 4-bit".into()));
            }
            Rows::Packed {
                stride: (width as usize + 1) / 2,
                buf: rle4_decode(width, height, &bytes[data_off as usize..])?,
                bpp: 4,
            }
        }
        3 => {
            // BI_BITFIELDS: 16 or 32-bit with explicit masks.
            if !matches!(dib.bpp, 16 | 32) {
                return Err(BmpError::Decode("BITFIELDS needs 16/32-bit".into()));
            }
            let stride = xor_stride(width, dib.bpp);
            let need = data_off as usize + stride * height as usize;
            if bytes.len() < need {
                return Err(BmpError::Decode("truncated BMP pixels".into()));
            }
            let raw = &bytes[data_off as usize..data_off as usize + stride * height as usize];
            Rows::Rgb {
                stride: width as usize * 3,
                buf: convert_bitfields(width, height, stride, raw, dib.bpp, dib_bytes)?,
            }
        }
        c => {
            return Err(BmpError::Unsupported(format!(
                "compressed BMP ({c}) unsupported"
            )))
        }
    };

    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        let src_row = if top_down { y } else { height - 1 - y } as usize;
        for x in 0..width {
            let (r, g, b, a) = match &rows {
                Rows::Packed { stride, buf, bpp } => {
                    let row = &buf[src_row * stride..(src_row + 1) * stride];
                    xor_pixel(row, x as usize, *bpp, &dib.palette).map_err(BmpError::Decode)?
                }
                Rows::Rgb { stride, buf } => {
                    let o = src_row * stride + x as usize * 3;
                    (buf[o], buf[o + 1], buf[o + 2], 255)
                }
            };
            let o = (y as usize * width as usize + x as usize) * 4;
            pixels[o..o + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    Ok(DecodedBmp { width, height, pixels })
}

/// Encode RGBA8 pixels as 32-bit BI_RGB BMP (bottom-up, universally readable).
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, BmpError> {
    if width == 0 || height == 0 {
        return Err(BmpError::Encode("width and height must be > 0".into()));
    }
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(BmpError::Encode("pixel buffer length mismatch".into()));
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(BmpError::Encode("image too large".into()));
    }
    let stride = width as usize * 4;
    let off = 14 + 40u32;
    let size = off + (stride * height as usize) as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]); // reserved
    out.extend_from_slice(&off.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes()); // info header
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes()); // positive = bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((stride * height as usize) as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes()); // 72 dpi
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // colors
    out.extend_from_slice(&0u32.to_le_bytes()); // important
    for y in (0..height as usize).rev() {
        for x in 0..width as usize {
            let o = (y * width as usize + x) * 4;
            out.extend_from_slice(&[rgba[o + 2], rgba[o + 1], rgba[o], rgba[o + 3]]);
        }
    }
    Ok(out)
}

// ── File header ───────────────────────────────────────────────

/// Split the 14-byte file header: `(pixel data offset, DIB bytes)`.
fn split_file(bytes: &[u8]) -> Result<(u32, &[u8]), BmpError> {
    if !is_bmp(bytes) {
        return Err(BmpError::Decode("not a BMP file".into()));
    }
    if bytes.len() < 14 {
        return Err(BmpError::Decode("truncated file header".into()));
    }
    let off = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
    if off < 14 || (off as usize) > bytes.len() {
        return Err(BmpError::Decode("invalid data offset".into()));
    }
    Ok((off, &bytes[14..]))
}

// ── RLE ───────────────────────────────────────────────────────

/// Decode BI_RLE8 into packed index rows (bottom-up file order).
fn rle8_decode(width: u32, height: u32, mut data: &[u8]) -> Result<Vec<u8>, BmpError> {
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * h];
    let (mut x, mut y) = (0usize, 0usize);
    let mut put = |out: &mut [u8], x: usize, y: usize, v: u8| {
        if x < w && y < h {
            out[y * w + x] = v;
        }
    };
    while y < h {
        if data.len() < 2 {
            return Err(BmpError::Decode("truncated RLE data".into()));
        }
        let (count, cmd) = (data[0], data[1]);
        data = &data[2..];
        if count > 0 {
            for _ in 0..count as usize {
                put(&mut out, x, y, cmd);
                x += 1;
            }
        } else {
            match cmd {
                0 => {
                    x = 0;
                    y += 1;
                } // end of line
                1 => break, // end of bitmap
                2 => {
                    if data.len() < 2 {
                        return Err(BmpError::Decode("truncated RLE delta".into()));
                    }
                    x += data[0] as usize;
                    y += data[1] as usize;
                    data = &data[2..];
                }
                n => {
                    let n = n as usize;
                    if data.len() < n {
                        return Err(BmpError::Decode("truncated RLE data".into()));
                    }
                    for &v in &data[..n] {
                        put(&mut out, x, y, v);
                        x += 1;
                    }
                    data = &data[n..];
                    if n % 2 == 1 {
                        if data.is_empty() {
                            return Err(BmpError::Decode("truncated RLE data".into()));
                        }
                        data = &data[1..]; // WORD-align pad byte
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Decode BI_RLE4 into packed 4-bit index rows (bottom-up file order),
/// matching plain 4-bit bitmap layout for the shared render path.
fn rle4_decode(width: u32, height: u32, mut data: &[u8]) -> Result<Vec<u8>, BmpError> {
    let (w, h) = (width as usize, height as usize);
    let stride = (w + 1) / 2;
    let mut out = vec![0u8; stride * h];
    let (mut x, mut y) = (0usize, 0usize);
    let mut put = |out: &mut [u8], x: usize, y: usize, v: u8| {
        if x < w && y < h {
            let o = y * stride + x / 2;
            if x % 2 == 0 {
                out[o] |= v << 4;
            } else {
                out[o] |= v & 15;
            }
        }
    };
    while y < h {
        if data.len() < 2 {
            return Err(BmpError::Decode("truncated RLE data".into()));
        }
        let (count, cmd) = (data[0] as usize, data[1]);
        data = &data[2..];
        if count > 0 {
            let (hi, lo) = (cmd >> 4, cmd & 15);
            for i in 0..count {
                put(&mut out, x, y, if i % 2 == 0 { hi } else { lo });
                x += 1;
            }
        } else {
            match cmd {
                0 => {
                    x = 0;
                    y += 1;
                }
                1 => break,
                2 => {
                    if data.len() < 2 {
                        return Err(BmpError::Decode("truncated RLE delta".into()));
                    }
                    x += data[0] as usize;
                    y += data[1] as usize;
                    data = &data[2..];
                }
                n => {
                    // Absolute mode: n nibbles follow (high first),
                    // padded with one byte when the count is odd.
                    let bytes = (n as usize + 1) / 2;
                    if data.len() < bytes {
                        return Err(BmpError::Decode("truncated RLE data".into()));
                    }
                    for i in 0..n as usize {
                        let b = data[i / 2];
                        put(&mut out, x, y, if i % 2 == 0 { b >> 4 } else { b & 15 });
                        x += 1;
                    }
                    data = &data[bytes..];
                    if bytes % 2 == 1 {
                        if data.is_empty() {
                            return Err(BmpError::Decode("truncated RLE data".into()));
                        }
                        data = &data[1..];
                    }
                }
            }
        }
    }
    Ok(out)
}

// ── 16-bit and bitfields ──────────────────────────────────────

/// Scale an n-bit sample (n <= 8... actually any width) to 8 bits.
fn scale_bits(v: u32, bits: u32) -> u8 {
    if bits == 0 {
        return 0;
    }
    if bits >= 8 {
        return (v >> (bits - 8)) as u8;
    }
    ((v * 255 + ((1 << bits) - 1) / 2) / ((1 << bits) - 1)) as u8
}

/// Read a little-endian sample of `bpp` bits (16 or 32) at pixel offset.
fn read_sample(raw: &[u8], stride: usize, x: usize, y: usize, bpp: u16) -> Result<u32, BmpError> {
    let o = y * stride + x * (bpp as usize / 8);
    if o + bpp as usize / 8 > raw.len() {
        return Err(BmpError::Decode("scanline overrun".into()));
    }
    Ok(match bpp {
        16 => u16::from_le_bytes([raw[o], raw[o + 1]]) as u32,
        _ => u32::from_le_bytes([raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]),
    })
}

/// Split a mask into (shift, width).
fn mask_info(mask: u32) -> (u32, u32) {
    if mask == 0 {
        return (0, 0);
    }
    let shift = mask.trailing_zeros();
    (shift, 32 - (mask >> shift).leading_zeros())
}

/// Convert 16-bit BI_RGB (XRGB 555) to packed RGB rows.
fn convert_16bit(
    width: u32,
    height: u32,
    stride: usize,
    raw: &[u8],
    _dib: &[u8],
) -> Result<Vec<u8>, BmpError> {
    let mut out = vec![0u8; width as usize * height as usize * 3];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let v = read_sample(raw, stride, x, y, 16)?;
            let o = (y * width as usize + x) * 3;
            out[o] = scale_bits((v >> 10) & 31, 5);
            out[o + 1] = scale_bits((v >> 5) & 31, 5);
            out[o + 2] = scale_bits(v & 31, 5);
        }
    }
    Ok(out)
}

/// Convert BI_BITFIELDS 16/32-bit with explicit masks from the DIB.
fn convert_bitfields(
    width: u32,
    height: u32,
    stride: usize,
    raw: &[u8],
    bpp: u16,
    dib: &[u8],
) -> Result<Vec<u8>, BmpError> {
    // Masks follow the 40-byte core for INFOHEADER; V4+ carry them inline.
    let header_size = u32::from_le_bytes([dib[0], dib[1], dib[2], dib[3]]) as usize;
    let (rm, gm, bm) = if header_size == 40 {
        if dib.len() < 52 {
            return Err(BmpError::Decode("truncated BITFIELDS masks".into()));
        }
        (
            u32::from_le_bytes([dib[40], dib[41], dib[42], dib[43]]),
            u32::from_le_bytes([dib[44], dib[45], dib[46], dib[47]]),
            u32::from_le_bytes([dib[48], dib[49], dib[50], dib[51]]),
        )
    } else {
        // V4/V5 red/green/blue masks live at fixed offsets 40/44/48.
        if dib.len() < 52 {
            return Err(BmpError::Decode("truncated BITFIELDS masks".into()));
        }
        (
            u32::from_le_bytes([dib[40], dib[41], dib[42], dib[43]]),
            u32::from_le_bytes([dib[44], dib[45], dib[46], dib[47]]),
            u32::from_le_bytes([dib[48], dib[49], dib[50], dib[51]]),
        )
    };
    if rm == 0 && gm == 0 && bm == 0 {
        return Err(BmpError::Decode("empty BITFIELDS masks".into()));
    }
    let (rs, rw) = mask_info(rm);
    let (gs, gw) = mask_info(gm);
    let (bs, bw) = mask_info(bm);
    let mut out = vec![0u8; width as usize * height as usize * 3];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let v = read_sample(raw, stride, x, y, bpp)?;
            let o = (y * width as usize + x) * 3;
            out[o] = scale_bits((v & rm) >> rs, rw);
            out[o + 1] = scale_bits((v & gm) >> gs, gw);
            out[o + 2] = scale_bits((v & bm) >> bs, bw);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_info_splits() {
        assert_eq!(mask_info(0x7C00), (10, 5));
        assert_eq!(mask_info(0x03E0), (5, 5));
        assert_eq!(mask_info(0x001F), (0, 5));
        assert_eq!(mask_info(0xF800), (11, 5));
        assert_eq!(mask_info(0x07E0), (5, 6));
        assert_eq!(mask_info(0), (0, 0));
    }

    #[test]
    fn scale_bits_rounds() {
        assert_eq!(scale_bits(0, 5), 0);
        assert_eq!(scale_bits(31, 5), 255);
        assert_eq!(scale_bits(1, 1), 255);
        assert_eq!(scale_bits(0x1234, 16), 0x12);
    }
}
