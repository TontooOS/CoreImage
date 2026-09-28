//! Pure-Rust ICO codec (icon containers, no third-party dependencies).
//!
//! An ICO file is an `ICONDIR` header plus one `ICONDIRENTRY` per image.
//! Each entry is either PNG-compressed (decoded with [`crate::codecs::png`])
//! or a BMP without file header: XOR bitmap (1/4/8-bit paletted, 24-bit
//! BGR, 32-bit BGRA) plus a 1-bit AND mask for transparency. The entry
//! height field counts XOR + AND rows, so the visible height is half the
//! BMP height. A zero width/height byte means 256.
//!
//! The BMP entry parser ([`decode_bmp_entry`]) is `pub(crate)` so the
//! future `bmp.rs` codec can reuse it.

use crate::codecs::png;

// ── Public API ────────────────────────────────────────────────

/// Errors of the ICO codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcoError {
    Decode(String),
    Encode(String),
    Unsupported(String),
}

impl std::fmt::Display for IcoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "ico decode error: {m}"),
            Self::Encode(m) => write!(f, "ico encode error: {m}"),
            Self::Unsupported(m) => write!(f, "ico unsupported: {m}"),
        }
    }
}

impl std::error::Error for IcoError {}

/// One decoded ICO entry: always RGBA8 pixels.
#[derive(Debug, Clone)]
pub struct IcoImage {
    pub width: u32,
    pub height: u32,
    /// Bits per pixel of the source entry (32 for PNG entries).
    pub bpp: u16,
    pub pixels: Vec<u8>,
}

/// Decoded ICO file: all entries, largest first.
#[derive(Debug, Clone)]
pub struct DecodedIco {
    pub images: Vec<IcoImage>,
}

impl DecodedIco {
    /// The largest entry (by area, then bit depth). Used for [`crate::TiImage`].
    pub fn largest(&self) -> Option<&IcoImage> {
        self.images.first()
    }
}

/// True for ICO files (reserved 0, type 1). Cursors (type 2) are excluded.
pub fn is_ico(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && bytes[0] == 0 && bytes[1] == 0 && bytes[2] == 1 && bytes[3] == 0
}

/// Fast probe: dimensions of the largest entry without full decode.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), IcoError> {
    let entries = parse_dir(bytes)?;
    let (w, h) = entries
        .iter()
        .map(|e| (e.width, e.height))
        .max_by_key(|(w, h)| (*w as u64) * (*h as u64))
        .ok_or_else(|| IcoError::Decode("no entries".into()))?;
    Ok((w, h))
}

/// Decode all entries of an ICO file, largest first.
pub fn decode(bytes: &[u8]) -> Result<DecodedIco, IcoError> {
    let entries = parse_dir(bytes)?;
    let mut images = Vec::with_capacity(entries.len());
    for e in &entries {
        let blob = bytes
            .get(e.offset as usize..e.offset as usize + e.bytes as usize)
            .ok_or_else(|| IcoError::Decode("entry out of bounds".into()))?;
        images.push(decode_entry(e, blob)?);
    }
    // Largest area first, then highest bit depth.
    images.sort_by(|a, b| {
        ((b.width as u64) * (b.height as u64))
            .cmp(&((a.width as u64) * (a.height as u64)))
            .then(b.bpp.cmp(&a.bpp))
    });
    Ok(DecodedIco { images })
}

/// Encode one RGBA8 image as a single PNG-compressed ICO entry.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, IcoError> {
    encode_images(&[(width, height, rgba)])
}

/// Encode several RGBA8 images as PNG-compressed ICO entries.
pub fn encode_images(images: &[ (u32, u32, &[u8]) ]) -> Result<Vec<u8>, IcoError> {
    if images.is_empty() {
        return Err(IcoError::Encode("no images".into()));
    }
    let mut blobs = Vec::with_capacity(images.len());
    for (w, h, px) in images {
        if *w == 0 || *h == 0 || *w > 256 || *h > 256 {
            return Err(IcoError::Encode("dimensions must be 1..=256".into()));
        }
        if px.len() != *w as usize * *h as usize * 4 {
            return Err(IcoError::Encode("pixel buffer length mismatch".into()));
        }
        let png = png::encode(*w, *h, px).map_err(|e| IcoError::Encode(e.to_string()))?;
        blobs.push((*w, *h, png));
    }
    let count = blobs.len();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(count as u16).to_le_bytes());
    let mut offset = 6 + 16 * count as u32;
    for (w, h, blob) in &blobs {
        out.push(if *w >= 256 { 0 } else { *w as u8 });
        out.push(if *h >= 256 { 0 } else { *h as u8 });
        out.extend_from_slice(&[0, 0]); // colors, reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += blob.len() as u32;
    }
    for (_, _, blob) in &blobs {
        out.extend_from_slice(blob);
    }
    Ok(out)
}

// ── Directory ─────────────────────────────────────────────────

struct Entry {
    width: u32,
    height: u32,
    bpp: u16,
    bytes: u32,
    offset: u32,
}

fn parse_dir(bytes: &[u8]) -> Result<Vec<Entry>, IcoError> {
    if bytes.len() < 6 {
        return Err(IcoError::Decode("truncated header".into()));
    }
    if bytes[0] != 0 || bytes[1] != 0 {
        return Err(IcoError::Decode("not an ICO file".into()));
    }
    match u16::from_le_bytes([bytes[2], bytes[3]]) {
        1 => {}
        2 => return Err(IcoError::Unsupported("cursor (.cur) files unsupported".into())),
        t => return Err(IcoError::Decode(format!("unknown type {t}"))),
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 {
        return Err(IcoError::Decode("no entries".into()));
    }
    if bytes.len() < 6 + 16 * count {
        return Err(IcoError::Decode("truncated directory".into()));
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let o = 6 + 16 * i;
        let w = bytes[o] as u32;
        let h = bytes[o + 1] as u32;
        let bpp = u16::from_le_bytes([bytes[o + 6], bytes[o + 7]]);
        let size = u32::from_le_bytes([bytes[o + 8], bytes[o + 9], bytes[o + 10], bytes[o + 11]]);
        let off = u32::from_le_bytes([bytes[o + 12], bytes[o + 13], bytes[o + 14], bytes[o + 15]]);
        if size == 0 {
            return Err(IcoError::Decode("empty entry".into()));
        }
        if off as u64 + size as u64 > bytes.len() as u64 {
            return Err(IcoError::Decode("entry out of bounds".into()));
        }
        entries.push(Entry {
            width: if w == 0 { 256 } else { w },
            height: if h == 0 { 256 } else { h },
            bpp,
            bytes: size,
            offset: off,
        });
    }
    Ok(entries)
}

fn decode_entry(e: &Entry, blob: &[u8]) -> Result<IcoImage, IcoError> {
    if png::is_png(blob) {
        let d = png::decode(blob).map_err(|err| IcoError::Decode(err.to_string()))?;
        return Ok(IcoImage { width: d.width, height: d.height, bpp: 32, pixels: d.pixels });
    }
    let (w, h, pixels) = decode_bmp_entry(blob)?;
    Ok(IcoImage { width: w, height: h, bpp: e.bpp, pixels })
}

// ── Shared DIB parsing (also used by bmp.rs) ────────────────────

/// Parsed BMP DIB: info header plus palette. Pixel data starts at
/// `row_data`. Errors are plain strings; callers map them.
#[derive(Debug, Clone)]
pub(crate) struct BmpDib<'a> {
    pub width: u32,
    /// Stored height, signed as in the file (negative = top-down).
    /// ICO entries stack XOR + AND rows; standalone BMPs store pixels.
    pub height: i32,
    pub planes: u16,
    pub bpp: u16,
    pub compression: u32,
    pub palette: Vec<[u8; 3]>,
    pub row_data: &'a [u8],
}

/// Parse any BMP DIB: BITMAPCOREHEADER (12), BITMAPINFOHEADER (40)
/// and V4/V5 (first 40 fields). No validation of planes, depth or
/// compression — the caller decides what it supports.
pub(crate) fn parse_bmp_dib(blob: &[u8]) -> Result<BmpDib<'_>, String> {
    if blob.len() < 4 {
        return Err("truncated BMP header".into());
    }
    let header_size = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]);
    let (width, height_signed, planes, bpp, compression, palette_len, row_data_off) =
        match header_size {
            12 => {
                if blob.len() < 12 {
                    return Err("truncated BMP core header".into());
                }
                let w = u16::from_le_bytes([blob[4], blob[5]]) as u32;
                let h = u16::from_le_bytes([blob[6], blob[7]]) as u32;
                let bpp = u16::from_le_bytes([blob[10], blob[11]]);
                let pal = if bpp <= 8 { 1u32 << bpp } else { 0 };
                (w, h as i32, 1u16, bpp, 0u32, pal, 12usize)
            }
            40 | 108 | 124 => {
                if blob.len() < 40 {
                    return Err("truncated BMP info header".into());
                }
                let w = i32::from_le_bytes([blob[4], blob[5], blob[6], blob[7]]);
                let h = i32::from_le_bytes([blob[8], blob[9], blob[10], blob[11]]);
                let planes = u16::from_le_bytes([blob[12], blob[13]]);
                let bpp = u16::from_le_bytes([blob[14], blob[15]]);
                let comp = u32::from_le_bytes([blob[16], blob[17], blob[18], blob[19]]);
                if w <= 0 {
                    return Err("invalid BMP width".into());
                }
                let pal = if bpp <= 8 {
                    let n = u32::from_le_bytes([blob[32], blob[33], blob[34], blob[35]]);
                    if n == 0 || n > (1 << bpp.min(8)) {
                        1 << bpp.min(8)
                    } else {
                        n
                    }
                } else {
                    0
                };
                (w as u32, h, planes, bpp, comp, pal, header_size as usize)
            }
            _ => return Err("unknown BMP header size".into()),
        };
    let entry_bytes = if header_size == 12 { 3 } else { 4 };
    let palette: Vec<[u8; 3]> = if bpp <= 8 {
        let need = row_data_off + palette_len as usize * entry_bytes;
        if blob.len() < need {
            return Err("truncated BMP palette".into());
        }
        (0..palette_len)
            .map(|i| {
                let o = row_data_off + i as usize * entry_bytes;
                // RGBQUAD/RGBTRIPLE store BGR(x).
                [blob[o + 2], blob[o + 1], blob[o]]
            })
            .collect()
    } else {
        Vec::new()
    };
    // Pixel data starts after headers AND palette.
    let row_data_off = row_data_off + palette.len() * entry_bytes;
    let row_data = blob.get(row_data_off..).ok_or("truncated BMP pixels")?;
    Ok(BmpDib { width, height: height_signed, planes, bpp, compression, palette, row_data })
}

/// Decode a BMP-without-file-header ICO entry to `(width, height, RGBA8)`.
///
/// BI_RGB depths 1/4/8 (palette), 24 (BGR) and 32 (BGRA), bottom-up or
/// top-down XOR plus the trailing 1-bit AND mask (1 = transparent).
pub(crate) fn decode_bmp_entry(blob: &[u8]) -> Result<(u32, u32, Vec<u8>), IcoError> {
    let dib = parse_bmp_dib(blob).map_err(IcoError::Decode)?;
    let width = dib.width;
    let bpp = dib.bpp;
    let palette = dib.palette;
    if dib.planes != 1 {
        return Err(IcoError::Decode("invalid BMP planes".into()));
    }
    if !matches!(bpp, 1 | 4 | 8 | 24 | 32) {
        return Err(IcoError::Decode(format!("bpp {bpp} unsupported")));
    }
    if dib.compression != 0 {
        return Err(IcoError::Unsupported(format!(
            "compressed BMP ({}) unsupported",
            dib.compression
        )));
    }
    // Visible height is half the BMP height (XOR + AND); negative XOR
    // height means top-down storage.
    let (abs_h, top_down) = if dib.height < 0 {
        (-dib.height as u32, true)
    } else {
        (dib.height as u32, false)
    };
    if abs_h == 0 || abs_h % 2 != 0 {
        return Err(IcoError::Decode("invalid BMP height".into()));
    }
    let height = abs_h / 2;
    if width == 0 || width > 1024 || height == 0 || height > 1024 {
        return Err(IcoError::Decode("invalid BMP dimensions".into()));
    }

    let data = dib.row_data;
    let xor_stride = xor_stride(width, bpp);
    let xor_len = xor_stride * height as usize;
    if data.len() < xor_len {
        return Err(IcoError::Decode("truncated BMP pixels".into()));
    }
    // AND mask: DWORD-padded rows per spec, but writers in the wild
    // (e.g. Pillow) emit tightly packed rows. Accept padded first,
    // then packed; a missing mask means fully opaque (lenient).
    let and_stride_padded: usize = ((width as usize + 31) / 32) * 4;
    let and_stride_packed: usize = (width as usize + 7) / 8;
    let and_avail = data.len() - xor_len;
    let (and_mask, and_stride): (Option<&[u8]>, usize) =
        if and_avail >= and_stride_padded * height as usize {
            (
                Some(&data[xor_len..xor_len + and_stride_padded * height as usize]),
                and_stride_padded,
            )
        } else if and_stride_packed != and_stride_padded
            && and_avail >= and_stride_packed * height as usize
        {
            (
                Some(&data[xor_len..xor_len + and_stride_packed * height as usize]),
                and_stride_packed,
            )
        } else {
            (None, and_stride_padded)
        };
    let xor = &data[..xor_len];

    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        // XOR rows are bottom-up unless top-down; AND rows are bottom-up.
        let xor_row = if top_down { y } else { height - 1 - y } as usize;
        let and_row = (height - 1 - y) as usize;
        let row = &xor[xor_row * xor_stride..(xor_row + 1) * xor_stride];
        for x in 0..width {
            let (r, g, b, a) =
                xor_pixel(row, x as usize, bpp, &palette).map_err(IcoError::Decode)?;
            let mut alpha = a;
            if let Some(mask) = and_mask {
                let byte = mask[and_row * and_stride + x as usize / 8];
                if (byte >> (7u32 - (x % 8u32))) & 1 == 1 {
                    alpha = 0;
                }
            }
            let o = (y as usize * width as usize + x as usize) * 4;
            pixels[o..o + 4].copy_from_slice(&[r, g, b, alpha]);
        }
    }
    Ok((width, height, pixels))
}

/// XOR row stride (32-bit aligned). Shared with bmp.rs.
pub(crate) fn xor_stride(width: u32, bpp: u16) -> usize {
    ((width as usize * bpp as usize + 31) / 32) * 4
}

/// One XOR pixel to RGBA. Shared with bmp.rs; errors are plain strings.
pub(crate) fn xor_pixel(
    row: &[u8],
    x: usize,
    bpp: u16,
    palette: &[[u8; 3]],
) -> Result<(u8, u8, u8, u8), String> {
    match bpp {
        1 | 4 | 8 => {
            let idx = match bpp {
                1 => (row[x / 8] >> (7 - (x % 8))) & 1,
                4 => {
                    let byte = row[x / 2];
                    if x % 2 == 0 { byte >> 4 } else { byte & 15 }
                }
                _ => row[x],
            } as usize;
            let entry = *palette
                .get(idx)
                .ok_or_else(|| "palette index out of range".to_string())?;
            Ok((entry[0], entry[1], entry[2], 255))
        }
        24 => {
            let o = x * 3;
            if o + 3 > row.len() {
                return Err("scanline overrun".into());
            }
            Ok((row[o + 2], row[o + 1], row[o], 255))
        }
        _ => {
            let o = x * 4;
            if o + 4 > row.len() {
                return Err("scanline overrun".into());
            }
            Ok((row[o + 2], row[o + 1], row[o], row[o + 3]))
        }
    }
}
