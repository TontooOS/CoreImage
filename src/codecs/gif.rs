//! Pure-Rust GIF codec (single frame, no third-party dependencies).
//!
//! Decode reads the first frame of GIF87a/GIF89a files: global and
//! local color tables, Graphic Control Extension transparency, plain
//! and interlaced image data, positioned sub-images composited onto
//! the logical screen. Later frames, animation delays, disposal and
//! NETSCAPE extensions are ignored by design (thumbnail semantics).
//!
//! Encode writes single-frame GIF89a with a median-cut palette
//! (exact palette up to 256 colors), optional transparency and
//! standard LZW compression.

use std::collections::HashMap;

// ── Public API ────────────────────────────────────────────────

/// Errors of the GIF codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GifError {
    Decode(String),
    Encode(String),
    Unsupported(String),
}

impl std::fmt::Display for GifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "gif decode error: {m}"),
            Self::Encode(m) => write!(f, "gif encode error: {m}"),
            Self::Unsupported(m) => write!(f, "gif unsupported: {m}"),
        }
    }
}

impl std::error::Error for GifError {}

/// Decoded first frame: always RGBA8 pixels (transparent index → alpha 0).
#[derive(Debug, Clone)]
pub struct DecodedGif {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// True for GIF87a/GIF89a files.
pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.len() >= 6
        && (bytes[0..6] == *b"GIF87a" || bytes[0..6] == *b"GIF89a")
}

/// Fast probe: logical screen dimensions without decoding pixels.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), GifError> {
    if !is_gif(bytes) {
        return Err(GifError::Decode("not a GIF file".into()));
    }
    if bytes.len() < 10 {
        return Err(GifError::Decode("truncated header".into()));
    }
    let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
    let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
    if w == 0 || h == 0 {
        return Err(GifError::Decode("zero image dimension".into()));
    }
    Ok((w, h))
}

/// Decode the first frame into RGBA8 pixels.
pub fn decode(bytes: &[u8]) -> Result<DecodedGif, GifError> {
    Decoder::new(bytes)?.decode_first_frame()
}

/// Encode RGBA8 pixels as single-frame GIF89a.
/// Colors quantize to 256 via median-cut (exact when already small);
/// pixels with alpha < 128 become transparent.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, GifError> {
    if width == 0 || height == 0 {
        return Err(GifError::Encode("width and height must be > 0".into()));
    }
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(GifError::Encode("pixel buffer length mismatch".into()));
    }
    if width > 65535 || height > 65535 {
        return Err(GifError::Encode("image too large".into()));
    }
    let (palette, indices, transparent) = quantize(rgba);
    encode_indexed(width, height, &palette, &indices, transparent)
}

// ── Decoder ───────────────────────────────────────────────────

struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
    width: u32,
    height: u32,
    gct: Option<Vec<[u8; 3]>>,
    // Pending Graphic Control Extension for the next frame.
    gce_transparent: Option<u8>,
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Result<Self, GifError> {
        if !is_gif(data) {
            return Err(GifError::Decode("not a GIF file".into()));
        }
        if data.len() < 13 {
            return Err(GifError::Decode("truncated header".into()));
        }
        let width = u16::from_le_bytes([data[6], data[7]]) as u32;
        let height = u16::from_le_bytes([data[8], data[9]]) as u32;
        if width == 0 || height == 0 {
            return Err(GifError::Decode("zero image dimension".into()));
        }
        if width > 65535 || height > 65535 {
            return Err(GifError::Decode("image too large".into()));
        }
        let packed = data[10];
        let mut pos = 13;
        let gct = if packed & 0x80 != 0 {
            let n = 2usize << (packed & 7);
            let table = read_table(data, &mut pos, n)?;
            Some(table)
        } else {
            None
        };
        Ok(Self { data, pos, width, height, gct, gce_transparent: None })
    }

    fn decode_first_frame(&mut self) -> Result<DecodedGif, GifError> {
        loop {
            if self.pos >= self.data.len() {
                return Err(GifError::Decode("no image data".into()));
            }
            match self.data[self.pos] {
                0x3B => return Err(GifError::Decode("no image data".into())),
                0x21 => self.read_extension()?,
                0x2C => {
                    let frame = self.read_frame()?;
                    return Ok(frame);
                }
                _ => return Err(GifError::Decode("unknown block".into())),
            }
        }
    }

    fn read_extension(&mut self) -> Result<(), GifError> {
        if self.pos + 2 > self.data.len() {
            return Err(GifError::Decode("truncated extension".into()));
        }
        let label = self.data[self.pos + 1];
        self.pos += 2;
        if label == 0xF9 {
            // Graphic Control Extension.
            if self.pos + 6 > self.data.len() {
                return Err(GifError::Decode("truncated GCE".into()));
            }
            if self.data[self.pos] != 4 {
                return Err(GifError::Decode("invalid GCE size".into()));
            }
            // Layout: [size][packed][delay_lo][delay_hi][trans][term].
            let packed = self.data[self.pos + 1];
            let index = self.data[self.pos + 4];
            self.gce_transparent =
                if packed & 1 != 0 { Some(index) } else { None };
            self.pos += 6;
        } else {
            skip_sub_blocks(self.data, &mut self.pos)?;
        }
        Ok(())
    }

    fn read_frame(&mut self) -> Result<DecodedGif, GifError> {
        if self.pos + 10 > self.data.len() {
            return Err(GifError::Decode("truncated descriptor".into()));
        }
        let left = u16::from_le_bytes([self.data[self.pos + 1], self.data[self.pos + 2]]) as u32;
        let top = u16::from_le_bytes([self.data[self.pos + 3], self.data[self.pos + 4]]) as u32;
        let fw = u16::from_le_bytes([self.data[self.pos + 5], self.data[self.pos + 6]]) as u32;
        let fh = u16::from_le_bytes([self.data[self.pos + 7], self.data[self.pos + 8]]) as u32;
        let packed = self.data[self.pos + 9];
        self.pos += 10;
        if fw == 0 || fh == 0 {
            return Err(GifError::Decode("zero frame dimension".into()));
        }
        let interlaced = packed & 0x40 != 0;
        let table = if packed & 0x80 != 0 {
            let n = 2usize << (packed & 7);
            Some(read_table(self.data, &mut self.pos, n)?)
        } else {
            self.gct.clone()
        };
        let table = table.ok_or_else(|| GifError::Decode("missing color table".into()))?;
        if self.pos >= self.data.len() {
            return Err(GifError::Decode("missing LZW data".into()));
        }
        let min_size = self.data[self.pos] as u32;
        self.pos += 1;
        if min_size == 0 || min_size > 8 {
            return Err(GifError::Decode("invalid LZW minimum code size".into()));
        }
        let stream = read_sub_blocks(self.data, &mut self.pos)?;
        let indices = lzw_decode(&stream, min_size, fw as usize * fh as usize)?;
        // Composite onto the logical screen (transparent canvas), clipped.
        let mut pixels = vec![0u8; self.width as usize * self.height as usize * 4];
        let trans = self.gce_transparent;
        if interlaced {
            let mut src_row = 0usize;
            for pass in 0..4 {
                let mut y = INTERLACE_START[pass];
                while y < fh as usize {
                    if src_row >= fh as usize {
                        return Err(GifError::Decode("interlace overrun".into()));
                    }
                    put_row(
                        &mut pixels, self.width, self.height,
                        &indices, fw as usize, src_row,
                        &table, trans, left, top + y as u32,
                    );
                    src_row += 1;
                    y += INTERLACE_STEP[pass];
                }
            }
            if src_row != fh as usize {
                return Err(GifError::Decode("interlace underrun".into()));
            }
        } else {
            for y in 0..fh as usize {
                put_row(
                    &mut pixels, self.width, self.height,
                    &indices, fw as usize, y,
                    &table, trans, left, top + y as u32,
                );
            }
        }
        Ok(DecodedGif { width: self.width, height: self.height, pixels })
    }
}

const INTERLACE_START: [usize; 4] = [0, 4, 2, 1];
const INTERLACE_STEP: [usize; 4] = [8, 8, 4, 2];

#[allow(clippy::too_many_arguments)]
fn put_row(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    indices: &[u8],
    fw: usize,
    src_row: usize,
    table: &[[u8; 3]],
    transparent: Option<u8>,
    left: u32,
    canvas_y: u32,
) {
    if canvas_y >= height {
        return;
    }
    let row = &indices[src_row * fw..(src_row + 1) * fw];
    for (dx, &idx) in row.iter().enumerate() {
        let x = left + dx as u32;
        if x >= width {
            continue;
        }
        // Transparent pixels keep their palette RGB with alpha 0
        // (like Pillow and the `image` crate); unknown indices stay
        // transparent black (canvas initial state).
        if let Some(&[r, g, b]) = table.get(idx as usize) {
            let a = if Some(idx) == transparent { 0 } else { 255 };
            let o = (canvas_y as usize * width as usize + x as usize) * 4;
            pixels[o..o + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
}

fn read_table(data: &[u8], pos: &mut usize, n: usize) -> Result<Vec<[u8; 3]>, GifError> {
    if *pos + 3 * n > data.len() {
        return Err(GifError::Decode("truncated color table".into()));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push([data[*pos + 3 * i], data[*pos + 3 * i + 1], data[*pos + 3 * i + 2]]);
    }
    *pos += 3 * n;
    Ok(out)
}

/// Skip extension sub-blocks (terminated by a zero-length block).
fn skip_sub_blocks(data: &[u8], pos: &mut usize) -> Result<(), GifError> {
    loop {
        if *pos >= data.len() {
            return Err(GifError::Decode("truncated extension".into()));
        }
        let n = data[*pos] as usize;
        *pos += 1;
        if n == 0 {
            return Ok(());
        }
        if *pos + n > data.len() {
            return Err(GifError::Decode("truncated extension".into()));
        }
        *pos += n;
    }
}

/// Collect data sub-blocks into one stream.
fn read_sub_blocks(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, GifError> {
    let mut out = Vec::new();
    loop {
        if *pos >= data.len() {
            return Err(GifError::Decode("truncated image data".into()));
        }
        let n = data[*pos] as usize;
        *pos += 1;
        if n == 0 {
            return Ok(out);
        }
        if *pos + n > data.len() {
            return Err(GifError::Decode("truncated image data".into()));
        }
        out.extend_from_slice(&data[*pos..*pos + n]);
        *pos += n;
    }
}

// ── LZW ───────────────────────────────────────────────────────

/// LSB-first bit reader over the LZW stream.
struct LzwReader<'a> {
    data: &'a [u8],
    bitpos: usize,
}

impl<'a> LzwReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bitpos: 0 }
    }
    fn read(&mut self, n: u32) -> Result<u32, GifError> {
        if n == 0 || n > 12 {
            return Err(GifError::Decode("bad code size".into()));
        }
        let mut val = 0u32;
        for i in 0..n {
            let byte = *self.data.get(self.bitpos / 8).ok_or_else(|| {
                GifError::Decode("truncated LZW stream".into())
            })?;
            val |= (((byte >> (self.bitpos % 8)) & 1) as u32) << i;
            self.bitpos += 1;
        }
        Ok(val)
    }
}

/// Output the dictionary string for `code` into `out`.
fn emit_string(
    prefix: &[u16],
    suffix: &[u8],
    clear: u32,
    code: u32,
    stack: &mut [u8; 4096],
    out: &mut Vec<u8>,
) {
    let mut top = 0usize;
    let mut c = code;
    while c >= clear {
        stack[top] = suffix[c as usize];
        top += 1;
        c = prefix[c as usize] as u32;
    }
    stack[top] = c as u8;
    top += 1;
    while top > 0 {
        top -= 1;
        out.push(stack[top]);
    }
}

/// First literal of the dictionary string for `code`.
fn first_char(prefix: &[u16], clear: u32, code: u32) -> u32 {
    let mut c = code;
    while c >= clear {
        c = prefix[c as usize] as u32;
    }
    c
}

/// Decompress GIF LZW into exactly `expect` pixel indices.
fn lzw_decode(stream: &[u8], min_size: u32, expect: usize) -> Result<Vec<u8>, GifError> {
    let clear = 1u32 << min_size;
    let eoi = clear + 1;
    // Dictionary as prefix/suffix chains; entries < clear are literals.
    let mut prefix = vec![0u16; 4096];
    let mut suffix = vec![0u8; 4096];
    for i in 0..clear {
        suffix[i as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(expect);
    let mut stack = [0u8; 4096];
    let mut br = LzwReader::new(stream);
    let mut code_size = min_size + 1;
    let mut next = eoi + 1;
    let mut prev: Option<u32> = None;

    let dbg = std::env::var("GIF_DEBUG").is_ok();
    loop {
        if out.len() > expect {
            return Err(GifError::Decode("too many pixels".into()));
        }
        let code = br.read(code_size)?;
        if dbg {
            eprintln!("DEC code={code} size={code_size} next={next} out={}", out.len());
        }
        if code == clear {
            code_size = min_size + 1;
            next = eoi + 1;
            prev = None;
            continue;
        }
        if code == eoi {
            break;
        }
        if code > 4095 {
            return Err(GifError::Decode("invalid LZW code".into()));
        }
        if prev.is_none() {
            // First code after start/clear must be a literal.
            if code >= clear {
                return Err(GifError::Decode("invalid first code".into()));
            }
            emit_string(&prefix, &suffix, clear, code, &mut stack, &mut out);
            prev = Some(code);
            continue;
        }
        let p = prev.unwrap();
        if code < next {
            emit_string(&prefix, &suffix, clear, code, &mut stack, &mut out);
            let fc = first_char(&prefix, clear, code);
            prefix[next as usize] = p as u16;
            suffix[next as usize] = fc as u8;
            next += 1;
            prev = Some(code);
        } else if code == next {
            // KwKwK case: prev + first char of prev.
            let fc = first_char(&prefix, clear, p);
            emit_string(&prefix, &suffix, clear, p, &mut stack, &mut out);
            out.push(fc as u8);
            prefix[next as usize] = p as u16;
            suffix[next as usize] = fc as u8;
            next += 1;
            prev = Some(code);
        } else {
            return Err(GifError::Decode("invalid LZW code".into()));
        }
        // Widen as soon as `next` no longer fits: the KwKwK code `next`
        // itself is readable, so `next == 2^size` already needs more bits.
        // NOTE: encoder uses `>` here; the asymmetry compensates the
        // decoder learning each entry one code later (see `lzw_encode`).
        if next >= (1 << code_size) && code_size < 12 {
            code_size += 1;
        }
        if out.len() == expect {
            // Pixels complete; trailing EOI/bits are not required.
            return Ok(out);
        }
    }
    if out.len() != expect {
        return Err(GifError::Decode(format!(
            "pixel count mismatch: {} != {expect}",
            out.len()
        )));
    }
    Ok(out)
}

// LSB-first bit writer.
struct LzwWriter {
    out: Vec<u8>,
    acc: u32,
    nbits: u32,
}

impl LzwWriter {
    fn new() -> Self {
        Self { out: Vec::new(), acc: 0, nbits: 0 }
    }
    fn write(&mut self, code: u32, len: u32) {
        self.acc |= code << self.nbits;
        self.nbits += len;
        while self.nbits >= 8 {
            self.out.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

/// Compress pixel indices with GIF LZW.
fn lzw_encode(indices: &[u8], min_size: u32) -> Vec<u8> {
    let clear = 1u32 << min_size;
    let eoi = clear + 1;
    let mut bw = LzwWriter::new();
    if indices.is_empty() {
        bw.write(clear, min_size + 1);
        bw.write(eoi, min_size + 1);
        return bw.finish();
    }
    let mut table: HashMap<u32, u32> = HashMap::new();
    let mut size = min_size + 1;
    let mut next = eoi + 1;
    bw.write(clear, size);
    let mut w = indices[0] as u32;
    for &byte in &indices[1..] {
        let k = byte as u32;
        let key = (w << 8) | k;
        if let Some(&code) = table.get(&key) {
            w = code;
        } else {
            bw.write(w, size);
            if next < 4096 {
                table.insert(key, next);
                next += 1;
                // NOTE: intentionally `>` while the decoder uses `>=`.
                // The decoder assigns each entry one code later (it learns
                // entry N by reading code N), so the decoder must widen
                // one step earlier for both sides to agree. See tests.
                if next > (1 << size) && size < 12 {
                    size += 1;
                }
            } else {
                bw.write(clear, size);
                table.clear();
                size = min_size + 1;
                next = eoi + 1;
            }
            w = k;
        }
    }
    bw.write(w, size);
    bw.write(eoi, size);
    bw.finish()
}

// ── Palette (median-cut) ──────────────────────────────────────

/// Quantize RGBA to a palette. Returns (palette, indices, transparent_index).
/// Transparent pixels (alpha < 128) share one trailing entry when present.
fn quantize(rgba: &[u8]) -> (Vec<[u8; 3]>, Vec<u8>, Option<u8>) {
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    let mut has_trans = false;
    for px in rgba.chunks_exact(4) {
        if px[3] < 128 {
            has_trans = true;
        } else {
            *counts.entry([px[0], px[1], px[2]]).or_insert(0) += 1;
        }
    }
    let slots = if has_trans { 255 } else { 256 } as usize;
    let mut colors: Vec<([u8; 3], u32)> = counts.into_iter().collect();
    // Deterministic order: count desc, then color asc.
    colors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let palette: Vec<[u8; 3]> = if colors.len() <= slots {
        colors.iter().map(|(c, _)| *c).collect()
    } else {
        median_cut(&mut colors, slots)
    };
    let map: HashMap<[u8; 3], u8> =
        palette.iter().enumerate().map(|(i, c)| (*c, i as u8)).collect();
    let trans_idx = has_trans.then(|| palette.len() as u8);
    let mut full_palette = palette;
    if has_trans {
        full_palette.push([0, 0, 0]);
    }
    let indices: Vec<u8> = rgba
        .chunks_exact(4)
        .map(|px| {
            if px[3] < 128 {
                trans_idx.unwrap()
            } else if let Some(&i) = map.get(&[px[0], px[1], px[2]]) {
                i
            } else {
                nearest(&full_palette[..full_palette.len() - has_trans as usize], px)
            }
        })
        .collect();
    (full_palette, indices, trans_idx)
}

/// Nearest palette entry by squared distance.
fn nearest(palette: &[[u8; 3]], px: &[u8]) -> u8 {
    let mut best = 0u8;
    let mut best_d = u32::MAX;
    for (i, c) in palette.iter().enumerate() {
        let d = (c[0] as i32 - px[0] as i32).pow(2) as u32
            + (c[1] as i32 - px[1] as i32).pow(2) as u32
            + (c[2] as i32 - px[2] as i32).pow(2) as u32;
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
    }
    best
}

/// Median-cut `colors` (sorted, nonempty) down to `slots` entries.
fn median_cut(colors: &mut [([u8; 3], u32)], slots: usize) -> Vec<[u8; 3]> {
    // Boxes as index ranges into `colors`; sort in place per split.
    let mut boxes: Vec<(usize, usize)> = vec![(0, colors.len())];
    while boxes.len() < slots {
        // Pick the box with the largest channel range (ties: largest count).
        let mut pick = None;
        let mut pick_score = (0u32, 0u32);
        for (bi, &(s, e)) in boxes.iter().enumerate() {
            if e - s < 2 {
                continue;
            }
            let (mut mins, mut maxs) = ([255u8; 3], [0u8; 3]);
            let mut count = 0u32;
            for (c, n) in &colors[s..e] {
                for ch in 0..3 {
                    mins[ch] = mins[ch].min(c[ch]);
                    maxs[ch] = maxs[ch].max(c[ch]);
                }
                count += *n;
            }
            let range = (0..3).map(|ch| (maxs[ch] - mins[ch]) as u32).max().unwrap();
            let score = (range, count);
            if pick.is_none() || score > pick_score {
                pick = Some(bi);
                pick_score = score;
            }
        }
        let bi = match pick {
            Some(b) => b,
            None => break, // No splittable box left.
        };
        let (s, e) = boxes.remove(bi);
        // Split along the widest channel at the count median.
        let (mut mins, mut maxs) = ([255u8; 3], [0u8; 3]);
        for (c, _) in &colors[s..e] {
            for ch in 0..3 {
                mins[ch] = mins[ch].min(c[ch]);
                maxs[ch] = maxs[ch].max(c[ch]);
            }
        }
        let mut ch = 0;
        for c in 1..3 {
            if maxs[c] - mins[c] > maxs[ch] - mins[ch] {
                ch = c;
            }
        }
        colors[s..e].sort_by(|a, b| a.0[ch].cmp(&b.0[ch]).then(a.0.cmp(&b.0)));
        let total: u32 = colors[s..e].iter().map(|(_, n)| *n).sum();
        let mut acc = 0u32;
        let mut mid = s + 1;
        while mid < e - 1 && acc < total / 2 {
            acc += colors[mid].1;
            mid += 1;
        }
        boxes.push((s, mid));
        boxes.push((mid, e));
    }
    boxes
        .iter()
        .map(|&(s, e)| {
            let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
            for (c, count) in &colors[s..e] {
                r += c[0] as u64 * *count as u64;
                g += c[1] as u64 * *count as u64;
                b += c[2] as u64 * *count as u64;
                n += *count as u64;
            }
            let n = n.max(1);
            [(r / n) as u8, (g / n) as u8, (b / n) as u8]
        })
        .collect()
}

// ── Encoder assembly ──────────────────────────────────────────

fn bits_needed(v: usize) -> u32 {
    (usize::BITS - v.leading_zeros()).max(1)
}

fn encode_indexed(
    width: u32,
    height: u32,
    palette: &[[u8; 3]],
    indices: &[u8],
    transparent: Option<u8>,
) -> Result<Vec<u8>, GifError> {
    // GCT padded up to a power of two (minimum 2 entries).
    let entries = palette.len().next_power_of_two().max(2);
    if entries > 256 || indices.len() != width as usize * height as usize {
        return Err(GifError::Encode("invalid palette or pixels".into()));
    }
    let min_size = bits_needed(palette.len().saturating_sub(1)).max(2).min(8);
    let lzw = lzw_encode(indices, min_size);

    let mut out = Vec::with_capacity(lzw.len() + 64);
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    let size_field = bits_needed(entries - 1) - 1; // 2<<(field) == entries
    out.push(0x80 | (7 << 4) | size_field as u8); // GCT flag, 8-bit color, size
    out.push(0); // background
    out.push(0); // aspect
    for i in 0..entries {
        let c = palette.get(i).copied().unwrap_or([0, 0, 0]);
        out.extend_from_slice(&c);
    }
    if let Some(t) = transparent {
        out.extend_from_slice(&[0x21, 0xF9, 4, 0x01, 0, 0, t, 0]);
    }
    out.push(0x2C); // image separator
    out.extend_from_slice(&[0, 0, 0, 0]); // left, top
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.push(0); // no LCT, no interlace
    out.push(min_size as u8);
    for chunk in lzw.chunks(255) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }
    out.push(0);
    out.push(0x3B); // trailer
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_tables_placeholder() {
        // The median-cut of a single color yields it back exactly.
        let mut colors = [([10, 20, 30], 5)];
        assert_eq!(median_cut(&mut colors, 4), vec![[10, 20, 30]]);
    }

    #[test]
    fn bits_needed_table() {
        assert_eq!(bits_needed(0), 1);
        assert_eq!(bits_needed(1), 1);
        assert_eq!(bits_needed(2), 2);
        assert_eq!(bits_needed(255), 8);
    }
}
