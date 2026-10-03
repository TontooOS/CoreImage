//! Pure-Rust image codecs for CoreImage.
//!
//! Each file extension maps to one codec module. `png`, `jpeg`,
//! `gif`, `bmp`, `ico` and `webp` are fully implemented in pure Rust
//! with no third-party dependencies. `avif` parses the ISOBMFF container
//! and the AV1 bitstream headers; pixel reconstruction lives in [`av1`].
//!
//! To add a codec: create `<format>.rs` with `decode`, `encode`,
//! `dimensions` and `is_<format>` functions mirroring [`png`],
//! register it below and route it in `crate::io`.

pub mod av1;
pub mod avif;
pub mod bmp;
pub mod gif;
pub mod ico;
pub mod jpeg;
pub mod png;
pub mod webp;
mod webp_vp8_tables;
