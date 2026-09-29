//! Pure-Rust PNG codec (ISO 15948, no third-party dependencies).
//!
//! Decode supports the full PNG feature set: color types 0, 2, 3, 4, 6
//! at every valid bit depth (1/2/4/8/16), all five filter types,
//! Adam7 interlacing, PLTE/tRNS transparency, multi-IDAT streams and
//! CRC verification. 16-bit samples scale to 8-bit with rounding.
//! Ancillary chunks (iCCP, sRGB, gAMA, tEXt, ...) are skipped; no color
//! management is applied in v1.
//!
//! Encode writes color type 6 (RGBA8) with per-row adaptive filtering
//! and a built-in LZ77/deflate compressor (dynamic Huffman blocks).

use std::sync::OnceLock;

// ── Public API ────────────────────────────────────────────────

/// Errors of the PNG codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngError {
    Decode(String),
    Encode(String),
    Unsupported(String),
}

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "png decode error: {m}"),
            Self::Encode(m) => write!(f, "png encode error: {m}"),
            Self::Unsupported(m) => write!(f, "png unsupported: {m}"),
        }
    }
}

impl std::error::Error for PngError {}

/// Decoded PNG image: always RGBA8 pixels.
#[derive(Debug, Clone)]
pub struct DecodedPng {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// PNG file signature.
pub const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// True when `bytes` starts with the PNG signature.
pub fn is_png(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[0..8] == SIGNATURE
}

/// Fast IHDR probe: `(width, height)` without decoding pixels.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), PngError> {
    let mut r = Reader::new(bytes)?;
    let (w, h, _, _, _) = r.read_ihdr()?;
    Ok((w, h))
}

/// Decode a full PNG file into RGBA8 pixels.
pub fn decode(bytes: &[u8]) -> Result<DecodedPng, PngError> {
    let mut r = Reader::new(bytes)?;
    let (width, height, bit_depth, color_type, interlace) = r.read_ihdr()?;
    let header = Ihdr { width, height, bit_depth, color_type, interlace };
    header.validate()?;

    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut trns: Option<Trns> = None;
    let mut idat: Vec<u8> = Vec::new();
    let mut seen_idat = false;

    loop {
        let (tag, data) = r.read_chunk()?;
        match &tag {
            b"IHDR" => return Err(PngError::Decode("multiple IHDR chunks".into())),
            b"PLTE" => {
                if seen_idat {
                    return Err(PngError::Decode("PLTE after IDAT".into()));
                }
                if data.is_empty() || data.len() % 3 != 0 || data.len() > 768 {
                    return Err(PngError::Decode("invalid PLTE length".into()));
                }
                palette = Some(data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect());
            }
            b"tRNS" => {
                trns = Some(parse_trns(&data, color_type)?);
            }
            b"IDAT" => {
                seen_idat = true;
                idat.extend_from_slice(&data);
            }
            b"IEND" => {
                if !data.is_empty() {
                    return Err(PngError::Decode("IEND with payload".into()));
                }
                break; // IEND terminates the stream; EOF before it errors out in read_chunk.
            }
            _ if is_critical(&tag) => {
                return Err(PngError::Unsupported(format!(
                    "unknown critical chunk {}",
                    String::from_utf8_lossy(&tag)
                )));
            }
            _ => {} // Skip unknown ancillary chunks.
        }
    }

    if !seen_idat {
        return Err(PngError::Decode("missing IDAT".into()));
    }
    if color_type == 3 && palette.is_none() {
        return Err(PngError::Decode("indexed image without PLTE".into()));
    }

    let raw = inflate_idat(&idat, &header)?;
    let pixels = reconstruct(&raw, &header, palette.as_deref(), trns.as_ref())?;
    Ok(DecodedPng { width, height, pixels })
}

/// Encode RGBA8 pixels (`rgba.len() == 4 * w * h`) as a PNG file.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, PngError> {
    if width == 0 || height == 0 {
        return Err(PngError::Encode("width and height must be > 0".into()));
    }
    let expect = (width as u64) * (height as u64) * 4;
    if rgba.len() as u64 != expect {
        return Err(PngError::Encode("pixel buffer length mismatch".into()));
    }
    if expect > MAX_PIXEL_BYTES {
        return Err(PngError::Encode("image too large".into()));
    }

    // Per-row adaptive filtering (bpp = 4 for RGBA8).
    let stride = width as usize * 4;
    let mut filtered = Vec::with_capacity((stride + 1) * height as usize);
    let mut prev = vec![0u8; stride];
    for y in 0..height as usize {
        let row = &rgba[y * stride..(y + 1) * stride];
        let (kind, data) = best_filter(row, &prev, 4);
        filtered.push(kind);
        filtered.extend_from_slice(&data);
        prev.copy_from_slice(row);
    }

    let compressed = deflate_zlib(&filtered);

    let mut out = Vec::with_capacity(compressed.len() + 128);
    out.extend_from_slice(&SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // no interlace
    write_chunk(&mut out, b"IHDR", &ihdr);
    let software = b"Software\0CoreImage png codec";
    write_chunk(&mut out, b"tEXt", software);
    write_chunk(&mut out, b"IDAT", &compressed);
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

// ── Limits ────────────────────────────────────────────────────

/// Max total pixel bytes (1 GiB) to bound allocation on corrupt input.
const MAX_PIXEL_BYTES: u64 = 1024 * 1024 * 1024;
/// Max image dimension to bound allocation on corrupt input.
const MAX_DIMENSION: u32 = 1_000_000;

// ── CRC32 / Adler32 ───────────────────────────────────────────

fn crc_table() -> &'static [u32; 256] {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

fn crc32(tag: &[u8; 4], data: &[u8]) -> u32 {
    let table = crc_table();
    let mut crc = 0xFFFF_FFFFu32;
    for &b in tag.iter().chain(data.iter()) {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

// ── Bit I/O (deflate is LSB-first) ────────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize, // bit position
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn read_bit(&mut self) -> Result<u32, PngError> {
        self.read_bits(1)
    }
    fn read_bits(&mut self, n: u32) -> Result<u32, PngError> {
        if n > 16 {
            return Err(PngError::Decode("bit read overflow".into()));
        }
        let mut val = 0u32;
        for i in 0..n {
            let byte = *self.data.get(self.pos / 8).ok_or_else(|| {
                PngError::Decode("unexpected end of deflate stream".into())
            })?;
            val |= (((byte >> (self.pos % 8)) & 1) as u32) << i;
            self.pos += 1;
        }
        Ok(val)
    }
    fn align_to_byte(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }
    fn remaining_bytes(&self) -> usize {
        self.data.len().saturating_sub((self.pos + 7) / 8)
    }
}

struct BitWriter {
    out: Vec<u8>,
    current: u32,
    filled: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self { out: Vec::new(), current: 0, filled: 0 }
    }
    fn write_bits(&mut self, mut val: u32, mut n: u32) {
        while n > 0 {
            let take = (8 - self.filled).min(n);
            self.current |= (val & ((1 << take) - 1)) << self.filled;
            val >>= take;
            n -= take;
            self.filled += take;
            if self.filled == 8 {
                self.out.push(self.current as u8);
                self.current = 0;
                self.filled = 0;
            }
        }
    }
    /// Pad with zero bits to the next byte boundary (required before
    /// stored blocks; must NOT be used between dynamic blocks).
    fn align_to_byte(&mut self) {
        if self.filled > 0 {
            self.out.push(self.current as u8);
            self.current = 0;
            self.filled = 0;
        }
    }
    /// Append raw bytes; caller must ensure byte alignment first.
    fn append_bytes(&mut self, bytes: &[u8]) {
        debug_assert_eq!(self.filled, 0);
        self.out.extend_from_slice(bytes);
    }
    fn into_bytes(mut self) -> Vec<u8> {
        if self.filled > 0 {
            self.out.push(self.current as u8);
        }
        self.out
    }
}

/// Emit one stored (uncompressed) block sequence for `raw` into the
/// bit stream `w`. Handles >64k splits; only the very last sub-block
/// carries BFINAL when `is_final` is set.
///
/// BFINAL/BTYPE are written at the current bit position, then the stream
/// is padded to the next byte boundary before LEN/NLEN (RFC 1951 §3.2.3).
/// The old code aligned *before* writing the header, corrupting every
/// stored block that followed a dynamic block mid-byte.
fn emit_stored_block(w: &mut BitWriter, raw: &[u8], is_final: bool) {
    if raw.is_empty() {
        w.write_bits(is_final as u32, 1);
        w.write_bits(0, 2); // BTYPE 00 stored
        w.align_to_byte();
        w.append_bytes(&[0x00, 0x00, 0xFF, 0xFF]);
        return;
    }
    let mut k = 0usize;
    while k < raw.len() {
        let chunk_len = (raw.len() - k).min(65535);
        let last = is_final && k + chunk_len >= raw.len();
        w.write_bits(last as u32, 1);
        w.write_bits(0, 2); // BTYPE 00 stored
        w.align_to_byte();
        w.append_bytes(&(chunk_len as u16).to_le_bytes());
        w.append_bytes(&(!(chunk_len as u16)).to_le_bytes());
        w.append_bytes(&raw[k..k + chunk_len]);
        k += chunk_len;
    }
}

// ── Huffman tables ────────────────────────────────────────────

/// Canonical codes from code lengths. Returns `(codes, max_len)`.
fn canonical_codes(lengths: &[u8]) -> (Vec<u32>, u32) {
    let max_len = lengths.iter().copied().max().unwrap_or(0);
    let mut bl_count = vec![0u32; (max_len as usize) + 1];
    for &l in lengths {
        if l > 0 {
            bl_count[l as usize] += 1;
        }
    }
    let mut next_code = vec![0u32; (max_len as usize) + 1];
    let mut code = 0u32;
    for bits in 1..=max_len as usize {
        code = (code + bl_count[bits - 1]) << 1;
        next_code[bits] = code;
    }
    let mut codes = vec![0u32; lengths.len()];
    for (sym, &len) in lengths.iter().enumerate() {
        if len > 0 {
            codes[sym] = next_code[len as usize];
            next_code[len as usize] += 1;
        }
    }
    (codes, max_len as u32)
}

/// Reverse the low `len` bits of `code` (deflate packs Huffman codes
/// MSB-first while plain integers go LSB-first).
fn reverse_bits(mut code: u32, len: u32) -> u32 {
    let mut rev = 0u32;
    for _ in 0..len {
        rev = (rev << 1) | (code & 1);
        code >>= 1;
    }
    rev
}

/// Binary decode tree built from canonical codes (codes read MSB-first).
struct HuffTree {
    child: Vec<[i32; 2]>,
    sym: Vec<i32>,
}

impl HuffTree {
    fn build(lengths: &[u8]) -> Result<Self, PngError> {
        let (codes, _) = canonical_codes(lengths);
        let mut child = vec![[-1i32, -1]];
        let mut sym = vec![-1i32];
        for (s, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let mut node = 0usize;
            for i in (0..len).rev() {
                let bit = ((codes[s] >> i) & 1) as usize;
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
                return Err(PngError::Decode("over-subscribed Huffman code".into()));
            }
            sym[node] = s as i32;
        }
        Ok(Self { child, sym })
    }

    fn decode(&self, br: &mut BitReader) -> Result<u16, PngError> {
        let mut node = 0usize;
        loop {
            if self.sym[node] >= 0 && node != 0 {
                return Ok(self.sym[node] as u16);
            }
            let bit = br.read_bit()? as usize;
            let next = self.child[node][bit];
            if next < 0 {
                return Err(PngError::Decode("invalid Huffman code".into()));
            }
            node = next as usize;
            if self.sym[node] >= 0
                && self.child[node] == [-1, -1]
            {
                return Ok(self.sym[node] as u16);
            }
        }
    }
}

// RFC 1951 length / distance tables.
const LEN_BASE: [u32; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83,
    99, 115, 131, 163, 195, 227, 258,
];
const LEN_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5,
    5, 5, 0,
];
const DIST_BASE: [u32; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513,
    769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10,
    11, 11, 12, 12, 13, 13,
];

fn fixed_litlen_lengths() -> [u8; 288] {
    let mut l = [0u8; 288];
    for (i, slot) in l.iter_mut().enumerate() {
        *slot = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    l
}

fn fixed_dist_lengths() -> [u8; 30] {
    [5u8; 30]
}

// ── Inflate ───────────────────────────────────────────────────

fn decode_trees(
    br: &mut BitReader,
) -> Result<(HuffTree, Option<HuffTree>), PngError> {
    let hlit = br.read_bits(5)? as usize + 257;
    let hdist = br.read_bits(5)? as usize + 1;
    let hclen = br.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(PngError::Decode("invalid dynamic header".into()));
    }
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut cl_lens = [0u8; 19];
    for i in 0..hclen {
        cl_lens[ORDER[i]] = br.read_bits(3)? as u8;
    }
    let cl_tree = HuffTree::build(&cl_lens)?;

    let total = hlit + hdist;
    let mut lengths = vec![0u8; total];
    let mut i = 0usize;
    let mut prev = 0u8;
    while i < total {
        let sym = cl_tree.decode(br)? as usize;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                prev = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(PngError::Decode("repeat with no previous length".into()));
                }
                let rep = br.read_bits(2)? as usize + 3;
                if i + rep > total {
                    return Err(PngError::Decode("length overflow".into()));
                }
                for _ in 0..rep {
                    lengths[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let rep = br.read_bits(3)? as usize + 3;
                if i + rep > total {
                    return Err(PngError::Decode("length overflow".into()));
                }
                for _ in 0..rep {
                    lengths[i] = 0;
                    i += 1;
                }
                prev = 0;
            }
            18 => {
                let rep = br.read_bits(7)? as usize + 11;
                if i + rep > total {
                    return Err(PngError::Decode("length overflow".into()));
                }
                for _ in 0..rep {
                    lengths[i] = 0;
                    i += 1;
                }
                prev = 0;
            }
            _ => return Err(PngError::Decode("invalid code-length symbol".into())),
        }
    }
    if lengths[256] == 0 {
        return Err(PngError::Decode("missing end-of-block code".into()));
    }
    // Over-subscribed check via tree build (incomplete codes are legal).
    let litlen = HuffTree::build(&lengths[..hlit])?;
    let dist = if hdist == 1 && lengths[hlit] == 0 {
        None // Single zero-length distance code: block has no matches.
    } else {
        Some(HuffTree::build(&lengths[hlit..])?)
    };
    Ok((litlen, dist))
}

fn inflate_block(
    br: &mut BitReader,
    litlen: &HuffTree,
    dist_tree: Option<&HuffTree>,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), PngError> {
    loop {
        if out.len() > limit {
            return Err(PngError::Decode("decompressed data too large".into()));
        }
        let sym = litlen.decode(br)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else if sym <= 285 {
            let idx = sym - 257;
            let len = LEN_BASE[idx] + br.read_bits(LEN_EXTRA[idx])?;
            let dsym = dist_tree
                .ok_or_else(|| PngError::Decode("distance with one-code tree".into()))?
                .decode(br)? as usize;
            if dsym > 29 {
                return Err(PngError::Decode("invalid distance symbol".into()));
            }
            let dist = (DIST_BASE[dsym] + br.read_bits(DIST_EXTRA[dsym])?) as usize;
            if dist == 0 || dist > out.len() {
                return Err(PngError::Decode("invalid match distance".into()));
            }
            for _ in 0..len as usize {
                let b = out[out.len() - dist];
                out.push(b);
                if out.len() > limit {
                    return Err(PngError::Decode("decompressed data too large".into()));
                }
            }
        } else {
            return Err(PngError::Decode("invalid literal/length symbol".into()));
        }
    }
}

/// Decompress one zlib stream. `limit` bounds the output allocation.
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, PngError> {
    if data.len() < 2 {
        return Err(PngError::Decode("truncated zlib stream".into()));
    }
    if data[0] & 0x0F != 8 || (data[0] >> 4) > 7 {
        return Err(PngError::Decode("unsupported zlib wrapper".into()));
    }
    if ((data[0] as u32 * 256 + data[1] as u32) % 31) != 0 {
        return Err(PngError::Decode("bad zlib header check".into()));
    }
    if data[1] & 0x20 != 0 {
        return Err(PngError::Decode("zlib preset dictionary unsupported".into()));
    }
    let mut br = BitReader::new(data);
    br.read_bits(16)?; // header consumed above
    let mut out = Vec::new();
    loop {
        let final_block = br.read_bit()? == 1;
        match br.read_bits(2)? {
            0 => {
                br.align_to_byte();
                let pos = br.pos / 8;
                if pos + 4 > data.len() {
                    return Err(PngError::Decode("truncated stored block".into()));
                }
                let len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
                let nlen = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
                if nlen != !((len as u16)) {
                    return Err(PngError::Decode("stored block length mismatch".into()));
                }
                br.pos += 32;
                if out.len() + len > limit {
                    return Err(PngError::Decode("decompressed data too large".into()));
                }
                if br.pos / 8 + len > data.len() {
                    return Err(PngError::Decode("truncated stored block".into()));
                }
                out.extend_from_slice(&data[br.pos / 8..br.pos / 8 + len]);
                br.pos += len * 8;
            }
            1 => {
                let ll = HuffTree::build(&fixed_litlen_lengths())?;
                let dd = HuffTree::build(&fixed_dist_lengths())?;
                inflate_block(&mut br, &ll, Some(&dd), &mut out, limit)?;
            }
            2 => {
                let (ll, dd) = decode_trees(&mut br)?;
                inflate_block(&mut br, &ll, dd.as_ref(), &mut out, limit)?;
            }
            _ => return Err(PngError::Decode("reserved block type".into())),
        }
        if final_block {
            break;
        }
    }
    br.align_to_byte();
    if br.remaining_bytes() < 4 {
        return Err(PngError::Decode("missing adler32".into()));
    }
    let expect = u32::from_be_bytes([
        data[br.pos / 8],
        data[br.pos / 8 + 1],
        data[br.pos / 8 + 2],
        data[br.pos / 8 + 3],
    ]);
    if adler32(&out) != expect {
        return Err(PngError::Decode("adler32 mismatch".into()));
    }
    br.pos += 32;
    br.align_to_byte();
    if br.remaining_bytes() > 0 {
        return Err(PngError::Decode("trailing data after zlib stream".into()));
    }
    Ok(out)
}

// ── Deflate (compressor) ──────────────────────────────────────

#[derive(Clone, Copy)]
enum Token {
    Lit(u8),
    Match { len: u32, dist: u32 },
}

const HASH_BITS: usize = 15;
const MAX_CHAIN: usize = 128;
const NICE_LEN: u32 = 64;
const MIN_MATCH: usize = 3;
const MAX_MATCH: u32 = 258;
const BLOCK_TOKENS: usize = 16384;

fn hash3(b: &[u8]) -> usize {
    (((b[0] as usize) << 10) ^ ((b[1] as usize) << 5) ^ (b[2] as usize)) & ((1 << HASH_BITS) - 1)
}

struct Matcher<'a> {
    input: &'a [u8],
    head: Vec<u32>, // hash -> pos+1
    prev: Vec<u32>, // pos -> prev pos+1
}

impl<'a> Matcher<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, head: vec![0u32; 1 << HASH_BITS], prev: vec![0u32; input.len()] }
    }
    fn insert(&mut self, pos: usize) {
        if pos + MIN_MATCH > self.input.len() {
            return;
        }
        let h = hash3(&self.input[pos..pos + 3]);
        self.prev[pos] = self.head[h];
        self.head[h] = (pos + 1) as u32;
    }
    fn find(&self, pos: usize) -> (u32, u32) {
        let n = self.input.len();
        if pos + MIN_MATCH > n {
            return (0, 0);
        }
        let h = hash3(&self.input[pos..pos + 3]);
        let mut best_len = 0u32;
        let mut best_dist = 0u32;
        let mut chain = MAX_CHAIN;
        let mut j = self.head[h];
        let max_len = ((n - pos) as u32).min(MAX_MATCH);
        while j != 0 && chain > 0 {
            let jp = (j - 1) as usize;
            let dist = (pos - jp) as u32;
            if dist > 32768 {
                break;
            }
            // Quick reject on first bytes is implicit in the length loop.
            let mut len = 0u32;
            while len < max_len && self.input[jp + len as usize] == self.input[pos + len as usize] {
                len += 1;
            }
            if len > best_len {
                best_len = len;
                best_dist = dist;
                if len >= NICE_LEN {
                    break;
                }
            }
            j = self.prev[jp];
            chain -= 1;
        }
        if best_len >= MIN_MATCH as u32 {
            (best_len, best_dist)
        } else {
            (0, 0)
        }
    }
}

/// Optimal-ish lengths limited to 15 bits (Huffman + overflow fix,
/// reassigned by descending frequency so the code stays canonical).
fn length_limited_lengths(freqs: &[u32]) -> Vec<u8> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let n = freqs.len();
    let active: Vec<usize> = (0..n).filter(|&s| freqs[s] > 0).collect();
    let mut lengths = vec![0u8; n];
    if active.is_empty() {
        return lengths;
    }
    if active.len() == 1 {
        lengths[active[0]] = 1;
        return lengths;
    }
    // Explicit Huffman tree build (clear and correct).
    let mut heap: BinaryHeap<(Reverse<u32>, Reverse<usize>, Tree)> = BinaryHeap::new();
    let mut seq = 0usize;
    for &s in &active {
        heap.push((Reverse(freqs[s]), Reverse(seq), Tree::Leaf(s)));
        seq += 1;
    }
    while heap.len() > 1 {
        let (_, _, a) = heap.pop().unwrap();
        let (_, _, b) = heap.pop().unwrap();
        let fa = tree_freq(&a, freqs);
        let fb = tree_freq(&b, freqs);
        heap.push((Reverse(fa + fb), Reverse(seq), Tree::Node(Box::new(a), Box::new(b))));
        seq += 1;
    }
    let (_, _, root) = heap.pop().unwrap();
    fn depth(t: &Tree, d: usize, out: &mut [u8]) {
        match t {
            Tree::Leaf(s) => out[*s] = d as u8,
            Tree::Node(a, b) => {
                depth(a, d + 1, out);
                depth(b, d + 1, out);
            }
        }
    }
    let mut init = vec![0u8; n];
    depth(&root, 0, &mut init);
    for &s in &active {
        if init[s] == 0 {
            init[s] = 1;
        }
    }

    // Clamp to 15 bits with the standard overflow correction.
    const MAX_BITS: usize = 15;
    let mut bl_count = [0usize; MAX_BITS + 1];
    let mut overflow = 0usize;
    for &s in &active {
        let mut bits = init[s] as usize;
        if bits > MAX_BITS {
            bits = MAX_BITS;
            overflow += 1;
        }
        if bits == 0 {
            bits = 1;
        }
        bl_count[bits] += 1;
        init[s] = bits as u8; // temporary store
    }
    if overflow > 0 {
        // zlib gen_bitlen overflow correction: move one leaf down and one
        // overflow item as its brother, dropping one MAX-length slot.
        let mut ov = overflow as i32;
        while ov > 0 {
            let mut bits = MAX_BITS - 1;
            while bits > 0 && bl_count[bits] == 0 {
                bits -= 1;
            }
            if bits == 0 {
                break; // cannot fix further; still a valid (suboptimal) code
            }
            bl_count[bits] -= 1;
            bl_count[bits + 1] += 2;
            bl_count[MAX_BITS] = bl_count[MAX_BITS].saturating_sub(1);
            ov -= 2;
        }
    }
    // Reassign shortest codes to most frequent symbols (valid Kraft set
    // stays valid; canonical assignment works for any such multiset).
    let mut by_freq = active.clone();
    by_freq.sort_by(|&a, &b| freqs[b].cmp(&freqs[a]).then(a.cmp(&b)));
    let mut pos = 0usize;
    for bits in 1..=MAX_BITS {
        for _ in 0..bl_count[bits] {
            if pos >= by_freq.len() {
                break;
            }
            lengths[by_freq[pos]] = bits as u8;
            pos += 1;
        }
    }
    for &s in &active {
        if lengths[s] == 0 {
            lengths[s] = MAX_BITS as u8;
        }
    }
    lengths
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Tree {
    Leaf(usize),
    Node(Box<Tree>, Box<Tree>),
}

fn tree_freq(t: &Tree, freqs: &[u32]) -> u32 {
    match t {
        Tree::Leaf(s) => freqs[*s],
        Tree::Node(a, b) => tree_freq(a, freqs) + tree_freq(b, freqs),
    }
}

/// Write one dynamic-Huffman block for `tokens` covering `raw`.
fn write_dynamic_block(
    w: &mut BitWriter,
    tokens: &[Token],
    is_final: bool,
) {
    let mut lit_freq = [0u32; 286];
    let mut dist_freq = [0u32; 30];
    for t in tokens {
        match *t {
            Token::Lit(b) => lit_freq[b as usize] += 1,
            Token::Match { len, dist } => {
                lit_freq[length_symbol(len) as usize] += 1;
                dist_freq[dist_symbol(dist) as usize] += 1;
            }
        }
    }
    lit_freq[256] += 1; // end of block
    if dist_freq.iter().all(|&f| f == 0) {
        dist_freq[0] = 1;
    }

    let lit_len = length_limited_lengths(&lit_freq);
    let dist_len = length_limited_lengths(&dist_freq);

    let mut hlit = 286usize;
    while hlit > 257 && lit_len[hlit - 1] == 0 {
        hlit -= 1;
    }
    let mut hdist = 30usize;
    while hdist > 1 && dist_len[hdist - 1] == 0 {
        hdist -= 1;
    }

    // Code-length RLE over concatenated lengths.
    let all: Vec<u8> = lit_len[..hlit].iter().chain(dist_len[..hdist].iter()).copied().collect();
    let mut cl_freq = [0u32; 19];
    let mut i = 0usize;
    // First pass: count frequencies of the RLE stream symbols.
    let mut stream: Vec<(u8, u32)> = Vec::new(); // (cl_symbol, extra)
    while i < all.len() {
        let v = all[i];
        let mut run = 1usize;
        while i + run < all.len() && all[i + run] == v {
            run += 1;
        }
        if v == 0 {
            let mut left = run;
            while left >= 11 {
                let take = left.min(138);
                stream.push((18, (take - 11) as u32));
                cl_freq[18] += 1;
                left -= take;
            }
            if left >= 3 {
                stream.push((17, (left - 3) as u32));
                cl_freq[17] += 1;
                left = 0;
            }
            for _ in 0..left {
                stream.push((0, 0));
                cl_freq[0] += 1;
            }
        } else {
            stream.push((v, 0));
            cl_freq[v as usize] += 1;
            let mut left = run - 1;
            while left >= 3 {
                let take = left.min(6);
                stream.push((16, (take - 3) as u32));
                cl_freq[16] += 1;
                left -= take;
            }
            for _ in 0..left {
                stream.push((v, 0));
                cl_freq[v as usize] += 1;
            }
        }
        i += run;
    }
    let cl_len = length_limited_lengths(&cl_freq);
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut hclen = 19usize;
    while hclen > 4 && cl_len[ORDER[hclen - 1]] == 0 {
        hclen -= 1;
    }
    let (cl_codes, _) = canonical_codes(&cl_len);
    let (lit_codes, _) = canonical_codes(&lit_len);
    let (dist_codes, _) = canonical_codes(&dist_len);

    w.write_bits(is_final as u32, 1);
    w.write_bits(2, 2); // dynamic
    w.write_bits((hlit - 257) as u32, 5);
    w.write_bits((hdist - 1) as u32, 5);
    w.write_bits((hclen - 4) as u32, 4);
    for k in 0..hclen {
        w.write_bits(cl_len[ORDER[k]] as u32, 3);
    }
    for (sym, extra) in &stream {
        let s = *sym as usize;
        w.write_bits(reverse_bits(cl_codes[s], cl_len[s] as u32), cl_len[s] as u32);
        match s {
            16 => w.write_bits(*extra, 2),
            17 => w.write_bits(*extra, 3),
            18 => w.write_bits(*extra, 7),
            _ => {}
        }
    }
    for t in tokens {
        match *t {
            Token::Lit(b) => {
                let s = b as usize;
                w.write_bits(reverse_bits(lit_codes[s], lit_len[s] as u32), lit_len[s] as u32);
            }
            Token::Match { len, dist } => {
                let ls = length_symbol(len) as usize;
                w.write_bits(reverse_bits(lit_codes[ls], lit_len[ls] as u32), lit_len[ls] as u32);
                w.write_bits(len - LEN_BASE[ls - 257], LEN_EXTRA[ls - 257]);
                let ds = dist_symbol(dist) as usize;
                w.write_bits(reverse_bits(dist_codes[ds], dist_len[ds] as u32), dist_len[ds] as u32);
                w.write_bits(dist - DIST_BASE[ds], DIST_EXTRA[ds]);
            }
        }
    }
    w.write_bits(
        reverse_bits(lit_codes[256], lit_len[256] as u32),
        lit_len[256] as u32,
    );
}

fn length_symbol(len: u32) -> u16 {
    for (i, (&base, &extra)) in LEN_BASE.iter().zip(LEN_EXTRA.iter()).enumerate() {
        let max = base + if extra == 0 { 0 } else { (1 << extra) - 1 };
        if len >= base && len <= max {
            return (257 + i) as u16;
        }
    }
    285
}

fn dist_symbol(dist: u32) -> u16 {
    for (i, (&base, &extra)) in DIST_BASE.iter().zip(DIST_EXTRA.iter()).enumerate() {
        let max = base + if extra == 0 { 0 } else { (1 << extra) - 1 };
        if dist >= base && dist <= max {
            return i as u16;
        }
    }
    29
}

/// Compress with zlib wrapper (header + deflate + adler32).
fn deflate_zlib(input: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if input.is_empty() {
        out.extend_from_slice(&[0x03, 0x00]);
        out.extend_from_slice(&adler32(input).to_be_bytes());
        return out;
    }
    let mut matcher = Matcher::new(input);
    let n = input.len();
    let mut i = 0usize;
    let mut seg_start = 0usize;
    let mut tokens: Vec<Token> = Vec::new();
    let mut consumed = 0usize; // bytes covered by `tokens` since seg_start

    // Single bit stream for all dynamic blocks. Deflate blocks are
    // bit-packed back-to-back; only stored blocks align to bytes.
    // (The old code finished each block with into_bytes(), inserting up
    // to 7 zero padding bits between blocks and corrupting every image
    // larger than one BLOCK_TOKENS chunk.)
    let mut w = BitWriter::new();
    let mut blocks_emitted = 0usize;
    let mut emitted_final = false;

    // Encode one token chunk into the shared stream.
    // Validates the dynamic block by round-tripping it through our own
    // inflate; any block that fails validation falls back to stored
    // (always valid). This guarantees no corrupt PNG ever leaves the
    // encoder, even for pathological frequency distributions.
    fn dynamic_block_roundtrips(tokens: &[Token], raw: &[u8]) -> bool {
        let mut tmp = BitWriter::new();
        write_dynamic_block(&mut tmp, tokens, true);
        let dyn_bytes = tmp.into_bytes();
        let mut z = Vec::with_capacity(dyn_bytes.len() + 6);
        z.extend_from_slice(&[0x78, 0x01]);
        z.extend_from_slice(&dyn_bytes);
        z.extend_from_slice(&adler32(raw).to_be_bytes());
        match inflate(&z, raw.len()) {
            Ok(out) => out == raw,
            Err(_) => false,
        }
    }
    fn flush_chunk(
        w: &mut BitWriter,
        input: &[u8],
        seg_start: usize,
        consumed: usize,
        tokens: &[Token],
        is_final: bool,
    ) {
        let raw = &input[seg_start..seg_start + consumed];
        if !dynamic_block_roundtrips(tokens, raw) {
            emit_stored_block(w, raw, is_final);
            return;
        }
        // Probe dynamic size with a temp writer (includes its own pad
        // byte; conservative but still valid when it picks stored).
        let mut tmp = BitWriter::new();
        write_dynamic_block(&mut tmp, tokens, is_final);
        let dyn_len = tmp.into_bytes().len();
        if dyn_len >= raw.len() + 5 {
            emit_stored_block(w, raw, is_final);
        } else {
            write_dynamic_block(w, tokens, is_final);
        }
    }

    let mut pend: Option<(u32, u32)> = None;
    while i < n {
        let (ml, md) = matcher.find(i);
        matcher.insert(i);
        match pend.take() {
            Some((pl, _)) if ml > pl => {
                tokens.push(Token::Lit(input[i - 1]));
                consumed += 1;
                pend = Some((ml, md));
                i += 1;
            }
            Some((pl, pd)) => {
                tokens.push(Token::Match { len: pl, dist: pd });
                let skip = pl as usize - 1;
                for k in 1..skip {
                    if i + k < n {
                        matcher.insert(i + k);
                    }
                }
                consumed += pl as usize;
                i += skip;
            }
            None => {
                if ml >= MIN_MATCH as u32 {
                    pend = Some((ml, md));
                    i += 1;
                } else {
                    tokens.push(Token::Lit(input[i]));
                    consumed += 1;
                    i += 1;
                }
            }
        }
        if pend.is_none() && (tokens.len() >= BLOCK_TOKENS || i >= n) {
            let is_final = i >= n;
            flush_chunk(&mut w, input, seg_start, consumed, &tokens, is_final);
            blocks_emitted += 1;
            if is_final {
                emitted_final = true;
            }
            tokens.clear();
            seg_start += consumed;
            consumed = 0;
        }
    }
    if let Some((pl, pd)) = pend {
        tokens.push(Token::Match { len: pl, dist: pd });
        consumed += pl as usize;
    }
    if !emitted_final && (!tokens.is_empty() || blocks_emitted == 0) {
        flush_chunk(&mut w, input, seg_start, consumed, &tokens, true);
    }

    out.extend_from_slice(&w.into_bytes());
    out.extend_from_slice(&adler32(input).to_be_bytes());
    out
}

// ── Chunk I/O ─────────────────────────────────────────────────

fn is_critical(tag: &[u8; 4]) -> bool {
    tag[0] & 0x20 == 0
}

fn write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(tag, data).to_be_bytes());
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Result<Self, PngError> {
        if !is_png(data) {
            return Err(PngError::Decode("not a PNG file".into()));
        }
        Ok(Self { data, pos: 8 })
    }
    fn read_u32(&mut self) -> Result<u32, PngError> {
        if self.pos + 4 > self.data.len() {
            return Err(PngError::Decode("truncated chunk header".into()));
        }
        let v = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }
    fn read_chunk(&mut self) -> Result<([u8; 4], Vec<u8>), PngError> {
        let len = self.read_u32()? as usize;
        if len > 1024 * 1024 * 128 {
            return Err(PngError::Decode("chunk too large".into()));
        }
        if self.pos + 4 > self.data.len() {
            return Err(PngError::Decode("truncated chunk type".into()));
        }
        let tag = [self.data[self.pos], self.data[self.pos + 1], self.data[self.pos + 2], self.data[self.pos + 3]];
        self.pos += 4;
        if self.pos + len + 4 > self.data.len() {
            return Err(PngError::Decode("truncated chunk data".into()));
        }
        let payload = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        let expect = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        if crc32(&tag, &payload) != expect {
            return Err(PngError::Decode(format!(
                "CRC mismatch in {}",
                String::from_utf8_lossy(&tag)
            )));
        }
        Ok((tag, payload))
    }
    fn read_ihdr(&mut self) -> Result<(u32, u32, u8, u8, u8), PngError> {
        let (tag, data) = self.read_chunk()?;
        if &tag != b"IHDR" {
            return Err(PngError::Decode("first chunk is not IHDR".into()));
        }
        if data.len() != 13 {
            return Err(PngError::Decode("invalid IHDR length".into()));
        }
        Ok((
            u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            data[8],
            data[9],
            data[12],
        ))
    }
}

// ── Header / transparency ─────────────────────────────────────

struct Ihdr {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    interlace: u8,
}

impl Ihdr {
    fn validate(&self) -> Result<(), PngError> {
        if self.width == 0 || self.height == 0 {
            return Err(PngError::Decode("zero image dimension".into()));
        }
        if self.width > MAX_DIMENSION || self.height > MAX_DIMENSION {
            return Err(PngError::Decode("image dimension too large".into()));
        }
        let pixels = self.width as u64 * self.height as u64;
        if pixels * 4 > MAX_PIXEL_BYTES {
            return Err(PngError::Decode("image too large".into()));
        }
        let ok_depth = match self.color_type {
            0 => matches!(self.bit_depth, 1 | 2 | 4 | 8 | 16),
            2 => matches!(self.bit_depth, 8 | 16),
            3 => matches!(self.bit_depth, 1 | 2 | 4 | 8),
            4 => matches!(self.bit_depth, 8 | 16),
            6 => matches!(self.bit_depth, 8 | 16),
            _ => false,
        };
        if !ok_depth {
            return Err(PngError::Decode("invalid color type / bit depth combo".into()));
        }
        if self.interlace > 1 {
            return Err(PngError::Decode("invalid interlace method".into()));
        }
        Ok(())
    }
    fn samples_per_pixel(&self) -> usize {
        match self.color_type {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            _ => 4,
        }
    }
    fn bits_per_pixel(&self) -> usize {
        self.samples_per_pixel() * self.bit_depth as usize
    }
}

#[derive(Debug, Clone)]
enum Trns {
    Gray(u16),
    Rgb(u16, u16, u16),
    Palette(Vec<u8>),
}

fn parse_trns(data: &[u8], color_type: u8) -> Result<Trns, PngError> {
    match color_type {
        0 => {
            if data.len() != 2 {
                return Err(PngError::Decode("invalid tRNS length".into()));
            }
            Ok(Trns::Gray(u16::from_be_bytes([data[0], data[1]])))
        }
        2 => {
            if data.len() != 6 {
                return Err(PngError::Decode("invalid tRNS length".into()));
            }
            Ok(Trns::Rgb(
                u16::from_be_bytes([data[0], data[1]]),
                u16::from_be_bytes([data[2], data[3]]),
                u16::from_be_bytes([data[4], data[5]]),
            ))
        }
        3 => Ok(Trns::Palette(data.to_vec())),
        _ => Ok(Trns::Palette(data.to_vec())), // Ignored for gray+alpha / RGBA.
    }
}

// ── Scanline filters ──────────────────────────────────────────

fn paeth(a: i32, b: i32, c: i32) -> u8 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    (if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }) as u8
}

fn unfilter_row(filter: u8, row: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), PngError> {
    match filter {
        0 => {}
        1 => {
            for i in bpp..row.len() {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..row.len() {
                row[i] = row[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..row.len() {
                let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
                row[i] = row[i].wrapping_add(((a + prev[i] as i32) / 2) as u8);
            }
        }
        4 => {
            for i in 0..row.len() {
                let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
                let b = prev[i] as i32;
                let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                row[i] = row[i].wrapping_add(paeth(a, b, c));
            }
        }
        _ => return Err(PngError::Decode("invalid filter type".into())),
    }
    Ok(())
}

/// Encoder side: pick the best filter for one row (min sum of absolutes).
fn best_filter(row: &[u8], prev: &[u8], bpp: usize) -> (u8, Vec<u8>) {
    let mut best_kind = 0u8;
    let mut best_sum = u64::MAX;
    let mut best_row = row.to_vec();
    for kind in 0..=4u8 {
        let mut tmp = row.to_vec();
        match kind {
            0 => {}
            1 => {
                for i in (bpp..tmp.len()).rev() {
                    tmp[i] = tmp[i].wrapping_sub(tmp[i - bpp]);
                }
            }
            2 => {
                for i in 0..tmp.len() {
                    tmp[i] = tmp[i].wrapping_sub(prev[i]);
                }
            }
            3 => {
                for i in (0..tmp.len()).rev() {
                    let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
                    tmp[i] = row[i].wrapping_sub(((a + prev[i] as i32) / 2) as u8);
                }
            }
            4 => {
                for i in (0..tmp.len()).rev() {
                    let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
                    let b = prev[i] as i32;
                    let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                    tmp[i] = row[i].wrapping_sub(paeth(a, b, c));
                }
            }
            _ => unreachable!(),
        }
        let sum: u64 = tmp.iter().map(|&b| (b as i8 as i32).abs() as u64).sum();
        if sum < best_sum {
            best_sum = sum;
            best_kind = kind;
            best_row = tmp;
        }
    }
    (best_kind, best_row)
}

// ── IDAT size / inflate ───────────────────────────────────────

fn row_bytes(width: u32, bits_per_pixel: usize) -> Result<usize, PngError> {
    let bits = width as u64 * bits_per_pixel as u64;
    let bytes = (bits + 7) / 8;
    if bytes > u32::MAX as u64 {
        return Err(PngError::Decode("row too large".into()));
    }
    Ok(bytes as usize)
}

fn expected_idat_len(h: &Ihdr) -> Result<usize, PngError> {
    let bpp = h.bits_per_pixel();
    if h.interlace == 0 {
        let rb = row_bytes(h.width, bpp)?;
        return h
            .height
            .checked_mul((rb + 1) as u32)
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| PngError::Decode("row too large".into()));
    }
    let mut total = 0usize;
    for &(x0, y0, dx, dy) in &ADAM7 {
        let (w, hh) = pass_dims(h.width, h.height, x0, y0, dx, dy);
        if w == 0 || hh == 0 {
            continue;
        }
        let rb = row_bytes(w, bpp)?;
        total = total
            .checked_add(hh as usize * (rb + 1))
            .ok_or_else(|| PngError::Decode("image too large".into()))?;
    }
    Ok(total)
}

fn inflate_idat(idat: &[u8], h: &Ihdr) -> Result<Vec<u8>, PngError> {
    let expect = expected_idat_len(h)?;
    if expect as u64 > MAX_PIXEL_BYTES + 1024 * 1024 {
        return Err(PngError::Decode("image too large".into()));
    }
    let raw = inflate(idat, expect)?;
    if raw.len() != expect {
        return Err(PngError::Decode(format!(
            "size mismatch: got {} bytes, expected {expect}",
            raw.len()
        )));
    }
    Ok(raw)
}

// ── Adam7 ─────────────────────────────────────────────────────

const ADAM7: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

fn pass_dims(w: u32, h: u32, x0: u32, y0: u32, dx: u32, dy: u32) -> (u32, u32) {
    let pw = if x0 < w { (w - x0 + dx - 1) / dx } else { 0 };
    let ph = if y0 < h { (h - y0 + dy - 1) / dy } else { 0 };
    (pw, ph)
}

// ── Sample unpacking / color conversion ───────────────────────

/// MSB-first bit reader over one scanline ( depths 1/2/4 ).
struct BitUnpack<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitUnpack<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }
    fn read(&mut self, n: u8) -> Result<u16, PngError> {
        let mut v = 0u16;
        for _ in 0..n {
            let byte = *self.data.get(self.bit / 8).ok_or_else(|| {
                PngError::Decode("scanline overrun".into())
            })?;
            v = (v << 1) | (((byte >> (7 - (self.bit % 8))) & 1) as u16);
            self.bit += 1;
        }
        Ok(v)
    }
}

fn scale_to_8(value: u16, bit_depth: u8) -> u8 {
    match bit_depth {
        16 => ((value as u32 * 255 + 32767) / 65535) as u8,
        8 => value as u8,
        _ => {
            let max = ((1u32 << bit_depth) - 1).max(1);
            ((value as u32 * 255 + max / 2) / max) as u8
        }
    }
}

fn reconstruct(
    raw: &[u8],
    h: &Ihdr,
    palette: Option<&[[u8; 3]]>,
    trns: Option<&Trns>,
) -> Result<Vec<u8>, PngError> {
    let mut pixels = vec![0u8; h.width as usize * h.height as usize * 4];
    let spp = h.samples_per_pixel();
    let depth = h.bit_depth as usize;
    // Bytes per complete pixel for filtering (rounded up, min 1).
    let filter_bpp = ((spp * depth + 7) / 8).max(1);
    let mut offset = 0usize;

    // Palette alpha table (tRNS), if any.
    let pal_alpha: Option<&[u8]> = match trns {
        Some(Trns::Palette(a)) => {
            if let Some(pal) = palette {
                if a.len() > pal.len() {
                    return Err(PngError::Decode("tRNS longer than PLTE".into()));
                }
            }
            Some(a)
        }
        _ => None,
    };

    let passes: Vec<(u32, u32, u32, u32)> = if h.interlace == 0 {
        vec![(0, 0, 1, 1)]
    } else {
        ADAM7.to_vec()
    };

    for &(x0, y0, dx, dy) in &passes {
        let (pw, ph) = if h.interlace == 0 {
            (h.width, h.height)
        } else {
            pass_dims(h.width, h.height, x0, y0, dx, dy)
        };
        if pw == 0 || ph == 0 {
            continue;
        }
        let rb = row_bytes(pw, h.bits_per_pixel())?;
        let mut prev = vec![0u8; rb];
        for py in 0..ph {
            if offset + 1 + rb > raw.len() {
                return Err(PngError::Decode("scanline overrun".into()));
            }
            let f = raw[offset];
            offset += 1;
            let mut row = raw[offset..offset + rb].to_vec();
            offset += rb;
            unfilter_row(f, &mut row, &prev, filter_bpp)?;
            prev = row.clone();

            // Convert this row into RGBA pixels.
            if depth < 8 {
                let mut up = BitUnpack::new(&row);
                for px in 0..pw {
                    let v = up.read(h.bit_depth)?;
                    let dst = pixel_offset(h.width, x0 + px * dx, y0 + py * dy);
                    write_pixel(
                        &mut pixels,
                        dst,
                        h,
                        &[v],
                        palette,
                        pal_alpha,
                        trns,
                    )?;
                }
            } else {
                let step = spp * (depth / 8);
                for px in 0..pw {
                    let base = px as usize * step;
                    if base + step > row.len() {
                        return Err(PngError::Decode("scanline overrun".into()));
                    }
                    let mut samples = [0u16; 4];
                    for s in 0..spp {
                        samples[s] = if depth == 16 {
                            u16::from_be_bytes([row[base + s * 2], row[base + s * 2 + 1]])
                        } else {
                            row[base + s] as u16
                        };
                    }
                    let dst = pixel_offset(h.width, x0 + px * dx, y0 + py * dy);
                    write_pixel(
                        &mut pixels,
                        dst,
                        h,
                        &samples[..spp],
                        palette,
                        pal_alpha,
                        trns,
                    )?;
                }
            }
        }
    }
    Ok(pixels)
}

fn pixel_offset(width: u32, x: u32, y: u32) -> usize {
    (y as usize * width as usize + x as usize) * 4
}

fn write_pixel(
    pixels: &mut [u8],
    dst: usize,
    h: &Ihdr,
    samples: &[u16],
    palette: Option<&[[u8; 3]]>,
    pal_alpha: Option<&[u8]>,
    trns: Option<&Trns>,
) -> Result<(), PngError> {
    if dst + 4 > pixels.len() {
        return Err(PngError::Decode("pixel out of bounds".into()));
    }
    let (r, g, b, a) = match h.color_type {
        0 => {
            let gray = scale_to_8(samples[0], h.bit_depth);
            let alpha = match trns {
                Some(Trns::Gray(key)) if *key == scale_key(samples[0], h.bit_depth) => 0,
                _ => 255,
            };
            (gray, gray, gray, alpha)
        }
        2 => {
            let r = scale_to_8(samples[0], h.bit_depth);
            let g = scale_to_8(samples[1], h.bit_depth);
            let b = scale_to_8(samples[2], h.bit_depth);
            let alpha = match trns {
                Some(Trns::Rgb(kr, kg, kb))
                    if samples[0] == scale_up(*kr, h.bit_depth)
                        && samples[1] == scale_up(*kg, h.bit_depth)
                        && samples[2] == scale_up(*kb, h.bit_depth) =>
                {
                    0
                }
                _ => 255,
            };
            (r, g, b, alpha)
        }
        3 => {
            let idx = samples[0] as usize;
            let pal = palette.ok_or_else(|| PngError::Decode("indexed without PLTE".into()))?;
            let entry = *pal.get(idx).ok_or_else(|| PngError::Decode("palette index out of range".into()))?;
            let alpha = pal_alpha.and_then(|t| t.get(idx).copied()).unwrap_or(255);
            (entry[0], entry[1], entry[2], alpha)
        }
        4 => {
            let gray = scale_to_8(samples[0], h.bit_depth);
            (gray, gray, gray, scale_to_8(samples[1], h.bit_depth))
        }
        _ => (
            scale_to_8(samples[0], h.bit_depth),
            scale_to_8(samples[1], h.bit_depth),
            scale_to_8(samples[2], h.bit_depth),
            scale_to_8(samples[3], h.bit_depth),
        ),
    };
    pixels[dst..dst + 4].copy_from_slice(&[r, g, b, a]);
    Ok(())
}

/// Compare a raw sample against a tRNS key stored at 8-bit scale.
/// tRNS keys for low-bit images use the sample value directly.
fn scale_key(sample: u16, bit_depth: u8) -> u16 {
    if bit_depth == 16 {
        sample
    } else if bit_depth == 8 {
        sample
    } else {
        // Keys are full-range; low-bit samples never equal out-of-range keys
        // unless the key itself fits the depth (then direct comparison works
        // because sample values are already in range).
        sample
    }
}

/// Scale an 8-bit-side tRNS key component up to the sample depth for comparison.
fn scale_up(key: u16, bit_depth: u8) -> u16 {
    if bit_depth == 16 {
        key * 257
    } else {
        key
    }
}
