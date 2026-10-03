//! Constant tables and helpers shared by the AV1 decoder.
//!
//! The numeric constants come from the symbols table of the specification and
//! are mirrored in [`super::spec_consts`]. The lookup tables here are the
//! conversion tables of section 7.

/// `MAX_SEGMENTS`.
pub const MAX_SEGMENTS: usize = 8;
/// `SEG_LVL_MAX`.
pub const SEG_LVL_MAX: usize = 8;
/// `SEG_LVL_ALT_Q`.
pub const SEG_LVL_ALT_Q: usize = 0;
/// `SEG_LVL_REF_FRAME`.
pub const SEG_LVL_REF_FRAME: usize = 5;
/// `MAX_TILE_WIDTH`.
pub const MAX_TILE_WIDTH: u32 = 4096;
/// `MAX_TILE_AREA`.
pub const MAX_TILE_AREA: u32 = 4096 * 2304;
/// `MAX_TILE_COLS`.
pub const MAX_TILE_COLS: usize = 64;
/// `MAX_TILE_ROWS`.
pub const MAX_TILE_ROWS: usize = 64;
/// `TOTAL_REFS_PER_FRAME`.
pub const TOTAL_REFS_PER_FRAME: usize = 8;
/// `RESTORATION_TILESIZE_MAX`.
pub const RESTORATION_TILESIZE_MAX: u32 = 256;
/// `MAX_LOOP_FILTER`.
pub const MAX_LOOP_FILTER: i32 = 63;

/// `Segmentation_Feature_Bits`.
pub const SEGMENTATION_FEATURE_BITS: [u32; SEG_LVL_MAX] = [8, 6, 6, 6, 6, 3, 0, 0];
/// `Segmentation_Feature_Signed`.
pub const SEGMENTATION_FEATURE_SIGNED: [bool; SEG_LVL_MAX] =
    [true, true, true, true, true, false, false, false];
/// `Segmentation_Feature_Max`.
pub const SEGMENTATION_FEATURE_MAX: [i32; SEG_LVL_MAX] =
    [255, MAX_LOOP_FILTER, MAX_LOOP_FILTER, MAX_LOOP_FILTER, MAX_LOOP_FILTER, 7, 0, 0];

/// `tile_log2( blkSize, target )`.
pub fn tile_log2(blk_size: u32, target: u32) -> u32 {
    let mut k = 0u32;
    while (blk_size << k) < target {
        k += 1;
    }
    k
}

/// `Clip3( low, high, value )`.
pub fn clip3(low: i64, high: i64, value: i64) -> i64 {
    value.max(low).min(high)
}

/// `Min( a, b )` for unsigned values.
pub fn min_u32(a: u32, b: u32) -> u32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `Max( a, b )` for unsigned values.
pub fn max_u32(a: u32, b: u32) -> u32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `Abs( x )`.
pub fn abs_i32(x: i32) -> i32 {
    x.abs()
}

/// `Round2( x, n )`.
pub fn round2(x: i64, n: u32) -> i64 {
    super::bit::round2(x, n)
}