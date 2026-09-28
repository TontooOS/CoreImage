//! Tests for the pure-Rust BMP codec (`src/codecs/bmp.rs`).
//!
//! An independent BMP assembler lives in this file (headers, palettes,
//! rows, RLE streams all hand-built). Reference fixtures in
//! `tests/data/` come from Pillow (`tests/gen_bmp.py`). BMP is
//! lossless: cross-checks against the `image` crate are exact.

use coreimage::codecs::bmp;

// ── Independent test-only BMP builder ─────────────────────────

fn info40(w: i32, h: i32, bpp: u16, comp: u32, colors: u32) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(&40u32.to_le_bytes());
    o.extend_from_slice(&w.to_le_bytes());
    o.extend_from_slice(&h.to_le_bytes());
    o.extend_from_slice(&1u16.to_le_bytes());
    o.extend_from_slice(&bpp.to_le_bytes());
    o.extend_from_slice(&comp.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&2835u32.to_le_bytes());
    o.extend_from_slice(&2835u32.to_le_bytes());
    o.extend_from_slice(&colors.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o
}

fn core12(w: u16, h: u16, bpp: u16) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(&12u32.to_le_bytes());
    o.extend_from_slice(&w.to_le_bytes());
    o.extend_from_slice(&h.to_le_bytes());
    o.extend_from_slice(&1u16.to_le_bytes());
    o.extend_from_slice(&bpp.to_le_bytes());
    o
}

fn pal_quad(p: &[[u8; 3]]) -> Vec<u8> {
    let mut o = Vec::new();
    for c in p {
        o.extend_from_slice(&[c[2], c[1], c[0], 0]);
    }
    o
}

fn pal_triple(p: &[[u8; 3]]) -> Vec<u8> {
    let mut o = Vec::new();
    for c in p {
        o.extend_from_slice(&[c[2], c[1], c[0]]);
    }
    o
}

fn file(dib: &[u8], gap: &[u8], rows: &[u8]) -> Vec<u8> {
    let off = 14 + dib.len() + gap.len();
    let size = (off + rows.len()) as u32;
    let mut o = Vec::new();
    o.extend_from_slice(b"BM");
    o.extend_from_slice(&size.to_le_bytes());
    o.extend_from_slice(&[0u8; 4]);
    o.extend_from_slice(&(off as u32).to_le_bytes());
    o.extend_from_slice(dib);
    o.extend_from_slice(gap);
    o.extend_from_slice(rows);
    o
}

// ── Hand-built vectors ────────────────────────────────────────

#[test]
fn hand_24bit_bottom_up_and_top_down() {
    // 3x2 BGR. Bottom-up: file row 0 = image y=1.
    let rows_bu = [
        30u8, 20, 10, 60, 50, 40, 90, 80, 70, 0, 0, 0, // y=1
        3, 2, 1, 6, 5, 4, 9, 8, 7, 0, 0, 0, // y=0
    ];
    let f = file(&info40(3, 2, 24, 0, 0), &[], &rows_bu);
    let dec = bmp::decode(&f).expect("decode");
    assert_eq!((dec.width, dec.height), (3, 2));
    assert_eq!(
        dec.pixels,
        vec![
            1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255,
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255,
        ]
    );
    // Top-down: negative height, file rows in image order.
    let rows_td = [
        3u8, 2, 1, 6, 5, 4, 9, 8, 7, 0, 0, 0,
        30, 20, 10, 60, 50, 40, 90, 80, 70, 0, 0, 0,
    ];
    let f = file(&info40(3, -2, 24, 0, 0), &[], &rows_td);
    let dec = bmp::decode(&f).expect("decode td");
    assert_eq!(
        dec.pixels,
        vec![
            1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255,
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255,
        ]
    );
}

#[test]
fn hand_32bit_alpha() {
    // 2x1 BGRA, alpha preserved.
    let rows = [30u8, 20, 10, 40, 70, 60, 50, 80];
    let f = file(&info40(2, 1, 32, 0, 0), &[], &rows);
    let dec = bmp::decode(&f).expect("decode");
    assert_eq!(dec.pixels, vec![10, 20, 30, 40, 50, 60, 70, 80]);
}

#[test]
fn hand_palettes_1_4_8() {
    // 1-bit 4x1: indices [1,0,1,0].
    let pal = [[0, 0, 0], [255, 255, 255]];
    let mut dib = core12(4, 1, 1);
    dib.extend_from_slice(&pal_triple(&pal));
    let f = file(&dib, &[], &[0xA0, 0, 0, 0]);
    let dec = bmp::decode(&f).expect("1bit");
    assert_eq!(
        dec.pixels,
        vec![255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255]
    );

    // 4-bit 2x1 core header with RGBTRIPLE palette: indices [2,13].
    let mut pal4 = [[0u8, 0, 0]; 16];
    pal4[2] = [1, 2, 3];
    pal4[13] = [4, 5, 6];
    let mut dib = core12(2, 1, 4);
    dib.extend_from_slice(&pal_triple(&pal4));
    let f = file(&dib, &[], &[0x2D, 0, 0, 0]);
    let dec = bmp::decode(&f).expect("4bit");
    assert_eq!(dec.pixels, vec![1, 2, 3, 255, 4, 5, 6, 255]);

    // 8-bit 3x1 info header, indices [0,128,255].
    let mut pal8 = [[0u8, 0, 0]; 256];
    pal8[0] = [9, 9, 9];
    pal8[128] = [1, 2, 3];
    pal8[255] = [7, 8, 9];
    let mut dib = info40(3, 1, 8, 0, 0);
    dib.extend_from_slice(&pal_quad(&pal8));
    let f = file(&dib, &[], &[0u8, 128, 255, 0]);
    let dec = bmp::decode(&f).expect("8bit");
    assert_eq!(
        dec.pixels,
        vec![9, 9, 9, 255, 1, 2, 3, 255, 7, 8, 9, 255]
    );
}

#[test]
fn hand_v4_and_gap() {
    // 108-byte V4 header (BI_RGB 24-bit) + 16 gap bytes before pixels.
    let mut full = info40(2, 1, 24, 0, 0);
    full.extend_from_slice(&[0u8; 68]);
    full[0..4].copy_from_slice(&108u32.to_le_bytes());
    let rows = [3u8, 2, 1, 6, 5, 4, 0, 0];
    let f = file(&full, &[0xAA; 16], &rows);
    let dec = bmp::decode(&f).expect("v4+gap");
    assert_eq!(dec.pixels, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    assert_eq!(bmp::dimensions(&f), Ok((2, 1)));
}

#[test]
fn hand_16bit_555() {
    // BI_RGB 16-bit = XRGB 555: white, red, green, blue.
    let mut rows = Vec::new();
    for v in [0x7FFFu16, 0x7C00, 0x03E0, 0x001F] {
        rows.extend_from_slice(&v.to_le_bytes());
    }
    let f = file(&info40(4, 1, 16, 0, 0), &[], &rows);
    let dec = bmp::decode(&f).expect("16bit");
    assert_eq!(
        dec.pixels,
        vec![
            255, 255, 255, 255, 255, 0, 0, 255,
            0, 255, 0, 255, 0, 0, 255, 255,
        ]
    );
}

#[test]
fn hand_16bit_565_bitfields() {
    // comp=3 with explicit 565 masks.
    let mut dib = info40(2, 1, 16, 3, 0);
    dib.extend_from_slice(&0xF800u32.to_le_bytes());
    dib.extend_from_slice(&0x07E0u32.to_le_bytes());
    dib.extend_from_slice(&0x001Fu32.to_le_bytes());
    let rows = [0xFFFFu16.to_le_bytes(), 0xF800u16.to_le_bytes()].concat();
    let f = file(&dib, &[], &rows);
    let dec = bmp::decode(&f).expect("565");
    assert_eq!(dec.pixels, vec![255, 255, 255, 255, 255, 0, 0, 255]);
}

#[test]
fn hand_32bit_bitfields_xrgb() {
    // Standard XRGB masks behave like plain 32-bit BGRA.
    let mut dib = info40(1, 1, 32, 3, 0);
    dib.extend_from_slice(&0xFF0000u32.to_le_bytes());
    dib.extend_from_slice(&0x00FF00u32.to_le_bytes());
    dib.extend_from_slice(&0x0000FFu32.to_le_bytes());
    let f = file(&dib, &[], &[30u8, 20, 10, 99]);
    let dec = bmp::decode(&f).expect("xrgb");
    assert_eq!(dec.pixels, vec![10, 20, 30, 255]);
}

#[test]
fn hand_rle8() {
    // 4x2. File rows bottom-up: y=1 run [2,2,2,2]; y=0 absolute [0,1,2,3].
    let mut pal = [[0u8, 0, 0]; 256];
    pal[0] = [9, 9, 9];
    pal[1] = [1, 2, 3];
    pal[2] = [4, 5, 6];
    pal[3] = [7, 8, 9];
    let mut dib = info40(4, 2, 8, 1, 0);
    dib.extend_from_slice(&pal_quad(&pal));
    let stream = [
        4, 2, // run
        0, 0, // eol
        0, 4, 0, 1, 2, 3, // absolute (even count, no pad)
        0, 0, // eol
        0, 1, // eobmp
    ];
    let f = file(&dib, &[], &stream);
    let dec = bmp::decode(&f).expect("rle8");
    assert_eq!(
        dec.pixels,
        vec![
            9, 9, 9, 255, 1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255,
            4, 5, 6, 255, 4, 5, 6, 255, 4, 5, 6, 255, 4, 5, 6, 255,
        ]
    );
}

#[test]
fn hand_rle8_delta_and_pad() {
    // 4x2: y=1 run; y=0 delta x+1 then absolute triple (odd count + pad).
    // Note: absolute counts below 3 collide with escape codes (0/1/2).
    let mut pal = [[0u8, 0, 0]; 256];
    pal[0] = [9, 9, 9];
    pal[1] = [1, 2, 3];
    pal[2] = [4, 5, 6];
    pal[3] = [7, 8, 9];
    let mut dib = info40(4, 2, 8, 1, 0);
    dib.extend_from_slice(&pal_quad(&pal));
    let stream = [
        4, 2, // run [2,2,2,2] (y=1)
        0, 0, // eol
        0, 2, 1, 0, // delta (1,0)
        0, 3, 3, 1, 2, 0x00, // absolute [3,1,2] + pad
        0, 0, // eol
        0, 1, // eobmp
    ];
    let f = file(&dib, &[], &stream);
    let dec = bmp::decode(&f).expect("rle8 delta");
    assert_eq!(
        dec.pixels,
        vec![
            9, 9, 9, 255, 7, 8, 9, 255, 1, 2, 3, 255, 4, 5, 6, 255,
            4, 5, 6, 255, 4, 5, 6, 255, 4, 5, 6, 255, 4, 5, 6, 255,
        ]
    );
}

#[test]
fn hand_rle4() {
    // 4x2: y=1 run [1,1,1,1]; y=0 absolute [0,1,2,3].
    let mut pal = [[0u8, 0, 0]; 16];
    pal[0] = [9, 9, 9];
    pal[1] = [1, 2, 3];
    pal[2] = [4, 5, 6];
    pal[3] = [7, 8, 9];
    let mut dib = info40(4, 2, 4, 2, 0);
    dib.extend_from_slice(&pal_quad(&pal));
    let stream = [
        4, 0x11, // run
        0, 0, // eol
        0, 4, 0x01, 0x23, // absolute, 2 bytes, no pad
        0, 0, // eol
        0, 1, // eobmp
    ];
    let f = file(&dib, &[], &stream);
    let dec = bmp::decode(&f).expect("rle4");
    assert_eq!(
        dec.pixels,
        vec![
            9, 9, 9, 255, 1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255,
            1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255,
        ]
    );
}

#[test]
fn hand_rle4_odd_absolute_pad() {
    // Absolute counts below 3 collide with escapes, so the smallest
    // padded case is n=5 (3 data bytes + pad). 5x1 -> indices 1..5.
    let mut pal = [[0u8, 0, 0]; 16];
    for (i, c) in pal.iter_mut().enumerate() {
        *c = [(i * 3) as u8, (i * 5) as u8, (i * 7) as u8];
    }
    let mut dib = info40(5, 1, 4, 2, 0);
    dib.extend_from_slice(&pal_quad(&pal));
    let stream = [0, 5, 0x12, 0x34, 0x50, 0x00, 0, 0, 0, 1];
    let f = file(&dib, &[], &stream);
    let dec = bmp::decode(&f).expect("rle4 odd");
    assert_eq!(
        dec.pixels,
        vec![
            3, 5, 7, 255, 6, 10, 14, 255, 9, 15, 21, 255,
            12, 20, 28, 255, 15, 25, 35, 255,
        ]
    );
}

// ── Roundtrips (exact: BMP is lossless) ────────────────────────

#[test]
fn roundtrip_exact() {
    for (w, h) in [(16u32, 16u32), (37, 23), (1, 1), (7, 5)] {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[
                    ((x * 11) % 256) as u8,
                    ((y * 13) % 256) as u8,
                    (((x + y) * 7) % 256) as u8,
                    ((x * 3 + y) % 256) as u8,
                ]);
            }
        }
        let enc = bmp::encode(w, h, &px).expect("encode");
        assert!(bmp::is_bmp(&enc));
        let dec = bmp::decode(&enc).expect("decode");
        assert_eq!((dec.width, dec.height), (w, h));
        assert_eq!(dec.pixels, px, "{w}x{h}");
        assert_eq!(bmp::dimensions(&enc), Ok((w, h)));
    }
}

// ── Cross-checks vs `image` crate (exact: lossless) ────────────

#[test]
fn cross_image_crate_exact() {
    // Opaque pixels: both implementations agree bit for bit.
    // (32-bit alpha handling differs: we preserve file alpha, the
    // `image` crate returns opaque — covered separately below.)
    let (w, h) = (21u32, 13u32);
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[((x * 37) % 256) as u8, ((y * 41) % 256) as u8, 9, 255]);
        }
    }
    // Our file decodes identically in the `image` crate.
    let enc = bmp::encode(w, h, &px).expect("encode");
    let tie = image::load_from_memory(&enc).expect("image decode").to_rgba8().into_raw();
    assert_eq!(tie, px);
    // The `image` crate's file decodes identically here.
    let buf = image::RgbaImage::from_raw(w, h, px.clone()).unwrap();
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut out, image::ImageFormat::Bmp)
        .unwrap();
    let dec = bmp::decode(out.get_ref()).expect("our decode");
    assert_eq!(dec.pixels, px);
}

#[test]
fn alpha_preserved_where_stored() {
    // Our 32-bit files keep per-pixel alpha; the `image` crate reads
    // them back opaque, so alpha is verified against our own decode.
    let px = vec![
        10, 20, 30, 40, 50, 60, 70, 80,
        90, 100, 110, 0, 130, 140, 150, 255,
    ];
    let enc = bmp::encode(4, 1, &px).expect("encode");
    // Alpha bytes are literally in the file (BGRA order).
    assert_eq!(enc[14 + 40 + 3], 40);
    assert_eq!(enc[14 + 40 + 11], 0);
    let dec = bmp::decode(&enc).expect("decode");
    assert_eq!(dec.pixels, px);
}

// ── Pillow fixtures ───────────────────────────────────────────

#[test]
fn pil_fixtures_exact() {
    for name in [
        "bmp_24bit.bmp",
        "bmp_odd_37x23.bmp",
        "bmp_32bit.bmp",
        "bmp_8bit.bmp",
        "bmp_1bit.bmp",
    ] {
        let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(&path).unwrap();
        assert!(bmp::is_bmp(&bytes), "{name}");
        let ours = bmp::decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let tie = image::load_from_memory(&bytes).expect("image").to_rgba8();
        assert_eq!((ours.width, ours.height), (tie.width(), tie.height()), "{name}");
        let tie = tie.into_raw();
        // RGB is bit-exact everywhere.
        for (i, (a, b)) in ours.pixels.iter().zip(tie.iter()).enumerate() {
            if i % 4 != 3 {
                assert_eq!(a, b, "{name} byte {i}");
            }
        }
        // Alpha: we preserve stored BGRA alpha, the `image` crate does
        // not, so alpha is verified against the file for 32-bit.
        if name == "bmp_32bit.bmp" {
            // Punched grid (x%3==0, y%3==0) carries alpha 80 in the file.
            let a00 = ours.pixels[3];
            let a10 = ours.pixels[(0 * 48 + 1) * 4 + 3];
            assert_eq!((a00, a10), (80, 255), "{name} alpha");
        } else {
            assert!(ours.pixels.chunks_exact(4).all(|p| p[3] == 255), "{name} opaque");
        }
    }
}

// ── Errors ────────────────────────────────────────────────────

#[test]
fn error_cases() {
    assert!(!bmp::is_bmp(b""));
    assert!(!bmp::is_bmp(b"BA...."));
    assert!(!bmp::is_bmp(&[0x42]));
    assert!(bmp::decode(b"BM").is_err());
    assert!(bmp::dimensions(b"nope").is_err());

    // Zero dimensions.
    let f = file(&info40(0, 4, 24, 0, 0), &[], &[]);
    assert!(bmp::decode(&f).is_err());
    assert!(bmp::dimensions(&f).is_err());

    // Bad planes.
    let mut dib = info40(2, 1, 24, 0, 0);
    dib[12..14].copy_from_slice(&2u16.to_le_bytes());
    assert!(bmp::decode(&file(&dib, &[], &[0u8; 16])).is_err());

    // 16-bit via BI_RGB decodes as XRGB 555 (not an error).
    let rows = [0x1Fu16.to_le_bytes(), 0x00u16.to_le_bytes()].concat();
    let dec = bmp::decode(&file(&info40(2, 1, 16, 0, 0), &[], &rows)).expect("555");
    assert_eq!(dec.pixels, vec![0, 0, 255, 255, 0, 0, 0, 255]);

    // RLE8 with 24-bit rejected; RLE4 with 8-bit rejected.
    assert!(bmp::decode(&file(&info40(2, 1, 24, 1, 0), &[], &[0u8; 8])).is_err());
    assert!(bmp::decode(&file(&info40(2, 1, 8, 2, 0), &[], &[0u8; 8])).is_err());

    // JPEG compression unsupported.
    assert!(matches!(
        bmp::decode(&file(&info40(4, 4, 24, 4, 0), &[], &[0u8; 48])),
        Err(bmp::BmpError::Unsupported(_))
    ));

    // BITFIELDS with empty masks.
    let mut dib = info40(2, 1, 16, 3, 0);
    dib.extend_from_slice(&[0u8; 12]);
    assert!(bmp::decode(&file(&dib, &[], &[0u8; 8])).is_err());

    // Truncated RLE + truncated pixels + bad offset.
    let mut dib = info40(4, 2, 8, 1, 0);
    dib.extend_from_slice(&pal_quad(&[[0, 0, 0]; 256]));
    assert!(bmp::decode(&file(&dib, &[], &[4, 2])).is_err());
    let f = file(&info40(4, 4, 24, 0, 0), &[], &[0u8; 8]);
    assert!(bmp::decode(&f).is_err());
    let mut f = file(&info40(4, 4, 24, 0, 0), &[], &[0u8; 48]);
    let off = u32::from_le_bytes([f[10], f[11], f[12], f[13]]);
    assert!(off >= 14);
    f[10..14].copy_from_slice(&8u32.to_le_bytes());
    assert!(bmp::decode(&f).is_err());

    // Unknown header size.
    let mut dib = info40(2, 1, 24, 0, 0);
    dib[0..4].copy_from_slice(&20u32.to_le_bytes());
    assert!(bmp::decode(&file(&dib, &[], &[0u8; 16])).is_err());

    // Encoder validation.
    assert!(bmp::encode(0, 4, &[]).is_err());
    assert!(bmp::encode(4, 4, &[1, 2, 3]).is_err());
}

// ── TiImage integration ───────────────────────────────────────

#[test]
fn tiimage_save_load_bmp() {
    use coreimage::{ImageFormat, TiImage};
    let mut px = Vec::new();
    for y in 0..20u32 {
        for x in 0..20u32 {
            px.extend_from_slice(&[(x * 12) as u8, (y * 12) as u8, (x + y) as u8, 200]);
        }
    }
    let img = TiImage::from_rgba(image::RgbaImage::from_raw(20, 20, px.clone()).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("img.bmp");
    img.save(path.to_str().unwrap(), ImageFormat::Bmp, 100).unwrap();
    let back = TiImage::load(path.to_str().unwrap()).unwrap();
    assert_eq!(back.dimensions(), (20, 20));
    assert_eq!(back.into_rgba().into_raw(), px);
    assert_eq!(TiImage::probe_format(path.to_str().unwrap()).unwrap(), ImageFormat::Bmp);
    let meta = TiImage::probe_metadata(path.to_str().unwrap()).unwrap();
    assert_eq!((meta.width, meta.height), (20, 20));
}
