//! Tests for the pure-Rust GIF codec (`src/codecs/gif.rs`).
//!
//! An independent GIF assembler lives in this file; LZW streams use
//! clear-literal-EOI sequences packed LSB-first by hand. Reference
//! fixtures in `tests/data/` come from Pillow (`tests/gen_gif.py`).
//! GIF is lossless: cross-checks against the `image` crate are exact.

use coreimage::codecs::gif;

// ── Independent test-only GIF builder ─────────────────────────

/// Pack (code, size) pairs LSB-first. Sizes are explicit per code so
/// streams stay valid across LZW code-size bumps (clears reset the size).
fn pack_codes(codes: &[(u32, u32)]) -> Vec<u8> {
    let mut acc = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::new();
    for &(c, size) in codes {
        acc |= c << nbits;
        nbits += size;
        while nbits >= 8 {
            out.push((acc & 0xFF) as u8);
            acc >>= 8;
            nbits -= 8;
        }
    }
    if nbits > 0 {
        out.push((acc & 0xFF) as u8);
    }
    out
}

/// Clear-delimited literal runs with one literal per segment, tracking
/// the decoder's code size exactly (clear resets it, each literal grows
/// the table by one and may bump it). Mirrors real encoder behavior
/// where clears ride at the current size. Mid-stream bumps get their
/// own test (`bump_midstream`).
fn lit_stream(clear: u32, eoi: u32, min_size: u32, px: &[u8]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut size = min_size + 1;
    let mut next = (1 << min_size) + 2; // eoi + 1
    for &v in px {
        out.push((clear, size));
        size = min_size + 1;
        next = (1 << min_size) + 2;
        out.push((v as u32, size));
        next += 1;
        if next >= (1 << size) && size < 12 {
            size += 1;
        }
    }
    out.push((eoi, size));
    out
}

fn table_bytes(pal: &[[u8; 3]]) -> Vec<u8> {
    pal.iter().flat_map(|c| *c).collect()
}

fn size_field(entries: usize) -> u8 {
    // entries is a power of two: field so that 2 << field == entries.
    (usize::BITS - (entries as u32).leading_zeros()) as u8 - 2
}

struct Frame {
    left: u16,
    top: u16,
    w: u16,
    h: u16,
    interlaced: bool,
    lct: Option<Vec<[u8; 3]>>,
    min_size: u8,
    codes: Vec<(u32, u32)>,
}

struct GifSpec {
    magic: &'static [u8; 6],
    width: u16,
    height: u16,
    gct: Option<Vec<[u8; 3]>>,
    gce_trans: Option<u8>,
    pre_exts: Vec<Vec<u8>>,
    frames: Vec<Frame>,
    trailer: bool,
}

fn gce(trans: Option<u8>) -> Vec<u8> {
    match trans {
        Some(t) => vec![0x21, 0xF9, 4, 0x01, 0, 0, t, 0],
        None => vec![],
    }
}

fn build_gif(s: &GifSpec) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(s.magic);
    out.extend_from_slice(&s.width.to_le_bytes());
    out.extend_from_slice(&s.height.to_le_bytes());
    match &s.gct {
        Some(g) => {
            out.push(0x80 | (7 << 4) | size_field(g.len()));
            out.push(0);
            out.push(0);
            out.extend_from_slice(&table_bytes(g));
        }
        None => out.extend_from_slice(&[0, 0, 0]),
    }
    out.extend_from_slice(&gce(s.gce_trans));
    for e in &s.pre_exts {
        out.extend_from_slice(e);
    }
    for f in &s.frames {
        out.push(0x2C);
        out.extend_from_slice(&f.left.to_le_bytes());
        out.extend_from_slice(&f.top.to_le_bytes());
        out.extend_from_slice(&f.w.to_le_bytes());
        out.extend_from_slice(&f.h.to_le_bytes());
        let mut packed = 0u8;
        if let Some(lct) = &f.lct {
            packed |= 0x80 | size_field(lct.len());
        }
        if f.interlaced {
            packed |= 0x40;
        }
        out.push(packed);
        if let Some(lct) = &f.lct {
            out.extend_from_slice(&table_bytes(lct));
        }
        out.push(f.min_size);
        let data = pack_codes(&f.codes);
        out.push(data.len() as u8);
        out.extend_from_slice(&data);
        out.push(0);
    }
    if s.trailer {
        out.push(0x3B);
    }
    out
}

fn comment_ext(text: &[u8]) -> Vec<u8> {
    let mut e = vec![0x21, 0xFE];
    e.push(text.len() as u8);
    e.extend_from_slice(text);
    e.push(0);
    e
}

fn netscape_ext() -> Vec<u8> {
    vec![0x21, 0xFF, 11, b'N', b'E', b'T', b'S', b'C', b'A', b'P', b'E', b'2', b'.', b'0', 3, 1, 0, 0, 0]
}

fn plaintext_ext() -> Vec<u8> {
    vec![0x21, 0x01, 12, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 1, b'X', 0]
}

// ── Hand-built vectors ────────────────────────────────────────

const BW: [[u8; 3]; 2] = [[10, 20, 30], [200, 210, 220]];

#[test]
fn basic_gct_decode() {
    // 4x2, min code size 2: clear-delimited literal groups at 3 bits.
    let px = [0, 1, 1, 0, 1, 0, 0, 1];
    let file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 4,
        height: 2,
        gct: Some(BW.to_vec()),
        gce_trans: None,
        pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 4, h: 2, interlaced: false,
            lct: None, min_size: 2, codes: lit_stream(4, 5, 2, &px),
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    assert_eq!((dec.width, dec.height), (4, 2));
    let mut expect = Vec::new();
    for &i in &px {
        let c = BW[i as usize];
        expect.extend_from_slice(&[c[0], c[1], c[2], 255]);
    }
    assert_eq!(dec.pixels, expect);
    assert_eq!(gif::dimensions(&file), Ok((4, 2)));
    assert!(gif::is_gif(&file));
}

#[test]
fn min_code_size_1_and_87a() {
    // 2x1 GIF87a, min code size 1: clear=2, eoi=3 at 2 bits.
    let file = build_gif(&GifSpec {
        magic: b"GIF87a",
        width: 2,
        height: 1,
        gct: Some(BW.to_vec()),
        gce_trans: None,
        pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 1, interlaced: false,
            lct: None, min_size: 1,
            // No table add happens for the first literal, so the width
            // stays 2 until after the second literal (next hits 5 >= 4).
            codes: vec![(2, 2), (0, 2), (2, 2), (1, 2), (3, 3)],
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    assert_eq!(dec.pixels, vec![10, 20, 30, 255, 200, 210, 220, 255]);
}

#[test]
fn lct_overrides_gct() {
    let red = [[255, 0, 0], [0, 0, 0]];
    let blue = [[0, 0, 255], [0, 0, 0]];
    let file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 1,
        height: 1,
        gct: Some(red.to_vec()),
        gce_trans: None,
        pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 1, h: 1, interlaced: false,
            lct: Some(blue.to_vec()), min_size: 2, codes: vec![(4, 3), (0, 3), (5, 3)],
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    assert_eq!(dec.pixels, vec![0, 0, 255, 255]);
}

#[test]
fn gce_transparency() {
    let file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 2,
        height: 1,
        gct: Some(BW.to_vec()),
        gce_trans: Some(1),
        pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 1, interlaced: false,
            lct: None, min_size: 2, codes: lit_stream(4, 5, 2, &[0, 1]),
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    assert_eq!(dec.pixels, vec![10, 20, 30, 255, 200, 210, 220, 0]);
}

#[test]
fn interlaced_decode() {
    // 8x8, rows in interlace pass order, min code size 3.
    let mut gray = [[0u8, 0, 0]; 8];
    for (i, c) in gray.iter_mut().enumerate() {
        *c = [(i * 32) as u8, 0, 0];
    }
    let order = [0usize, 4, 2, 6, 1, 3, 5, 7];
    let mut px = Vec::new();
    for &r in &order {
        px.extend([r as u8; 8]);
    }
    let file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 8,
        height: 8,
        gct: Some(gray.to_vec()),
        gce_trans: None,
        pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 8, h: 8, interlaced: true,
            lct: None, min_size: 3, codes: lit_stream(8, 9, 3, &px),
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    for y in 0..8usize {
        for x in 0..8usize {
            let o = (y * 8 + x) * 4;
            assert_eq!(dec.pixels[o], (y * 32) as u8, "row {y}");
            assert_eq!(dec.pixels[o + 3], 255);
        }
    }
}

#[test]
fn bump_midstream() {
    // 8x1, min code size 2. The decoder adds nothing for the first
    // literal, so the width grows only after the third literal
    // (next hits 8 == 2^3): first three literals ride at 3 bits,
    // the rest plus EOI at 4 bits.
    let file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 8,
        height: 1,
        gct: Some(BW.to_vec()),
        gce_trans: None,
        pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 8, h: 1, interlaced: false,
            lct: None, min_size: 2,
            codes: vec![
                (4, 3), (0, 3), (1, 3), (0, 3),
                (1, 4), (0, 4), (1, 4), (0, 4), (1, 4), (5, 4),
            ],
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    let mut expect = Vec::new();
    for &i in &[0, 1, 0, 1, 0, 1, 0, 1] {
        let c = BW[i as usize];
        expect.extend_from_slice(&[c[0], c[1], c[2], 255]);
    }
    assert_eq!(dec.pixels, expect);
}

#[test]
fn extensions_skipped() {
    let file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 1,
        height: 1,
        gct: Some(BW.to_vec()),
        gce_trans: None,
        pre_exts: vec![comment_ext(b"hello"), netscape_ext(), plaintext_ext()],
        frames: vec![Frame {
            left: 0, top: 0, w: 1, h: 1, interlaced: false,
            lct: None, min_size: 2, codes: vec![(4, 3), (1, 3), (5, 3)],
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    assert_eq!(dec.pixels, vec![200, 210, 220, 255]);
}

#[test]
fn multi_frame_first_wins() {
    let red = [[255, 0, 0], [0, 0, 0]];
    let blue = [[0, 0, 255], [0, 0, 0]];
    let mut file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 2,
        height: 2,
        gct: Some(red.to_vec()),
        gce_trans: None,
        pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 2, interlaced: false,
            lct: None, min_size: 2,
            codes: lit_stream(4, 5, 2, &[0, 0, 0, 0]),
        }],
        trailer: false,
    });
    file.extend_from_slice(&gce(None));
    file.push(0x2C);
    file.extend_from_slice(&[0, 0, 0, 0, 2, 0, 2, 0, 0x80]);
    file.extend_from_slice(&table_bytes(&blue));
    file.push(2);
    let data = pack_codes(&[(4, 3), (1, 3), (1, 3), (1, 3), (1, 3), (5, 3)]);
    file.push(data.len() as u8);
    file.extend_from_slice(&data);
    file.extend_from_slice(&[0, 0x3B]);
    let dec = gif::decode(&file).expect("decode");
    assert_eq!(dec.pixels, vec![255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255]);
}

#[test]
fn positioned_subimage() {
    // 6x4 canvas, 2x2 red frame at (2,1), rest transparent.
    let red = [[255, 0, 0], [0, 0, 0]];
    let file = build_gif(&GifSpec {
        magic: b"GIF89a",
        width: 6,
        height: 4,
        gct: Some(red.to_vec()),
        gce_trans: Some(1),
        pre_exts: vec![],
        frames: vec![Frame {
            left: 2, top: 1, w: 2, h: 2, interlaced: false,
            lct: None, min_size: 2, codes: lit_stream(4, 5, 2, &[0, 0, 0, 0]),
        }],
        trailer: true,
    });
    let dec = gif::decode(&file).expect("decode");
    assert_eq!((dec.width, dec.height), (6, 4));
    for y in 0..4u32 {
        for x in 0..6u32 {
            let o = ((y * 6 + x) * 4) as usize;
            if (2..4).contains(&x) && (1..3).contains(&y) {
                assert_eq!(&dec.pixels[o..o + 4], &[255, 0, 0, 255], "at {x},{y}");
            } else {
                assert_eq!(dec.pixels[o + 3], 0, "transparent at {x},{y}");
            }
        }
    }
}

// ── Errors ────────────────────────────────────────────────────

#[test]
fn error_cases() {
    assert!(!gif::is_gif(b""));
    assert!(!gif::is_gif(b"GIF38a"));
    assert!(gif::dimensions(b"nope").is_err());
    assert!(gif::decode(b"GIF89a\x01\x00").is_err()); // truncated LSD
    assert!(gif::decode(&[0u8; 20]).is_err()); // bad magic

    // Zero dimensions.
    let mut f = build_gif(&GifSpec {
        magic: b"GIF89a", width: 0, height: 0, gct: None,
        gce_trans: None, pre_exts: vec![], frames: vec![], trailer: true,
    });
    assert!(gif::decode(&f).is_err());
    assert!(gif::dimensions(&f).is_err());

    // No color table anywhere.
    f = build_gif(&GifSpec {
        magic: b"GIF89a", width: 2, height: 2, gct: None,
        gce_trans: None, pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 2, interlaced: false,
            lct: None, min_size: 2, codes: lit_stream(4, 5, 2, &[0, 0, 0, 0]),
        }],
        trailer: true,
    });
    assert!(gif::decode(&f).is_err());

    // Bad minimum code sizes.
    for min in [0u8, 9] {
        let f = build_gif(&GifSpec {
            magic: b"GIF89a", width: 2, height: 2, gct: Some(BW.to_vec()),
            gce_trans: None, pre_exts: vec![],
            frames: vec![Frame {
                left: 0, top: 0, w: 2, h: 2, interlaced: false,
                lct: None, min_size: min,
                codes: lit_stream(4, 5, 2, &[0, 0, 0, 0]),
            }],
            trailer: true,
        });
        assert!(gif::decode(&f).is_err(), "min {min}");
    }

    // Invalid first code (clear then table code with no prefix).
    let f = build_gif(&GifSpec {
        magic: b"GIF89a", width: 2, height: 2, gct: Some(BW.to_vec()),
        gce_trans: None, pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 2, interlaced: false,
            lct: None, min_size: 2, codes: vec![(4, 3), (6, 3), (5, 3)],
        }],
        trailer: true,
    });
    assert!(gif::decode(&f).is_err());

    // Code beyond the table (min size 4 keeps widths stable).
    let f = build_gif(&GifSpec {
        magic: b"GIF89a", width: 2, height: 2, gct: Some(BW.to_vec()),
        gce_trans: None, pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 2, interlaced: false,
            lct: None, min_size: 4, codes: vec![(16, 5), (0, 5), (32, 5), (17, 5)],
        }],
        trailer: true,
    });
    assert!(gif::decode(&f).is_err());

    // Too many / too few pixels.
    for px in [vec![0, 0, 0, 0, 0], vec![0, 0, 0]] {
        let f = build_gif(&GifSpec {
            magic: b"GIF89a", width: 2, height: 2, gct: Some(BW.to_vec()),
            gce_trans: None, pre_exts: vec![],
            frames: vec![Frame {
                left: 0, top: 0, w: 2, h: 2, interlaced: false,
                lct: None, min_size: 2, codes: lit_stream(4, 5, 2, &px),
            }],
            trailer: true,
        });
        assert!(gif::decode(&f).is_err());
    }

    // Truncated stream.
    let mut f = build_gif(&GifSpec {
        magic: b"GIF89a", width: 2, height: 2, gct: Some(BW.to_vec()),
        gce_trans: None, pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 2, interlaced: false,
            lct: None, min_size: 2, codes: lit_stream(4, 5, 2, &[0, 0, 0, 0]),
        }],
        trailer: true,
    });
    f.truncate(f.len() - 4);
    assert!(gif::decode(&f).is_err());

    // Unknown top-level block (header followed by 0x00).
    let mut g = f[..19].to_vec();
    g.push(0x00);
    assert!(gif::decode(&g).is_err());

    // Missing trailer is lenient (frame complete).
    let f = build_gif(&GifSpec {
        magic: b"GIF89a", width: 2, height: 2, gct: Some(BW.to_vec()),
        gce_trans: None, pre_exts: vec![],
        frames: vec![Frame {
            left: 0, top: 0, w: 2, h: 2, interlaced: false,
            lct: None, min_size: 2, codes: lit_stream(4, 5, 2, &[0, 1, 1, 0]),
        }],
        trailer: false,
    });
    assert!(gif::decode(&f).is_ok());

    // Encoder validation.
    assert!(gif::encode(0, 4, &[]).is_err());
    assert!(gif::encode(4, 4, &[1, 2, 3]).is_err());
}

// ── Roundtrips (exact: GIF is lossless) ───────────────────────

fn checkboard_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            if (x / 4 + y / 4) % 2 == 0 {
                px.extend_from_slice(&[200, 30, 40, 255]);
            } else if (x + y) % 7 == 0 {
                px.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                px.extend_from_slice(&[20, 160, 90, 255]);
            }
        }
    }
    px
}

#[test]
fn roundtrip_exact_small_palette() {
    for (w, h) in [(16u32, 16u32), (37, 23), (1, 1), (7, 5)] {
        let px = checkboard_rgba(w, h);
        let enc = gif::encode(w, h, &px).expect("encode");
        let dec = gif::decode(&enc).expect("decode");
        assert_eq!((dec.width, dec.height), (w, h));
        assert_eq!(dec.pixels, px, "{w}x{h}");
    }
}

#[test]
fn roundtrip_256_gradient_exact() {
    let (w, h) = (16u32, 16u32);
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, 7, 255]);
        }
    }
    let enc = gif::encode(w, h, &px).expect("encode");
    let dec = gif::decode(&enc).expect("decode");
    assert_eq!(dec.pixels, px);
}

#[test]
fn roundtrip_quantized_many_colors() {
    // Photo-like content exceeds 256 colors: median-cut path.
    let (w, h) = (64u32, 48u32);
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[
                ((x * 4 + y * 2) % 256) as u8,
                ((y * 5 + x) % 256) as u8,
                (((x * x + y * y) / 7) % 256) as u8,
                255,
            ]);
        }
    }
    let enc = gif::encode(w, h, &px).expect("encode");
    let dec = gif::decode(&enc).expect("decode");
    assert_eq!((dec.width, dec.height), (w, h));
    let unique: std::collections::HashSet<[u8; 3]> = dec
        .pixels
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    assert!(unique.len() <= 256, "palette overflow: {}", unique.len());
    assert!(dec.pixels.chunks_exact(4).all(|p| p[3] == 255));
}

#[test]
fn roundtrip_all_transparent() {
    let px = vec![0u8; 8 * 8 * 4];
    let enc = gif::encode(8, 8, &px).expect("encode");
    let dec = gif::decode(&enc).expect("decode");
    assert!(dec.pixels.chunks_exact(4).all(|p| p[3] == 0));
}

// ── Cross-checks vs `image` crate (exact: lossless) ───────────

#[test]
fn cross_image_crate_exact() {
    let px = checkboard_rgba(32, 24);
    let enc = gif::encode(32, 24, &px).expect("encode");
    let tie = image::load_from_memory(&enc).expect("image decode").to_rgba8().into_raw();
    assert_eq!(tie, px);

    // The `image` crate's encoder output decodes identically here.
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut e = image::codecs::gif::GifEncoder::new(&mut out);
        e.encode(&px, 32, 24, image::ExtendedColorType::Rgba8).unwrap();
    }
    let dec = gif::decode(out.get_ref()).expect("our decode");
    assert_eq!(dec.pixels, px);
}

// ── Pillow fixtures (exact: lossless) ─────────────────────────

#[test]
fn pil_fixtures_exact() {
    for name in [
        "gif_8bit.gif",
        "gif_4bit.gif",
        "gif_2bit.gif",
        "gif_gray.gif",
        "gif_odd_37x23.gif",
        "gif_interlaced.gif",
        "gif_transparent.gif",
    ] {
        let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(&path).unwrap();
        assert!(gif::is_gif(&bytes), "{name}");
        let ours = gif::decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let tie = image::load_from_memory(&bytes).expect("image").to_rgba8();
        assert_eq!((ours.width, ours.height), (tie.width(), tie.height()), "{name}");
        assert_eq!(ours.pixels, tie.into_raw(), "{name}");
    }
    // Animated: first frame is solid red.
    let bytes = std::fs::read(format!(
        "{}/tests/data/gif_animated.gif",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let dec = gif::decode(&bytes).expect("animated");
    assert_eq!((dec.width, dec.height), (24, 18));
    assert!(dec.pixels.chunks_exact(4).all(|p| p[..3] == [200, 10, 10]));
    // Transparency fixture: punched circle is transparent, corner opaque.
    let bytes = std::fs::read(format!(
        "{}/tests/data/gif_transparent.gif",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let dec = gif::decode(&bytes).expect("transparent");
    assert_eq!(dec.pixels[(16 * 48 + 24) * 4 + 3], 0);
    assert_eq!(dec.pixels[3], 255);
}

// ── TiImage integration ───────────────────────────────────────

#[test]
fn tiimage_save_load_gif() {
    use coreimage::{ImageFormat, TiImage};
    let mut px = Vec::new();
    for y in 0..16u32 {
        for x in 0..16u32 {
            px.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, 7, 255]);
        }
    }
    let img = TiImage::from_rgba(image::RgbaImage::from_raw(16, 16, px.clone()).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("anim.gif");
    img.save(path.to_str().unwrap(), ImageFormat::Gif, 100).unwrap();
    let back = TiImage::load(path.to_str().unwrap()).unwrap();
    assert_eq!(back.dimensions(), (16, 16));
    assert_eq!(back.into_rgba().into_raw(), px);
    assert_eq!(TiImage::probe_format(path.to_str().unwrap()).unwrap(), ImageFormat::Gif);
}
