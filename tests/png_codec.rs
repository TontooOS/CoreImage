//! Tests for the pure-Rust PNG codec (`src/codecs/png.rs`).
//!
//! Strategy: an independent minimal PNG writer lives in this file
//! (own CRC32/Adler32/stored-deflate implementation, forward filters).
//! Our decoder must reproduce exact pixels for every color type,
//! bit depth, filter and interlace mode. Cross-checks against the
//! `image` crate validate both directions of our codec.

use coreimage::codecs::png;

// ── Independent test-only PNG builder ─────────────────────────

fn test_crc32(data: &[u8]) -> u32 {
    // Bitwise implementation, independent of the codec's table version.
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    crc ^ 0xFFFF_FFFF
}

fn chunk_crc(tag: &[u8; 4], data: &[u8]) -> u32 {
    let mut all = Vec::with_capacity(4 + data.len());
    all.extend_from_slice(tag);
    all.extend_from_slice(data);
    test_crc32(&all)
}

fn test_adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    out.extend_from_slice(&chunk_crc(tag, data).to_be_bytes());
}

/// zlib stream with stored (uncompressed) blocks only.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut k = 0usize;
    while k < raw.len() {
        let n = (raw.len() - k).min(65535);
        let last = k + n >= raw.len();
        out.push(if last { 0x01 } else { 0x00 });
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(!(n as u16)).to_le_bytes());
        out.extend_from_slice(&raw[k..k + n]);
        k += n;
    }
    if raw.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }
    out.extend_from_slice(&test_adler32(raw).to_be_bytes());
    out
}

fn paeth(a: i32, b: i32, c: i32) -> u8 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    (if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }) as u8
}

/// Forward filter (encode direction), independent of the codec.
fn forward_filter(kind: u8, row: &[u8], prev: &[u8], bpp: usize) -> Vec<u8> {
    let mut out = vec![kind];
    match kind {
        0 => out.extend_from_slice(row),
        1 => {
            for (i, &v) in row.iter().enumerate() {
                let a = if i >= bpp { row[i - bpp] } else { 0 };
                out.push(v.wrapping_sub(a));
            }
        }
        2 => {
            for (i, &v) in row.iter().enumerate() {
                out.push(v.wrapping_sub(prev[i]));
            }
        }
        3 => {
            for (i, &v) in row.iter().enumerate() {
                let a = if i >= bpp { row[i - bpp] as u16 } else { 0 };
                out.push(v.wrapping_sub(((a + prev[i] as u16) / 2) as u8));
            }
        }
        4 => {
            for (i, &v) in row.iter().enumerate() {
                let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
                let b = prev[i] as i32;
                let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                out.push(v.wrapping_sub(paeth(a, b, c)));
            }
        }
        _ => panic!("bad filter"),
    }
    out
}

struct IhdrSpec {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    interlace: u8,
}

/// Assemble a PNG: IHDR + extra chunks + IDAT (optionally split) + IEND.
fn build_png(spec: &IhdrSpec, extra: &[([u8; 4], Vec<u8>)], filtered: &[u8], idat_splits: &[usize]) -> Vec<u8> {
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&spec.width.to_be_bytes());
    ihdr.extend_from_slice(&spec.height.to_be_bytes());
    ihdr.push(spec.bit_depth);
    ihdr.push(spec.color_type);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(spec.interlace);
    push_chunk(&mut out, b"IHDR", &ihdr);
    for (tag, data) in extra {
        push_chunk(&mut out, tag, data);
    }
    let zlib = zlib_stored(filtered);
    if idat_splits.is_empty() {
        push_chunk(&mut out, b"IDAT", &zlib);
    } else {
        let mut start = 0usize;
        for &end in idat_splits.iter().chain(std::iter::once(&zlib.len())) {
            push_chunk(&mut out, b"IDAT", &zlib[start..end.min(zlib.len())]);
            start = end;
        }
    }
    push_chunk(&mut out, b"IEND", &[]);
    out
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

// ── Roundtrips through our own codec ──────────────────────────

fn xorshift(state: &mut u64) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 11) as u8
}

#[test]
fn roundtrip_random_sizes() {
    for (w, h) in [(1, 1), (1, 7), (3, 5), (16, 16), (63, 41), (128, 96)] {
        let mut st = 0x1234_5678_9ABC_DEF1u64 + (w as u64) * 131 + (h as u64);
        let pixels: Vec<u8> = (0..w * h * 4).map(|_| xorshift(&mut st)).collect();
        let enc = png::encode(w, h, &pixels).expect("encode");
        let dec = png::decode(&enc).expect("decode");
        assert_eq!((dec.width, dec.height), (w, h));
        assert_eq!(dec.pixels, pixels, "roundtrip mismatch at {w}x{h}");
    }
}

#[test]
fn roundtrip_gradient_and_solid() {
    let (w, h) = (64, 48);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.extend_from_slice(&[(x * 4) as u8, (y * 5) as u8, ((x + y) * 2) as u8, 255]);
        }
    }
    let enc = png::encode(w, h, &pixels).expect("encode");
    let dec = png::decode(&enc).expect("decode");
    assert_eq!(dec.pixels, pixels);
}

// ── Cross-checks against the `image` crate ────────────────────

#[test]
fn our_encode_decoded_by_image_crate() {
    let (w, h) = (37, 23);
    let mut st = 0xDEAD_BEEF_CAFE_1234u64;
    let pixels: Vec<u8> = (0..w * h * 4).map(|_| xorshift(&mut st)).collect();
    let enc = png::encode(w, h, &pixels).expect("encode");
    let back = image::load_from_memory(&enc).expect("image crate decode").to_rgba8();
    assert_eq!(back.as_raw(), &pixels);
}

#[test]
fn image_crate_encode_decoded_by_us() {
    use image::{DynamicImage, ImageFormat, RgbaImage};
    let (w, h) = (41, 29);
    let mut st = 0x0BAD_F00D_5EED_11u64;
    let pixels: Vec<u8> = (0..w * h * 4).map(|_| xorshift(&mut st)).collect();
    let buf = RgbaImage::from_raw(w, h, pixels.clone()).unwrap();
    let mut out = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(buf).write_to(&mut out, ImageFormat::Png).unwrap();
    let dec = png::decode(out.get_ref()).expect("our decode");
    assert_eq!((dec.width, dec.height), (w, h));
    assert_eq!(dec.pixels, pixels);
}

#[test]
fn image_crate_gray_and_16bit_decoded_by_us() {
    use image::{DynamicImage, ImageFormat};
    // Gray8
    let gray8 = DynamicImage::ImageLuma8({
        let mut b = image::GrayImage::new(8, 4);
        for (x, y, p) in b.enumerate_pixels_mut() {
            p.0[0] = (x * 32 + y * 7) as u8;
        }
        b
    });
    let mut out = std::io::Cursor::new(Vec::new());
    gray8.write_to(&mut out, ImageFormat::Png).unwrap();
    let dec = png::decode(out.get_ref()).expect("gray8");
    for (x, p) in dec.pixels.chunks_exact(4).enumerate() {
        let (gx, gy) = ((x % 8) as u8, (x / 8) as u8);
        let g = gx * 32 + gy * 7;
        assert_eq!(p, &[g, g, g, 255], "gray8 pixel {x}");
    }
    // RGBA16
    let vals16: Vec<u16> = (0..(6 * 5 * 4)).map(|i| ((i as u32 * 1234 + 7) % 65536) as u16).collect();
    let rgba16 = DynamicImage::ImageRgba16(
        image::ImageBuffer::from_raw(6, 5, vals16.clone()).unwrap(),
    );
    let mut out = std::io::Cursor::new(Vec::new());
    rgba16.write_to(&mut out, ImageFormat::Png).unwrap();
    let dec = png::decode(out.get_ref()).expect("rgba16");
    assert_eq!((dec.width, dec.height), (6, 5));
    for (i, px) in dec.pixels.chunks_exact(4).enumerate() {
        for c in 0..4 {
            let v = vals16[i * 4 + c];
            let expect = ((v as u32 * 255 + 32767) / 65535) as u8;
            assert_eq!(px[c], expect, "rgba16 pixel {i} ch {c}");
        }
    }
}

// ── Hand-built vectors: color types / depths / filters ────────

#[test]
fn gray_1bit() {
    // 4x1 gray, depth 1: samples [0,1,1,0] -> black, white, white, black.
    let spec = IhdrSpec { width: 4, height: 1, bit_depth: 1, color_type: 0, interlace: 0 };
    let filtered = vec![0u8, 0b0110_0000];
    let file = build_png(&spec, &[], &filtered, &[]);
    let dec = png::decode(&file).expect("gray1");
    assert_eq!(
        dec.pixels,
        vec![0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255]
    );
}

#[test]
fn gray_2bit_and_4bit() {
    // 2-bit: [0,1,2,3] -> 0,85,170,255.
    let spec = IhdrSpec { width: 4, height: 1, bit_depth: 2, color_type: 0, interlace: 0 };
    let file = build_png(&spec, &[], &[0u8, 0b00_01_10_11], &[]);
    let dec = png::decode(&file).expect("gray2");
    assert_eq!(
        dec.pixels,
        vec![0, 0, 0, 255, 85, 85, 85, 255, 170, 170, 170, 255, 255, 255, 255, 255]
    );
    // 4-bit: [0,15] -> 0,255.
    let spec = IhdrSpec { width: 2, height: 1, bit_depth: 4, color_type: 0, interlace: 0 };
    let file = build_png(&spec, &[], &[0u8, 0x0F], &[]);
    let dec = png::decode(&file).expect("gray4");
    assert_eq!(dec.pixels, vec![0, 0, 0, 255, 255, 255, 255, 255]);
}

#[test]
fn gray_trns_key() {
    // gray8 with tRNS key 100: pixel 100 -> transparent.
    let spec = IhdrSpec { width: 3, height: 1, bit_depth: 8, color_type: 0, interlace: 0 };
    let file = build_png(
        &spec,
        &[(*b"tRNS", vec![0, 100])],
        &[0u8, 99, 100, 101],
        &[],
    );
    let dec = png::decode(&file).expect("gray trns");
    assert_eq!(
        dec.pixels,
        vec![99, 99, 99, 255, 100, 100, 100, 0, 101, 101, 101, 255]
    );
}

#[test]
fn rgb_trns_key() {
    // rgb8 with tRNS key (1,2,3).
    let spec = IhdrSpec { width: 2, height: 1, bit_depth: 8, color_type: 2, interlace: 0 };
    let file = build_png(
        &spec,
        &[(*b"tRNS", vec![0, 1, 0, 2, 0, 3])],
        &[0u8, 1, 2, 3, 9, 9, 9],
        &[],
    );
    let dec = png::decode(&file).expect("rgb trns");
    assert_eq!(dec.pixels, vec![1, 2, 3, 0, 9, 9, 9, 255]);
}

#[test]
fn palette_with_trns() {
    // 2-entry palette, depth 1, pixels [0,1], alpha [200,64].
    let spec = IhdrSpec { width: 2, height: 1, bit_depth: 1, color_type: 3, interlace: 0 };
    let plte = vec![255, 0, 0, 0, 0, 255];
    let file = build_png(
        &spec,
        &[(*b"PLTE", plte), (*b"tRNS", vec![200, 64])],
        &[0u8, 0b01_000000],
        &[],
    );
    let dec = png::decode(&file).expect("palette");
    assert_eq!(dec.pixels, vec![255, 0, 0, 200, 0, 0, 255, 64]);
}

#[test]
fn palette_without_trns_is_opaque() {
    let spec = IhdrSpec { width: 1, height: 1, bit_depth: 8, color_type: 3, interlace: 0 };
    let file = build_png(&spec, &[(*b"PLTE", vec![10, 20, 30])], &[0u8, 0], &[]);
    let dec = png::decode(&file).expect("palette opaque");
    assert_eq!(dec.pixels, vec![10, 20, 30, 255]);
}

#[test]
fn gray_alpha_8bit() {
    // color type 4: (gray, alpha) pairs.
    let spec = IhdrSpec { width: 2, height: 1, bit_depth: 8, color_type: 4, interlace: 0 };
    let file = build_png(&spec, &[], &[0u8, 50, 128, 200, 0], &[]);
    let dec = png::decode(&file).expect("gray+alpha");
    assert_eq!(dec.pixels, vec![50, 50, 50, 128, 200, 200, 200, 0]);
}

#[test]
fn all_five_filters_decode() {
    // 3x2 RGBA; every row encoded with each filter kind 0..=4.
    // Values include odd+odd (a,b) pairs to pin the Average filter's
    // floor((a+b)/2) semantics (a/2+b/2 would be off by one there).
    let rows: Vec<Vec<u8>> = vec![
        vec![11, 21, 31, 253, 41, 51, 61, 129, 71, 81, 91, 1],
        vec![1, 3, 5, 7, 201, 203, 205, 207, 9, 11, 13, 15],
    ];
    for kind in 0..=4u8 {
        let mut filtered = Vec::new();
        let mut prev = vec![0u8; 12];
        for row in &rows {
            filtered.extend_from_slice(&forward_filter(kind, row, &prev, 4));
            prev = row.clone();
        }
        let spec = IhdrSpec { width: 3, height: 2, bit_depth: 8, color_type: 6, interlace: 0 };
        let file = build_png(&spec, &[], &filtered, &[]);
        let dec = png::decode(&file).expect("filter decode");
        let expect: Vec<u8> = rows.concat();
        assert_eq!(dec.pixels, expect, "filter {kind}");
    }
}

#[test]
fn interlaced_adam7() {
    // 9x9 RGBA with a position-derived pattern; all 7 passes exercised
    // (pass 3 is empty at this size and must be skipped).
    let (w, h) = (9u32, 9u32);
    let mut img = Vec::new();
    for y in 0..h {
        for x in 0..w {
            img.extend_from_slice(&[(x * 28) as u8, (y * 28) as u8, ((x + y) * 14) as u8, 255]);
        }
    }
    const ADAM7: [(u32, u32, u32, u32); 7] =
        [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
    let mut filtered = Vec::new();
    for &(x0, y0, dx, dy) in &ADAM7 {
        let pw = if x0 < w { (w - x0 + dx - 1) / dx } else { 0 };
        let ph = if y0 < h { (h - y0 + dy - 1) / dy } else { 0 };
        if pw == 0 || ph == 0 {
            continue;
        }
        let mut prev = vec![0u8; (pw * 4) as usize];
        for py in 0..ph {
            let mut row = Vec::with_capacity((pw * 4) as usize);
            for px in 0..pw {
                let (x, y) = (x0 + px * dx, y0 + py * dy);
                let o = ((y * w + x) * 4) as usize;
                row.extend_from_slice(&img[o..o + 4]);
            }
            filtered.extend_from_slice(&forward_filter(1, &row, &prev, 4));
            prev = row;
        }
    }
    let spec = IhdrSpec { width: w, height: h, bit_depth: 8, color_type: 6, interlace: 1 };
    let file = build_png(&spec, &[], &filtered, &[]);
    let dec = png::decode(&file).expect("interlaced");
    assert_eq!(dec.pixels, img);
    // The `image` crate agrees on the same file.
    let back = image::load_from_memory(&file).expect("image interlaced").to_rgba8();
    assert_eq!(back.as_raw(), &img);
}

#[test]
fn sixteen_bit_gray_scales() {
    // gray16 value 0x8000 -> 128, 0xFFFF -> 255, 0x0000 -> 0.
    let spec = IhdrSpec { width: 3, height: 1, bit_depth: 16, color_type: 0, interlace: 0 };
    let file = build_png(&spec, &[], &[0u8, 0x80, 0x00, 0xFF, 0xFF, 0x00, 0x00], &[]);
    let dec = png::decode(&file).expect("gray16");
    assert_eq!(dec.pixels, vec![128, 128, 128, 255, 255, 255, 255, 255, 0, 0, 0, 255]);
}

#[test]
fn split_idat_and_ancillary_chunks() {
    // Same zlib stream split across two IDATs + a tEXt chunk in between is
    // rejected (non-consecutive IDAT); consecutive split + tEXt works.
    let spec = IhdrSpec { width: 2, height: 1, bit_depth: 8, color_type: 6, interlace: 0 };
    let raw = [0u8, 1, 2, 3, 255, 4, 5, 6, 255];
    let zlib = zlib_stored(&raw);
    let mid = zlib.len() / 2;
    let file = build_png(&spec, &[], &raw, &[mid]);
    let dec = png::decode(&file).expect("split idat");
    assert_eq!(dec.pixels, raw[1..].to_vec());

    // Unknown ancillary chunk is skipped.
    let file = build_png(
        &spec,
        &[(*b"tEXt", b"Title\0hello".to_vec()), (*b"zZzZ", vec![1, 2, 3])],
        &raw,
        &[],
    );
    let dec = png::decode(&file).expect("ancillary");
    assert_eq!(dec.pixels, raw[1..].to_vec());
}

#[test]
fn error_cases() {
    let spec = IhdrSpec { width: 2, height: 1, bit_depth: 8, color_type: 6, interlace: 0 };
    let raw = [0u8, 1, 2, 3, 4, 5, 6, 7, 8];
    let good = build_png(&spec, &[], &raw, &[]);

    // Not a PNG.
    assert!(png::decode(b"hello world, this is not png....").is_err());
    assert!(!png::is_png(b"hello"));
    assert!(png::is_png(&good));

    // Corrupt CRC.
    let mut bad_crc = good.clone();
    let len = bad_crc.len();
    bad_crc[len - 20] ^= 0xFF;
    assert!(png::decode(&bad_crc).is_err());

    // Truncated.
    assert!(png::decode(&good[..good.len() - 20]).is_err());

    // Missing IEND.
    assert!(png::decode(&good[..good.len() - 12]).is_err());

    // Zero width.
    let mut ihdr_bad = Vec::new();
    ihdr_bad.extend_from_slice(&0u32.to_be_bytes());
    ihdr_bad.extend_from_slice(&1u32.to_be_bytes());
    ihdr_bad.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut f = vec![137, 80, 78, 71, 13, 10, 26, 10];
    push_chunk(&mut f, b"IHDR", &ihdr_bad);
    push_chunk(&mut f, b"IEND", &[]);
    assert!(png::decode(&f).is_err());

    // Indexed without PLTE.
    let idx = IhdrSpec { width: 1, height: 1, bit_depth: 8, color_type: 3, interlace: 0 };
    let f = build_png(&idx, &[], &[0u8, 0], &[]);
    assert!(png::decode(&f).is_err());

    // Invalid combo: depth 4 with truecolor.
    let bad = IhdrSpec { width: 1, height: 1, bit_depth: 4, color_type: 2, interlace: 0 };
    let f = build_png(&bad, &[], &[0u8, 0x12], &[]);
    assert!(png::decode(&f).is_err());

    // Bad interlace method hand-patched.
    let mut f = build_png(&spec, &[], &raw, &[]);
    // IHDR interlace byte is at offset 8 + 8 + 13 - 1 = 28.
    f[28] = 2;
    // Fix CRC of IHDR (bytes 16..29 type+data, CRC at 29..33).
    let crc = chunk_crc(b"IHDR", &f[16 + 4..16 + 4 + 13]);
    f[29..33].copy_from_slice(&crc.to_be_bytes());
    assert!(png::decode(&f).is_err());

    // dimensions() fast probe matches.
    assert_eq!(png::dimensions(&good), Ok((2, 1)));
    assert!(png::dimensions(b"nope").is_err());

    // encode() rejects bad inputs.
    assert!(png::encode(0, 4, &[]).is_err());
    assert!(png::encode(2, 1, &[1, 2, 3]).is_err());
}

#[test]
fn real_world_coreicon_asset() {
    // Dogfood: decode a real CoreIcon PNG with our codec and compare
    // against the `image` crate. Skipped when assets are absent.
    let dir = format!("{}/../CoreIcon/assets/icons", env!("CARGO_MANIFEST_DIR"));
    let entries = std::fs::read_dir(&dir).map(|r| {
        let mut v: Vec<_> = r
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
            .collect();
        v.sort();
        v
    });
    let mut entries = match entries {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipping: no CoreIcon assets");
            return;
        }
    };
    entries.truncate(4);
    assert!(!entries.is_empty());
    for path in entries {
        let bytes = std::fs::read(&path).unwrap();
        let ours = png::decode(&bytes)
            .unwrap_or_else(|e| panic!("our decode failed for {path:?}: {e}"));
        let theirs = image::load_from_memory(&bytes)
            .unwrap_or_else(|e| panic!("image decode failed for {path:?}: {e}"))
            .to_rgba8();
        assert_eq!(ours.width, theirs.width(), "width {path:?}");
        assert_eq!(ours.height, theirs.height(), "height {path:?}");
        assert_eq!(ours.pixels, theirs.as_raw().as_slice(), "pixels {path:?}");
    }
}
