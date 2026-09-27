//! Pure-Rust image codecs for CoreImage.
//!
//! Each file extension maps to one codec module. `png` and `jpeg`
//! are fully implemented in pure Rust with no third-party
//! dependencies. GIF, WebP and BMP still decode/encode through the
//! `image` crate until their own codec modules land here (`gif.rs`,
//! `webp.rs`, `bmp.rs`).
//!
//! To add a codec: create `<format>.rs` with `decode`, `encode`,
//! `dimensions` and `is_<format>` functions mirroring [`png`],
//! register it below and route it in `crate::io`.

pub mod jpeg;
pub mod png;
