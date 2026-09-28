//! Tests for the pure-Rust ICO codec (`src/codecs/ico.rs`).
//!
//! An independent directory/BMP assembler lives in this file; pixel
//! expectations are hand-computed. PNG entries reuse our PNG encoder
//! (itself cross-validated in `png_codec.rs`).

use coreimage::codecs::ico;

// ── Independent test-only ICO builder ─────────────────────────

/// Assemble an ICO directory from (w_byte, h_byte, bpp, blob) entries.
fn dir(entries: &[(u8, u8, u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut off = 6 + 16 * entries.len() as u32;
    for (w, h, bpp, blob) in entries {
        out.push(*w);
        out.push(*h);
        out.extend_from_slice(&[0, 0]); // colors, reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&bpp.to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        off += blob.len() as u32;
    }
    for (_, _, _, blob) in entries {
        out.extend_from_slice(blob);
    }
    out
}

/// 40-byte BMP info header + palette (BGRx) + XOR + AND bytes.
#[allow(clippy::too_many_arguments)]
fn bmp40(
    width: i32,
    height_signed: i32,
    bpp: u16,
    comp: u32,
    colors_field: u32,
    palette: &[[u8; 3]],
    xor: &[u8],
    and: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height_signed.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&comp.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // image size
    out.extend_from_slice(&0u32.to_le_bytes()); // x ppm
    out.extend_from_slice(&0u32.to_le_bytes()); // y ppm
    out.extend_from_slice(&colors_field.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // important
    for c in palette {
        out.extend_from_slice(&[c[2], c[1], c[0], 0]);
    }
    out.extend_from_slice(xor);
    out.extend_from_slice(and);
    out
}

// ── Hand-built BMP vectors ────────────────────────────────────

#[test]
fn bmp_1bit_and_mask() {
    // 4x2, palette black/white. Top [W,B,W,B], bottom [B,W,B,W].
    // Transparent: (0,0) and (3,1).
    let pal = [[0, 0, 0], [255, 255, 255]];
    let xor = [
        0x50, 0, 0, 0, // bottom-up row 0 = image y=1: [0,1,0,1]
        0xA0, 0, 0, 0, // image y=0: [1,0,1,0]
    ];
    let and = [
        0x10, 0, 0, 0, // image y=1: x=3 transparent
        0x80, 0, 0, 0, // image y=0: x=0 transparent
    ];
    let blob = bmp40(4, 4, 1, 0, 0, &pal, &xor, &and);
    let file = dir(&[(4, 4, 1, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(dec.images.len(), 1);
    let img = &dec.images[0];
    assert_eq!((img.width, img.height), (4, 2));
    assert_eq!(
        img.pixels,
        vec![
            255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255,
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 0,
        ]
    );
}

#[test]
fn bmp_4bit_palette() {
    // 2x2, top [2,1], bottom [0,3].
    let mut pal = [[0u8, 0, 0]; 16];
    pal[0] = [10, 20, 30];
    pal[1] = [40, 50, 60];
    pal[2] = [70, 80, 90];
    pal[3] = [100, 110, 120];
    let xor = [
        0x03, 0, 0, 0, // image y=1
        0x21, 0, 0, 0, // image y=0
    ];
    let and = [0u8; 8];
    let blob = bmp40(2, 4, 4, 0, 0, &pal, &xor, &and);
    let file = dir(&[(2, 2, 4, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(
        dec.images[0].pixels,
        vec![
            70, 80, 90, 255, 40, 50, 60, 255,
            10, 20, 30, 255, 100, 110, 120, 255,
        ]
    );
}

#[test]
fn bmp_8bit_palette() {
    // 3x1, indices [0, 128, 255].
    let mut pal = [[0u8, 0, 0]; 256];
    for (i, c) in pal.iter_mut().enumerate() {
        *c = [i as u8, (255 - i) as u8, ((i * 2) % 256) as u8];
    }
    let xor = [0u8, 128, 255, 0]; // stride 4 with pad
    let and = [0u8; 4];
    let blob = bmp40(3, 2, 8, 0, 0, &pal, &xor, &and);
    let file = dir(&[(3, 1, 8, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(
        dec.images[0].pixels,
        vec![0, 255, 0, 255, 128, 127, 0, 255, 255, 0, 254, 255]
    );
}

#[test]
fn bmp_24bit_and_mask() {
    // 2x2: top [red, green(transparent)], bottom [blue, white].
    let xor = [
        255, 0, 0, 255, 255, 255, 0, 0, // image y=1: blue, white
        0, 0, 255, 0, 255, 0, 0, 0, // image y=0: red, green
    ];
    let and = [
        0, 0, 0, 0, // image y=1 opaque
        0x40, 0, 0, 0, // image y=0: x=1 transparent
    ];
    let blob = bmp40(2, 4, 24, 0, 0, &[], &xor, &and);
    let file = dir(&[(2, 2, 24, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(
        dec.images[0].pixels,
        vec![
            255, 0, 0, 255, 0, 255, 0, 0,
            0, 0, 255, 255, 255, 255, 255, 255,
        ]
    );
}

#[test]
fn bmp_32bit_alpha_and_mask() {
    // 2x1 BGRA; AND forces x=0 transparent, x=1 keeps file alpha.
    let xor = [30, 20, 10, 40, 70, 60, 50, 80];
    let and = [0x80, 0, 0, 0];
    let blob = bmp40(2, 2, 32, 0, 0, &[], &xor, &and);
    let file = dir(&[(2, 1, 32, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(dec.images[0].pixels, vec![10, 20, 30, 0, 50, 60, 70, 80]);
}

#[test]
fn bmp_top_down_and_missing_and_mask() {
    // Negative height: XOR rows stored top-down; no AND mask = opaque.
    let xor = [
        3, 2, 1, 6, 5, 4, 0, 0, // image y=0: (1,2,3),(4,5,6)
        9, 8, 7, 12, 11, 10, 0, 0, // image y=1
    ];
    let blob = bmp40(2, -4, 24, 0, 0, &[], &xor, &[]);
    let file = dir(&[(2, 2, 24, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(
        dec.images[0].pixels,
        vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
    );
}

#[test]
fn bmp_small_palette_index_oob() {
    // 4-bit entry claiming only 2 palette colors; index 5 is out of range.
    let pal = [[1, 2, 3], [4, 5, 6]];
    let xor = [0x51, 0, 0, 0, 0x00, 0, 0, 0];
    let and = [0u8; 8];
    let blob = bmp40(2, 4, 4, 0, 2, &pal, &xor, &and);
    let file = dir(&[(2, 2, 4, blob)]);
    assert!(ico::decode(&file).is_err());
}

// ── PNG entries ───────────────────────────────────────────────

fn solid_rgba(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
    px.repeat(w as usize * h as usize)
}

#[test]
fn png_entries_multi_and_best() {
    let small = coreimage::codecs::png::encode(16, 16, &solid_rgba(16, 16, [200, 10, 10, 255])).unwrap();
    let mut grad = Vec::new();
    for y in 0..32u32 {
        for x in 0..32u32 {
            grad.extend_from_slice(&[(x * 8) as u8, (y * 8) as u8, 128, 255]);
        }
    }
    let big = coreimage::codecs::png::encode(32, 32, &grad).unwrap();
    let file = dir(&[(16, 16, 32, small), (32, 32, 32, big.clone())]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(dec.images.len(), 2);
    // Largest first.
    assert_eq!((dec.images[0].width, dec.images[0].height), (32, 32));
    assert_eq!(dec.images[0].pixels, grad);
    assert_eq!((dec.images[1].width, dec.images[1].height), (16, 16));
    // dimensions() probe reports the largest.
    assert_eq!(ico::dimensions(&file), Ok((32, 32)));
    assert!(ico::is_ico(&file));
}

#[test]
fn entry_256_uses_zero_byte() {
    let px = solid_rgba(256, 256, [1, 2, 3, 255]);
    let blob = coreimage::codecs::png::encode(256, 256, &px).unwrap();
    let file = dir(&[(0, 0, 32, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!((dec.images[0].width, dec.images[0].height), (256, 256));
    assert_eq!(dec.images[0].pixels, px);
}

// ── Our encoder roundtrip ─────────────────────────────────────

#[test]
fn encode_roundtrip_exact() {
    let mut px = Vec::new();
    for y in 0..24u32 {
        for x in 0..24u32 {
            px.extend_from_slice(&[(x * 9) as u8, (y * 9) as u8, ((x + y) * 5) as u8, (x * 3 + 100) as u8]);
        }
    }
    let file = ico::encode(24, 24, &px).expect("encode");
    assert!(ico::is_ico(&file));
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(dec.images.len(), 1);
    assert_eq!(dec.images[0].pixels, px);

    // Multi-image encode keeps every entry.
    let file = ico::encode_images(&[(16, 16, &solid_rgba(16, 16, [9, 9, 9, 255])), (24, 24, &px)])
        .expect("encode multi");
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(dec.images.len(), 2);
    assert_eq!((dec.images[0].width, dec.images[0].height), (24, 24));
    assert!(ico::encode(0, 4, &[]).is_err());
    assert!(ico::encode(300, 4, &vec![0u8; 300 * 4 * 4]).is_err());
    assert!(ico::encode_images(&[]).is_err());
}

// ── Pillow oracle ─────────────────────────────────────────────
// Fixtures + RGBA dumps generated by tests/gen_ico.py. Pillow is an
// independent implementation; agreement proves our BMP paths.

fn pil_pixels(name: &str, w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
    let base = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&base).unwrap();
    let dec = ico::decode(&bytes).expect("decode");
    let img = dec
        .images
        .iter()
        .find(|i| i.width == w && i.height == h)
        .unwrap_or_else(|| panic!("{name} missing {w}x{h}"));
    let tie =
        std::fs::read(format!("{base}.{w}x{h}.rgba")).expect("pillow dump");
    assert_eq!(img.pixels.len(), tie.len(), "{name} size");
    (img.pixels.clone(), tie)
}

#[test]
fn pil_oracle_exact_32px() {
    // 32 px wide: padded and packed AND strides coincide, so these
    // must match Pillow byte for byte (1/4/8/24/32-bit paths).
    for (name, w, h) in [
        ("ref32_1bit.ico", 32, 32),
        ("ref32_8bit.ico", 32, 32),
        ("ref32_24bit.ico", 32, 32),
        ("ref32_32bit.ico", 32, 32),
        ("ref32n_1bit.ico", 32, 32),
        ("ref32n_8bit.ico", 32, 32),
        ("ref32n_24bit.ico", 32, 32),
        ("ref_multi_bmp.ico", 32, 32),
        ("ref_32bit.ico", 16, 16),
    ] {
        let (ours, tie) = pil_pixels(name, w, h);
        assert_eq!(ours, tie, "{name}");
    }
}

#[test]
fn pil_packed_and_layout() {
    // 16 px wide BMP entries use tightly packed AND rows (2 bytes, not
    // the spec's padded 4). RGB matches Pillow exactly in all files.
    // Alpha: Pillow's own reader assumes padded rows and misreads its
    // own packed files (phantom transparency), so alpha is verified
    // against writer intent instead (see below + `packed_and_rows`).
    for name in ["ref_1bit.ico", "ref_8bit.ico", "ref_24bit.ico"] {
        let (ours, tie) = pil_pixels(name, 16, 16);
        let rgb_ours: Vec<u8> = ours.chunks_exact(4).flat_map(|p| p[..3].to_vec()).collect();
        let rgb_tie: Vec<u8> = tie.chunks_exact(4).flat_map(|p| p[..3].to_vec()).collect();
        assert_eq!(rgb_ours, rgb_tie, "{name} rgb");
    }
    // ref_8bit/ref_24bit carry all-zero AND masks: fully opaque here.
    for name in ["ref_8bit.ico", "ref_24bit.ico"] {
        let (ours, _) = pil_pixels(name, 16, 16);
        assert!(ours.chunks_exact(4).all(|p| p[3] == 255), "{name} opaque");
    }
    // ref_1bit carries nondeterministic AND bytes from Pillow (writer
    // quirk for mode "1"), so only RGB is pinned for that file.
}

#[test]
fn packed_and_rows() {
    // 4 px wide: padded AND stride would be 4 bytes/row, this file uses
    // packed 1-byte rows. Bottom-up: image y=1 <- 0x40 (x=1 transparent),
    // image y=0 <- 0x00 (opaque).
    // XOR stride is 12 (no padding needed); AND rows are packed (1 byte).
    let xor = [
        150, 140, 130, 180, 170, 160, 210, 200, 190, 240, 230, 220,
        30, 20, 10, 60, 50, 40, 90, 80, 70, 120, 110, 100,
    ];
    let and = [0x40, 0x00];
    let blob = bmp40(4, 4, 24, 0, 0, &[], &xor, &and);
    let file = dir(&[(4, 2, 24, blob)]);
    let dec = ico::decode(&file).expect("decode");
    assert_eq!(
        dec.images[0].pixels,
        vec![
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
            130, 140, 150, 255, 160, 170, 180, 0, 190, 200, 210, 255, 220, 230, 240, 255,
        ]
    );
}

// ── Real-world file ───────────────────────────────────────────

#[test]
fn real_world_tontoo_ico() {
    let path = format!(
        "{}/../TontooOS/TontooIconAssets/tontoo_default.ico",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("skipping: no tontoo_default.ico");
            return;
        }
    };
    let dec = ico::decode(&bytes).expect("decode tontoo_default.ico");
    assert_eq!(dec.images.len(), 6);
    let sizes: Vec<(u32, u32)> = dec.images.iter().map(|i| (i.width, i.height)).collect();
    assert_eq!(sizes, vec![(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)]);
    assert_eq!(ico::dimensions(&bytes), Ok((256, 256)));
    // Sanity: largest entry is real content, not blank/fully transparent.
    let big = &dec.images[0];
    assert!(big.pixels.chunks_exact(4).any(|p| p[3] > 0));
    assert!(big.pixels.chunks_exact(4).any(|p| p[3] < 255) || big.pixels.iter().any(|&v| v > 0));
    // Every PNG blob inside re-decodes with the `image` crate identically.
    let mut pos = 6usize;
    let mut checked = 0;
    for _ in 0..6 {
        let size = u32::from_le_bytes([bytes[pos + 8], bytes[pos + 9], bytes[pos + 10], bytes[pos + 11]]) as usize;
        let off = u32::from_le_bytes([bytes[pos + 12], bytes[pos + 13], bytes[pos + 14], bytes[pos + 15]]) as usize;
        let blob = &bytes[off..off + size];
        let tie = image::load_from_memory(blob).expect("image png").to_rgba8().into_raw();
        let ours = dec
            .images
            .iter()
            .find(|i| i.width as usize * i.height as usize * 4 == tie.len())
            .expect("match entry");
        assert_eq!(ours.pixels, tie);
        checked += 1;
        pos += 16;
    }
    assert_eq!(checked, 6);
}

// ── Errors ────────────────────────────────────────────────────

#[test]
fn error_cases() {
    assert!(!ico::is_ico(b""));
    assert!(!ico::is_ico(&[0, 0, 2, 0, 1, 0])); // cursor type
    assert!(!ico::is_ico(&coreimage::codecs::png::encode(2, 2, &vec![0u8; 16]).unwrap()));
    assert!(ico::decode(b"").is_err());
    // Cursor rejected as Unsupported.
    let mut cur = vec![0, 0, 2, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0, 64, 0, 0, 0, 22, 0, 0, 0];
    cur.extend_from_slice(&[0u8; 64]);
    assert!(matches!(ico::decode(&cur), Err(ico::IcoError::Unsupported(_))));
    // Zero entries.
    assert!(ico::decode(&[0, 0, 1, 0, 0, 0]).is_err());
    // Truncated directory.
    assert!(ico::decode(&[0, 0, 1, 0, 1, 0, 16]).is_err());
    // Entry out of bounds.
    let bad = dir(&[(4, 4, 32, vec![1, 2, 3])]);
    let mut bad = bad;
    let off_pos = 6 + 14;
    bad[off_pos..off_pos + 4].copy_from_slice(&1000u32.to_le_bytes());
    assert!(ico::decode(&bad).is_err());
    // RLE-compressed BMP entry.
    let blob = bmp40(4, 4, 8, 1, 0, &[[0, 0, 0]; 256], &[0u8; 8], &[0u8; 8]);
    assert!(matches!(
        ico::decode(&dir(&[(4, 2, 8, blob)])),
        Err(ico::IcoError::Unsupported(_))
    ));
    // 16-bit BMP.
    let blob = bmp40(4, 4, 16, 0, 0, &[], &[0u8; 16], &[0u8; 8]);
    assert!(ico::decode(&dir(&[(4, 2, 16, blob)])).is_err());
    // Unknown BMP header size.
    let mut blob = bmp40(4, 4, 24, 0, 0, &[], &[0u8; 16], &[0u8; 8]);
    blob[0..4].copy_from_slice(&20u32.to_le_bytes());
    assert!(ico::decode(&dir(&[(4, 2, 24, blob)])).is_err());
    assert!(ico::dimensions(b"nope").is_err());
}

// ── TiImage integration ───────────────────────────────────────

#[test]
fn tiimage_save_load_ico() {
    use coreimage::{ImageFormat, TiImage};
    let mut px = Vec::new();
    for y in 0..20u32 {
        for x in 0..20u32 {
            px.extend_from_slice(&[(x * 12) as u8, (y * 12) as u8, 200, (x + y) as u8]);
        }
    }
    let img = TiImage::from_rgba(
        image::RgbaImage::from_raw(20, 20, px.clone()).unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("icon.ico");
    img.save(path.to_str().unwrap(), ImageFormat::Ico, 100).unwrap();
    let back = TiImage::load(path.to_str().unwrap()).unwrap();
    assert_eq!(back.dimensions(), (20, 20));
    assert_eq!(back.into_rgba().into_raw(), px);
    assert_eq!(TiImage::probe_format(path.to_str().unwrap()).unwrap(), ImageFormat::Ico);
    let meta = TiImage::probe_metadata(path.to_str().unwrap()).unwrap();
    assert_eq!((meta.width, meta.height), (20, 20));
    assert_eq!(meta.format, Some(ImageFormat::Ico));
    // Oversize ICO rejected.
    let big = TiImage::solid(300, 300, coreimage::Rgba8::WHITE).unwrap();
    assert!(big.save(path.to_str().unwrap(), ImageFormat::Ico, 100).is_err());
}
