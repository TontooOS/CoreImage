//! Pure-Rust WebP codec (lossy VP8 decode, lossless VP8L decode/encode).
//!
//! Decode supports simple (VP8/VP8L) and extended (VP8X) containers,
//! including alpha (ALPH) and first-frame animation (ANIM/ANMF).
//! Encode writes lossless VP8L. No third-party dependencies.

// -- Public API --

/// Errors of the WebP codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebpError {
    Decode(String),
    Encode(String),
    Unsupported(String),
}

impl std::fmt::Display for WebpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "webp decode error: {m}"),
            Self::Encode(m) => write!(f, "webp encode error: {m}"),
            Self::Unsupported(m) => write!(f, "webp unsupported: {m}"),
        }
    }
}

impl std::error::Error for WebpError {}

/// Decoded WebP image: always RGBA8 pixels.
#[derive(Debug, Clone)]
pub struct DecodedWebp {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// True for RIFF....WEBP files.
pub fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes[0..4] == *b"RIFF" && bytes[8..12] == *b"WEBP"
}

/// Fast probe: canvas dimensions without decoding pixels.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), WebpError> {
    probe(bytes)
}

/// Decode a WebP file into RGBA8 pixels (first frame wins for animation).
pub fn decode(bytes: &[u8]) -> Result<DecodedWebp, WebpError> {
    decode_inner(bytes)
}

/// Encode RGBA8 pixels as lossless VP8L WebP. Quality is validated
/// (1-100) for API parity but does not change lossless output.
pub fn encode(width: u32, height: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, WebpError> {
    if quality == 0 || quality > 100 {
        return Err(WebpError::Encode("quality must be 1-100".into()));
    }
    encode_lossless(width, height, rgba)
}

// -- Container --

fn u16le(b: &[u8]) -> u16 {
    (b[0] as u16) | ((b[1] as u16) << 8)
}
fn u24le(b: &[u8]) -> u32 {
    (b[0] as u32) | ((b[1] as u32) << 8) | ((b[2] as u32) << 16)
}
fn u32le(b: &[u8]) -> u32 {
    (b[0] as u32) | ((b[1] as u32) << 8) | ((b[2] as u32) << 16) | ((b[3] as u32) << 24)
}

struct Chunk<'a> {
    tag: [u8; 4],
    data: &'a [u8],
}

fn tag_eq(tag: &[u8; 4], s: &[u8; 4]) -> bool {
    tag == s
}

fn parse_top(bytes: &[u8]) -> Result<Vec<Chunk<'_>>, WebpError> {
    if !is_webp(bytes) {
        return Err(WebpError::Decode("not a WebP file".into()));
    }
    let mut out = Vec::new();
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let tag = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        let size = u32le(&bytes[pos + 4..pos + 8]) as usize;
        let start = pos + 8;
        let end = start.saturating_add(size).min(bytes.len());
        if start > bytes.len() {
            break;
        }
        out.push(Chunk { tag, data: &bytes[start..end] });
        pos = end;
        // RIFF chunks are padded to even sizes.
        if (size & 1) == 1 {
            pos += 1;
        }
        if pos > bytes.len() {
            break;
        }
    }
    if out.is_empty() {
        return Err(WebpError::Decode("empty WebP container".into()));
    }
    Ok(out)
}

fn parse_subchunks(mut data: &[u8]) -> Result<Vec<Chunk<'_>>, WebpError> {
    let mut out = Vec::new();
    while data.len() >= 8 {
        let tag = [data[0], data[1], data[2], data[3]];
        let size = u32le(&data[4..8]) as usize;
        if data.len() < 8 + size {
            return Err(WebpError::Decode("truncated subchunk".into()));
        }
        out.push(Chunk { tag, data: &data[8..8 + size] });
        let mut next = 8 + size;
        if (size & 1) == 1 {
            next += 1;
        }
        if next > data.len() {
            break;
        }
        data = &data[next..];
    }
    Ok(out)
}

fn probe(bytes: &[u8]) -> Result<(u32, u32), WebpError> {
    let chunks = parse_top(bytes)?;
    if chunks.len() == 1 {
        let c = &chunks[0];
        if tag_eq(&c.tag, b"VP8L") {
            return vp8l_dimensions(c.data);
        }
        if tag_eq(&c.tag, b"VP8 ") {
            return vp8_dimensions(c.data);
        }
    }
    // Extended: VP8X canvas size wins.
    for c in &chunks {
        if tag_eq(&c.tag, b"VP8X") {
            if c.data.len() < 10 {
                return Err(WebpError::Decode("truncated VP8X".into()));
            }
            let w = u24le(&c.data[4..7]) + 1;
            let h = u24le(&c.data[7..10]) + 1;
            if w == 0 || h == 0 {
                return Err(WebpError::Decode("zero canvas dimension".into()));
            }
            return Ok((w, h));
        }
    }
    // Fallback: first VP8/VP8L chunk dims.
    for c in &chunks {
        if tag_eq(&c.tag, b"VP8L") {
            return vp8l_dimensions(c.data);
        }
        if tag_eq(&c.tag, b"VP8 ") {
            return vp8_dimensions(c.data);
        }
    }
    Err(WebpError::Unsupported("unknown WebP chunk layout".into()))
}

fn decode_inner(bytes: &[u8]) -> Result<DecodedWebp, WebpError> {
    let chunks = parse_top(bytes)?;
    if chunks.len() == 1 {
        let c = &chunks[0];
        if tag_eq(&c.tag, b"VP8L") {
            return decode_vp8l_file(c.data);
        }
        if tag_eq(&c.tag, b"VP8 ") {
            return decode_vp8_file(c.data, None);
        }
        return Err(WebpError::Unsupported("unknown WebP chunk".into()));
    }
    decode_extended(&chunks)
}

fn decode_extended(chunks: &[Chunk<'_>]) -> Result<DecodedWebp, WebpError> {
    let mut vp8x: Option<&[u8]> = None;
    for c in chunks {
        if tag_eq(&c.tag, b"VP8X") {
            vp8x = Some(c.data);
            break;
        }
    }
    let Some(vp8x_data) = vp8x else {
        // No VP8X but several chunks: pick first decodable image chunk.
        for c in chunks {
            if tag_eq(&c.tag, b"VP8L") {
                return decode_vp8l_file(c.data);
            }
            if tag_eq(&c.tag, b"VP8 ") {
                return decode_vp8_file(c.data, None);
            }
        }
        return Err(WebpError::Unsupported("unknown WebP chunk layout".into()));
    };
    if vp8x_data.len() < 10 {
        return Err(WebpError::Decode("truncated VP8X".into()));
    }
    let flags = vp8x_data[0];
    let animated = flags & 0x02 != 0;
    let canvas_w = u24le(&vp8x_data[4..7]) + 1;
    let canvas_h = u24le(&vp8x_data[7..10]) + 1;
    if canvas_w == 0 || canvas_h == 0 || canvas_w > 16384 || canvas_h > 16384 {
        return Err(WebpError::Decode("invalid canvas size".into()));
    }
    // Gather payload after VP8X.
    let mut payload: Vec<&Chunk<'_>> = Vec::new();
    let mut seen_vp8x = false;
    for c in chunks {
        if !seen_vp8x {
            if tag_eq(&c.tag, b"VP8X") {
                seen_vp8x = true;
            }
            continue;
        }
        // Skip metadata chunks.
        if tag_eq(&c.tag, b"ICCP")
            || tag_eq(&c.tag, b"EXIF")
            || tag_eq(&c.tag, b"XMP ")
            || tag_eq(&c.tag, b"ANIM")
        {
            continue;
        }
        payload.push(c);
    }
    if animated {
        // First ANMF frame wins.
        for c in payload {
            if tag_eq(&c.tag, b"ANMF") {
                return decode_anmf(c.data, canvas_w, canvas_h);
            }
        }
        return Err(WebpError::Decode("animated WebP without frames".into()));
    }
    // Still extended: optional ALPH + VP8/VP8L.
    let mut alpha: Option<&[u8]> = None;
    let mut bitstream: Option<(&[u8; 4], &[u8])> = None;
    for c in payload {
        if tag_eq(&c.tag, b"ALPH") {
            alpha = Some(c.data);
        } else if tag_eq(&c.tag, b"VP8 ") {
            bitstream = Some((b"VP8 ", c.data));
            break;
        } else if tag_eq(&c.tag, b"VP8L") {
            bitstream = Some((b"VP8L", c.data));
            break;
        }
    }
    let Some((kind, data)) = bitstream else {
        return Err(WebpError::Decode("extended WebP without image".into()));
    };
    if kind == b"VP8L" {
        let img = decode_vp8l_file(data)?;
        // VP8L already carries alpha; canvas should match.
        if img.width == canvas_w && img.height == canvas_h {
            return Ok(img);
        }
        return place_on_canvas(&img.pixels, img.width, img.height, 0, 0, canvas_w, canvas_h);
    }
    decode_vp8_file(data, alpha)
}

fn place_on_canvas(
    px: &[u8],
    w: u32,
    h: u32,
    ox: u32,
    oy: u32,
    cw: u32,
    ch: u32,
) -> Result<DecodedWebp, WebpError> {
    let mut canvas = vec![0u8; (cw as usize) * (ch as usize) * 4];
    for y in 0..h {
        let dy = oy + y;
        if dy >= ch {
            break;
        }
        for x in 0..w {
            let dx = ox + x;
            if dx >= cw {
                break;
            }
            let s = ((y * w + x) as usize) * 4;
            let d = ((dy * cw + dx) as usize) * 4;
            // Alpha-blend over transparent black (premultiplied-safe simple over).
            let sa = px[s + 3] as u32;
            if sa == 255 {
                canvas[d..d + 4].copy_from_slice(&px[s..s + 4]);
            } else if sa != 0 {
                // Blend onto current canvas pixel.
                let da = canvas[d + 3] as u32;
                let out_a = sa + da * (255 - sa) / 255;
                for k in 0..3 {
                    let v = (px[s + k] as u32 * sa
                        + canvas[d + k] as u32 * da * (255 - sa) / 255)
                        / out_a.max(1);
                    canvas[d + k] = v.min(255) as u8;
                }
                canvas[d + 3] = out_a.min(255) as u8;
            }
        }
    }
    Ok(DecodedWebp { width: cw, height: ch, pixels: canvas })
}

fn decode_anmf(data: &[u8], cw: u32, ch: u32) -> Result<DecodedWebp, WebpError> {
    if data.len() < 16 {
        return Err(WebpError::Decode("truncated ANMF".into()));
    }
    let fx = u24le(&data[0..3]) * 2;
    let fy = u24le(&data[3..6]) * 2;
    let fw = u24le(&data[6..9]) + 1;
    let fh = u24le(&data[9..12]) + 1;
    let subs = parse_subchunks(&data[16..])?;
    let mut alpha: Option<&[u8]> = None;
    let mut bitstream: Option<(&[u8; 4], &[u8])> = None;
    for c in &subs {
        if tag_eq(&c.tag, b"ALPH") {
            alpha = Some(c.data);
        } else if tag_eq(&c.tag, b"VP8 ") {
            bitstream = Some((b"VP8 ", c.data));
            break;
        } else if tag_eq(&c.tag, b"VP8L") {
            bitstream = Some((b"VP8L", c.data));
            break;
        }
    }
    let Some((kind, bs)) = bitstream else {
        return Err(WebpError::Decode("ANMF without image".into()));
    };
    if kind == b"VP8L" {
        let img = decode_vp8l_data(bs, Some((fw, fh)))?;
        return place_on_canvas(&img.pixels, img.width, img.height, fx, fy, cw, ch);
    }
    // Lossy frame: decode then place. Alpha subchunk dimension must match frame.
    let img = decode_vp8_data(bs, alpha, Some((fw, fh)))?;
    place_on_canvas(&img.pixels, img.width, img.height, fx, fy, cw, ch)
}

// ---- VP8L lossless ----

fn vp8l_dimensions(data: &[u8]) -> Result<(u32, u32), WebpError> {
    if data.len() < 5 || data[0] != 0x2f {
        return Err(WebpError::Decode("bad VP8L signature".into()));
    }
    let b1 = data[1] as u32;
    let b2 = data[2] as u32;
    let b3 = data[3] as u32;
    let b4 = data[4] as u32;
    let w = 1 + (((b2 & 0x3f) << 8) | b1);
    let h = 1 + (((b4 & 0x0f) << 10) | (b3 << 2) | ((b2 & 0xc0) >> 6));
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err(WebpError::Decode("invalid VP8L size".into()));
    }
    Ok((w, h))
}

fn decode_vp8l_file(data: &[u8]) -> Result<DecodedWebp, WebpError> {
    decode_vp8l_data(data, None)
}

fn decode_vp8l_data(data: &[u8], expected: Option<(u32, u32)>) -> Result<DecodedWebp, WebpError> {
    let mut r = LsbReader::new(data);
    let sig = r.read(8)?;
    if sig != 0x2f {
        return Err(WebpError::Decode("bad VP8L signature".into()));
    }
    let w = r.read(14)? + 1;
    let h = r.read(14)? + 1;
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err(WebpError::Decode("invalid VP8L size".into()));
    }
    if let Some((ew, eh)) = expected {
        if ew != w || eh != h {
            return Err(WebpError::Decode("VP8L size mismatch".into()));
        }
    }
    let _alpha_used = r.read(1)?;
    let ver = r.read(3)?;
    if ver != 0 {
        return Err(WebpError::Decode("bad VP8L version".into()));
    }
    let mut dec = Vp8lDecoder { r };
    let (tw, transforms) = dec.read_transforms(w, h)?;
    let raw = dec.decode_stream(tw, h, true, Vec::new())?;
    if std::env::var("WEBP_DEBUG").is_ok() {
        eprintln!("main raw {}x{} first {:?}", tw, h, &raw[..raw.len().min(40)]);
    }
    let out = finish_vp8l(w, h, tw, &transforms, raw)?;
    // out holds ARGB u32 pixels in scan order.
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    for (i, p) in out.iter().enumerate() {
        rgba[i * 4] = ((p >> 16) & 0xff) as u8;
        rgba[i * 4 + 1] = ((p >> 8) & 0xff) as u8;
        rgba[i * 4 + 2] = (p & 0xff) as u8;
        rgba[i * 4 + 3] = ((p >> 24) & 0xff) as u8;
    }
    Ok(DecodedWebp { width: w, height: h, pixels: rgba })
}

struct LsbReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> LsbReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }
    fn read(&mut self, n: u32) -> Result<u32, WebpError> {
        if n > 24 {
            return Err(WebpError::Decode("bit read overflow".into()));
        }
        let mut v: u32 = 0;
        for i in 0..n {
            let p = self.bit + i as usize;
            let byte = p / 8;
            if byte >= self.data.len() {
                return Err(WebpError::Decode("truncated VP8L".into()));
            }
            let b = (self.data[byte] >> (p % 8)) & 1;
            v |= (b as u32) << i;
        }
        self.bit += n as usize;
        Ok(v)
    }
}

const CODE_ORDER: [usize; 19] = [17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const DIST_MAP: [(i8, i8); 120] = [
    (0, 1), (1, 0), (1, 1), (-1, 1), (0, 2), (2, 0), (1, 2), (-1, 2),
    (2, 1), (-2, 1), (2, 2), (-2, 2), (0, 3), (3, 0), (1, 3), (-1, 3),
    (3, 1), (-3, 1), (2, 3), (-2, 3), (3, 2), (-3, 2), (0, 4), (4, 0),
    (1, 4), (-1, 4), (4, 1), (-4, 1), (3, 3), (-3, 3), (2, 4), (-2, 4),
    (4, 2), (-4, 2), (0, 5), (3, 4), (-3, 4), (4, 3), (-4, 3), (5, 0),
    (1, 5), (-1, 5), (5, 1), (-5, 1), (2, 5), (-2, 5), (5, 2), (-5, 2),
    (4, 4), (-4, 4), (3, 5), (-3, 5), (5, 3), (-5, 3), (0, 6), (6, 0),
    (1, 6), (-1, 6), (6, 1), (-6, 1), (2, 6), (-2, 6), (6, 2), (-6, 2),
    (4, 5), (-4, 5), (5, 4), (-5, 4), (3, 6), (-3, 6), (6, 3), (-6, 3),
    (0, 7), (7, 0), (1, 7), (-1, 7), (5, 5), (-5, 5), (7, 1), (-7, 1),
    (4, 6), (-4, 6), (6, 4), (-6, 4), (2, 7), (-2, 7), (7, 2), (-7, 2),
    (3, 7), (-3, 7), (7, 3), (-7, 3), (5, 6), (-5, 6), (6, 5), (-6, 5),
    (8, 0), (4, 7), (-4, 7), (7, 4), (-7, 4), (8, 1), (8, 2), (6, 6),
    (-6, 6), (8, 3), (5, 7), (-5, 7), (7, 5), (-7, 5), (8, 4), (6, 7),
    (-6, 7), (7, 6), (-7, 6), (8, 5), (7, 7), (-7, 7), (8, 6), (8, 7),
];

#[derive(Clone)]
struct HuffNode {
    child: [i32; 2],
    sym: i32,
}

#[derive(Clone)]
struct HuffTable {
    nodes: Vec<HuffNode>,
    single: Option<u16>,
}

fn reverse_bits_len(mut code: u32, len: u16) -> u32 {
    let mut r = 0u32;
    for _ in 0..len {
        r = (r << 1) | (code & 1);
        code >>= 1;
    }
    r
}

impl HuffTable {
    fn build(lengths: &[u16]) -> Result<Self, WebpError> {
        let mut count = 0u32;
        let mut max_len: u16 = 0;
        for &l in lengths {
            if l != 0 {
                count += 1;
                max_len = max_len.max(l);
            }
        }
        if count == 0 {
            return Err(WebpError::Decode("empty huffman".into()));
        }
        if count == 1 {
            let sym = lengths.iter().position(|&x| x != 0).unwrap() as u16;
            return Ok(Self { nodes: vec![], single: Some(sym) });
        }
        if max_len > 15 {
            return Err(WebpError::Decode("bad huffman length".into()));
        }
        // Canonical codes: group by length, symbols in index order.
        let mut hist = [0u32; 16];
        for &l in lengths {
            if l != 0 {
                hist[l as usize] += 1;
            }
        }
        let mut code: u32 = 0;
        let mut next: [u32; 16] = [0; 16];
        for len in 1..16 {
            next[len] = code;
            code = (code + hist[len]) << 1;
        }
        // Loop runs to 16, so a full tree sums to 2^16.
        if code != (1u32 << 16) {
            return Err(WebpError::Decode("bad huffman tree".into()));
        }
        // Build LSB-first binary tree from reversed canonical codes.
        let mut nodes = vec![HuffNode { child: [-1, -1], sym: -1 }];
        for len in 1..16u16 {
            for (sym, &l) in lengths.iter().enumerate() {
                if l != len {
                    continue;
                }
                let c = next[len as usize];
                next[len as usize] += 1;
                if c >= (1 << len) {
                    return Err(WebpError::Decode("bad huffman code".into()));
                }
                let wire = reverse_bits_len(c, len);
                let mut idx = 0usize;
                for k in 0..len {
                    let bit = ((wire >> k) & 1) as usize;
                    if k + 1 == len {
                        if nodes[idx].child[bit] != -1 || nodes[idx].sym != -1 {
                            // Leaf position already taken.
                            return Err(WebpError::Decode("bad huffman tree".into()));
                        }
                        // Store leaf as child link with bias to disambiguate.
                        let leaf = nodes.len() as i32;
                        nodes.push(HuffNode { child: [-1, -1], sym: sym as i32 });
                        nodes[idx].child[bit] = leaf | (1 << 30);
                    } else {
                        let nxt = nodes[idx].child[bit];
                        if nxt == -1 {
                            let ni = nodes.len() as i32;
                            nodes.push(HuffNode { child: [-1, -1], sym: -1 });
                            nodes[idx].child[bit] = ni;
                            idx = ni as usize;
                        } else if nxt & (1 << 30) != 0 {
                            return Err(WebpError::Decode("bad huffman tree".into()));
                        } else {
                            idx = nxt as usize;
                        }
                    }
                }
            }
        }
        Ok(Self { nodes, single: None })
    }

    fn read(&self, r: &mut LsbReader<'_>) -> Result<u16, WebpError> {
        if let Some(s) = self.single {
            return Ok(s);
        }
        let mut idx = 0usize;
        loop {
            let bit: u32 = r.read(1)?;
            let nxt = self.nodes[idx].child[bit as usize];
            if nxt == -1 {
                return Err(WebpError::Decode("bad huffman code".into()));
            }
            if nxt & (1 << 30) != 0 {
                let leaf = (nxt & !(1 << 30)) as usize;
                let s = self.nodes[leaf].sym;
                if s < 0 {
                    return Err(WebpError::Decode("bad huffman code".into()));
                }
                return Ok(s as u16);
            }
            idx = nxt as usize;
            if idx >= self.nodes.len() {
                return Err(WebpError::Decode("bad huffman code".into()));
            }
        }
    }
}

// ---- VP8L image stream ----

fn read_prefix_lengths(
    r: &mut LsbReader<'_>,
    alphabet: u16,
    _cache_bits: u32,
) -> Result<Vec<u16>, WebpError> {
    let simple = r.read(1)?;
    if simple == 1 {
        let n = r.read(1)? + 1;
        let first_len = r.read(1)?; // 0 => 1 bit, 1 => 8 bits
        let mut lens = vec![0u16; alphabet as usize];
        let s0 = if first_len == 1 { r.read(8)? } else { r.read(1)? };
        if (s0 as usize) >= lens.len() {
            return Err(WebpError::Decode("bad simple huffman".into()));
        }
        lens[s0 as usize] = 1;
        if n == 2 {
            let s1 = r.read(8)?;
            if (s1 as usize) >= lens.len() {
                return Err(WebpError::Decode("bad simple huffman".into()));
            }
            lens[s1 as usize] = 1;
        }
        return Ok(lens);
    }
    let num_lens = r.read(4)? + 4;
    if num_lens as usize > CODE_ORDER.len() {
        return Err(WebpError::Decode("bad code length count".into()));
    }
    let mut cl_lens = [0u16; 19];
    for i in 0..num_lens as usize {
        cl_lens[CODE_ORDER[i]] = r.read(3)? as u16;
    }
    let cl_table = HuffTable::build(&cl_lens)?;
    // Number of RLE ops to read (count semantics, not position bound).
    let mut remaining: u32 = if r.read(1)? == 0 {
        alphabet as u32
    } else {
        let lb = r.read(3)? as u32;
        let nbits = 2 + 2 * lb;
        if nbits > 12 {
            return Err(WebpError::Decode("bad length bits".into()));
        }
        2 + r.read(nbits)?
    };
    if remaining == 0 || remaining > alphabet as u32 {
        return Err(WebpError::Decode("bad max symbol".into()));
    }
    let mut lens = vec![0u16; alphabet as usize];
    let mut i = 0usize;
    let mut prev: u16 = 8;
    while (i as u32) < alphabet as u32 && remaining > 0 {
        remaining -= 1;
        let sym = cl_table.read(r)?;
        match sym {
            0..=15 => {
                lens[i] = sym;
                if sym != 0 {
                    prev = sym;
                }
                i += 1;
            }
            16 => {
                let rep = 3 + r.read(2)?;
                for _ in 0..rep {
                    if i >= lens.len() {
                        return Err(WebpError::Decode("bad length repeat".into()));
                    }
                    lens[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let rep = 3 + r.read(3)?;
                for _ in 0..rep {
                    if i >= lens.len() {
                        return Err(WebpError::Decode("bad length repeat".into()));
                    }
                    lens[i] = 0;
                    i += 1;
                }
            }
            18 => {
                let rep = 11 + r.read(7)?;
                for _ in 0..rep {
                    if i >= lens.len() {
                        return Err(WebpError::Decode("bad length repeat".into()));
                    }
                    lens[i] = 0;
                    i += 1;
                }
            }
            _ => return Err(WebpError::Decode("bad length symbol".into())),
        }
    }
    Ok(lens)
}

fn lz_value(prefix: u32, extra: u32) -> u32 {
    if prefix < 4 {
        return prefix + 1;
    }
    let eb = ((prefix - 2) >> 1) as u32;
    let off = (2 + (prefix & 1)) << eb;
    off + extra + 1
}

struct Vp8lDecoder<'a> {
    r: LsbReader<'a>,
}

#[derive(Clone)]
enum Transform {
    Predictor { bits: u32, data: Vec<u32> },
    Color { bits: u32, data: Vec<(u8, u8, u8)> },
    SubtractGreen,
    ColorIndex { table: Vec<u32>, width_bits: u32, orig_w: u32 },
}

impl<'a> Vp8lDecoder<'a> {
    fn read_transforms(&mut self, w: u32, h: u32) -> Result<(u32, Vec<Transform>), WebpError> {
        let mut list: Vec<Transform> = Vec::new();
        let mut cur_w = w;
        let mut seen = [false; 4];
        while self.r.read(1)? == 1 {
            let t = self.r.read(2)?;
            if seen[t as usize] {
                return Err(WebpError::Decode("duplicate transform".into()));
            }
            seen[t as usize] = true;
            match t {
                0 => {
                    let bits = self.r.read(3)? + 2;
                    let sw = div_up(cur_w, 1 << bits);
                    let sh = div_up(h, 1 << bits);
                    let px = self.decode_stream(sw, sh, false, Vec::new())?;
                    if std::env::var("WEBP_DEBUG").is_ok() {
                        eprintln!("pred sub {}x{} bitpos {} px {:?}", sw, sh, self.r.bit, &px[..px.len().min(16)]);
                    }
                    list.push(Transform::Predictor { bits, data: px });
                }
                1 => {
                    let bits = self.r.read(3)? + 2;
                    let sw = div_up(cur_w, 1 << bits);
                    let sh = div_up(h, 1 << bits);
                    let px = self.decode_stream(sw, sh, false, Vec::new())?;
                    // Stored as ARGB: A=255, R=red_to_blue, G=green_to_blue, B=green_to_red.
                    let mut fixed = Vec::with_capacity(px.len());
                    for &p in &px {
                        let rtb = ((p >> 16) & 0xff) as u8;
                        let gtb = ((p >> 8) & 0xff) as u8;
                        let gtr = (p & 0xff) as u8;
                        fixed.push((gtr, gtb, rtb));
                    }
                    if std::env::var("WEBP_DEBUG").is_ok() {
                        eprintln!("color sub {}x{} bitpos {} data {:?}", sw, sh, self.r.bit, &fixed[..fixed.len().min(16)]);
                    }
                    list.push(Transform::Color { bits, data: fixed });
                }
                2 => list.push(Transform::SubtractGreen),                3 => {
                    let size = self.r.read(8)? + 1;
                    if size > 256 {
                        return Err(WebpError::Decode("bad color table".into()));
                    }
                    let mut px = self.decode_stream(size, 1, false, Vec::new())?;
                    // Subtraction-coded palette.
                    for i in 1..px.len() {
                        px[i] = add_argb(px[i], px[i - 1]);
                    }
                    let wb = if size <= 2 {
                        3
                    } else if size <= 4 {
                        2
                    } else if size <= 16 {
                        1
                    } else {
                        0
                    };
                    let orig_w = cur_w;
                    cur_w = div_up(cur_w, 1 << wb);
                    list.push(Transform::ColorIndex { table: px, width_bits: wb, orig_w });
                }
                _ => unreachable!(),
            }
            if list.len() > 4 {
                return Err(WebpError::Decode("too many transforms".into()));
            }
        }
        Ok((cur_w, list))
    }

    fn decode_stream(
        &mut self,
        w: u32,
        h: u32,
        is_main: bool,
        transforms: Vec<Transform>,
    ) -> Result<Vec<u32>, WebpError> {
        let npix = (w as usize).checked_mul(h as usize).ok_or_else(|| WebpError::Decode("image too large".into()))?;
        if npix == 0 {
            return Err(WebpError::Decode("zero image dimension".into()));
        }
        if npix > (1usize << 28) {
            return Err(WebpError::Decode("image too large".into()));
        }
        // Color cache.
        let use_cache = self.r.read(1)?;
        let cache_bits = if use_cache == 1 { self.r.read(4)? } else { 0 };
        if cache_bits > 11 || (use_cache == 0 && cache_bits != 0) {
            return Err(WebpError::Decode("bad color cache".into()));
        }
        if use_cache == 1 && cache_bits == 0 {
            return Err(WebpError::Decode("bad color cache".into()));
        }
        let cache_size = if use_cache == 1 { 1usize << cache_bits } else { 0 };
        // Meta prefix?
        let mut groups: Vec<[HuffTable; 5]> = Vec::new();
        let mut entropy_map: Vec<u16> = Vec::new();
        let mut entropy_w = 0u32;
        let mut entropy_pbits = 0u32;
        if is_main {
            let use_meta = self.r.read(1)?;
            if use_meta == 1 {
                let pbits = self.r.read(3)? + 2;
                entropy_pbits = pbits;
                entropy_w = div_up(w, 1 << pbits);
                let entropy_h = div_up(h, 1 << pbits);
                let entropy_px = self.decode_stream(entropy_w, entropy_h, false, Vec::new())?;
                let mut max_code = 0u32;
                entropy_map.reserve(entropy_px.len());
                for p in &entropy_px {
                    let code = (p >> 8) & 0xffff;
                    entropy_map.push(code as u16);
                    max_code = max_code.max(code);
                }
                let ngroups = (max_code + 1) as usize;
                if ngroups > 256 {
                    return Err(WebpError::Decode("too many prefix groups".into()));
                }
                for _ in 0..ngroups {
                    groups.push(self.read_group(cache_bits)?);
                }
                let _ = pbits;
            } else {
                groups.push(self.read_group(cache_bits)?);
            }
        } else {
            groups.push(self.read_group(cache_bits)?);
        }
        // Pixel loop.
        let mut out: Vec<u32> = vec![0; npix];
        let mut cache = vec![0u32; cache_size];
        if std::env::var("WEBP_DEBUG").is_ok() {
            eprintln!("pixel start bitpos {}", self.r.bit);
        }
        let single_group = groups.len() == 1 && entropy_map.is_empty();
        let mut idx = 0usize;
        while idx < npix {
            let g: &[HuffTable; 5] = if single_group {
                &groups[0]
            } else {
                let x = (idx as u32) % w;
                let y = (idx as u32) / w;
                let ex = ((x >> entropy_pbits) as usize).min(entropy_w.saturating_sub(1) as usize);
                let ey = (y >> entropy_pbits) as usize;
                let mi = ey * entropy_w as usize + ex;
                let code = entropy_map[mi.min(entropy_map.len() - 1)] as usize;
                &groups[code.min(groups.len() - 1)]
            };
            let sym = g[0].read(&mut self.r)?;
            if (sym as u32) < 256 {
                let green = sym as u32;
                let red = g[1].read(&mut self.r)? as u32;
                let blue = g[2].read(&mut self.r)? as u32;
                let alpha = g[3].read(&mut self.r)? as u32;
                let px = (alpha << 24) | (red << 16) | (green << 8) | blue;
                out[idx] = px;
                if use_cache == 1 {
                    cache[hash_color(px, cache_bits) as usize] = px;
                }
                idx += 1;
            } else if (sym as u32) < 256 + 24 {
                let prefix = sym as u32 - 256;
                let eb = if prefix < 4 { 0 } else { ((prefix - 2) >> 1) as u32 };
                let extra = if eb == 0 { 0 } else { self.r.read(eb)? };
                let length = lz_value(prefix, extra);
                let dsym = g[4].read(&mut self.r)? as u32;
                if dsym >= 40 {
                    return Err(WebpError::Decode("bad distance".into()));
                }
                let deb = if dsym < 4 { 0 } else { ((dsym - 2) >> 1) as u32 };
                let dextra = if deb == 0 { 0 } else { self.r.read(deb)? };
                let dist = map_distance(lz_value(dsym, dextra), w)?;
                if dist == 0 || dist as usize > idx {
                    return Err(WebpError::Decode("bad LZ77 distance".into()));
                }
                if length > 4096 {
                    return Err(WebpError::Decode("bad LZ77 length".into()));
                }
                for _ in 0..length {
                    if idx >= npix {
                        return Err(WebpError::Decode("LZ77 overflow".into()));
                    }
                    let v = out[idx - dist as usize];
                    out[idx] = v;
                    if use_cache == 1 {
                        cache[hash_color(v, cache_bits) as usize] = v;
                    }
                    idx += 1;
                }
            } else {
                if use_cache != 1 {
                    return Err(WebpError::Decode("bad cache index".into()));
                }
                let cidx = sym as usize - 256 - 24;
                if cidx >= cache.len() {
                    return Err(WebpError::Decode("bad cache index".into()));
                }
                out[idx] = cache[cidx];
                idx += 1;
            }
        }
        // Transforms are applied by the caller in reverse order.
        // Here `transforms` is only populated for the non-main path (empty).
        // Main-path transforms are handled in decode_vp8l_data.
        let _ = transforms;
        Ok(out)
    }

    fn read_group(&mut self, cache_bits: u32) -> Result<[HuffTable; 5], WebpError> {
        let green_size = 256 + 24 + if cache_bits > 0 { 1 << cache_bits } else { 0 };
        let mut tables = Vec::with_capacity(5);
        let sizes = [green_size, 256, 256, 256, 40];
        for (k, &sz) in sizes.iter().enumerate() {
            if std::env::var("WEBP_DEBUG").is_ok() {
                eprintln!("read table {k} alphabet {sz} at bit {}", self.r.bit);
            }
            let lens = read_prefix_lengths(&mut self.r, sz as u16, if k == 0 { cache_bits } else { 0 })?;
            if std::env::var("WEBP_DEBUG").is_ok() {
                let nz: Vec<(usize, u16)> =
                    lens.iter().enumerate().filter(|&(_, &l)| l != 0).map(|(i, &l)| (i, l)).collect();
                eprintln!("table {k} alphabet {sz} nonzero {nz:?}");
            }
            tables.push(HuffTable::build(&lens)?);
        }
        Ok([tables.remove(0), tables.remove(0), tables.remove(0), tables.remove(0), tables.remove(0)])
    }
}

fn div_up(a: u32, b: u32) -> u32 {
    (a + b - 1) / b
}

fn add_argb(a: u32, b: u32) -> u32 {
    let a0 = (a & 0xff).wrapping_add(b & 0xff) & 0xff;
    let a1 = ((a >> 8) & 0xff).wrapping_add((b >> 8) & 0xff) & 0xff;
    let a2 = ((a >> 16) & 0xff).wrapping_add((b >> 16) & 0xff) & 0xff;
    let a3 = ((a >> 24) & 0xff).wrapping_add((b >> 24) & 0xff) & 0xff;
    (a3 << 24) | (a2 << 16) | (a1 << 8) | a0
}

fn hash_color(px: u32, bits: u32) -> u32 {
    (((px.wrapping_mul(0x1e35_a7bd)) >> (32 - bits)) & ((1 << bits) - 1)) as u32
}

fn map_distance(code: u32, width: u32) -> Result<u32, WebpError> {
    if code <= 120 {
        let (dx, dy) = DIST_MAP[(code - 1) as usize];
        let d = dx as i32 + dy as i32 * width as i32;
        Ok(d.max(1) as u32)
    } else {
        Ok(code - 120)
    }
}

fn avg2(a: u32, b: u32) -> u32 {
    (a + b) / 2
}

fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

fn predictor_of(mode: u32, l: u32, t: u32, tl: u32, tr: u32) -> u32 {
    // Mode 0 is solid black 0xff000000.
    if mode == 0 {
        return 0xff00_0000;
    }
    // Mode 11 (Select) picks ONE neighbor for all channels based on the
    // joint Manhattan distance over ARGB (not per channel).
    if mode == 11 {
        let la = (l >> 24) & 0xff;
        let lr = (l >> 16) & 0xff;
        let lg = (l >> 8) & 0xff;
        let lb = l & 0xff;
        let ta = (t >> 24) & 0xff;
        let trr = (t >> 16) & 0xff;
        let tg = (t >> 8) & 0xff;
        let tb = t & 0xff;
        let tla = (tl >> 24) & 0xff;
        let tlr = (tl >> 16) & 0xff;
        let tlg = (tl >> 8) & 0xff;
        let tlb = tl & 0xff;
        let pa = la as i32 + ta as i32 - tla as i32;
        let pr = lr as i32 + trr as i32 - tlr as i32;
        let pg = lg as i32 + tg as i32 - tlg as i32;
        let pb = lb as i32 + tb as i32 - tlb as i32;
        let pl = (pa - la as i32).abs()
            + (pr - lr as i32).abs()
            + (pg - lg as i32).abs()
            + (pb - lb as i32).abs();
        let pt = (pa - ta as i32).abs()
            + (pr - trr as i32).abs()
            + (pg - tg as i32).abs()
            + (pb - tb as i32).abs();
        if pl < pt {
            return l;
        } else {
            return t;
        }
    }
    let la = (l >> 24) & 0xff;
    let lr = (l >> 16) & 0xff;
    let lg = (l >> 8) & 0xff;
    let lb = l & 0xff;
    let ta = (t >> 24) & 0xff;
    let trr = (t >> 16) & 0xff;
    let tg = (t >> 8) & 0xff;
    let tb = t & 0xff;
    let tla = (tl >> 24) & 0xff;
    let tlr = (tl >> 16) & 0xff;
    let tlg = (tl >> 8) & 0xff;
    let tlb = tl & 0xff;
    let tra = (tr >> 24) & 0xff;
    let trr2 = (tr >> 16) & 0xff;
    let trg = (tr >> 8) & 0xff;
    let trb = tr & 0xff;
    let ch = |a: u32, b: u32, c: u32, d: u32| -> u8 {
        match mode {
            1 => a as u8,
            2 => b as u8,
            3 => d as u8,
            4 => c as u8,
            5 => avg2(avg2(a, d), b) as u8,
            6 => avg2(a, c) as u8,
            7 => avg2(a, b) as u8,
            8 => avg2(c, b) as u8,
            9 => avg2(b, d) as u8,
            10 => avg2(avg2(a, c), avg2(b, d)) as u8,
            // 11 (Select) is handled above (joint decision); unreachable here.
            12 => clamp8(a as i32 + b as i32 - c as i32),
            _ => 0,
        }
    };
    // Mode 13 per spec: ClampAddSubtractHalf(Average2(L,T), TL).
    let m13 = |a: u32, b: u32, c: u32| -> u8 {
        let avg = avg2(a, b) as i32;
        clamp8(avg + (avg - c as i32) / 2)
    };
    let (pa, pr, pg, pb) = if mode == 13 {
        (m13(la, ta, tla), m13(lr, trr, tlr), m13(lg, tg, tlg), m13(lb, tb, tlb))
    } else {
        (ch(la, ta, tla, tra), ch(lr, trr, tlr, trr2), ch(lg, tg, tlg, trg), ch(lb, tb, tlb, trb))
    };
    // Mode 0 is solid black 0xff000000.
    if mode == 0 {
        return 0xff00_0000;
    }
    ((pa as u32) << 24) | ((pr as u32) << 16) | ((pg as u32) << 8) | pb as u32
}

fn apply_predictor(
    img: &mut [u32],
    w: u32,
    h: u32,
    bits: u32,
    pred: &[u32],
) -> Result<(), WebpError> {
    let tw = div_up(w, 1 << bits);
    if pred.len() < (tw as usize) * div_up(h, 1 << bits) as usize {
        return Err(WebpError::Decode("bad predictor data".into()));
    }
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let bi = ((y >> bits) * tw + (x >> bits)) as usize;
            let mode = ((pred[bi] >> 8) & 0xff) % 14;
            let (l, t, tl, tr) = if x == 0 && y == 0 {
                (0xff00_0000, 0xff00_0000, 0xff00_0000, 0xff00_0000)
            } else if y == 0 {
                let l = img[idx - 1];
                (l, l, l, l)
            } else if x == 0 {
                let t = img[idx - w as usize];
                (t, t, t, t)
            } else {
                let l = img[idx - 1];
                let t = img[idx - w as usize];
                let tl = img[idx - w as usize - 1];
                // Rightmost column uses leftmost pixel of same row as TR.
                let tr = if x + 1 >= w { img[(y * w) as usize] } else { img[idx - w as usize + 1] };
                (l, t, tl, tr)
            };
            let p = if x == 0 && y == 0 {
                0xff00_0000
            } else if mode == 0 {
                // Border pixels ignore the mode: top row uses L, left
                // column uses T (top-left handled above).
                if y == 0 { l } else { t }
            } else {
                predictor_of(mode, l, t, tl, tr)
            };
            img[idx] = add_argb(img[idx], p);
        }
    }
    Ok(())
}

fn cdelta(t: u8, c: u8) -> i32 {
    let ts = (t as i8) as i32;
    let cs = (c as i8) as i32;
    (ts * cs) >> 5
}

fn apply_color(
    img: &mut [u32],
    w: u32,
    h: u32,
    bits: u32,
    data: &[(u8, u8, u8)],
) -> Result<(), WebpError> {
    let tw = div_up(w, 1 << bits);
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let bi = ((y >> bits) * tw + (x >> bits)) as usize;
            if bi >= data.len() {
                return Err(WebpError::Decode("bad color data".into()));
            }
            let (gtr, gtb, rtb) = data[bi];
            let p = img[idx];
            let g = ((p >> 8) & 0xff) as u8;
            let r = ((p >> 16) & 0xff) as u8;
            let b = (p & 0xff) as u8;
            let tmp_r = (r as i32 + cdelta(gtr, g)) & 0xff;
            let tmp_b = (b as i32 + cdelta(gtb, g) + cdelta(rtb, tmp_r as u8)) & 0xff;
            img[idx] = (p & 0xff00_0000) | ((tmp_r as u32) << 16) | (p & 0x0000_ff00) | tmp_b as u32;
            let _ = g;
        }
    }
    Ok(())
}

fn apply_subtract_green(img: &mut [u32]) {
    for p in img.iter_mut() {
        let g = ((*p >> 8) & 0xff) as u32;
        let r = (((*p >> 16) & 0xff) + g) & 0xff;
        let b = ((*p & 0xff) + g) & 0xff;
        *p = (*p & 0xff00_ff00) | (r << 16) | b;
    }
}

fn apply_color_index(
    img: &[u32],
    sw: u32,
    h: u32,
    table: &[u32],
    width_bits: u32,
    orig_w: u32,
) -> Result<Vec<u32>, WebpError> {
    let n = table.len();
    if width_bits == 0 {
        if img.len() != (sw as usize) * (h as usize) {
            return Err(WebpError::Decode("bad indexed size".into()));
        }
        let mut out = Vec::with_capacity(img.len());
        for &p in img {
            let gi = ((p >> 8) & 0xff) as usize;
            out.push(if gi < n { table[gi] } else { 0 });
        }
        return Ok(out);
    }
    let per = 1u32 << width_bits;
    let bpp = 8 >> width_bits;
    let mask = (1u32 << bpp) - 1;
    let mut out = Vec::with_capacity((orig_w as usize) * (h as usize));
    let mut pos = 0usize;
    for _ in 0..h {
        let mut row: Vec<u32> = Vec::with_capacity((sw * per) as usize);
        for _ in 0..sw {
            if pos >= img.len() {
                return Err(WebpError::Decode("bad indexed size".into()));
            }
            let g = ((img[pos] >> 8) & 0xff) as u32;
            pos += 1;
            for k in 0..per {
                let gi = ((g >> (k * bpp)) & mask) as usize;
                row.push(if gi < n { table[gi] } else { 0 });
            }
        }
        row.truncate(orig_w as usize);
        if row.len() != orig_w as usize {
            return Err(WebpError::Decode("bad indexed size".into()));
        }
        out.extend_from_slice(&row);
    }
    Ok(out)
}

// ---- VP8 lossy (keyframes only) ----
use super::webp_vp8_tables as VT;

type Vp8Prob = u8;

const SEG_ID_TREE: [i8; 6] = [2, 4, -0, -1, -2, -3];
const YMODE_TREE: [i8; 8] = [-4, 2, 4, 6, -0, -1, -2, -3];
const YMODE_PROBS: [Vp8Prob; 4] = [145, 156, 163, 128];
const BPRED_TREE: [i8; 18] = [
    -0, 2, -1, 4, -2, 6, 8, 12, -3, 10, -5, -6, -4, 14, -7, 16, -8, -9,
];
const UVMODE_TREE: [i8; 6] = [-0, 2, -1, 4, -2, -3];
const UVMODE_PROBS: [Vp8Prob; 3] = [142, 114, 183];
const ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

fn vp8_dimensions(data: &[u8]) -> Result<(u32, u32), WebpError> {
    let (w, h, _) = vp8_frame_size(data)?;
    Ok((w, h))
}

fn vp8_frame_size(data: &[u8]) -> Result<(u32, u32, bool), WebpError> {
    if data.len() < 10 {
        return Err(WebpError::Decode("truncated VP8".into()));
    }
    let tag = u32le(&data[0..4]);
    let keyframe = tag & 1 == 0;
    // let _version = (tag >> 1) & 7;
    // let _show = (tag >> 4) & 1;
    // let _part = (tag >> 5) & 0x7ffff;
    if !keyframe {
        return Err(WebpError::Unsupported("VP8 inter frames unsupported".into()));
    }
    if data[3] != 0x9d || data[4] != 0x01 || data[5] != 0x2a {
        return Err(WebpError::Decode("bad VP8 start code".into()));
    }
    let w = (u16le(&data[6..8]) & 0x3fff) as u32;
    let h = (u16le(&data[8..10]) & 0x3fff) as u32;
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err(WebpError::Decode("invalid VP8 size".into()));
    }
    Ok((w, h, keyframe))
}

/// MSB-first boolean decoder (RFC 6386 section 7).
/// Big-endian 4-byte groups with a short tail give bit-exact results.
struct BoolDec<'a> {
    src: &'a [u8],
    nfull: usize,
    nchk: usize,
    pos: usize,
    tail: [u8; 3],
    tail_n: usize,
    tail_pos: usize,
    // Token debug positions (kept for WEBP_TOKENS traces, removed later).
    byte: usize,
    bit: u8,
    val: u64,
    rng: u32,
    have: i32,
}

impl<'a> BoolDec<'a> {
    fn new(data: &'a [u8]) -> Result<Self, WebpError> {
        // Empty partitions read back as zeros (tolerated like libwebp).
        let n = data.len();
        let full = n / 4 * 4;
        let mut tail = [0u8; 3];
        for i in 0..n - full {
            tail[i] = data[full + i];
        }
        Ok(Self {
            src: data,
            nfull: full,
            nchk: full / 4,
            pos: 0,
            tail,
            tail_n: n - full,
            tail_pos: 0,
            byte: 0,
            bit: 0,
            val: 0,
            rng: 255,
            have: -8,
        })
    }
    fn pull(&mut self) {
        // Load next input unit into val/have when more bits are needed.
        // Full 4-byte big-endian groups first, then tail bytes, then zeros.
        if self.pos < self.nchk {
            let o = self.pos * 4;
            let w = ((self.src[o] as u32) << 24)
                | ((self.src[o + 1] as u32) << 16)
                | ((self.src[o + 2] as u32) << 8)
                | (self.src[o + 3] as u32);
            self.pos += 1;
            self.val = (self.val << 32) | (w as u64);
            self.have += 32;
            self.byte += 4;
            return;
        }
        if self.tail_pos < self.tail_n {
            let b = self.tail[self.tail_pos];
            self.tail_pos += 1;
            self.val = (self.val << 8) | (b as u64);
            self.have += 8;
            self.byte += 1;
            return;
        }
        // Past end: zeros (1-byte-past-end tolerance included).
        self.val <<= 8;
        self.have += 8;
    }
    fn read_bool(&mut self, prob: u8) -> Result<bool, WebpError> {
        if self.have < 0 {
            self.pull();
        }
        let split = 1 + (((self.rng - 1) * prob as u32) >> 8);
        let big = (split as u64) << (self.have as u32);
        let out = if self.val >= big {
            self.val -= big;
            self.rng -= split;
            true
        } else {
            self.rng = split;
            false
        };
        // Renormalize to keep rng >= 128, consuming lookahead bits.
        let mut sh = 0u32;
        let mut r = self.rng;
        while r < 128 {
            r <<= 1;
            sh += 1;
        }
        // Equivalent to leading-zero count minus 24 for u32 rng.
        // Computed via loop above to avoid copying bit tricks verbatim.
        self.rng = r;
        self.have -= sh as i32;
        Ok(out)
    }
    fn read_flag(&mut self) -> Result<bool, WebpError> {
        self.read_bool(128)
    }
    fn read_literal(&mut self, n: u8) -> Result<u32, WebpError> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | (self.read_flag()? as u32);
        }
        Ok(v)
    }
    fn read_tree(&mut self, tree: &[i8], probs: &[u8]) -> Result<i8, WebpError> {
        self.read_tree_from(tree, probs, 0)
    }
    fn read_optional_signed(&mut self, n: u8) -> Result<i32, WebpError> {
        if !self.read_flag()? {
            return Ok(0);
        }
        let mag = self.read_literal(n)? as i32;
        if self.read_flag()? {
            Ok(-mag)
        } else {
            Ok(mag)
        }
    }
    /// Walk a VP8 binary tree. Positive values address the next node
    /// (value / 2); non-positive values are leaf values (negated).
    fn read_tree_from(
        &mut self,
        tree: &[i8],
        probs: &[u8],
        mut node: usize,
    ) -> Result<i8, WebpError> {
        loop {
            if node >= probs.len() || 2 * node + 1 >= tree.len() {
                return Err(WebpError::Decode("bad VP8 tree".into()));
            }
            let b = self.read_bool(probs[node])? as usize;
            let v = tree[2 * node + b];
            if v <= 0 {
                return Ok(-v);
            }
            node = (v as usize) / 2;
        }
    }
}

fn decode_vp8_file(data: &[u8], alpha: Option<&[u8]>) -> Result<DecodedWebp, WebpError> {
    decode_vp8_data(data, alpha, None)
}

fn decode_vp8_data(
    data: &[u8],
    alpha: Option<&[u8]>,
    expected: Option<(u32, u32)>,
) -> Result<DecodedWebp, WebpError> {
    let (w, h, _) = vp8_frame_size(data)?;
    if let Some((ew, eh)) = expected {
        if ew != w || eh != h {
            return Err(WebpError::Decode("VP8 size mismatch".into()));
        }
    }
    let tag = u32le(&data[0..4]);
    let part_size = ((tag >> 5) & 0x7ffff) as usize;
    if data.len() < 10 + part_size {
        return Err(WebpError::Decode("truncated VP8".into()));
    }
    let mb_w = div_up(w, 16);
    let mb_h = div_up(h, 16);
    let nmbs = (mb_w as usize) * (mb_h as usize);

    // First partition: modes + token probs + residual flag stream.
    let mut hdr = BoolDec::new(&data[10..10 + part_size])?;
    // Colorspace + clamping (2 bits, must be 0).
    let _colorspace = hdr.read_literal(1)?;
    let _clamp = hdr.read_literal(1)?;
    // Segmentation.
    let mut seg_q = [0i16; 4];
    let mut seg_lf = [0i16; 4];
    let mut seg_abs = true;
    let seg_on = hdr.read_flag()?;
    let mut update_map = false;
    let mut seg_ids = vec![0u8; nmbs];
    let mut seg_probs = [255u8; 3];
    if seg_on {
        update_map = hdr.read_flag()?;
        let update_data = hdr.read_flag()?;
        if update_data {
            seg_abs = hdr.read_flag()?;
            for i in 0..4 {
                seg_q[i] = hdr.read_optional_signed(7)? as i16;
            }
            for i in 0..4 {
                seg_lf[i] = hdr.read_optional_signed(6)? as i16;
            }
        }
        if update_map {
            // Segment tree probs; ids are read inline per macroblock.
            for p in seg_probs.iter_mut() {
                if hdr.read_flag()? {
                    *p = hdr.read_literal(8)? as u8;
                }
            }
        }
    }
    // Loop filter.
    let filter_type = hdr.read_flag()?; // 0 = normal, 1 = simple
    let filter_level = hdr.read_literal(6)? as i32;
    let sharpness = hdr.read_literal(3)? as i32;
    let lf_delta_update = hdr.read_flag()?;
    let mut lf_ref_delta = [0i32; 4];
    let mut lf_mode_delta = [0i32; 4];
    if lf_delta_update {
        for d in lf_ref_delta.iter_mut().chain(lf_mode_delta.iter_mut()) {
            if hdr.read_flag()? {
                *d = hdr.read_literal(6)? as i32;
                if hdr.read_flag()? {
                    *d = -*d;
                }
            }
        }
    }
    let _ = (filter_type, sharpness, lf_ref_delta, lf_mode_delta);
    // Token partitions (RFC 6386 9.3): sizes for all but the last
    // partition follow the first partition; the last takes the rest.
    let parts_log2 = hdr.read_literal(2)? as usize;
    if parts_log2 > 3 {
        return Err(WebpError::Decode("bad VP8 partitions".into()));
    }
    let nparts = 1usize << parts_log2;
    if std::env::var("WEBP_DEBUG").is_ok() {
        eprintln!("vp8 {w}x{h} part0={part_size} log2={parts_log2} len={}", data.len());
    }
    // Partition sizes live in raw bytes after first partition.
    let mut raw = 10 + part_size;
    // Token partitions decoders (built after header parse below).
    // Quantizer (absolute base + signed deltas, RFC 6386 9.6).
    let yac_abs = hdr.read_literal(7)? as i32;
    let ydc_d = hdr.read_optional_signed(4)?;
    let y2dc_d = hdr.read_optional_signed(4)?;
    let y2ac_d = hdr.read_optional_signed(4)?;
    let uvdc_d = hdr.read_optional_signed(4)?;
    let uvac_d = hdr.read_optional_signed(4)?;
    // Refresh entropy probs (1 bit, RFC 6386 9.7): must be consumed
    // BEFORE the token probability updates (matches libwebp).
    let _ = hdr.read_literal(1)?;
    if std::env::var("WEBP_TOKENS").is_ok() {
        eprintln!("OURS pre-upd byte={} bit={} value={} range={} have={}", hdr.byte, hdr.bit, hdr.val, hdr.rng, hdr.have);
    }
    // Token prob update.
    let mut token_probs = VT::COEFF_PROBS;
    for i in 0..4 {
        for j in 0..8 {
            for k in 0..3 {
                for t in 0..11 {
                    let upd_prob = VT::COEFF_UPDATE_PROBS[i][j][k][t];
                    let before = (hdr.byte, hdr.bit, hdr.val, hdr.rng, hdr.have);
                    let do_upd = hdr.read_bool(upd_prob)?;
                    if std::env::var("WEBP_TOKENS").is_ok() && i == 2 && j == 0 {
                        eprintln!("OURS upd-check {i} {j} {k} {t} prob={upd_prob} do={do_upd} before={before:?}");
                    }
                    if do_upd {
                        let v = hdr.read_literal(8)? as u8;
                        if std::env::var("WEBP_TOKENS").is_ok() {
                            eprintln!("OURS upd {i} {j} {k} {t} {} -> {v}", token_probs[i][j][k][t]);
                        }
                        token_probs[i][j][k][t] = v;
                    }
                }
            }
        }
    }
    if std::env::var("WEBP_TOKENS").is_ok() {
        eprintln!("vq yac={yac_abs} d={ydc_d},{y2dc_d},{y2ac_d},{uvdc_d},{uvac_d}");
        eprintln!("OURS probs p3b0c1 {:?}", token_probs[3][0][1]);
        let mut h: u64 = 0;
        for i in 0..4 {
            for j in 0..8 {
                for k in 0..3 {
                    for t in 0..11 {
                        h = h.wrapping_mul(31).wrapping_add(token_probs[i][j][k][t] as u64);
                    }
                }
            }
        }
        eprintln!("OURS probs_hash {h}");
        eprintln!("OURS filter type={filter_type} level={filter_level} sharp={sharpness} seg_on={seg_on} sq={seg_q:?} slf={seg_lf:?}");
    }
    // Skip coefficient flag prob.
    let skip_prob: Option<u8> = if hdr.read_literal(1)? == 1 {
        Some(hdr.read_literal(8)? as u8)
    } else {
        None
    };
    // Intra modes per macroblock (keyframes: no MV).
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum YMode {
        Dc,
        V,
        H,
        Tm,
        B,
    }
    let mut ymodes = vec![YMode::Dc; nmbs];
    let mut bmodes = vec![[0i8; 16]; nmbs];
    let mut uvs = vec![0i8; nmbs];
    let mut skips = vec![false; nmbs];
    // Above/left context for bpred modes (IntraMode values).
    let mut top_b: Vec<[i8; 16]> = vec![[2i8; 16]; mb_w as usize]; // B_VE_PRED default? use DC
    for i in top_b.iter_mut() {
        *i = [0; 16];
    }
    for my in 0..mb_h {
        for mx in 0..mb_w {
            let mi = (my * mb_w + mx) as usize;
            if seg_on && update_map {
                let sid = hdr.read_tree(&SEG_ID_TREE, &seg_probs)? as u8;
                if sid > 3 {
                    return Err(WebpError::Decode("bad segment id".into()));
                }
                seg_ids[mi] = sid;
            }
            let skip = if let Some(p) = skip_prob { hdr.read_bool(p)? } else { false };
            skips[mi] = skip;
            let ym = hdr.read_tree(&YMODE_TREE, &YMODE_PROBS)? as i8;
            ymodes[mi] = match ym {
                0 => YMode::Dc,
                1 => YMode::V,
                2 => YMode::H,
                3 => YMode::Tm,
                4 => YMode::B,
                _ => return Err(WebpError::Decode("bad ymode".into())),
            };
            if ymodes[mi] == YMode::B {
                for r in 0..4usize {
                    // Running left neighbor within this row.
                    let mut row_left = if mx == 0 {
                        0i8
                    } else {
                        let p = bmodes[(my * mb_w + mx - 1) as usize];
                        p[r * 4 + 3]
                    };
                    for c in 0..4usize {
                        let a = if r == 0 {
                            if my == 0 {
                                0i8
                            } else {
                                top_b[mx as usize][12 + c]
                            }
                        } else {
                            bmodes[mi][(r - 1) * 4 + c]
                        };
                        let probs = &VT::KEYFRAME_BPRED_MODE_PROBS[a as usize][row_left as usize];
                        let m = hdr.read_tree(&BPRED_TREE, probs)?;
                        if m < 0 || m > 9 {
                            return Err(WebpError::Decode("bad bmode".into()));
                        }
                        bmodes[mi][r * 4 + c] = m;
                        row_left = m;
                    }
                }
            }
            let uvm = hdr.read_tree(&UVMODE_TREE, &UVMODE_PROBS)? as i8;
            if uvm < 0 || uvm > 3 {
                return Err(WebpError::Decode("bad uvmode".into()));
            }
            uvs[mi] = uvm;
            if std::env::var("WEBP_TOKENS").is_ok() {
                eprintln!("OURMB mi={mi} seg={} skip={} ymode={} uvmode={uvm}", seg_ids[mi], skips[mi], ymodes[mi] as u8);
            }
        }
        // roll top context
        if my < mb_h {
            for mx in 0..mb_w {
                let mi = (my * mb_w + mx) as usize;
                top_b[mx as usize] = bmodes[mi];
            }
        }
    }
    // Token partitions decoders: explicit sizes for all but the last;
    // the last partition takes the remaining bytes.
    let mut token_dec: Vec<BoolDec<'_>> = Vec::with_capacity(nparts);
    for _ in 0..nparts.saturating_sub(1) {
        if data.len() < raw + 3 {
            return Err(WebpError::Decode("truncated VP8 partitions".into()));
        }
        let s = (data[raw] as usize)
            | ((data[raw + 1] as usize) << 8)
            | ((data[raw + 2] as usize) << 16);
        raw += 3;
        if data.len() < raw + s {
            return Err(WebpError::Decode("truncated VP8 token data".into()));
        }
        token_dec.push(BoolDec::new(&data[raw..raw + s])?);
        raw += s;
    }
    token_dec.push(BoolDec::new(&data[raw..])?);
    if std::env::var("WEBP_DEBUG").is_ok() {
        eprintln!("vp8 {w}x{h} part0={part_size} nparts={nparts} raw={raw} len={}", data.len());
    }

    // Residual tokens -> dequantized coefficient blocks.
    // Layout: per MB: y2[16] (if present), y[16][16], u[4][16], v[4][16].
    // Per-MB dequantizers with Y2 scaling (RFC 6386 9.6).
    let mb_deq = |mi: usize| -> (i16, i16, i16, i16, i16, i16) {
        let seg = seg_ids[mi] as usize;
        let base = if seg_on {
            if seg_abs {
                seg_q[seg] as i32
            } else {
                seg_q[seg] as i32 + yac_abs
            }
        } else {
            yac_abs
        };
        let yac = VT::AC_QUANT[base.clamp(0, 127) as usize];
        let ydc = VT::DC_QUANT[(base + ydc_d).clamp(0, 127) as usize];
        let y2dc = VT::DC_QUANT[(base + y2dc_d).clamp(0, 127) as usize].wrapping_mul(2);
        let mut y2ac =
            (VT::AC_QUANT[(base + y2ac_d).clamp(0, 127) as usize] as i32 * 155 / 100) as i16;
        if y2ac < 8 {
            y2ac = 8;
        }
        let mut uvdc = VT::DC_QUANT[(base + uvdc_d).clamp(0, 127) as usize];
        if uvdc > 132 {
            uvdc = 132;
        }
        let uvac = VT::AC_QUANT[(base + uvac_d).clamp(0, 127) as usize];
        (yac, ydc, y2dc, y2ac, uvdc, uvac)
    };
    // Coeff storage.
    let mut y2blk = vec![[0i32; 16]; nmbs];
    let mut yblk = vec![[[0i32; 16]; 16]; nmbs];
    let mut ublk = vec![[[0i32; 16]; 4]; nmbs];
    let mut vblk = vec![[[0i32; 16]; 4]; nmbs];
    // Fine-grained nonzero contexts (RFC 6386 13.3).
    let mut top_y = vec![0u8; mb_w as usize * 4];
    let mut left_y = vec![0u8; mb_h as usize * 4];
    let mut top_y2 = vec![0u8; mb_w as usize];
    let mut left_y2 = vec![0u8; mb_h as usize];
    let mut top_u = vec![0u8; mb_w as usize * 2];
    let mut left_u = vec![0u8; mb_h as usize * 2];
    let mut top_v = vec![0u8; mb_w as usize * 2];
    let mut left_v = vec![0u8; mb_h as usize * 2];
    let mut mb_nz = vec![false; nmbs];
    // Helper: decode one 4x4 block's tokens. Returns nonzero flag.
    fn decode_block(
        dec: &mut BoolDec<'_>,
        probs: &[[[u8; 11]; 3]; 8],
        start: usize,
        deq_dc: i16,
        deq_ac: i16,
        above: u8,
        left: u8,
        out: &mut [i32; 16],
    ) -> Result<u8, WebpError> {
        let mut has = 0u8;
        let mut skip = false;
        let mut ctx = (above + left).min(2) as usize;
        let mut c = start;
        while c < 16 {
            let band = VT::COEFF_BANDS[c] as usize;
            let tok = dec
                .read_tree_from(&VT::DCT_TOKEN_TREE, &probs[band][ctx], if skip { 1 } else { 0 })?
                as u8;
            skip = false;
            if std::env::var("WEBP_TOKENS").is_ok() {
                eprintln!("tok band={band} ctx={ctx} tok={tok}");
            }
            match tok {
                11 => break, // EOB
                0 => {
                    has = 1;
                    ctx = 0;
                    c += 1;
                    skip = true;
                }
                1 | 2 | 3 | 4 => {
                    let v = tok as i32;
                    let sign = dec.read_flag()? as i32;
                    let q = if ZIGZAG[c] == 0 { deq_dc } else { deq_ac } as i32;
                    out[ZIGZAG[c]] = if sign == 1 { -v * q } else { v * q };
                    has = 1;
                    ctx = if v == 1 { 1 } else { 2 };
                    c += 1;
                }
                5..=10 => {
                    let cat = (tok - 5) as usize;
                    let mut extra = 0i32;
                    for &p in VT::PROB_DCT_CAT[cat].iter() {
                        if p == 0 {
                            break;
                        }
                        extra = extra + extra + dec.read_bool(p)? as i32;
                    }
                    let v = VT::DCT_CAT_BASE[cat] as i32 + extra;
                    let sign = dec.read_flag()? as i32;
                    let q = if ZIGZAG[c] == 0 { deq_dc } else { deq_ac } as i32;
                    out[ZIGZAG[c]] = if sign == 1 { -v * q } else { v * q };
                    has = 1;
                    ctx = 2;
                    c += 1;
                }
                _ => return Err(WebpError::Decode("bad token".into())),
            }
        }
        Ok(has)
    }

    for my in 0..mb_h {
        for mx in 0..mb_w {
            let mi = (my * mb_w + mx) as usize;
            // Token partition by macroblock row (RFC 6386 9.3).
            let part = (my as usize) % token_dec.len();
            let dec = &mut token_dec[part];
            let (q_yac, q_ydc, q_y2dc, q_y2ac, q_uvdc, q_uvac) = mb_deq(mi);
            if skips[mi] {
                // Skipped macroblock: zero contexts.
                top_y2[mx as usize] = 0;
                left_y2[my as usize] = 0;
                for bx in 0..4 {
                    top_y[mx as usize * 4 + bx] = 0;
                }
                for by in 0..4 {
                    left_y[my as usize * 4 + by] = 0;
                }
                for bx in 0..2 {
                    top_u[mx as usize * 2 + bx] = 0;
                    top_v[mx as usize * 2 + bx] = 0;
                }
                for by in 0..2 {
                    left_u[my as usize * 2 + by] = 0;
                    left_v[my as usize * 2 + by] = 0;
                }
                continue;
            }
            let mut nz_any = false;
            let need_y2 = ymodes[mi] != YMode::B;
            if std::env::var("WEBP_TOKENS").is_ok() && mi == 0 {
                eprintln!("tokpos y2 byte {} bit {}", dec.byte, dec.bit);
            }
            if need_y2 {
                let a = if my == 0 { 0 } else { top_y2[mx as usize] };
                let l = if mx == 0 { 0 } else { left_y2[my as usize] };
                let mut blk = [0i32; 16];
                let nz = decode_block(dec, &token_probs[1], 0, q_y2dc, q_y2ac, a, l, &mut blk)?;
                y2blk[mi] = blk;
                top_y2[mx as usize] = nz;
                left_y2[my as usize] = nz;
                nz_any |= nz != 0;
            }
            // Y blocks.
            if std::env::var("WEBP_TOKENS").is_ok() && mi == 0 {
                eprintln!("tokpos Y byte {} bit {}", dec.byte, dec.bit);
            }
            let mut y_cur = [0u8; 16];
            for bi in 0..16 {
                let bx = bi % 4;
                let by = bi / 4;
                let (tbl, st) = if need_y2 { (&token_probs[0], 1) } else { (&token_probs[3], 0) };
                let a = if by == 0 {
                    if my == 0 { 0 } else { top_y[mx as usize * 4 + bx] }
                } else {
                    y_cur[(by - 1) * 4 + bx]
                };
                let l = if bx == 0 {
                    if mx == 0 { 0 } else { left_y[my as usize * 4 + by] }
                } else {
                    y_cur[by * 4 + bx - 1]
                };
                let mut blk = [0i32; 16];
                let nz = decode_block(dec, tbl, st, q_ydc, q_yac, a, l, &mut blk)?;
                yblk[mi][bi] = blk;
                y_cur[bi] = nz;
                nz_any |= nz != 0;
            }
            for bx in 0..4 {
                top_y[mx as usize * 4 + bx] = y_cur[12 + bx];
            }
            for by in 0..4 {
                left_y[my as usize * 4 + by] = y_cur[by * 4 + 3];
            }
            // U/V blocks.
            if std::env::var("WEBP_TOKENS").is_ok() && mi == 0 {
                eprintln!("tokpos U byte {} bit {}", dec.byte, dec.bit);
            }
            let mut u_cur = [0u8; 4];
            let mut v_cur = [0u8; 4];
            for bi in 0..4 {
                let bx = bi % 2;
                let by = bi / 2;
                let au = if by == 0 {
                    if my == 0 { 0 } else { top_u[mx as usize * 2 + bx] }
                } else {
                    u_cur[(by - 1) * 2 + bx]
                };
                let lu = if bx == 0 {
                    if mx == 0 { 0 } else { left_u[my as usize * 2 + by] }
                } else {
                    u_cur[by * 2 + bx - 1]
                };
                let mut blk = [0i32; 16];
                let nz = decode_block(dec, &token_probs[2], 0, q_uvdc, q_uvac, au, lu, &mut blk)?;
                ublk[mi][bi] = blk;
                u_cur[bi] = nz;
                nz_any |= nz != 0;
                if std::env::var("WEBP_TOKENS").is_ok() && mi == 0 && bi == 0 {
                    eprintln!("tokpos U0done byte {} bit {}", dec.byte, dec.bit);
                }
            }
            for bi in 0..4 {
                let bx = bi % 2;
                let by = bi / 2;
                let av = if by == 0 {
                    if my == 0 { 0 } else { top_v[mx as usize * 2 + bx] }
                } else {
                    v_cur[(by - 1) * 2 + bx]
                };
                let lv = if bx == 0 {
                    if mx == 0 { 0 } else { left_v[my as usize * 2 + by] }
                } else {
                    v_cur[by * 2 + bx - 1]
                };
                let mut blk = [0i32; 16];
                let nz = decode_block(dec, &token_probs[2], 0, q_uvdc, q_uvac, av, lv, &mut blk)?;
                vblk[mi][bi] = blk;
                v_cur[bi] = nz;
                nz_any |= nz != 0;
                if std::env::var("WEBP_DEBUG").is_ok() && mi == 0 && bi == 3 {
                    eprintln!("tokpos Vend byte {} bit {}", dec.byte, dec.bit);
                }
            }
            for bx in 0..2 {
                top_u[mx as usize * 2 + bx] = u_cur[2 + bx];
                top_v[mx as usize * 2 + bx] = v_cur[2 + bx];
            }
            for by in 0..2 {
                left_u[my as usize * 2 + by] = u_cur[by * 2 + 1];
                left_v[my as usize * 2 + by] = v_cur[by * 2 + 1];
            }
            mb_nz[mi] = nz_any;
        }
    }

    // ---- Reconstruction ----
    fn iwht4(input: &[i32; 16]) -> [i32; 16] {
        let mut tmp = [0i32; 16];
        for i in 0..4 {
            let a0 = input[i] + input[12 + i];
            let a1 = input[4 + i] + input[8 + i];
            let a2 = input[4 + i] - input[8 + i];
            let a3 = input[i] - input[12 + i];
            tmp[i] = a0 + a1;
            tmp[4 + i] = a3 + a2;
            tmp[8 + i] = a0 - a1;
            tmp[12 + i] = a3 - a2;
        }
        let mut out = [0i32; 16];
        for i in (0..16).step_by(4) {
            let a0 = tmp[i] + tmp[i + 3];
            let a1 = tmp[i + 1] + tmp[i + 2];
            let a2 = tmp[i + 1] - tmp[i + 2];
            let a3 = tmp[i] - tmp[i + 3];
            out[i] = (a0 + a1 + 3) >> 3;
            out[i + 1] = (a3 + a2 + 3) >> 3;
            out[i + 2] = (a0 - a1 + 3) >> 3;
            out[i + 3] = (a3 - a2 + 3) >> 3;
        }
        out
    }
    fn idct4(input: &[i32; 16]) -> [i32; 16] {
        // RFC 6386 section 14.4 integer inverse DCT. Uses the spec
        // cosine constants with 64-bit intermediates to avoid overflow.
        const C1: i64 = 20091;
        const C2: i64 = 35468;
        let mut w = [0i64; 16];
        for k in 0..16 {
            w[k] = input[k] as i64;
        }
        for c in 0..4 {
            let s0 = w[c] + w[8 + c];
            let d0 = w[c] - w[8 + c];
            let t0 = (w[4 + c] * C2) >> 16;
            let t1 = w[12 + c] + ((w[12 + c] * C1) >> 16);
            let e0 = t0 - t1;
            let t2 = w[4 + c] + ((w[4 + c] * C1) >> 16);
            let t3 = (w[12 + c] * C2) >> 16;
            let f0 = t2 + t3;
            w[c] = s0 + f0;
            w[4 + c] = d0 + e0;
            w[8 + c] = d0 - e0;
            w[12 + c] = s0 - f0;
        }
        let mut out = [0i32; 16];
        for r in 0..4 {
            let b = r * 4;
            let s0 = w[b] + w[b + 2];
            let d0 = w[b] - w[b + 2];
            let t0 = (w[b + 1] * C2) >> 16;
            let t1 = w[b + 3] + ((w[b + 3] * C1) >> 16);
            let e0 = t0 - t1;
            let t2 = w[b + 1] + ((w[b + 1] * C1) >> 16);
            let t3 = (w[b + 3] * C2) >> 16;
            let f0 = t2 + t3;
            out[b] = ((s0 + f0 + 4) >> 3) as i32;
            out[b + 1] = ((d0 + e0 + 4) >> 3) as i32;
            out[b + 2] = ((d0 - e0 + 4) >> 3) as i32;
            out[b + 3] = ((s0 - f0 + 4) >> 3) as i32;
        }
        out
    }
    fn avg3p(a: u8, b: u8, c: u8) -> u8 {
        ((a as u16 + 2 * b as u16 + c as u16 + 2) >> 2) as u8
    }
    fn avg2p(a: u8, b: u8) -> u8 {
        ((a as u16 + b as u16 + 1) >> 1) as u8
    }
    // Exact 4x4 intra prediction (RFC 6386 12.3, cf. libvpx predict_*).
    fn bpred_block(m: i8, a: &[u8; 8], l: &[u8; 4], c: u8) -> [u8; 16] {
        let mut o = [0u8; 16];
        match m {
            0 => {
                let mut v = 4u32;
                for i in 0..4 {
                    v += a[i] as u32 + l[i] as u32;
                }
                let p = (v >> 3) as u8;
                o = [p; 16];
            }
            1 => {
                for y in 0..4 {
                    for x in 0..4 {
                        o[y * 4 + x] =
                            (l[y] as i32 + a[x] as i32 - c as i32).clamp(0, 255) as u8;
                    }
                }
            }
            2 => {
                let p = [
                    avg3p(c, a[0], a[1]),
                    avg3p(a[0], a[1], a[2]),
                    avg3p(a[1], a[2], a[3]),
                    avg3p(a[2], a[3], a[4]),
                ];
                for y in 0..4 {
                    o[y * 4..y * 4 + 4].copy_from_slice(&p);
                }
            }
            3 => {
                let p = [
                    avg3p(c, l[0], l[1]),
                    avg3p(l[0], l[1], l[2]),
                    avg3p(l[1], l[2], l[3]),
                    avg3p(l[2], l[3], l[3]),
                ];
                for y in 0..4 {
                    for x in 0..4 {
                        o[y * 4 + x] = p[y];
                    }
                }
            }
            4 => {
                let av = [
                    avg3p(a[0], a[1], a[2]),
                    avg3p(a[1], a[2], a[3]),
                    avg3p(a[2], a[3], a[4]),
                    avg3p(a[3], a[4], a[5]),
                    avg3p(a[4], a[5], a[6]),
                    avg3p(a[5], a[6], a[7]),
                    avg3p(a[6], a[7], a[7]),
                ];
                for y in 0..4 {
                    o[y * 4..y * 4 + 4].copy_from_slice(&av[y..y + 4]);
                }
            }
            5 => {
                // edge e0..e8 = l3,l2,l1,l0,c,a0,a1,a2,a3
                let e = [l[3], l[2], l[1], l[0], c, a[0], a[1], a[2], a[3]];
                let av = [
                    avg3p(e[0], e[1], e[2]),
                    avg3p(e[1], e[2], e[3]),
                    avg3p(e[2], e[3], e[4]),
                    avg3p(e[3], e[4], e[5]),
                    avg3p(e[4], e[5], e[6]),
                    avg3p(e[5], e[6], e[7]),
                    avg3p(e[6], e[7], e[8]),
                ];
                for y in 0..4 {
                    o[y * 4..y * 4 + 4].copy_from_slice(&av[3 - y..7 - y]);
                }
            }
            6 => {
                let e = [l[3], l[2], l[1], l[0], c, a[0], a[1], a[2], a[3]];
                o[3 * 4] = avg3p(e[1], e[2], e[3]);
                o[2 * 4] = avg3p(e[2], e[3], e[4]);
                o[3 * 4 + 1] = avg3p(e[3], e[4], e[5]);
                o[1 * 4] = avg3p(e[3], e[4], e[5]);
                o[2 * 4 + 1] = avg2p(e[4], e[5]);
                o[0 * 4] = avg2p(e[4], e[5]);
                o[3 * 4 + 2] = avg3p(e[4], e[5], e[6]);
                o[1 * 4 + 1] = avg3p(e[4], e[5], e[6]);
                o[2 * 4 + 2] = avg2p(e[5], e[6]);
                o[0 * 4 + 1] = avg2p(e[5], e[6]);
                o[3 * 4 + 3] = avg3p(e[5], e[6], e[7]);
                o[1 * 4 + 2] = avg3p(e[5], e[6], e[7]);
                o[2 * 4 + 3] = avg2p(e[6], e[7]);
                o[0 * 4 + 2] = avg2p(e[6], e[7]);
                o[1 * 4 + 3] = avg3p(e[6], e[7], e[8]);
                o[0 * 4 + 3] = avg2p(e[7], e[8]);
            }
            7 => {
                o[0 * 4] = avg2p(a[0], a[1]);
                o[1 * 4] = avg3p(a[0], a[1], a[2]);
                o[2 * 4] = avg2p(a[1], a[2]);
                o[0 * 4 + 1] = avg2p(a[1], a[2]);
                o[1 * 4 + 1] = avg3p(a[1], a[2], a[3]);
                o[3 * 4] = avg3p(a[1], a[2], a[3]);
                o[2 * 4 + 1] = avg2p(a[2], a[3]);
                o[0 * 4 + 2] = avg2p(a[2], a[3]);
                o[3 * 4 + 1] = avg3p(a[2], a[3], a[4]);
                o[1 * 4 + 2] = avg3p(a[2], a[3], a[4]);
                o[2 * 4 + 2] = avg2p(a[3], a[4]);
                o[0 * 4 + 3] = avg2p(a[3], a[4]);
                o[3 * 4 + 2] = avg3p(a[3], a[4], a[5]);
                o[1 * 4 + 3] = avg3p(a[3], a[4], a[5]);
                o[2 * 4 + 3] = avg3p(a[4], a[5], a[6]);
                o[3 * 4 + 3] = avg3p(a[5], a[6], a[7]);
            }
            8 => {
                let e = [l[3], l[2], l[1], l[0], c, a[0], a[1], a[2], a[3]];
                o[3 * 4] = avg2p(e[0], e[1]);
                o[3 * 4 + 1] = avg3p(e[0], e[1], e[2]);
                o[2 * 4] = avg2p(e[1], e[2]);
                o[3 * 4 + 2] = avg2p(e[1], e[2]);
                o[2 * 4 + 1] = avg3p(e[1], e[2], e[3]);
                o[3 * 4 + 3] = avg3p(e[1], e[2], e[3]);
                o[2 * 4 + 2] = avg2p(e[2], e[3]);
                o[1 * 4] = avg2p(e[2], e[3]);
                o[2 * 4 + 3] = avg3p(e[2], e[3], e[4]);
                o[1 * 4 + 1] = avg3p(e[2], e[3], e[4]);
                o[1 * 4 + 2] = avg2p(e[3], e[4]);
                o[0 * 4] = avg2p(e[3], e[4]);
                o[1 * 4 + 3] = avg3p(e[3], e[4], e[5]);
                o[0 * 4 + 1] = avg3p(e[3], e[4], e[5]);
                o[0 * 4 + 2] = avg3p(e[4], e[5], e[6]);
                o[0 * 4 + 3] = avg3p(e[5], e[6], e[7]);
            }
            _ => {
                o[0 * 4] = avg2p(l[0], l[1]);
                o[0 * 4 + 1] = avg3p(l[0], l[1], l[2]);
                o[0 * 4 + 2] = avg2p(l[1], l[2]);
                o[1 * 4] = avg2p(l[1], l[2]);
                o[0 * 4 + 3] = avg3p(l[1], l[2], l[3]);
                o[1 * 4 + 1] = avg3p(l[1], l[2], l[3]);
                o[1 * 4 + 2] = avg2p(l[2], l[3]);
                o[2 * 4] = avg2p(l[2], l[3]);
                o[1 * 4 + 3] = avg3p(l[2], l[3], l[3]);
                o[2 * 4 + 1] = avg3p(l[2], l[3], l[3]);
                o[2 * 4 + 2] = l[3];
                o[2 * 4 + 3] = l[3];
                o[3 * 4] = l[3];
                o[3 * 4 + 1] = l[3];
                o[3 * 4 + 2] = l[3];
                o[3 * 4 + 3] = l[3];
            }
        }
        o
    }
    // Loop filter primitives (RFC 6386 15.2/15.3).
    fn lf_clamp(v: i32) -> i32 {
        v.clamp(-128, 127)
    }
    fn lf_u2s(v: u8) -> i32 {
        v as i32 - 128
    }
    fn lf_s2u(v: i32) -> u8 {
        (lf_clamp(v) + 128) as u8
    }
    fn lf_common_v(use_outer: bool, px: &mut [u8], pt: usize, st: usize) -> i32 {
        let p1 = lf_u2s(px[pt - 2 * st]);
        let p0 = lf_u2s(px[pt - st]);
        let q0 = lf_u2s(px[pt]);
        let q1 = lf_u2s(px[pt + st]);
        let outer = if use_outer { lf_clamp(p1 - q1) } else { 0 };
        let a = lf_clamp(outer + 3 * (q0 - p0));
        let b = lf_clamp(a + 3) >> 3;
        let av = lf_clamp(a + 4) >> 3;
        px[pt] = lf_s2u(q0 - av);
        px[pt - st] = lf_s2u(p0 + b);
        av
    }
    fn lf_common_h(use_outer: bool, px: &mut [u8]) -> i32 {
        let p1 = lf_u2s(px[2]);
        let p0 = lf_u2s(px[3]);
        let q0 = lf_u2s(px[4]);
        let q1 = lf_u2s(px[5]);
        let outer = if use_outer { lf_clamp(p1 - q1) } else { 0 };
        let a = lf_clamp(outer + 3 * (q0 - p0));
        let b = lf_clamp(a + 3) >> 3;
        let av = lf_clamp(a + 4) >> 3;
        px[4] = lf_s2u(q0 - av);
        px[3] = lf_s2u(p0 + b);
        av
    }
    fn lf_should_v(interior: u8, edge: u8, px: &[u8], pt: usize, st: usize) -> bool {
        let simple = (px[pt - st].abs_diff(px[pt]) as i32) * 2
            + (px[pt - 2 * st].abs_diff(px[pt + st]) as i32) / 2
            <= edge as i32;
        simple
            && px[pt - 4 * st].abs_diff(px[pt - 3 * st]) <= interior
            && px[pt - 3 * st].abs_diff(px[pt - 2 * st]) <= interior
            && px[pt - 2 * st].abs_diff(px[pt - st]) <= interior
            && px[pt + 3 * st].abs_diff(px[pt + 2 * st]) <= interior
            && px[pt + 2 * st].abs_diff(px[pt + st]) <= interior
            && px[pt + st].abs_diff(px[pt]) <= interior
    }
    fn lf_should_h(interior: u8, edge: u8, px: &[u8]) -> bool {
        let simple = (px[3].abs_diff(px[4]) as i32) * 2 + (px[2].abs_diff(px[5]) as i32) / 2
            <= edge as i32;
        simple
            && px[0].abs_diff(px[1]) <= interior
            && px[1].abs_diff(px[2]) <= interior
            && px[2].abs_diff(px[3]) <= interior
            && px[7].abs_diff(px[6]) <= interior
            && px[6].abs_diff(px[5]) <= interior
            && px[5].abs_diff(px[4]) <= interior
    }
    fn lf_hev_v(th: u8, px: &[u8], pt: usize, st: usize) -> bool {
        px[pt - 2 * st].abs_diff(px[pt - st]) > th || px[pt + st].abs_diff(px[pt]) > th
    }
    fn lf_hev_h(th: u8, px: &[u8]) -> bool {
        px[2].abs_diff(px[3]) > th || px[5].abs_diff(px[4]) > th
    }
    fn lf_sub_v(hev: u8, interior: u8, edge: u8, px: &mut [u8], pt: usize, st: usize) {
        if lf_should_v(interior, edge, px, pt, st) {
            let hv = lf_hev_v(hev, px, pt, st);
            let a = (lf_common_v(hv, px, pt, st) + 1) >> 1;
            if !hv {
                px[pt + st] = lf_s2u(lf_u2s(px[pt + st]) - a);
                px[pt - 2 * st] = lf_s2u(lf_u2s(px[pt - 2 * st]) + a);
            }
        }
    }
    fn lf_sub_h(hev: u8, interior: u8, edge: u8, px: &mut [u8]) {
        if lf_should_h(interior, edge, px) {
            let hv = lf_hev_h(hev, px);
            let a = (lf_common_h(hv, px) + 1) >> 1;
            if !hv {
                px[5] = lf_s2u(lf_u2s(px[5]) - a);
                px[2] = lf_s2u(lf_u2s(px[2]) + a);
            }
        }
    }
    fn lf_mb_v(hev: u8, interior: u8, edge: u8, px: &mut [u8], pt: usize, st: usize) {
        if lf_should_v(interior, edge, px, pt, st) {
            if !lf_hev_v(hev, px, pt, st) {
                let p2 = lf_u2s(px[pt - 3 * st]);
                let p1 = lf_u2s(px[pt - 2 * st]);
                let p0 = lf_u2s(px[pt - st]);
                let q0 = lf_u2s(px[pt]);
                let q1 = lf_u2s(px[pt + st]);
                let q2 = lf_u2s(px[pt + 2 * st]);
                let w = lf_clamp(lf_clamp(p1 - q1) + 3 * (q0 - p0));
                let a = lf_clamp((27 * w + 63) >> 7);
                px[pt] = lf_s2u(q0 - a);
                px[pt - st] = lf_s2u(p0 + a);
                let a = lf_clamp((18 * w + 63) >> 7);
                px[pt + st] = lf_s2u(q1 - a);
                px[pt - 2 * st] = lf_s2u(p1 + a);
                let a = lf_clamp((9 * w + 63) >> 7);
                px[pt + 2 * st] = lf_s2u(q2 - a);
                px[pt - 3 * st] = lf_s2u(p2 + a);
            } else {
                lf_common_v(true, px, pt, st);
            }
        }
    }
    fn lf_mb_h(hev: u8, interior: u8, edge: u8, px: &mut [u8]) {
        if lf_should_h(interior, edge, px) {
            if !lf_hev_h(hev, px) {
                let p2 = lf_u2s(px[1]);
                let p1 = lf_u2s(px[2]);
                let p0 = lf_u2s(px[3]);
                let q0 = lf_u2s(px[4]);
                let q1 = lf_u2s(px[5]);
                let q2 = lf_u2s(px[6]);
                let w = lf_clamp(lf_clamp(p1 - q1) + 3 * (q0 - p0));
                let a = lf_clamp((27 * w + 63) >> 7);
                px[4] = lf_s2u(q0 - a);
                px[3] = lf_s2u(p0 + a);
                let a = lf_clamp((18 * w + 63) >> 7);
                px[5] = lf_s2u(q1 - a);
                px[2] = lf_s2u(p1 + a);
                let a = lf_clamp((9 * w + 63) >> 7);
                px[6] = lf_s2u(q2 - a);
                px[1] = lf_s2u(p2 + a);
            } else {
                lf_common_h(true, px);
            }
        }
    }
    fn lf_simple_v(edge: u8, px: &mut [u8], pt: usize, st: usize) {
        let ok = (px[pt - st].abs_diff(px[pt]) as i32) * 2
            + (px[pt - 2 * st].abs_diff(px[pt + st]) as i32) / 2
            <= edge as i32;
        if ok {
            lf_common_v(true, px, pt, st);
        }
    }
    // ---- Planes ----
    fn get_y(yp: &[u8], ys: usize, pw: usize, ph: usize, gx: i32, gy: i32) -> u8 {
        if gy < 0 {
            127
        } else if gx < 0 {
            129
        } else {
            let cx = (gx as usize).min(pw - 1);
            let cy = (gy as usize).min(ph - 1);
            yp[(cy + 16) * ys + cx + 16]
        }
    }
    fn get_c(cp: &[u8], cs: usize, uw: usize, uh: usize, gx: i32, gy: i32) -> u8 {
        if gy < 0 {
            127
        } else if gx < 0 {
            129
        } else {
            let cx = (gx as usize).min(uw - 1);
            let cy = (gy as usize).min(uh - 1);
            cp[(cy + 8) * cs + cx + 8]
        }
    }
    fn set_y(yp: &mut [u8], ys: usize, gx: i32, gy: i32, v: i32) {
        if gx < -16 || gy < -16 {
            return;
        }
        let (ux, uy) = (gx + 16, gy + 16);
        if ux < 0 || uy < 0 {
            return;
        }
        yp[uy as usize * ys + ux as usize] = v.clamp(0, 255) as u8;
    }
    let pw = w as usize;
    let ph = h as usize;
    let uw = div_up(w, 2) as usize;
    let uh = div_up(h, 2) as usize;
    let ys = pw + 32;
    let mut yp = vec![129u8; ys * (ph + 32)];
    for y in 0..16 {
        for x in 0..ys {
            yp[y * ys + x] = 127;
        }
    }
    let us = uw + 16;
    let mut up = vec![129u8; us * (uh + 16)];
    let mut vp = vec![129u8; us * (uh + 16)];
    for y in 0..8 {
        for x in 0..us {
            up[y * us + x] = 127;
            vp[y * us + x] = 127;
        }
    }
    for my in 0..mb_h as usize {
        for mx in 0..mb_w as usize {
            let mi = my * mb_w as usize + mx;
            let bx = (mx * 16) as i32;
            let by = (my * 16) as i32;
            if ymodes[mi] != YMode::B {
                let d = iwht4(&y2blk[mi]);
                for bi in 0..16 {
                    yblk[mi][bi][0] = d[bi];
                }
            }
            // Dequantized luma blocks need an inverse DCT before prediction
            // (RFC 6386 section 14). B-mode does it per sub-block below.
            let mut yres = [[0i32; 16]; 16];
            if ymodes[mi] != YMode::B {
                for bi in 0..16 {
                    yres[bi] = idct4(&yblk[mi][bi]);
                }
            }
            // Luma.
            match ymodes[mi] {
                YMode::Dc => {
                    let first = mx == 0 && my == 0;
                    let mut sum = 0u32;
                    let mut n = 0u32;
                    if !first {
                        if my > 0 {
                            for i in 0..16 {
                                sum += get_y(&yp, ys, pw, ph, bx + i as i32, by - 1) as u32;
                            }
                            n += 16;
                        }
                        if mx > 0 {
                            for i in 0..16 {
                                sum += get_y(&yp, ys, pw, ph, bx - 1, by + i as i32) as u32;
                            }
                            n += 16;
                        }
                    }
                    let dc = if n == 0 { 128 } else { ((sum + n / 2) / n) as i32 };
                    for yy in 0..16 {
                        for xx in 0..16 {
                            let r = yres[(yy / 4 * 4) + xx / 4][(yy % 4) * 4 + xx % 4];
                            set_y(&mut yp, ys, bx + xx as i32, by + yy as i32, dc + r);
                        }
                    }
                }
                YMode::V => {
                    for yy in 0..16 {
                        for xx in 0..16 {
                            let p = get_y(&yp, ys, pw, ph, bx + xx as i32, by - 1) as i32;
                            let r = yres[(yy / 4 * 4) + xx / 4][(yy % 4) * 4 + xx % 4];
                            set_y(&mut yp, ys, bx + xx as i32, by + yy as i32, p + r);
                        }
                    }
                }
                YMode::H => {
                    for yy in 0..16 {
                        for xx in 0..16 {
                            let p = get_y(&yp, ys, pw, ph, bx - 1, by + yy as i32) as i32;
                            let r = yres[(yy / 4 * 4) + xx / 4][(yy % 4) * 4 + xx % 4];
                            set_y(&mut yp, ys, bx + xx as i32, by + yy as i32, p + r);
                        }
                    }
                }
                YMode::Tm => {
                    let c = get_y(&yp, ys, pw, ph, bx - 1, by - 1) as i32;
                    for yy in 0..16 {
                        for xx in 0..16 {
                            let p = get_y(&yp, ys, pw, ph, bx - 1, by + yy as i32) as i32
                                + get_y(&yp, ys, pw, ph, bx + xx as i32, by - 1) as i32
                                - c;
                            let r = yres[(yy / 4 * 4) + xx / 4][(yy % 4) * 4 + xx % 4];
                            set_y(&mut yp, ys, bx + xx as i32, by + yy as i32, p + r);
                        }
                    }
                }
                YMode::B => {
                    for bi in 0..16 {
                        let sx = (bi % 4) as i32 * 4;
                        let sy = (bi / 4) as i32 * 4;
                        let mut a = [0u8; 8];
                        for k in 0..8 {
                            a[k] = get_y(&yp, ys, pw, ph, bx + sx + k as i32 - 0, by + sy - 1);
                        }
                        // above-right pixels for LD/VL (k=4..8).
                        let mut l = [0u8; 4];
                        for k in 0..4 {
                            l[k] = get_y(&yp, ys, pw, ph, bx + sx - 1, by + sy + k as i32);
                        }
                        let c = get_y(&yp, ys, pw, ph, bx + sx - 1, by + sy - 1);
                        // For top rows the border default 127/129 already applies.
                        let pred = bpred_block(bmodes[mi][bi], &a, &l, c);
                        let res = idct4(&yblk[mi][bi]);
                        for yy in 0..4 {
                            for xx in 0..4 {
                                set_y(
                                    &mut yp,
                                    ys,
                                    bx + sx + xx,
                                    by + sy + yy,
                                    pred[yy as usize * 4 + xx as usize] as i32 + res[yy as usize * 4 + xx as usize],
                                );
                            }
                        }
                    }
                }
            }
            // Chroma.
            let cbx = (mx * 8) as i32;
            let cby = (my * 8) as i32;
            let cm = uvs[mi];
            // Gather borders once (shared for U and V).
            let mut ca = [0u8; 8];
            let mut cl = [0u8; 8];
            for k in 0..8 {
                ca[k] = get_c(&up, us, uw, uh, cbx + k as i32, cby - 1);
                cl[k] = get_c(&up, us, uw, uh, cbx - 1, cby + k as i32);
            }
            let cc = get_c(&up, us, uw, uh, cbx - 1, cby - 1);
            let first_c = mx == 0 && my == 0;
            for (plane, blk) in [(&mut up, &ublk[mi]), (&mut vp, &vblk[mi])] {
                for yy in 0..8 {
                    for xx in 0..8 {
                        let bi = (yy / 4 * 2 + xx / 4) as usize;
                        let p = match cm {
                            0 => {
                                if first_c {
                                    128
                                } else {
                                    let mut s = 0u32;
                                    let mut n = 0u32;
                                    if my > 0 {
                                        for k in 0..8 {
                                            s += get_c(plane, us, uw, uh, cbx + k, cby - 1) as u32;
                                        }
                                        n += 8;
                                    }
                                    if mx > 0 {
                                        for k in 0..8 {
                                            s += get_c(plane, us, uw, uh, cbx - 1, cby + k) as u32;
                                        }
                                        n += 8;
                                    }
                                    if n == 0 { 128 } else { ((s + n / 2) / n) as i32 }
                                }
                            }
                            1 => get_c(plane, us, uw, uh, cbx + xx as i32, cby - 1) as i32,
                            2 => get_c(plane, us, uw, uh, cbx - 1, cby + yy as i32) as i32,
                            _ => {
                                get_c(plane, us, uw, uh, cbx - 1, cby + yy as i32) as i32
                                    + get_c(plane, us, uw, uh, cbx + xx as i32, cby - 1) as i32
                                    - get_c(plane, us, uw, uh, cbx - 1, cby - 1) as i32
                            }
                        };
                        let r = idct4(&blk[bi])[(yy % 4) as usize * 4 + xx as usize % 4];
                        // Full-block write incl. padding (matches reference).
                        let gx = cbx + xx as i32;
                        let gy = cby + yy as i32;
                        plane[(gy + 8) as usize * us + (gx + 8) as usize] =
                            (p + r).clamp(0, 255) as u8;
                    }
                }
                let _ = (ca, cl, cc);
            }
        }
    }
    // ---- Loop filter driver (per MB, reference order) ----
    let mb_level = |mi: usize| -> i32 {
        let mut lvl = filter_level;
        if seg_on {
            if seg_abs {
                lvl = seg_lf[seg_ids[mi] as usize] as i32;
            } else {
                lvl += seg_lf[seg_ids[mi] as usize] as i32;
            }
        }
        if lf_delta_update {
            lvl += lf_ref_delta[0];
            if ymodes[mi] == YMode::B {
                lvl += lf_mode_delta[0];
            }
        }
        lvl.clamp(0, 63)
    };
    let mb_limits = |lvl: i32| -> (u8, u8) {
        // Returns (interior, hev); edge limits derived by caller.
        if lvl == 0 {
            return (0, 0);
        }
        let mut interior = lvl;
        if sharpness > 0 {
            interior >>= if sharpness > 4 { 2 } else { 1 };
            if interior > 9 - sharpness {
                interior = 9 - sharpness;
            }
        }
        if interior == 0 {
            interior = 1;
        }
        let hev = if lvl >= 40 { 2 } else if lvl >= 15 { 1 } else { 0 };
        (interior as u8, hev as u8)
    };
    let simple = filter_type;
    for my in 0..mb_h as usize {
        for mx in 0..mb_w as usize {
            let mi = my * mb_w as usize + mx;
            let lvl = mb_level(mi);
            if lvl == 0 {
                continue;
            }
            let (interior, hev) = mb_limits(lvl);
            let mbedge = ((lvl + 2) * 2 + interior as i32).min(255) as u8;
            let subedge = (lvl * 2 + interior as i32).min(255) as u8;
            let do_sub = ymodes[mi] == YMode::B || (!skips[mi] && mb_nz[mi]);
            let bx = mx * 16;
            let by = my * 16;
            // Vertical edges (windows along rows).
            if mx > 0 {
                for y in by..(by + 16).min(ph) {
                    let row = (y + 16) * ys + (bx + 16);
                    let mut win = [0u8; 8];
                    win.copy_from_slice(&yp[row - 4..row + 4]);
                    if simple {
                        if lf_should_h(interior, mbedge, &win) {
                            lf_common_h(true, &mut win);
                        }
                    } else {
                        lf_mb_h(hev, interior, mbedge, &mut win);
                    }
                    yp[row - 4..row + 4].copy_from_slice(&win);
                }
                // Chroma MB edge.
                if !simple {
                    let cbx = mx * 8;
                    let cby = my * 8;
                    for y in cby..(cby + 8).min(uh) {
                        for (plane, _tag) in [(&mut up, 0), (&mut vp, 1)] {
                            let row = (y + 8) * us + (cbx + 8);
                            let mut win = [0u8; 8];
                            win.copy_from_slice(&plane[row - 4..row + 4]);
                            lf_mb_h(hev, interior, mbedge, &mut win);
                            plane[row - 4..row + 4].copy_from_slice(&win);
                        }
                    }
                }
            }
            if do_sub {
                let edges = if simple { [4, 8, 12] } else { [4, 8, 12] };
                for &e in edges.iter() {
                    let x = bx + e;
                    if x >= pw {
                        continue;
                    }
                    for y in by..(by + 16).min(ph) {
                        let row = (y + 16) * ys + (x + 16);
                        let mut win = [0u8; 8];
                        win.copy_from_slice(&yp[row - 4..row + 4]);
                        if simple {
                            if lf_should_h(interior, subedge, &win) {
                                lf_common_h(true, &mut win);
                            }
                        } else {
                            lf_sub_h(hev, interior, subedge, &mut win);
                        }
                        yp[row - 4..row + 4].copy_from_slice(&win);
                    }
                }
                if !simple {
                    let x = mx * 8 + 4;
                    if x < uw {
                        let cby = my * 8;
                        for y in cby..(cby + 8).min(uh) {
                            for plane in [&mut up, &mut vp] {
                                let row = (y + 8) * us + (x + 8);
                                let mut win = [0u8; 8];
                                win.copy_from_slice(&plane[row - 4..row + 4]);
                                lf_sub_h(hev, interior, subedge, &mut win);
                                plane[row - 4..row + 4].copy_from_slice(&win);
                            }
                        }
                    }
                }
            }
            // Horizontal edges.
            if my > 0 {
                for x in bx..bx + 16 {
                    let pt = (by + 16) * ys + (x + 16);
                    if simple {
                        lf_simple_v(mbedge, &mut yp, pt, ys);
                    } else {
                        lf_mb_v(hev, interior, mbedge, &mut yp, pt, ys);
                    }
                }
                if !simple {
                    let cby = my * 8;
                    for x in mx * 8..(mx * 8 + 8).min(uw) {
                        let pt = (cby + 8) * us + (x + 8);
                        lf_mb_v(hev, interior, mbedge, &mut up, pt, us);
                        lf_mb_v(hev, interior, mbedge, &mut vp, pt, us);
                    }
                }
            }
            if do_sub {
                for e in [4usize, 8, 12] {
                    let y = by + e;
                    if y >= ph {
                        continue;
                    }
                    for x in bx..bx + 16 {
                        let pt = (y + 16) * ys + (x + 16);
                        if simple {
                            let ok = (yp[pt - ys].abs_diff(yp[pt]) as i32) * 2
                                + (yp[pt - 2 * ys].abs_diff(yp[pt + ys]) as i32) / 2
                                <= subedge as i32;
                            if ok {
                                lf_common_v(true, &mut yp, pt, ys);
                            }
                        } else {
                            lf_sub_v(hev, interior, subedge, &mut yp, pt, ys);
                        }
                    }
                }
                if !simple {
                    let y = my * 8 + 4;
                    if y < uh {
                        for x in mx * 8..(mx * 8 + 8).min(uw) {
                            let pt = (y + 8) * us + (x + 8);
                            lf_sub_v(hev, interior, subedge, &mut up, pt, us);
                            lf_sub_v(hev, interior, subedge, &mut vp, pt, us);
                        }
                    }
                }
            }
        }
    }
    // ---- YUV to RGB ----
    if std::env::var("WEBP_DEBUG").is_ok() {
        let mut counts = [0usize; 5];
        for m in ymodes.iter() {
            counts[match m {
                YMode::Dc => 0,
                YMode::V => 1,
                YMode::H => 2,
                YMode::Tm => 3,
                YMode::B => 4,
            }] += 1;
        }
        let nskip = skips.iter().filter(|&&s| s).count();
        let nnz = mb_nz.iter().filter(|&&s| s).count();
        let mut coeff_nz = 0usize;
        for mb in yblk.iter() {
            for b in mb.iter() {
                coeff_nz += b.iter().filter(|&&c| c != 0).count();
            }
        }
        eprintln!("vp8 modes Dc/V/H/Tm/B={counts:?} skip={nskip} mbnz={nnz} ynzcoeff={coeff_nz}");
        eprintln!("vp8 mb0 y2={:?} y0={:?} u0={:?} v0={:?}", y2blk[0], yblk[0][0], ublk[0][0], vblk[0][0]);
        eprintln!("vp8 mb0 u1={:?} u2={:?} u3={:?} v1={:?} v2={:?} v3={:?}", ublk[0][1], ublk[0][2], ublk[0][3], vblk[0][1], vblk[0][2], vblk[0][3]);
        for bad in [7usize, 11] {
            let dcs: Vec<i32> = yblk[bad].iter().map(|b| b[0]).collect();
            eprintln!("vp8 mb{bad} bmodes={:?} ydcs={dcs:?}", bmodes[bad]);
            let udcs: Vec<i32> = ublk[bad].iter().map(|b| b[0]).collect();
            let vdcs: Vec<i32> = vblk[bad].iter().map(|b| b[0]).collect();
            eprintln!("vp8 mb{bad} udcs={udcs:?} vdcs={vdcs:?}");
        }
    }
    let mut rgba = vec![0u8; pw * ph * 4];
    for y in 0..ph {
        for x in 0..pw {
            let yy = yp[(y + 16) * ys + x + 16] as i32;
            let u = up[(y / 2 + 8) * us + x / 2 + 8] as i32 - 128;
            let v = vp[(y / 2 + 8) * us + x / 2 + 8] as i32 - 128;
            let c = yy - 16;
            let r = (298 * c + 409 * v + 128) >> 8;
            let g = (298 * c - 100 * u - 208 * v + 128) >> 8;
            let b = (298 * c + 516 * u + 128) >> 8;
            let o = (y * pw + x) * 4;
            rgba[o] = r.clamp(0, 255) as u8;
            rgba[o + 1] = g.clamp(0, 255) as u8;
            rgba[o + 2] = b.clamp(0, 255) as u8;
            rgba[o + 3] = 255;
        }
    }
    if let Some(a) = alpha {
        let am = decode_alpha(a, w, h)?;
        for i in 0..pw * ph {
            rgba[i * 4 + 3] = am[i];
        }
    }
    Ok(DecodedWebp { width: w, height: h, pixels: rgba })
}

fn finish_vp8l(
    w: u32,
    h: u32,
    tw: u32,
    transforms: &[Transform],
    mut out: Vec<u32>,
) -> Result<Vec<u32>, WebpError> {
    let mut cur_w = tw;
    for t in transforms.iter().rev() {
        match t {
            Transform::SubtractGreen => apply_subtract_green(&mut out),
            Transform::Predictor { bits, data } => {
                apply_predictor(&mut out, cur_w, h, *bits, data)?;
            }
            Transform::Color { bits, data } => {
                apply_color(&mut out, cur_w, h, *bits, data)?;
            }
            Transform::ColorIndex { table, width_bits, orig_w } => {
                out = apply_color_index(&out, cur_w, h, table, *width_bits, *orig_w)?;
                cur_w = *orig_w;
            }
        }
    }
    if cur_w != w {
        return Err(WebpError::Decode("transform width mismatch".into()));
    }
    if out.len() != (w as usize) * (h as usize) {
        return Err(WebpError::Decode("transform size mismatch".into()));
    }
    Ok(out)
}

// ---- ALPH (transparency) ----

fn decode_alpha(data: &[u8], w: u32, h: u32) -> Result<Vec<u8>, WebpError> {
    if data.is_empty() {
        return Err(WebpError::Decode("truncated ALPH".into()));
    }
    let b = data[0];
    let method = (b >> 2) & 3;
    let comp = b & 3;
    let n = (w as usize)
        .checked_mul(h as usize)
        .ok_or_else(|| WebpError::Decode("image too large".into()))?;
    let filtered: Vec<u8> = if comp == 0 {
        if data.len() - 1 < n {
            return Err(WebpError::Decode("truncated ALPH".into()));
        }
        data[1..1 + n].to_vec()
    } else if comp == 1 {
        let mut dec = Vp8lDecoder { r: LsbReader::new(&data[1..]) };
        let (tw, ts) = dec.read_transforms(w, h)?;
        let raw = dec.decode_stream(tw, h, true, Vec::new())?;
        let px = finish_vp8l(w, h, tw, &ts, raw)?;
        px.iter().map(|p| ((p >> 8) & 0xff) as u8).collect()
    } else {
        return Err(WebpError::Unsupported("bad ALPH compression".into()));
    };
    if filtered.len() != n {
        return Err(WebpError::Decode("bad ALPH size".into()));
    }
    let mut out = vec![0u8; n];
    let stride = w as usize;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let fv = filtered[y * stride + x] as i32;
            let p: i32 = match method {
                0 => 0,
                1 => {
                    if x == 0 {
                        if y == 0 { 0 } else { out[(y - 1) * stride] as i32 }
                    } else {
                        out[y * stride + x - 1] as i32
                    }
                }
                2 => {
                    if y == 0 {
                        if x == 0 { 0 } else { out[x - 1] as i32 }
                    } else {
                        out[(y - 1) * stride + x] as i32
                    }
                }
                _ => {
                    let a = if x == 0 {
                        if y == 0 { 0 } else { out[(y - 1) * stride] as i32 }
                    } else {
                        out[y * stride + x - 1] as i32
                    };
                    let bb = if y == 0 {
                        if x == 0 { 0 } else { out[x - 1] as i32 }
                    } else {
                        out[(y - 1) * stride + x] as i32
                    };
                    let cc = if x == 0 || y == 0 {
                        if x == 0 && y == 0 {
                            0
                        } else if y == 0 {
                            out[x - 1] as i32
                        } else {
                            out[(y - 1) * stride] as i32
                        }
                    } else {
                        out[(y - 1) * stride + x - 1] as i32
                    };
                    (a + bb - cc).clamp(0, 255)
                }
            };
            out[y * stride + x] = ((fv + p) & 0xff) as u8;
        }
    }
    Ok(out)
}

// ---- Lossless encoder (literals only, single group) ----

use std::collections::BinaryHeap;

struct BitWriter {
    buf: Vec<u8>,
    acc: u32,
    n: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self { buf: Vec::new(), acc: 0, n: 0 }
    }
    fn write(&mut self, mut v: u32, mut nbits: u32) {
        while nbits > 0 {
            let take = (8 - self.n as u32).min(nbits);
            self.acc |= (v & ((1 << take) - 1)) << self.n;
            v >>= take;
            nbits -= take;
            self.n += take as u8;
            if self.n == 8 {
                self.buf.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn write_sym(&mut self, code: u32, len: u16) {
        self.write(reverse_bits_len(code, len), len as u32);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.buf.push(self.acc as u8);
        }
        self.buf
    }
    #[allow(dead_code)]
    fn pos(&self) -> usize {
        self.buf.len() * 8 + self.n as usize
    }
}

#[derive(PartialEq, Eq)]
struct HNode {
    freq: u64,
    order: usize,
}

impl Ord for HNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse for min-heap (with deterministic tie-break).
        other.freq.cmp(&self.freq).then(other.order.cmp(&self.order))
    }
}

impl PartialOrd for HNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn huff_try(freqs: &[u64], syms: &[usize]) -> Vec<u16> {
    let n = freqs.len();
    let mut lens = vec![0u16; n];
    if syms.len() == 1 {
        lens[syms[0]] = 1;
        return lens;
    }
    // Arena: (freq, order, left, right, sym).
    struct T {
        freq: u64,
        order: usize,
        left: Option<usize>,
        right: Option<usize>,
        sym: Option<usize>,
    }
    let mut arena: Vec<T> = Vec::new();
    let mut heap = BinaryHeap::new();
    for &s in syms {
        let idx = arena.len();
        arena.push(T { freq: freqs[s], order: s, left: None, right: None, sym: Some(s) });
        heap.push(HNode { freq: freqs[s], order: s });
        let _ = idx;
    }
    let mut next_order = n;
    // Map heap entries to arena indices via (freq, order) lookup.
    // Simpler: push arena indices directly with custom ordering below.
    // Rebuild heap over arena indices:
    let mut heap2 = BinaryHeap::new();
    for (i, t) in arena.iter().enumerate() {
        heap2.push((std::cmp::Reverse((t.freq, t.order)), i));
    }
    while heap2.len() > 1 {
        let (_, a) = heap2.pop().unwrap();
        let (_, b) = heap2.pop().unwrap();
        let idx = arena.len();
        let f = arena[a].freq + arena[b].freq;
        arena.push(T { freq: f, order: next_order, left: Some(a), right: Some(b), sym: None });
        next_order += 1;
        heap2.push((std::cmp::Reverse((f, next_order - 1)), idx));
    }
    let (_, root) = heap2.pop().unwrap();
    // Depths via stack.
    let mut stack = vec![(root, 0u16)];
    while let Some((i, d)) = stack.pop() {
        if let Some(s) = arena[i].sym {
            lens[s] = d.max(1);
        } else {
            let l = arena[i].left.unwrap();
            let r = arena[i].right.unwrap();
            stack.push((l, d + 1));
            stack.push((r, d + 1));
        }
    }
    let _ = heap;
    lens
}

fn huff_lengths(freqs: &[u32], limit: u16) -> Result<Vec<u16>, WebpError> {
    let n = freqs.len();
    let syms: Vec<usize> = (0..n).filter(|&i| freqs[i] > 0).collect();
    if syms.is_empty() {
        return Err(WebpError::Encode("empty alphabet".into()));
    }
    let mut f: Vec<u64> = freqs.iter().map(|&x| x as u64).collect();
    for _ in 0..24 {
        let lens = huff_try(&f, &syms);
        if lens.iter().all(|&x| x <= limit) {
            return Ok(lens);
        }
        for i in 0..n {
            if freqs[i] == 0 {
                f[i] = 0;
            } else {
                f[i] = (f[i] / 2).max(1);
            }
        }
    }
    Err(WebpError::Encode("cannot build huffman".into()))
}

fn canon(lengths: &[u16]) -> Vec<u32> {
    let mut codes = vec![0u32; lengths.len()];
    let mut code = 0u32;
    for len in 1..16u16 {
        for (s, &l) in lengths.iter().enumerate() {
            if l == len {
                codes[s] = code;
                code += 1;
            }
        }
        code <<= 1;
    }
    codes
}

enum RleOp {
    Lit(u32),
    Rep16(u32), // extra 2 bits: repeat-3
    Rep17(u32), // extra 3 bits: zeros-3
    Rep18(u32), // extra 7 bits: zeros-11
}

fn rle_op_sym(op: &RleOp) -> usize {
    match op {
        RleOp::Lit(s) => *s as usize,
        RleOp::Rep16(_) => 16,
        RleOp::Rep17(_) => 17,
        RleOp::Rep18(_) => 18,
    }
}

fn write_table(bw: &mut BitWriter, lengths: &[u16], alphabet: usize) -> Result<(), WebpError> {
    let mut ops: Vec<RleOp> = Vec::new();
    let mut i = 0;
    while i < alphabet {
        let l = lengths[i];
        let mut run = 1usize;
        while i + run < alphabet && lengths[i + run] == l && run < 138 {
            run += 1;
        }
        if l == 0 {
            let mut k = run;
            while k > 0 {
                if k >= 11 {
                    let use_ = k.min(138);
                    ops.push(RleOp::Rep18(use_ as u32 - 11));
                    k -= use_;
                } else if k >= 3 {
                    ops.push(RleOp::Rep17(k as u32 - 3));
                    k = 0;
                } else {
                    for _ in 0..k {
                        ops.push(RleOp::Lit(0));
                    }
                    k = 0;
                }
            }
        } else {
            ops.push(RleOp::Lit(l as u32));
            let mut k = run - 1;
            while k > 0 {
                if k >= 3 {
                    let use_ = k.min(6);
                    ops.push(RleOp::Rep16(use_ as u32 - 3));
                    k -= use_;
                } else {
                    for _ in 0..k {
                        ops.push(RleOp::Lit(l as u32));
                    }
                    k = 0;
                }
            }
        }
        i += run;
    }
    let mut cf = [0u32; 19];
    for op in &ops {
        cf[rle_op_sym(op)] += 1;
    }
    let cl_lens = huff_lengths(&cf, 7)?;
    let cl_codes = canon(&cl_lens);
    let mut last = 0usize;
    for (k, &o) in CODE_ORDER.iter().enumerate() {
        if cl_lens[o] != 0 {
            last = k + 1;
        }
    }
    let count = last.max(4);
    bw.write(0, 1); // normal (non-simple) code length code
    bw.write(count as u32 - 4, 4);
    for k in 0..count {
        bw.write(cl_lens[CODE_ORDER[k]] as u32, 3);
    }
    bw.write(0, 1); // full alphabet size
    // A single-symbol code book consumes no bits on the wire.
    let cl_single = cl_lens.iter().filter(|&&l| l != 0).count() == 1;
    for op in &ops {
        match op {
            RleOp::Lit(s) => {
                if !cl_single {
                    bw.write_sym(cl_codes[*s as usize], cl_lens[*s as usize]);
                }
            }
            RleOp::Rep16(e) => {
                if !cl_single {
                    bw.write_sym(cl_codes[16], cl_lens[16]);
                }
                bw.write(*e, 2);
            }
            RleOp::Rep17(e) => {
                if !cl_single {
                    bw.write_sym(cl_codes[17], cl_lens[17]);
                }
                bw.write(*e, 3);
            }
            RleOp::Rep18(e) => {
                if !cl_single {
                    bw.write_sym(cl_codes[18], cl_lens[18]);
                }
                bw.write(*e, 7);
            }
        }
    }
    Ok(())
}

fn encode_lossless(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, WebpError> {
    if width == 0 || height == 0 {
        return Err(WebpError::Encode("width and height must be > 0".into()));
    }
    if width > 16384 || height > 16384 {
        return Err(WebpError::Encode("image too large".into()));
    }
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(WebpError::Encode("pixel buffer length mismatch".into()));
    }
    let n = (width as usize) * (height as usize);
    let mut px = Vec::with_capacity(n);
    let mut opaque = true;
    for i in 0..n {
        let r = rgba[i * 4];
        let g = rgba[i * 4 + 1];
        let b = rgba[i * 4 + 2];
        let a = rgba[i * 4 + 3];
        if a != 255 {
            opaque = false;
        }
        px.push(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32);
    }
    let mut fg = [0u32; 280];
    let mut fr = [0u32; 256];
    let mut fb = [0u32; 256];
    let mut fa = [0u32; 256];
    for &p in &px {
        fg[((p >> 8) & 0xff) as usize] += 1;
        fr[((p >> 16) & 0xff) as usize] += 1;
        fb[(p & 0xff) as usize] += 1;
        fa[((p >> 24) & 0xff) as usize] += 1;
    }
    let lg = huff_lengths(&fg, 15)?;
    let lr = huff_lengths(&fr, 15)?;
    let lb = huff_lengths(&fb, 15)?;
    let la = huff_lengths(&fa, 15)?;
    if std::env::var("WEBP_DEBUG").is_ok() {
        let nz = |v: &Vec<u16>| -> Vec<(usize, u16)> {
            v.iter().enumerate().filter(|&(_, &l)| l != 0).map(|(i, &l)| (i, l)).collect()
        };
        eprintln!("enc green {:?} red {:?} blue {:?} alpha {:?}", nz(&lg), nz(&lr), nz(&lb), nz(&la));
    }
    let mut ld = vec![0u16; 40];
    ld[0] = 1;
    let cg = canon(&lg);
    let cr = canon(&lr);
    let cb = canon(&lb);
    let ca = canon(&la);
    // Single-symbol books transmit no bits per symbol (VP8L spec).
    let single = [
        lg.iter().filter(|&&l| l != 0).count() == 1,
        lr.iter().filter(|&&l| l != 0).count() == 1,
        lb.iter().filter(|&&l| l != 0).count() == 1,
        la.iter().filter(|&&l| l != 0).count() == 1,
    ];
    if std::env::var("WEBP_DEBUG").is_ok() {
        eprintln!("px {:08x?}", px);
        eprintln!("cg3={} cr4={} ca7={} lg3={} lr4={} la7={}", cg[3], cr[4], ca[7], lg[3], lr[4], la[7]);
    }
    let mut bw = BitWriter::new();
    bw.write(0x2f, 8);
    bw.write(width - 1, 14);
    bw.write(height - 1, 14);
    bw.write((!opaque) as u32, 1);
    bw.write(0, 3);
    bw.write(0, 1); // no transforms
    bw.write(0, 1); // no color cache
    bw.write(0, 1); // single prefix group
    write_table(&mut bw, &lg, 280)?;
    write_table(&mut bw, &lr, 256)?;
    write_table(&mut bw, &lb, 256)?;
    write_table(&mut bw, &la, 256)?;
    write_table(&mut bw, &ld, 40)?;
    if std::env::var("WEBP_DEBUG").is_ok() {
        eprintln!("enc pixel start bitpos {}", bw.pos());
    }
    for &p in &px {
        let g = ((p >> 8) & 0xff) as usize;
        let r = ((p >> 16) & 0xff) as usize;
        let b = (p & 0xff) as usize;
        let a = ((p >> 24) & 0xff) as usize;
        if !single[0] {
            bw.write_sym(cg[g], lg[g]);
        }
        if !single[1] {
            bw.write_sym(cr[r], lr[r]);
        }
        if !single[2] {
            bw.write_sym(cb[b], lb[b]);
        }
        if !single[3] {
            bw.write_sym(ca[a], la[a]);
        }
    }
    let payload = bw.finish();
    let mut out = Vec::with_capacity(12 + 8 + payload.len() + 1);
    out.extend_from_slice(b"RIFF");
    let riff_size = (4 + 8 + payload.len() + (payload.len() & 1)) as u32;
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(b"VP8L");
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    if payload.len() & 1 == 1 {
        out.push(0);
    }
    Ok(out)
}







