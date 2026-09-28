//! Pure-Rust image codecs for CoreImage.
//!
//! Each file extension maps to one codec module. `png`, `jpeg`,
//! `gif` and `ico` are fully implemented in pure Rust with no
//! third-party dependencies. WebP and BMP still decode/encode through
//! the `image` crate until their own codec modules land here
//! (`webp.rs`, `bmp.rs`; the BMP entry parser in [`ico`] is reusable).
//!
//! To add a codec: create `<format>.rs` with `decode`, `encode`,
//! `dimensions` and `is_<format>` functions mirroring [`png`],
//! register it below and route it in `crate::io`.

pub mod gif;
pub mod ico;
pub mod jpeg;
pub mod png;
