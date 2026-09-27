//! Pure-Rust JPEG codec (sequential baseline, no third-party dependencies).
//!
//! Decode supports sequential Huffman JPEG (SOF0/SOF1, 8-bit): grayscale
//! and YCbCr with sampling factors 1-4, standard or custom quantization
//! and Huffman tables, restart markers (DRI/RSTn), multi-scan sequential
//! images and all APPn/COM segments (EXIF APP1 is skipped here;
//! `crate::io` reads it separately with `kamadak-exif`).
//!
//! Progressive (SOF2), arithmetic coding (DAC), 12-bit precision and
//! CMYK are detected and reported as `Unsupported` with a clear message.
//!
//! Encode writes sequential baseline JPEG with libjpeg-compatible
//! quality scaling (1-100), 4:2:0 or 4:4:4 sampling and a grayscale
//! fast path.

use std::collections::HashMap;

// ── Public API ────────────────────────────────────────────────

/// Errors of the JPEG codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JpegError {
    Decode(String),
    Encode(String),
    Unsupported(String),
}

impl std::fmt::Display for JpegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "jpeg decode error: {m}"),
            Self::Encode(m) => write!(f, "jpeg encode error: {m}"),
            Self::Unsupported(m) => write!(f, "jpeg unsupported: {m}"),
        }
    }
}

impl std::error::Error for JpegError {}

/// Decoded JPEG image: always RGBA8 pixels (alpha 255, JPEG has none).
#[derive(Debug, Clone)]
pub struct DecodedJpeg {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Chroma sampling for [`encode_with_sampling`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegSampling {
    /// No subsampling (largest, sharpest chroma).
    Yuv444,
    /// Half chroma resolution (default, like cameras).
    Yuv420,
}

/// True when `bytes` starts with the SOI marker (FFD8).
pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8
}

/// Fast SOF probe: `(width, height)` without decoding pixels.
/// Works for any SOFn, including progressive (which [`decode`] rejects).
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), JpegError> {
    Parser::new(bytes)?.probe_dimensions()
}

/// Decode a sequential baseline JPEG into RGBA8 pixels.
pub fn decode(bytes: &[u8]) -> Result<DecodedJpeg, JpegError> {
    Decoder::new(bytes)?.decode()
}

/// Encode RGBA8 pixels as baseline JPEG, 4:2:0 sampling, `quality` 1-100.
pub fn encode(width: u32, height: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, JpegError> {
    encode_with_sampling(width, height, rgba, quality, JpegSampling::Yuv420)
}

/// Encode RGBA8 pixels with explicit chroma sampling, `quality` 1-100.
pub fn encode_with_sampling(
    width: u32,
    height: u32,
    rgba: &[u8],
    quality: u8,
    sampling: JpegSampling,
) -> Result<Vec<u8>, JpegError> {
    check_image(width, height, rgba.len(), 4).map_err(JpegError::Encode)?;
    check_quality(quality).map_err(JpegError::Encode)?;
    let (y, cb, cr) = rgb_to_ycbcr_planes(width, height, rgba, sampling);
    Encoder::encode_planes(width, height, &y, Some((&cb, &cr)), quality, sampling)
}

/// Encode one gray byte per pixel as a single-component JPEG, `quality` 1-100.
pub fn encode_grayscale(
    width: u32,
    height: u32,
    gray: &[u8],
    quality: u8,
) -> Result<Vec<u8>, JpegError> {
    check_image(width, height, gray.len(), 1).map_err(JpegError::Encode)?;
    check_quality(quality).map_err(JpegError::Encode)?;
    Encoder::encode_planes(width, height, gray, None, quality, JpegSampling::Yuv444)
}

fn check_image(width: u32, height: u32, len: usize, comps: usize) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("width and height must be > 0".into());
    }
    if len != width as usize * height as usize * comps {
        return Err("pixel buffer length mismatch".into());
    }
    if width > 100_000 || height > 100_000 {
        return Err("image too large".into());
    }
    Ok(())
}

fn check_quality(quality: u8) -> Result<(), String> {
    if quality == 0 || quality > 100 {
        return Err("quality must be 1..=100".into());
    }
    Ok(())
}

// ── Shared tables ────────────────────────────────────────────

/// Zigzag order: ZIG[z] is the natural coefficient index at zigzag position z.
const ZIG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40,
    48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61,
    54, 47, 55, 62, 63,
];

/// Annex K.1 luminance quantization base table (natural order).
const Q_LUM: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13,
    16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56,
    68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103,
    121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

/// Annex K.2 chrominance quantization base table (natural order).
const Q_CHROMA: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26,
    56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// Standard Huffman tables: (code-length counts[16], symbols).
const STD_LUM_DC: ([u8; 16], &[u8]) = (
    [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
);
const STD_CHROMA_DC: ([u8; 16], &[u8]) = (
    [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
);
const STD_LUM_AC: ([u8; 16], &[u8]) = (
    [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d],
    &[
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41,
        0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91,
        0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24,
        0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a,
        0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38,
        0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53,
        0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66,
        0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79,
        0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93,
        0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
        0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7,
        0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9,
        0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1,
        0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2,
        0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
);
const STD_CHROMA_AC: ([u8; 16], &[u8]) = (
    [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77],
    &[
        0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12,
        0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14,
        0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15,
        0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17,
        0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a,
        0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65,
        0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78,
        0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a,
        0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3,
        0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5,
        0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9,
        0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2,
        0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
);

/// libjpeg-compatible quality scaling of a base quantization table.
fn scale_quant_table(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = quality.clamp(1, 100) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - q * 2 };
    let mut out = [0u16; 64];
    for (i, &b) in base.iter().enumerate() {
        out[i] = ((b as u32 * scale + 50) / 100).clamp(1, 255) as u16;
    }
    out
}

/// Canonical (code, len) per symbol from (counts[16], symbols).
fn table_codes(counts: &[u8; 16], symbols: &[u8]) -> ([(u32, u8); 256], Vec<u8>) {
    let mut codes = [(0u32, 0u8); 256];
    let mut lengths = vec![0u8; 256];
    let mut code = 0u32;
    let mut k = 0usize;
    for (len, &count) in counts.iter().enumerate() {
        let len = (len + 1) as u8;
        for _ in 0..count {
            if k >= symbols.len() {
                break;
            }
            let sym = symbols[k] as usize;
            k += 1;
            codes[sym] = (code, len);
            lengths[sym] = len;
            code += 1;
        }
        code <<= 1;
    }
    (codes, lengths)
}

/// Binary Huffman decode tree (JPEG codes are MSB-first in the stream).
struct HuffTree {
    child: Vec<[i32; 2]>,
    sym: Vec<i32>,
}

impl HuffTree {
    fn build(counts: &[u8; 16], symbols: &[u8]) -> Result<Self, JpegError> {
        if symbols.len() > 256 || symbols.is_empty() {
            return Err(JpegError::Decode("invalid Huffman table".into()));
        }
        let (codes, lengths) = table_codes(counts, symbols);
        let mut child = vec![[-1i32, -1]];
        let mut sym = vec![-1i32];
        for s in 0..256 {
            let len = lengths[s];
            if len == 0 {
                continue;
            }
            let mut node = 0usize;
            for i in (0..len).rev() {
                let bit = ((codes[s].0 >> i) & 1) as usize;
                let next = child[node][bit];
                let next = if next < 0 {
                    child.push([-1, -1]);
                    sym.push(-1);
                    let id = (child.len() - 1) as i32;
                    child[node][bit] = id;
                    id
                } else {
                    next
                };
                node = next as usize;
            }
            if sym[node] >= 0 {
                return Err(JpegError::Decode("over-subscribed Huffman code".into()));
            }
            sym[node] = s as i32;
        }
        Ok(Self { child, sym })
    }

    fn decode(&self, br: &mut ScanReader) -> Result<u8, JpegError> {
        let mut node = 0usize;
        loop {
            let bit = br.get_bits(1)? as usize;
            let next = self.child[node][bit];
            if next < 0 {
                return Err(JpegError::Decode("invalid Huffman code".into()));
            }
            node = next as usize;
            if self.sym[node] >= 0 && self.child[node] == [-1, -1] {
                return Ok(self.sym[node] as u8);
            }
        }
    }
}

// ── Marker parser ─────────────────────────────────────────────

struct Component {
    id: u8,
    h: u32,
    v: u32,
    tq: usize,
}

struct Frame {
    width: u32,
    height: u32,
    comps: Vec<Component>,
    h_max: u32,
    v_max: u32,
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    qtables: [Option<[u16; 64]>; 4],
    dtables: HashMap<(bool, usize), HuffTree>, // (is_ac, id)
    frame: Option<Frame>,
    restart_interval: u16,
    scans_done: usize,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Result<Self, JpegError> {
        if !is_jpeg(data) {
            return Err(JpegError::Decode("not a JPEG file".into()));
        }
        Ok(Self {
            data,
            pos: 2,
            qtables: [None, None, None, None],
            dtables: HashMap::new(),
            frame: None,
            restart_interval: 0,
            scans_done: 0,
        })
    }

    fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn read_u16(&mut self) -> Result<u16, JpegError> {
        if self.pos + 2 > self.data.len() {
            return Err(JpegError::Decode("truncated segment".into()));
        }
        let v = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    /// Read the next marker, skipping fill bytes. Returns None on EOF.
    fn next_marker(&mut self) -> Result<Option<u8>, JpegError> {
        loop {
            if self.eof() {
                return Ok(None);
            }
            let b = self.data[self.pos];
            self.pos += 1;
            if b != 0xFF {
                return Err(JpegError::Decode("expected marker".into()));
            }
            if self.eof() {
                return Err(JpegError::Decode("truncated marker".into()));
            }
            let m = self.data[self.pos];
            self.pos += 1;
            if m == 0xFF {
                // Fill byte: step back one so the next 0xFF is re-examined.
                self.pos -= 1;
                continue;
            }
            if m == 0x00 {
                return Err(JpegError::Decode("unexpected stuffed zero".into()));
            }
            return Ok(Some(m));
        }
    }

    fn segment(&mut self) -> Result<&'a [u8], JpegError> {
        let len = self.read_u16()? as usize;
        if len < 2 || self.pos + len - 2 > self.data.len() {
            return Err(JpegError::Decode("truncated segment".into()));
        }
        let s = &self.data[self.pos..self.pos + len - 2];
        self.pos += len - 2;
        Ok(s)
    }

    /// Parse headers up to the first SOS/EOI. Used by [`dimensions`].
    fn probe_dimensions(&mut self) -> Result<(u32, u32), JpegError> {
        loop {
            match self.next_marker()? {
                None => return Err(JpegError::Decode("no image data".into())),
                Some(0xD9) => return Err(JpegError::Decode("no image data".into())),
                Some(m @ 0xC0..=0xCF)
                    if m != 0xC4 && m != 0xC8 && m != 0xCC =>
                {
                    let seg = self.segment()?;
                    if seg.len() < 6 {
                        return Err(JpegError::Decode("truncated SOF".into()));
                    }
                    let h = u16::from_be_bytes([seg[1], seg[2]]) as u32;
                    let w = u16::from_be_bytes([seg[3], seg[4]]) as u32;
                    if w == 0 || h == 0 {
                        return Err(JpegError::Decode("zero image dimension".into()));
                    }
                    return Ok((w, h));
                }
                Some(0xDA) => return Err(JpegError::Decode("SOS before SOF".into())),
                Some(_) => {
                    let _ = self.segment()?;
                }
            }
        }
    }
}

// ── Decoder ───────────────────────────────────────────────────

struct Decoder<'a> {
    p: Parser<'a>,
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Result<Self, JpegError> {
        Ok(Self { p: Parser::new(data)? })
    }

    fn decode(&mut self) -> Result<DecodedJpeg, JpegError> {
        // Coefficient buffers per component (filled across scans).
        let mut coeffs: Vec<Vec<i16>> = Vec::new();
        loop {
            let marker = match self.p.next_marker()? {
                None => break, // Lenient: missing EOI after complete scans.
                Some(m) => m,
            };
            match marker {
                0xD9 => break,
                0xC0 | 0xC1 => {
                    let seg = self.p.segment()?;
                    self.read_sof(seg)?;
                    coeffs = self.alloc_coeffs();
                }
                0xC2 => return Err(JpegError::Unsupported("progressive JPEG (SOF2) unsupported".into())),
                0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF => {
                    return Err(JpegError::Unsupported("lossless/arithmetic/hierarchical JPEG unsupported".into()));
                }
                0xC4 => {
                    let seg = self.p.segment()?;
                    self.read_dht(seg)?;
                }
                0xCC => return Err(JpegError::Unsupported("arithmetic coded JPEG (DAC) unsupported".into())),
                0xDB => {
                    let seg = self.p.segment()?;
                    self.read_dqt(seg)?;
                }
                0xDD => {
                    let seg = self.p.segment()?;
                    if seg.len() != 2 {
                        return Err(JpegError::Decode("invalid DRI".into()));
                    }
                    self.p.restart_interval = u16::from_be_bytes([seg[0], seg[1]]);
                }
                0xDA => {
                    if self.p.frame.is_none() {
                        return Err(JpegError::Decode("SOS before SOF".into()));
                    }
                    let seg = self.p.segment()?;
                    self.read_scan(seg, &mut coeffs)?;
                    self.p.scans_done += 1;
                }
                0xD8 => return Err(JpegError::Decode("unexpected SOI".into())),
                0xD0..=0xD7 => {
                    return Err(JpegError::Decode("unexpected restart marker".into()))
                }
                0x01 => {} // TEM has no payload.
                0xDC => {
                    // DNL: skip (number of lines follows in later scans).
                    let _ = self.p.segment()?;
                }
                0xE0..=0xEF | 0xFE => {
                    let _ = self.p.segment()?;
                } // APPn + COM skipped (EXIF handled by crate::io)
                _ => return Err(JpegError::Decode("unknown marker".into())),
            }
        }
        if self.p.frame.is_none() || self.p.scans_done == 0 {
            return Err(JpegError::Decode("no image data".into()));
        }
        self.render(&coeffs)
    }

    fn read_sof(&mut self, seg: &[u8]) -> Result<(), JpegError> {
        if seg.len() < 6 {
            return Err(JpegError::Decode("truncated SOF".into()));
        }
        let precision = seg[0];
        if precision != 8 {
            return Err(JpegError::Unsupported(format!("{precision}-bit JPEG unsupported")));
        }
        let height = u16::from_be_bytes([seg[1], seg[2]]) as u32;
        let width = u16::from_be_bytes([seg[3], seg[4]]) as u32;
        if width == 0 || height == 0 {
            return Err(JpegError::Decode("zero image dimension".into()));
        }
        if width > 100_000 || height > 100_000 {
            return Err(JpegError::Decode("image too large".into()));
        }
        let nf = seg[5] as usize;
        if nf != 1 && nf != 3 {
            if nf == 4 {
                return Err(JpegError::Unsupported("CMYK JPEG unsupported".into()));
            }
            return Err(JpegError::Unsupported(format!("{nf}-component JPEG unsupported")));
        }
        if seg.len() != 6 + 3 * nf {
            return Err(JpegError::Decode("truncated SOF".into()));
        }
        let mut comps = Vec::with_capacity(nf);
        let mut h_max = 0u32;
        let mut v_max = 0u32;
        for i in 0..nf {
            let id = seg[6 + 3 * i];
            let h = (seg[7 + 3 * i] >> 4) as u32;
            let v = (seg[7 + 3 * i] & 15) as u32;
            let tq = seg[8 + 3 * i] as usize;
            if h == 0 || h > 4 || v == 0 || v > 4 {
                return Err(JpegError::Unsupported("sampling factor out of range".into()));
            }
            if tq > 3 {
                return Err(JpegError::Decode("invalid quant table selector".into()));
            }
            h_max = h_max.max(h);
            v_max = v_max.max(v);
            comps.push(Component { id, h, v, tq });
        }
        self.p.frame = Some(Frame { width, height, comps, h_max, v_max });
        Ok(())
    }

    fn read_dqt(&mut self, seg: &[u8]) -> Result<(), JpegError> {
        let mut pos = 0usize;
        while pos < seg.len() {
            let info = seg[pos];
            pos += 1;
            let pq = (info >> 4) as usize;
            let tq = (info & 15) as usize;
            if tq > 3 {
                return Err(JpegError::Decode("invalid quant table selector".into()));
            }
            let mut table = [0u16; 64];
            if pq == 0 {
                if pos + 64 > seg.len() {
                    return Err(JpegError::Decode("truncated DQT".into()));
                }
                for z in 0..64 {
                    table[ZIG[z]] = seg[pos + z] as u16;
                }
                pos += 64;
            } else if pq == 1 {
                if pos + 128 > seg.len() {
                    return Err(JpegError::Decode("truncated DQT".into()));
                }
                for z in 0..64 {
                    table[ZIG[z]] =
                        u16::from_be_bytes([seg[pos + 2 * z], seg[pos + 2 * z + 1]]);
                }
                pos += 128;
            } else {
                return Err(JpegError::Decode("invalid DQT precision".into()));
            }
            if table.iter().any(|&v| v == 0) {
                return Err(JpegError::Decode("zero quant value".into()));
            }
            self.p.qtables[tq] = Some(table);
        }
        Ok(())
    }

    fn read_dht(&mut self, seg: &[u8]) -> Result<(), JpegError> {
        let mut pos = 0usize;
        while pos < seg.len() {
            let info = seg[pos];
            pos += 1;
            let class = info >> 4; // 0 = DC, 1 = AC
            let id = (info & 15) as usize;
            if class > 1 || id > 3 {
                return Err(JpegError::Decode("invalid Huffman table selector".into()));
            }
            if pos + 16 > seg.len() {
                return Err(JpegError::Decode("truncated DHT".into()));
            }
            let mut counts = [0u8; 16];
            counts.copy_from_slice(&seg[pos..pos + 16]);
            pos += 16;
            let total: usize = counts.iter().map(|&c| c as usize).sum();
            if total == 0 || total > 256 || pos + total > seg.len() {
                return Err(JpegError::Decode("invalid DHT counts".into()));
            }
            let symbols = seg[pos..pos + total].to_vec();
            pos += total;
            let tree = HuffTree::build(&counts, &symbols)?;
            self.p.dtables.insert((class == 1, id), tree);
        }
        Ok(())
    }

    fn alloc_coeffs(&self) -> Vec<Vec<i16>> {
        let frame = self.p.frame.as_ref().expect("frame");
        frame
            .comps
            .iter()
            .map(|c| {
                let bw = blocks_w(frame.width, frame.h_max, c.h);
                let bh = blocks_h(frame.height, frame.v_max, c.v);
                vec![0i16; bw as usize * bh as usize * 64]
            })
            .collect()
    }

    fn read_scan(&mut self, seg: &[u8], coeffs: &mut [Vec<i16>]) -> Result<(), JpegError> {
        if seg.len() < 4 {
            return Err(JpegError::Decode("truncated SOS".into()));
        }
        let frame = self.p.frame.as_ref().expect("frame");
        let ns = seg[0] as usize;
        if ns == 0 || ns > frame.comps.len() {
            return Err(JpegError::Decode("invalid scan component count".into()));
        }
        if seg.len() != 4 + 2 * ns {
            return Err(JpegError::Decode("truncated SOS".into()));
        }
        struct ScanComp {
            idx: usize,
            td: usize,
            ta: usize,
        }
        let mut scan_comps = Vec::with_capacity(ns);
        let mut mcu_blocks = 0u32;
        for i in 0..ns {
            let cs = seg[1 + 2 * i];
            let td = (seg[2 + 2 * i] >> 4) as usize;
            let ta = (seg[2 + 2 * i] & 15) as usize;
            let idx = frame
                .comps
                .iter()
                .position(|c| c.id == cs)
                .ok_or_else(|| JpegError::Decode("unknown scan component".into()))?;
            if !self.p.dtables.contains_key(&(false, td))
                || !self.p.dtables.contains_key(&(true, ta))
            {
                return Err(JpegError::Decode("missing Huffman table".into()));
            }
            mcu_blocks += frame.comps[idx].h * frame.comps[idx].v;
            scan_comps.push(ScanComp { idx, td, ta });
        }
        if mcu_blocks > 10 {
            return Err(JpegError::Decode("too many blocks per MCU".into()));
        }
        let ss = seg[1 + 2 * ns];
        let se = seg[2 + 2 * ns];
        let ah_al = seg[3 + 2 * ns];
        if ss != 0 || se != 63 || ah_al != 0 {
            return Err(JpegError::Unsupported(
                "progressive/successive JPEG scan unsupported".into(),
            ));
        }
        for sc in &scan_comps {
            if self.p.qtables[frame.comps[sc.idx].tq].is_none() {
                return Err(JpegError::Decode("missing quant table".into()));
            }
        }

        // MCU grid over the whole image.
        let mcu_w = 8 * frame.h_max;
        let mcu_h = 8 * frame.v_max;
        let mcus_x = (frame.width + mcu_w - 1) / mcu_w;
        let mcus_y = (frame.height + mcu_h - 1) / mcu_h;

        let mut sr = ScanReader::new(self.p.data, self.p.pos);
        let mut dc_pred = vec![0i32; frame.comps.len()];
        let mut mcu_count = 0u32;
        let mut rst_expected = 0u8;
        let ri = self.p.restart_interval;

        for my in 0..mcus_y {
            for mx in 0..mcus_x {
                for sc in &scan_comps {
                    let comp = &frame.comps[sc.idx];
                    let bw = blocks_w(frame.width, frame.h_max, comp.h);
                    for vy in 0..comp.v {
                        for hx in 0..comp.h {
                            let bx = mx * comp.h + hx;
                            let by = my * comp.v + vy;
                            if bx >= bw {
                                continue;
                            }
                            let bh = blocks_h(frame.height, frame.v_max, comp.v);
                            if by >= bh {
                                continue;
                            }
                            let base = (by as usize * bw as usize + bx as usize) * 64;
                            let block = &mut coeffs[sc.idx][base..base + 64];
                            decode_block(
                                &mut sr,
                                &self.p.dtables[&(false, sc.td)],
                                &self.p.dtables[&(true, sc.ta)],
                                block,
                                &mut dc_pred[sc.idx],
                            )?;
                        }
                    }
                }
                mcu_count += 1;
                if ri > 0 && mcu_count % ri as u32 == 0 {
                    let is_last = mx + 1 == mcus_x && my + 1 == mcus_y;
                    if is_last {
                        // Trailing RST after the final interval is optional
                        // in the wild: consume when present, accept EOI too.
                        sr.try_consume_restart(rst_expected);
                    } else {
                        sr.expect_restart(rst_expected)?;
                        rst_expected = (rst_expected + 1) % 8;
                        dc_pred.fill(0);
                    }
                }
            }
        }
        // Trailing restart markers at the exact end carry no data; consume
        // them when present so the header loop resumes at the next marker.
        sr.align()?;
        self.p.pos = sr.pos;
        Ok(())
    }

    fn render(&self, coeffs: &[Vec<i16>]) -> Result<DecodedJpeg, JpegError> {
        let frame = self.p.frame.as_ref().expect("frame");
        // IDCT each component into its own plane.
        let mut planes: Vec<(u32, u32, Vec<f32>)> = Vec::new();
        for (ci, comp) in frame.comps.iter().enumerate() {
            let bw = blocks_w(frame.width, frame.h_max, comp.h);
            let bh = blocks_h(frame.height, frame.v_max, comp.v);
            let qt = self.p.qtables[comp.tq]
                .ok_or_else(|| JpegError::Decode("missing quant table".into()))?;
            let mut plane = vec![0f32; bw as usize * 8 * bh as usize * 8];
            let stride = bw as usize * 8;
            for by in 0..bh {
                for bx in 0..bw {
                    let base = (by as usize * bw as usize + bx as usize) * 64;
                    let mut deq = [0f32; 64];
                    for (i, &c) in coeffs[ci][base..base + 64].iter().enumerate() {
                        deq[i] = c as f32 * qt[i] as f32;
                    }
                    let px = idct8(&deq);
                    for y in 0..8 {
                        for x in 0..8 {
                            plane[(by as usize * 8 + y) * stride + bx as usize * 8 + x] =
                                px[y * 8 + x] + 128.0;
                        }
                    }
                }
            }
            planes.push((bw * 8, bh * 8, plane));
        }

        let w = frame.width as usize;
        let h = frame.height as usize;
        let mut pixels = vec![0u8; w * h * 4];
        if frame.comps.len() == 1 {
            let (pw, ph, plane) = &planes[0];
            for y in 0..h {
                for x in 0..w {
                    let g = bilinear(plane, *pw, *ph, w, h, x, y);
                    let o = (y * w + x) * 4;
                    let v = g.round().clamp(0.0, 255.0) as u8;
                    pixels[o..o + 4].copy_from_slice(&[v, v, v, 255]);
                }
            }
        } else {
            let (yw, yh, yp) = &planes[0];
            let (cbw, cbh, cbp) = &planes[1];
            let (crw, crh, crp) = &planes[2];
            for y in 0..h {
                for x in 0..w {
                    let yy = bilinear(yp, *yw, *yh, w, h, x, y);
                    let cb = bilinear(cbp, *cbw, *cbh, w, h, x, y) - 128.0;
                    let cr = bilinear(crp, *crw, *crh, w, h, x, y) - 128.0;
                    let (r, g, b) = ycbcr_to_rgb(yy, cb, cr);
                    let o = (y * w + x) * 4;
                    pixels[o..o + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
        Ok(DecodedJpeg { width: frame.width, height: frame.height, pixels })
    }
}

fn blocks_w(width: u32, h_max: u32, h: u32) -> u32 {
    (width * h + 8 * h_max - 1) / (8 * h_max)
}

fn blocks_h(height: u32, v_max: u32, v: u32) -> u32 {
    (height * v + 8 * v_max - 1) / (8 * v_max)
}

// ── Scan bit reader (MSB-first, stuffed zeros, restart aware) ─

struct ScanReader<'a> {
    data: &'a [u8],
    pos: usize, // next unread byte
    buf: u32,   // pending bits, MSB-aligned
    nbits: u32,
}

impl<'a> ScanReader<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos, buf: 0, nbits: 0 }
    }

    fn fill(&mut self) -> Result<(), JpegError> {
        if self.pos >= self.data.len() {
            return Err(JpegError::Decode("truncated scan data".into()));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        if b == 0xFF {
            if self.pos >= self.data.len() {
                return Err(JpegError::Decode("truncated scan data".into()));
            }
            let m = self.data[self.pos];
            self.pos += 1;
            match m {
                0x00 => {
                    self.buf = (self.buf << 8) | 0xFF;
                    self.nbits += 8;
                    Ok(())
                }
                _ => Err(JpegError::Decode(format!("unexpected marker FF{m:02X} in scan"))),
            }
        } else {
            self.buf = (self.buf << 8) | b as u32;
            self.nbits += 8;
            Ok(())
        }
    }

    fn get_bits(&mut self, n: u32) -> Result<u32, JpegError> {
        if n == 0 || n > 16 {
            return Err(JpegError::Decode("bad bit count".into()));
        }
        while self.nbits < n {
            self.fill()?;
        }
        self.nbits -= n;
        Ok((self.buf >> self.nbits) & ((1 << n) - 1))
    }

    /// Drop pad bits so the header loop resumes at a marker.
    fn align(&mut self) -> Result<(), JpegError> {
        self.nbits = 0;
        Ok(())
    }

    /// After an MCU boundary with restarts enabled: consume RSTn.
    fn expect_restart(&mut self, expected: u8) -> Result<(), JpegError> {
        self.nbits = 0;
        if self.pos + 1 >= self.data.len() {
            return Err(JpegError::Decode("missing restart marker".into()));
        }
        if self.data[self.pos] != 0xFF || self.data[self.pos + 1] != 0xD0 + expected {
            return Err(JpegError::Decode("missing restart marker".into()));
        }
        self.pos += 2;
        Ok(())
    }

    /// Optional trailing restart after the final interval: consume when
    /// present, otherwise leave the position for the header loop.
    fn try_consume_restart(&mut self, expected: u8) {
        self.nbits = 0;
        if self.pos + 1 < self.data.len()
            && self.data[self.pos] == 0xFF
            && self.data[self.pos + 1] == 0xD0 + expected
        {
            self.pos += 2;
        }
    }
}

fn extend_extra(v: u32, size: u32) -> i32 {
    if size == 0 {
        0
    } else if v < (1 << (size - 1)) {
        v as i32 + ((-1i32) << size) + 1
    } else {
        v as i32
    }
}

fn decode_block(
    sr: &mut ScanReader,
    dc_tree: &HuffTree,
    ac_tree: &HuffTree,
    block: &mut [i16],
    dc_pred: &mut i32,
) -> Result<(), JpegError> {
    block.fill(0);
    let cat = dc_tree.decode(sr)? as u32;
    if cat > 11 {
        return Err(JpegError::Decode("invalid DC category".into()));
    }
    let diff = if cat == 0 { 0 } else { extend_extra(sr.get_bits(cat)?, cat) };
    *dc_pred = dc_pred.wrapping_add(diff);
    if *dc_pred < -2048 || *dc_pred > 2047 {
        return Err(JpegError::Decode("DC out of range".into()));
    }
    block[0] = *dc_pred as i16;
    let mut k = 1usize;
    while k < 64 {
        let rs = ac_tree.decode(sr)?;
        let run = (rs >> 4) as usize;
        let size = (rs & 15) as u32;
        if size == 0 {
            if run == 15 {
                k += 16;
                if k > 64 {
                    return Err(JpegError::Decode("bad zero run".into()));
                }
            } else {
                break; // EOB
            }
        } else {
            if size > 10 || k + run >= 64 {
                return Err(JpegError::Decode("invalid AC symbol".into()));
            }
            k += run;
            block[ZIG[k]] = extend_extra(sr.get_bits(size)?, size) as i16;
            k += 1;
        }
    }
    Ok(())
}

// ── IDCT ──────────────────────────────────────────────────────

/// 8x8 inverse DCT (float, AAN-free direct form). Input dequantized.
fn idct8(coef: &[f32; 64]) -> [f32; 64] {
    // M[u][x] = C(u) * cos((2x+1)*u*pi/16) / 2, folded 0.25 across both passes.
    let mut m = [[0f32; 8]; 8];
    for u in 0..8 {
        let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
        for x in 0..8 {
            m[u][x] = cu
                * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos()
                / 2.0;
        }
    }
    let mut tmp = [[0f32; 8]; 8];
    for y in 0..8 {
        for x in 0..8 {
            let mut s = 0f32;
            for u in 0..8 {
                s += m[u][x] * coef[u * 8 + y];
            }
            tmp[y][x] = s;
        }
    }
    let mut out = [0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut s = 0f32;
            for v in 0..8 {
                s += m[v][y] * tmp[v][x];
            }
            out[y * 8 + x] = s;
        }
    }
    out
}

// ── Color ─────────────────────────────────────────────────────

fn ycbcr_to_rgb(y: f32, cb: f32, cr: f32) -> (u8, u8, u8) {
    let r = (y + 1.402 * cr).round().clamp(0.0, 255.0) as u8;
    let g = (y - 0.344136 * cb - 0.714136 * cr).round().clamp(0.0, 255.0) as u8;
    let b = (y + 1.772 * cb).round().clamp(0.0, 255.0) as u8;
    (r, g, b)
}

/// Bilinear sample of a component plane mapped onto (w, h) output.
fn bilinear(plane: &[f32], pw: u32, ph: u32, w: usize, h: usize, x: usize, y: usize) -> f32 {
    if pw as usize == w && ph as usize == h {
        return plane[y * w + x];
    }
    let (pw, ph) = (pw as f32, ph as f32);
    let sx = (x as f32 + 0.5) * pw / w as f32 - 0.5;
    let sy = (y as f32 + 0.5) * ph / h as f32 - 0.5;
    let x0 = sx.floor().clamp(0.0, pw - 1.0) as usize;
    let y0 = sy.floor().clamp(0.0, ph - 1.0) as usize;
    let x1 = (x0 + 1).min(pw as usize - 1);
    let y1 = (y0 + 1).min(ph as usize - 1);
    let fx = (sx - x0 as f32).clamp(0.0, 1.0);
    let fy = (sy - y0 as f32).clamp(0.0, 1.0);
    let pwu = pw as usize;
    let a = plane[y0 * pwu + x0];
    let b = plane[y0 * pwu + x1];
    let c = plane[y1 * pwu + x0];
    let d = plane[y1 * pwu + x1];
    a * (1.0 - fx) * (1.0 - fy) + b * fx * (1.0 - fy) + c * (1.0 - fx) * fy + d * fx * fy
}

// ── Encoder ───────────────────────────────────────────────────

struct Encoder {
    out: Vec<u8>,
}

impl Encoder {
    fn encode_planes(
        width: u32,
        height: u32,
        y: &[u8],
        chroma: Option<(&[u8], &[u8])>,
        quality: u8,
        sampling: JpegSampling,
    ) -> Result<Vec<u8>, JpegError> {
        let gray = chroma.is_none();
        let (cw, ch) = match (sampling, gray) {
            (_, true) => (width, height),
            (JpegSampling::Yuv444, _) => (width, height),
            (JpegSampling::Yuv420, _) => ((width + 1) / 2, (height + 1) / 2),
        };
        let qlum = scale_quant_table(&Q_LUM, quality);
        let qchroma = scale_quant_table(&Q_CHROMA, quality);

        let mut enc = Self { out: Vec::new() };
        enc.out.extend_from_slice(&[0xFF, 0xD8]); // SOI
        // JFIF APP0.
        enc.out.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        enc.out.extend_from_slice(b"JFIF\0");
        enc.out.extend_from_slice(&[1, 2, 0, 0, 1, 0, 1, 0, 0]);
        // DQT.
        enc.write_dqt(0, &qlum);
        if !gray {
            enc.write_dqt(1, &qchroma);
        }
        // SOF0.
        {
            let nf = if gray { 1 } else { 3 };
            enc.marker_with_len(0xC0, 6 + 3 * nf);
            enc.out.push(8);
            enc.out.extend_from_slice(&(height as u16).to_be_bytes());
            enc.out.extend_from_slice(&(width as u16).to_be_bytes());
            enc.out.push(nf as u8);
            let (hy, vy) = match (sampling, gray) {
                (_, true) => (1u8, 1u8),
                (JpegSampling::Yuv444, _) => (1, 1),
                (JpegSampling::Yuv420, _) => (2, 2),
            };
            enc.out.extend_from_slice(&[1, (hy << 4) | vy, 0]);
            if !gray {
                enc.out.extend_from_slice(&[2, 0x11, 1]);
                enc.out.extend_from_slice(&[3, 0x11, 1]);
            }
        }
        // DHT standard tables.
        enc.write_dht(0, 0, &STD_LUM_DC);
        enc.write_dht(0, 1, &STD_CHROMA_DC);
        enc.write_dht(1, 0, &STD_LUM_AC);
        enc.write_dht(1, 1, &STD_CHROMA_AC);
        // SOS.
        {
            let nf = if gray { 1 } else { 3 };
            enc.marker_with_len(0xDA, 4 + 2 * nf);
            enc.out.push(nf as u8);
            enc.out.extend_from_slice(&[1, 0x00]);
            if !gray {
                enc.out.extend_from_slice(&[2, 0x11, 3, 0x11]);
            }
            enc.out.extend_from_slice(&[0x00, 0x3F, 0x00]);
        }

        // Entropy-coded scan.
        let mut bits = JpegBitWriter::new();
        let (dc_lum, _) = table_codes(&STD_LUM_DC.0, STD_LUM_DC.1);
        let (dc_ch, _) = table_codes(&STD_CHROMA_DC.0, STD_CHROMA_DC.1);
        let (ac_lum, _) = table_codes(&STD_LUM_AC.0, STD_LUM_AC.1);
        let (ac_ch, _) = table_codes(&STD_CHROMA_AC.0, STD_CHROMA_AC.1);

        let planes: Vec<(&[u8], u32, u32, &[u16; 64], &[(u32, u8); 256], &[(u32, u8); 256], u32, u32)> =
            if gray {
                vec![(y, width, height, &qlum, &dc_lum, &ac_lum, 1, 1)]
            } else {
                let (cb, cr) = chroma.unwrap();
                let (hy, vy) = match sampling {
                    JpegSampling::Yuv444 => (1u32, 1u32),
                    JpegSampling::Yuv420 => (2u32, 2u32),
                };
                vec![
                    (y, width, height, &qlum, &dc_lum, &ac_lum, hy, vy),
                    (cb, cw, ch, &qchroma, &dc_ch, &ac_ch, 1, 1),
                    (cr, cw, ch, &qchroma, &dc_ch, &ac_ch, 1, 1),
                ]
            };

        // MCU grid.
        let h_max = planes.iter().map(|p| p.6).max().unwrap();
        let v_max = planes.iter().map(|p| p.7).max().unwrap();
        let mcus_x = (width + 8 * h_max - 1) / (8 * h_max);
        let mcus_y = (height + 8 * v_max - 1) / (8 * v_max);
        let mut dc_preds = vec![0i32; planes.len()];
        for my in 0..mcus_y {
            for mx in 0..mcus_x {
                for (pi, (plane, pw, ph, qt, dc_t, ac_t, hh, vv)) in planes.iter().enumerate() {
                    let bw = (*pw + 7) / 8;
                    let bh = (*ph + 7) / 8;
                    for vy in 0..*vv {
                        for hx in 0..*hh {
                            let bx = mx * hh + hx;
                            let by = my * vv + vy;
                            if bx >= bw || by >= bh {
                                continue;
                            }
                            let mut block = [0f32; 64];
                            for yy in 0..8 {
                                for xx in 0..8 {
                                    let sx = ((bx * 8 + xx as u32).min(pw - 1)) as usize;
                                    let sy = ((by * 8 + yy as u32).min(ph - 1)) as usize;
                                    block[yy * 8 + xx] = plane[sy * *pw as usize + sx] as f32 - 128.0;
                                }
                            }
                            let coef = fdct8(&block);
                            let mut q = [0i16; 64];
                            for i in 0..64 {
                                q[i] = (coef[i] / qt[i] as f32).round() as i16;
                            }
                            write_block(&mut bits, &q, dc_t, ac_t, &mut dc_preds[pi]);
                        }
                    }
                }
            }
        }
        bits.flush_into(&mut enc.out);
        enc.out.extend_from_slice(&[0xFF, 0xD9]); // EOI
        Ok(enc.out)
    }

    fn marker_with_len(&mut self, marker: u8, payload_len: usize) {
        self.out.extend_from_slice(&[0xFF, marker]);
        self.out.extend_from_slice(&((payload_len + 2) as u16).to_be_bytes());
    }

    fn write_dqt(&mut self, tq: u8, table: &[u16; 64]) {
        self.marker_with_len(0xDB, 1 + 64);
        self.out.push(tq);
        for z in 0..64 {
            self.out.push(table[ZIG[z]].min(255) as u8);
        }
    }

    fn write_dht(&mut self, class: u8, id: u8, table: &([u8; 16], &[u8])) {
        let total: usize = table.0.iter().map(|&c| c as usize).sum();
        self.marker_with_len(0xC4, 1 + 16 + total);
        self.out.push((class << 4) | id);
        self.out.extend_from_slice(&table.0);
        self.out.extend_from_slice(&table.1[..total]);
    }
}

struct JpegBitWriter {
    acc: u32,
    nbits: u32,
    out: Vec<u8>,
}

impl JpegBitWriter {
    fn new() -> Self {
        Self { acc: 0, nbits: 0, out: Vec::new() }
    }
    fn write(&mut self, code: u32, len: u8) {
        self.acc = (self.acc << len) | code;
        self.nbits += len as u32;
        while self.nbits >= 8 {
            self.nbits -= 8;
            let b = ((self.acc >> self.nbits) & 0xFF) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0x00);
            }
        }
    }
    fn flush_into(mut self, out: &mut Vec<u8>) {
        if self.nbits > 0 {
            let b = ((self.acc << (8 - self.nbits)) & 0xFF) as u8 | ((1 << (8 - self.nbits)) - 1) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0x00);
            }
        }
        out.extend_from_slice(&self.out);
    }
}

fn category(v: i32) -> u32 {
    if v == 0 {
        0
    } else {
        32 - (v.unsigned_abs() as u32).leading_zeros()
    }
}

fn write_block(
    bw: &mut JpegBitWriter,
    q: &[i16; 64],
    dc_t: &[(u32, u8); 256],
    ac_t: &[(u32, u8); 256],
    dc_pred: &mut i32,
) {
    let diff = q[0] as i32 - *dc_pred;
    *dc_pred = q[0] as i32;
    let cat = category(diff);
    let (code, len) = dc_t[cat as usize];
    bw.write(code, len);
    if cat > 0 {
        let bits = if diff < 0 { (diff + (1 << cat) - 1) as u32 } else { diff as u32 };
        bw.write(bits, cat as u8);
    }
    let mut run = 0u32;
    for z in 1..64 {
        let v = q[ZIG[z]];
        if v == 0 {
            run += 1;
        } else {
            while run > 15 {
                let (code, len) = ac_t[0xF0];
                bw.write(code, len);
                run -= 16;
            }
            let size = category(v as i32);
            let sym = ((run << 4) | size) as usize;
            let (code, len) = ac_t[sym];
            bw.write(code, len);
            let bits = if v < 0 { (v as i32 + (1 << size) - 1) as u32 } else { v as u32 };
            bw.write(bits, size as u8);
            run = 0;
        }
    }
    if run > 0 {
        let (code, len) = ac_t[0x00];
        bw.write(code, len);
    }
}

/// 8x8 forward DCT.
fn fdct8(block: &[f32; 64]) -> [f32; 64] {
    let mut m = [[0f32; 8]; 8];
    for u in 0..8 {
        let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
        for x in 0..8 {
            m[u][x] = cu
                * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos()
                / 2.0;
        }
    }
    let mut tmp = [[0f32; 8]; 8];
    for u in 0..8 {
        for y in 0..8 {
            let mut s = 0f32;
            for x in 0..8 {
                s += m[u][x] * block[y * 8 + x];
            }
            tmp[u][y] = s;
        }
    }
    let mut out = [0f32; 64];
    for u in 0..8 {
        for v in 0..8 {
            let mut s = 0f32;
            for y in 0..8 {
                s += m[v][y] * tmp[u][y];
            }
            out[u * 8 + v] = s;
        }
    }
    out
}

/// RGBA8 to Y + downsampled Cb/Cr planes (values 0-255, no level shift).
fn rgb_to_ycbcr_planes(
    width: u32,
    height: u32,
    rgba: &[u8],
    sampling: JpegSampling,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (w, h) = (width as usize, height as usize);
    let mut y = vec![0u8; w * h];
    let (cw, ch) = match sampling {
        JpegSampling::Yuv444 => (w, h),
        JpegSampling::Yuv420 => ((w + 1) / 2, (h + 1) / 2),
    };
    let mut cb_acc = vec![0u32; cw * ch];
    let mut cr_acc = vec![0u32; cw * ch];
    let mut cnt = vec![0u32; cw * ch];
    for yy in 0..h {
        for xx in 0..w {
            let o = (yy * w + xx) * 4;
            let (r, g, b) = (rgba[o] as f32, rgba[o + 1] as f32, rgba[o + 2] as f32);
            let yv = 0.299 * r + 0.587 * g + 0.114 * b;
            let cbv = 128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b;
            let crv = 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b;
            y[yy * w + xx] = yv.round().clamp(0.0, 255.0) as u8;
            let (cx, cy) = match sampling {
                JpegSampling::Yuv444 => (xx, yy),
                JpegSampling::Yuv420 => (xx / 2, yy / 2),
            };
            let i = cy * cw + cx;
            cb_acc[i] += cbv.round().clamp(0.0, 255.0) as u32;
            cr_acc[i] += crv.round().clamp(0.0, 255.0) as u32;
            cnt[i] += 1;
        }
    }
    let cb = cb_acc.iter().zip(cnt.iter()).map(|(&s, &c)| (s / c) as u8).collect();
    let cr = cr_acc.iter().zip(cnt.iter()).map(|(&s, &c)| (s / c) as u8).collect();
    (y, cb, cr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_tables_are_consistent() {
        for (counts, values) in [STD_LUM_DC, STD_CHROMA_DC, STD_LUM_AC, STD_CHROMA_AC] {
            let total: usize = counts.iter().map(|&c| c as usize).sum();
            assert_eq!(total, values.len());
            assert!(total <= 256 && total > 0);
        }
        // Zigzag is a permutation of 0..64.
        let mut seen = [false; 64];
        for &z in &ZIG {
            assert!(!seen[z]);
            seen[z] = true;
        }
    }

    #[test]
    fn quality_scaling_matches_libjpeg_shape() {
        // q=50 leaves base tables untouched; q=100 clamps to 1.
        assert_eq!(scale_quant_table(&Q_LUM, 50), Q_LUM);
        assert!(scale_quant_table(&Q_LUM, 100).iter().all(|&v| v == 1));
        // q=1 saturates at 255.
        assert!(scale_quant_table(&Q_LUM, 1).iter().all(|&v| v == 255));
    }
}
