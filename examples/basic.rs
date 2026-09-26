use coreimage::filter::{Filter, FilterChain};
use coreimage::{BlendMode, ImageFormat, Rgba8, TiImage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Solid canvas as a stand-in for a loaded wallpaper.
    let img = TiImage::solid(800, 600, Rgba8::new(30, 90, 160, 255))?;

    // Filter pipeline: blur for a dock background + slight warm grade.
    let dock_bg = FilterChain::new()
        .push(Filter::GaussianBlur(8.0))
        .push(Filter::WhiteBalance { temperature: 12.0 })
        .push(Filter::Brightness(6))
        .apply(&img)?;

    // Overlay layer + SF Pro watermark via CoreText font resolution.
    let badge = TiImage::solid(200, 80, Rgba8::new(255, 255, 255, 220))?;
    let composed = dock_bg
        .blend(&badge, BlendMode::Normal, 0.9, 40, 40)
        .watermark_text("TontooOS", 56, 56, 28.0, Rgba8::BLACK)?;

    // Analysis for adaptive UI.
    let dom = composed.dominant_color()?;
    println!("dominant: #{:02x}{:02x}{:02x}", dom.r, dom.g, dom.b);
    println!("dark UI: {}", composed.is_dark());

    composed.save("/tmp/coreimage-basic.png", ImageFormat::Png, 100)?;
    println!("saved /tmp/coreimage-basic.png");
    Ok(())
}
