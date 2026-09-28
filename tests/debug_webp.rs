//! Temporary debug helper: dump encoder output for analysis.
use coreimage::codecs::webp;

#[test]
fn debug_dump_2x2() {
    let mut px = Vec::new();
    for y in 0..2u32 {
        for x in 0..2u32 {
            px.extend_from_slice(&[
                ((x * 4 + y * 2) % 256) as u8,
                ((y * 5 + x * 3) % 256) as u8,
                (((x * x + y * y) / 7) % 256) as u8,
                ((x * 7 + y * 11) % 256) as u8,
            ]);
        }
    }
    let enc = webp::encode(2, 2, &px, 90).unwrap();
    std::fs::write("/tmp/t22.webp", &enc).unwrap();
    eprintln!("wrote {} bytes: {:02x?}", enc.len(), &enc[..enc.len().min(64)]);
    let dec = webp::decode(&enc).unwrap();
    eprintln!("decoded: {:?}", dec.pixels);
    eprintln!("expected: {:?}", px);
}
