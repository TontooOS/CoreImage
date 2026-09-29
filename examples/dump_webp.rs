use coreimage::codecs::webp;
fn main() {
    for name in ["webp_lossy_q80.webp", "webp_lossy_alpha_q80.webp", "webp_animated.webp", "webp_lossless_rgba.webp", "webp_odd_37x23_lossless.webp"] {
        let path = format!("/mnt/c/Users/arlo1/Documents/TontooLibs/CoreImage/tests/data/{name}");
        let bytes = std::fs::read(&path).unwrap();
        match webp::decode(&bytes) {
            Ok(d) => {
                let out = format!("/tmp/ours_{}.raw", name.replace(".webp", ""));
                std::fs::write(&out, &d.pixels).unwrap();
                println!("{name}: {}x{} -> {out}", d.width, d.height);
            }
            Err(e) => println!("{name}: ERROR {e}"),
        }
    }
}
