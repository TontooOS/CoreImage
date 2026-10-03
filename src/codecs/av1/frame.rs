//! AV1 frame header and tile group parsing for still images.
//!
//! Only intra frames are decoded, so the inter frame branches of
//! `uncompressed_header( )` are reported as `Unsupported` instead of being
//! parsed. Everything an AVIF still image can carry is parsed faithfully.

use super::Av1Error;
use super::SequenceHeader;
use super::bit::BitReader;
use super::tables;

fn err(m: impl Into<String>) -> Av1Error {
    Av1Error::Decode(m.into())
}

fn unsupported(m: impl Into<String>) -> Av1Error {
    Av1Error::Unsupported(m.into())
}

/// One tile of a tile group: its byte range inside the OBU payload and the
/// mode info range it covers.
#[derive(Debug, Clone)]
pub struct TileInfo {
    /// Tile number inside the frame, row major.
    pub num: usize,
    pub row: usize,
    pub col: usize,
    /// Entropy coded payload size in bytes.
    pub size: usize,
    /// Offset of the payload inside the tile group OBU payload.
    pub offset: usize,
    pub mi_row_start: usize,
    pub mi_row_end: usize,
    pub mi_col_start: usize,
    pub mi_col_end: usize,
}

impl TileInfo {
    /// Width of the tile in mode info units.
    pub fn mi_width(&self) -> usize {
        self.mi_col_end - self.mi_col_start
    }

    /// Height of the tile in mode info units.
    pub fn mi_height(&self) -> usize {
        self.mi_rows_range()
    }

    /// Height of the tile in mode info units.
    pub fn mi_rows_range(&self) -> usize {
        self.mi_row_end - self.mi_row_start
    }

    /// First luma sample row of the tile.
    pub fn row_start(&self) -> u32 {
        (self.mi_row_start * 4) as u32
    }

    /// First luma sample column of the tile.
    pub fn col_start(&self) -> u32 {
        (self.mi_col_start * 4) as u32
    }
}

/// `uncompressed_header( )` for key and intra-only frames.
#[derive(Debug, Clone, Default)]
pub struct FrameHeader {
    pub show_existing_frame: bool,
    pub frame_type: u8,
    pub show_frame: bool,
    pub showable_frame: bool,
    pub error_resilient_mode: bool,
    pub disable_cdf_update: bool,
    pub allow_screen_content_tools: bool,
    pub force_integer_mv: bool,
    pub allow_intrabc: bool,
    pub frame_size_override_flag: bool,
    pub order_hint: u32,
    pub primary_ref_frame: u8,
    pub refresh_frame_flags: u8,
    pub frame_width: u32,
    pub frame_height: u32,
    pub upscaled_width: u32,
    pub render_width: u32,
    pub render_height: u32,
    pub superres_denom: u32,
    pub use_superres: bool,
    pub disable_frame_end_update_cdf: bool,
    pub tile_cols: usize,
    pub tile_rows: usize,
    pub tile_cols_log2: u8,
    pub tile_rows_log2: u8,
    pub context_update_tile_id: u32,
    pub tile_size_bytes: u32,
    pub mi_cols: usize,
    pub mi_rows: usize,
    pub mi_col_starts: Vec<usize>,
    pub mi_row_starts: Vec<usize>,
    pub base_q_idx: u32,
    pub delta_q_y_dc: i32,
    pub delta_q_u_dc: i32,
    pub delta_q_u_ac: i32,
    pub delta_q_v_dc: i32,
    pub delta_q_v_ac: i32,
    pub using_qmatrix: bool,
    pub qm_y: u32,
    pub qm_u: u32,
    pub qm_v: u32,
    pub delta_q_present: bool,
    pub delta_q_res: u32,
    pub delta_lf_present: bool,
    pub delta_lf_res: u32,
    pub delta_lf_multi: bool,
    pub segmentation_enabled: bool,
    pub segmentation_update_map: bool,
    pub segmentation_temporal_update: bool,
    pub segmentation_update_data: bool,
    pub feature_enabled: [[bool; tables::SEG_LVL_MAX]; tables::MAX_SEGMENTS],
    pub feature_data: [[i32; tables::SEG_LVL_MAX]; tables::MAX_SEGMENTS],
    pub seg_id_pre_skip: usize,
    pub last_active_seg_id: usize,
    pub coded_lossless: bool,
    pub all_lossless: bool,
    pub loop_filter_level: [u32; 4],
    pub loop_filter_ref_deltas: [i32; 8],
    pub loop_filter_mode_deltas: [i32; 2],
    pub loop_filter_sharpness: u32,
    pub loop_filter_delta_enabled: bool,
    pub cdef_damping: u32,
    pub cdef_bits: u32,
    pub cdef_y_pri_strength: [u32; 8],
    pub cdef_y_sec_strength: [u32; 8],
    pub cdef_uv_pri_strength: [u32; 8],
    pub cdef_uv_sec_strength: [u32; 8],
    pub uses_lr: bool,
    pub frame_restoration_type: [u8; 3],
    pub loop_restoration_size: [u32; 3],
    pub lr_unit_shift: u32,
    pub tx_mode: u32,
    pub reduced_tx_set: bool,
    pub film_grain_params: Option<FilmGrainParams>,
}

/// Film grain parameters of `film_grain_params( )`.
#[derive(Debug, Clone, Default)]
pub struct FilmGrainParams {
    pub apply_grain: bool,
    pub grain_seed: u32,
    pub num_y_points: u32,
    pub point_y_value: [u32; 14],
    pub point_y_scaling: [u32; 14],
    pub chroma_scaling_from_luma: bool,
    pub num_cb_points: u32,
    pub point_cb_value: [u32; 8],
    pub point_cb_scaling: [u32; 8],
    pub num_cr_points: u32,
    pub point_cr_value: [u32; 8],
    pub point_cr_scaling: [u32; 8],
    pub grain_scaling_minus_8: u32,
    pub ar_coeff_lag: u32,
    pub ar_coeffs_y_plus_128: Vec<u32>,
    pub ar_coeffs_cb_plus_128: Vec<u32>,
    pub ar_coeffs_cr_plus_128: Vec<u32>,
    pub grain_scale_shift: u32,
    pub cb_mult: u32,
    pub cb_luma_mult: u32,
    pub cb_offset: u32,
    pub cr_mult: u32,
    pub cr_luma_mult: u32,
    pub cr_offset: u32,
}

const KEY_FRAME: u8 = 0;
const INTRA_ONLY_FRAME: u8 = 1;
const SWITCH_FRAME: u8 = 2;
const PRIMARY_REF_NONE: u8 = 7;
const TX_MODE_LARGEST: u32 = 0;
const TX_MODE_SELECT: u32 = 1;
const ONLY_4X4: u32 = 3;
const RESTORE_NONE: u8 = 0;
const RESTORE_SWITCHABLE: u8 = 1;
const RESTORE_WIENER: u8 = 2;
const RESTORE_SGRPROJ: u8 = 3;

/// `frame_header_obu( )` including `uncompressed_header( )`.
pub fn frame_header_obu(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
) -> Result<FrameHeader, Av1Error> {
    uncompressed_header(r, seq)
}

fn uncompressed_header(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
) -> Result<FrameHeader, Av1Error> {
    let mut fh = FrameHeader {
        qm_y: 15,
        qm_u: 15,
        qm_v: 15,
        loop_filter_ref_deltas: [1, 0, 0, 0, -1, -1, -1, -1],
        ..FrameHeader::default()
    };
    let num_planes = seq.chroma_format().planes();
    let sb_shift = if seq.use_128x128_superblock { 5 } else { 4 };
    if seq.reduced_still_picture_header {
        fh.frame_type = KEY_FRAME;
        fh.show_frame = true;
    } else {
        fh.show_existing_frame = r.flag()?;
        if fh.show_existing_frame {
            // Still images never reuse a frame; stop before the reference
            // handling that this decoder does not implement.
            return Err(unsupported("show_existing_frame is not a still image"));
        }
        fh.frame_type = r.f(2)? as u8;
        if fh.frame_type == SWITCH_FRAME {
            return Err(unsupported("switch frames are not still images"));
        }
        if fh.frame_type != KEY_FRAME && fh.frame_type != INTRA_ONLY_FRAME {
            return Err(unsupported(format!(
                "inter frame type {} is not a still image",
                fh.frame_type
            )));
        }
        fh.show_frame = r.flag()?;
        if fh.show_frame {
            fh.showable_frame = fh.frame_type != KEY_FRAME;
        } else {
            fh.showable_frame = r.flag()?;
            if !fh.show_frame {
                return Err(unsupported("hidden intra frame is not a still image"));
            }
        }
        fh.error_resilient_mode = if fh.frame_type == KEY_FRAME && fh.show_frame {
            true
        } else {
            r.flag()?
        };
    }
    fh.disable_cdf_update = r.flag()?;
    if seq.seq_force_screen_content_tools == 2 {
        fh.allow_screen_content_tools = r.flag()?;
    } else {
        fh.allow_screen_content_tools = seq.seq_force_screen_content_tools != 0;
    }
    if fh.allow_screen_content_tools {
        if seq.seq_force_integer_mv == 2 {
            let _force_integer_mv = r.f(1)?;
        }
    }
    // Intra frames always use integer motion vectors.
    fh.force_integer_mv = true;
    if seq.frame_id_numbers_present_flag {
        let _current_frame_id = r.f(
            seq.additional_frame_id_length_minus_1 as u32 + seq.delta_frame_id_length_minus_2 as u32 + 3,
        )?;
    }
    fh.frame_size_override_flag = if seq.reduced_still_picture_header {
        false
    } else {
        r.flag()?
    };
    if seq.enable_order_hint {
        fh.order_hint = r.f(seq.order_hint_bits as u32)?;
    }
    fh.primary_ref_frame = PRIMARY_REF_NONE;
    fh.refresh_frame_flags = if fh.frame_type == KEY_FRAME && fh.show_frame {
        0xff
    } else {
        r.f(8)? as u8
    };
    if fh.frame_type == KEY_FRAME && fh.show_frame {
        // Reference state is reset; only the order hints are used later and
        // those are irrelevant for intra decoding.
    }
    // frame_size( ) and render_size( )
    if fh.frame_size_override_flag {
        fh.frame_width = r.f(seq.frame_width_bits_minus_1 as u32 + 1)? + 1;
        fh.frame_height = r.f(seq.frame_height_bits_minus_1 as u32 + 1)? + 1;
    } else {
        fh.frame_width = seq.max_width();
        fh.frame_height = seq.max_height();
    }
    // superres_params( )
    fh.use_superres = if seq.enable_superres { r.flag()? } else { false };
    fh.superres_denom = if fh.use_superres {
        let coded_denom = r.f(9)?;
        coded_denom + 9
    } else {
        8
    };
    fh.upscaled_width = fh.frame_width;
    fh.frame_width = (fh.upscaled_width * 8 + (fh.superres_denom / 2)) / fh.superres_denom;
    // render_size( )
    if r.flag()? {
        fh.render_width = r.f(16)? + 1;
        fh.render_height = r.f(16)? + 1;
    } else {
        fh.render_width = fh.upscaled_width;
        fh.render_height = fh.frame_height;
    }
    if fh.use_superres {
        return Err(unsupported("superres is not supported"));
    }
    if fh.allow_screen_content_tools && fh.upscaled_width == fh.frame_width {
        fh.allow_intrabc = r.flag()?;
    }
    if fh.disable_cdf_update || seq.reduced_still_picture_header {
        fh.disable_frame_end_update_cdf = true;
    } else {
        fh.disable_frame_end_update_cdf = r.flag()?;
    }
    // tile_info( )
    fh.mi_cols = (fh.frame_width + 3) as usize >> 2;
    fh.mi_rows = (fh.frame_height + 3) as usize >> 2;
    tile_info(r, seq, sb_shift, &mut fh)?;
    // quantization_params( )
    fh.base_q_idx = r.f(8)?;
    fh.delta_q_y_dc = read_delta_q(r)?;
    if num_planes > 1 {
        let diff_uv_delta = if seq.separate_uv_delta_q { r.flag()? } else { false };
        fh.delta_q_u_dc = read_delta_q(r)?;
        fh.delta_q_u_ac = read_delta_q(r)?;
        if diff_uv_delta {
            fh.delta_q_v_dc = read_delta_q(r)?;
            fh.delta_q_v_ac = read_delta_q(r)?;
        } else {
            fh.delta_q_v_dc = fh.delta_q_u_dc;
            fh.delta_q_v_ac = fh.delta_q_u_ac;
        }
    }
    fh.using_qmatrix = r.flag()?;
    if fh.using_qmatrix {
        fh.qm_y = r.f(4)?;
        fh.qm_u = r.f(4)?;
        fh.qm_v = if seq.separate_uv_delta_q { r.f(4)? } else { fh.qm_u };
    }
    // segmentation_params( )
    segmentation_params(r, &mut fh)?;
    // delta_q_params( ) and delta_lf_params( )
    if fh.base_q_idx > 0 {
        fh.delta_q_present = r.flag()?;
    }
    if fh.delta_q_present {
        fh.delta_q_res = r.f(2)?;
    }
    if fh.delta_q_present && !fh.allow_intrabc {
        fh.delta_lf_present = r.flag()?;
        if fh.delta_lf_present {
            fh.delta_lf_res = r.f(2)?;
            fh.delta_lf_multi = r.flag()?;
        }
    }
    // Lossless flags depend on every segment's qindex.
    fh.coded_lossless = true;
    for segment_id in 0..tables::MAX_SEGMENTS {
        let qindex = get_qindex(true, segment_id, &fh);
        let lossless = qindex == 0
            && fh.delta_q_y_dc == 0
            && fh.delta_q_u_ac == 0
            && fh.delta_q_u_dc == 0
            && fh.delta_q_v_ac == 0
            && fh.delta_q_v_dc == 0;
        if !lossless {
            fh.coded_lossless = false;
        }
    }
    fh.all_lossless = fh.coded_lossless && fh.frame_width == fh.upscaled_width;
    // loop_filter_params( )
    if fh.coded_lossless || fh.allow_intrabc {
        return Err(err(
            "lossless and intrabc frames are not supported by this decoder",
        ));
    }
    loop_filter_params(r, &mut fh, num_planes)?;
    // cdef_params( )
    cdef_params(r, &mut fh, num_planes, seq.enable_cdef)?;
    // lr_params( )
    lr_params(r, seq, &mut fh, num_planes)?;
    // read_tx_mode( )
    fh.tx_mode = if fh.coded_lossless {
        ONLY_4X4
    } else if r.flag()? {
        TX_MODE_SELECT
    } else {
        TX_MODE_LARGEST
    };
    // frame_reference_mode( ) and skip_mode_params( ) only apply to inter
    // frames but are still coded in the bitstream.
    skip_mode_params(r, seq, &mut fh)?;
    // allow_warped_motion is not read for intra frames with error resilience.
    let _allow_warped_motion = if fh.error_resilient_mode || !seq.enable_warped_motion {
        false
    } else {
        r.flag()?
    };
    fh.reduced_tx_set = r.flag()?;
    global_motion_params(r, seq, &mut fh)?;
    fh.film_grain_params = film_grain_params(r, seq, &fh)?;
    Ok(fh)
}

fn read_delta_q(r: &mut BitReader<'_>) -> Result<i32, Av1Error> {
    if r.flag()? {
        r.su(1 + 6)
    } else {
        Ok(0)
    }
}

/// `tile_info( )`.
fn tile_info(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    sb_shift: u32,
    fh: &mut FrameHeader,
) -> Result<(), Av1Error> {
    let sb_cols = (if seq.use_128x128_superblock {
        (fh.mi_cols + 31) >> 5
    } else {
        (fh.mi_cols + 15) >> 4
    }) as u32;
    let sb_rows = (if seq.use_128x128_superblock {
        (fh.mi_rows + 31) >> 5
    } else {
        (fh.mi_rows + 15) >> 4
    }) as u32;
    let sb_size = sb_shift + 2;
    let max_tile_width_sb = tables::MAX_TILE_WIDTH >> sb_size;
    let max_tile_area_sb = tables::MAX_TILE_AREA >> (2 * sb_size);
    let min_log2_tile_cols = tables::tile_log2(max_tile_width_sb, sb_cols);
    let max_log2_tile_cols =
        tables::tile_log2(1, sb_cols.min(tables::MAX_TILE_COLS as u32));
    let max_log2_tile_rows =
        tables::tile_log2(1, sb_rows.min(tables::MAX_TILE_ROWS as u32));
    let min_log2_tiles = min_log2_tile_cols.max(tables::tile_log2(
        max_tile_area_sb,
        sb_rows * sb_cols,
    ));
    let uniform_tile_spacing_flag = r.flag()?;
    if uniform_tile_spacing_flag {
        fh.tile_cols_log2 = min_log2_tile_cols as u8;
        while (fh.tile_cols_log2 as u32) < max_log2_tile_cols {
            if r.flag()? {
                fh.tile_cols_log2 += 1;
            } else {
                break;
            }
        }
        let tile_width_sb =
            (sb_cols + (1u32 << fh.tile_cols_log2) - 1) >> fh.tile_cols_log2;
        let mut i = 0usize;
        let mut start_sb = 0u32;
        while start_sb < sb_cols {
            fh.mi_col_starts.push((start_sb << sb_shift) as usize);
            i += 1;
            start_sb += tile_width_sb;
        }
        fh.mi_col_starts.push(fh.mi_cols);
        fh.tile_cols = i;
        let min_log2_tile_rows = min_log2_tiles
            .saturating_sub(fh.tile_cols_log2 as u32)
            .max(0);
        fh.tile_rows_log2 = min_log2_tile_rows as u8;
        while (fh.tile_rows_log2 as u32) < max_log2_tile_rows {
            if r.flag()? {
                fh.tile_rows_log2 += 1;
            } else {
                break;
            }
        }
        let tile_height_sb =
            (sb_rows + (1u32 << fh.tile_rows_log2) - 1) >> fh.tile_rows_log2;
        let mut j = 0usize;
        let mut start_sb = 0u32;
        while start_sb < sb_rows {
            fh.mi_row_starts.push((start_sb << sb_shift) as usize);
            j += 1;
            start_sb += tile_height_sb;
        }
        fh.mi_row_starts.push(fh.mi_rows);
        fh.tile_rows = j;
    } else {
        let mut widest_tile_sb = 0u32;
        let mut i = 0usize;
        let mut start_sb = 0u32;
        while start_sb < sb_cols {
            fh.mi_col_starts.push((start_sb << sb_shift) as usize);
            let max_width = (sb_cols - start_sb).min(max_tile_width_sb);
            let size_sb = r.ns(max_width)? + 1;
            widest_tile_sb = size_sb.max(widest_tile_sb);
            start_sb += size_sb;
            i += 1;
        }
        fh.mi_col_starts.push(fh.mi_cols);
        fh.tile_cols = i;
        fh.tile_cols_log2 = tables::tile_log2(1, fh.tile_cols as u32) as u8;
        let max_tile_area_sb = if min_log2_tiles > 0 {
            (sb_rows * sb_cols) >> (min_log2_tiles + 1)
        } else {
            sb_rows * sb_cols
        };
        let max_tile_height_sb = (max_tile_area_sb / widest_tile_sb).max(1);
        let mut j = 0usize;
        let mut start_sb = 0u32;
        while start_sb < sb_rows {
            fh.mi_row_starts.push((start_sb << sb_shift) as usize);
            let max_height = (sb_rows - start_sb).min(max_tile_height_sb);
            let size_sb = r.ns(max_height)? + 1;
            start_sb += size_sb;
            j += 1;
        }
        fh.mi_row_starts.push(fh.mi_rows);
        fh.tile_rows = j;
        fh.tile_rows_log2 = tables::tile_log2(1, fh.tile_rows as u32) as u8;
    }
    if fh.tile_cols_log2 > 0 || fh.tile_rows_log2 > 0 {
        fh.context_update_tile_id = r.f(fh.tile_rows_log2 as u32 + fh.tile_cols_log2 as u32)?;
        fh.tile_size_bytes = r.f(2)? + 1;
    } else {
        fh.context_update_tile_id = 0;
    }
    Ok(())
}

fn segmentation_params(r: &mut BitReader<'_>, fh: &mut FrameHeader) -> Result<(), Av1Error> {
    fh.segmentation_enabled = r.flag()?;
    if fh.segmentation_enabled {
        fh.segmentation_update_map = true;
        fh.segmentation_temporal_update = false;
        fh.segmentation_update_data = true;
        if fh.segmentation_update_data {
            for i in 0..tables::MAX_SEGMENTS {
                for j in 0..tables::SEG_LVL_MAX {
                    let feature_enabled = r.flag()?;
                    fh.feature_enabled[i][j] = feature_enabled;
                    if feature_enabled {
                        let bits = tables::SEGMENTATION_FEATURE_BITS[j] as u32;
                        let limit = tables::SEGMENTATION_FEATURE_MAX[j] as i64;
                        let clipped = if tables::SEGMENTATION_FEATURE_SIGNED[j] {
                            let v = r.su(1 + bits)?;
                            tables::clip3(-limit, limit, v as i64) as i32
                        } else {
                            let v = r.f(bits)? as i64;
                            tables::clip3(0, limit, v) as i32
                        };
                        fh.feature_data[i][j] = clipped;
                    } else {
                        fh.feature_data[i][j] = 0;
                    }
                }
            }
        }
    }
    fh.seg_id_pre_skip = 0;
    fh.last_active_seg_id = 0;
    if fh.segmentation_enabled {
        for i in 0..tables::MAX_SEGMENTS {
            for j in 0..tables::SEG_LVL_MAX {
                if fh.feature_enabled[i][j] {
                    fh.last_active_seg_id = i;
                    if j >= tables::SEG_LVL_REF_FRAME {
                        fh.seg_id_pre_skip = 1;
                    }
                }
            }
        }
    }
    Ok(())
}

/// `get_qindex( ignoreDeltaQ, segmentId )`.
pub fn get_qindex(ignore_delta_q: bool, segment_id: usize, fh: &FrameHeader) -> u32 {
    if fh.segmentation_enabled && fh.feature_enabled[segment_id][tables::SEG_LVL_ALT_Q] {
        let data = fh.feature_data[segment_id][tables::SEG_LVL_ALT_Q];
        let qindex = fh.base_q_idx as i64 + data as i64;
        let qindex = tables::clip3(0, 255, qindex);
        if !ignore_delta_q && fh.delta_q_present {
            tables::clip3(0, 255, qindex + fh.delta_q_res as i64) as u32
        } else {
            qindex as u32
        }
    } else if !ignore_delta_q && fh.delta_q_present {
        tables::clip3(0, 255, (fh.base_q_idx + fh.delta_q_res) as i64) as u32
    } else {
        fh.base_q_idx
    }
}

fn loop_filter_params(
    r: &mut BitReader<'_>,
    fh: &mut FrameHeader,
    num_planes: u32,
) -> Result<(), Av1Error> {
    fh.loop_filter_level[0] = r.f(6)?;
    fh.loop_filter_level[1] = r.f(6)?;
    if num_planes > 1 && (fh.loop_filter_level[0] != 0 || fh.loop_filter_level[1] != 0) {
        fh.loop_filter_level[2] = r.f(6)?;
        fh.loop_filter_level[3] = r.f(6)?;
    }
    fh.loop_filter_sharpness = r.f(3)?;
    fh.loop_filter_delta_enabled = r.flag()?;
    if fh.loop_filter_delta_enabled && r.flag()? {
        for i in 0..tables::TOTAL_REFS_PER_FRAME {
            if r.flag()? {
                fh.loop_filter_ref_deltas[i] = r.su(1 + 6)?;
            }
        }
        for i in 0..2 {
            if r.flag()? {
                fh.loop_filter_mode_deltas[i] = r.su(1 + 6)?;
            }
        }
    }
    Ok(())
}

fn cdef_params(
    r: &mut BitReader<'_>,
    fh: &mut FrameHeader,
    num_planes: u32,
    enable_cdef: bool,
) -> Result<(), Av1Error> {
    if fh.coded_lossless || fh.allow_intrabc || !enable_cdef {
        fh.cdef_bits = 0;
        fh.cdef_damping = 3;
        return Ok(());
    }
    fh.cdef_damping = r.f(2)? + 3;
    fh.cdef_bits = r.f(2)?;
    for i in 0..(1usize << fh.cdef_bits) {
        fh.cdef_y_pri_strength[i] = r.f(4)?;
        let mut sec = r.f(2)?;
        if sec == 3 {
            sec += 1;
        }
        fh.cdef_y_sec_strength[i] = sec;
        if num_planes > 1 {
            fh.cdef_uv_pri_strength[i] = r.f(4)?;
            let mut sec = r.f(2)?;
            if sec == 3 {
                sec += 1;
            }
            fh.cdef_uv_sec_strength[i] = sec;
        }
    }
    Ok(())
}

fn lr_params(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    fh: &mut FrameHeader,
    num_planes: u32,
) -> Result<(), Av1Error> {
    fh.frame_restoration_type = [RESTORE_NONE; 3];
    if fh.all_lossless || fh.allow_intrabc || !seq.enable_restoration {
        return Ok(());
    }
    fh.uses_lr = false;
    let mut uses_chroma_lr = false;
    for i in 0..num_planes as usize {
        let lr_type = r.f(2)? as usize;
        let remapped = match lr_type {
            0 => RESTORE_NONE,
            1 => RESTORE_SWITCHABLE,
            2 => RESTORE_WIENER,
            _ => RESTORE_SGRPROJ,
        };
        fh.frame_restoration_type[i] = remapped;
        if remapped != RESTORE_NONE {
            fh.uses_lr = true;
            if i > 0 {
                uses_chroma_lr = true;
            }
        }
    }
    if fh.uses_lr {
        if seq.use_128x128_superblock {
            fh.lr_unit_shift = r.f(1)? + 1;
        } else {
            fh.lr_unit_shift = r.f(1)?;
            if fh.lr_unit_shift != 0 {
                fh.lr_unit_shift += r.f(1)?;
            }
        }
        fh.loop_restoration_size[0] = tables::RESTORATION_TILESIZE_MAX >> (2 - fh.lr_unit_shift);
        let lr_uv_shift = if seq.subsampling_x == 1 && seq.subsampling_y == 1 && uses_chroma_lr {
            r.f(1)?
        } else {
            0
        };
        fh.loop_restoration_size[1] = fh.loop_restoration_size[0] >> lr_uv_shift;
        fh.loop_restoration_size[2] = fh.loop_restoration_size[0] >> lr_uv_shift;
    }
    Ok(())
}

fn skip_mode_params(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    fh: &mut FrameHeader,
) -> Result<(), Av1Error> {
    // Intra frames never allow skip mode, so no bits are read.
    let _ = (r, seq, fh);
    Ok(())
}

fn global_motion_params(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    fh: &mut FrameHeader,
) -> Result<(), Av1Error> {
    if fh.frame_type == KEY_FRAME || fh.frame_type == INTRA_ONLY_FRAME {
        return Ok(());
    }
    let _ = (r, seq);
    Ok(())
}

fn film_grain_params(
    r: &mut BitReader<'_>,
    seq: &SequenceHeader,
    fh: &FrameHeader,
) -> Result<Option<FilmGrainParams>, Av1Error> {
    let mut fg = FilmGrainParams::default();
    if !seq.film_grain_params_present || (!fh.show_frame && !fh.showable_frame) {
        return Ok(None);
    }
    fg.apply_grain = r.flag()?;
    if !fg.apply_grain {
        return Ok(Some(fg));
    }
    fg.grain_seed = r.f(16)?;
    // update_grain is 1 for every frame type except inter frames.
    fg.num_y_points = r.f(4)?;
    for i in 0..fg.num_y_points as usize {
        fg.point_y_value[i] = r.f(8)?;
        fg.point_y_scaling[i] = r.f(8)?;
    }
    fg.chroma_scaling_from_luma = if seq.mono_chrome {
        false
    } else {
        r.flag()?
    };
    if seq.mono_chrome
        || fg.chroma_scaling_from_luma
        || (seq.subsampling_x == 1 && seq.subsampling_y == 1 && fg.num_y_points == 0)
    {
        fg.num_cb_points = 0;
        fg.num_cr_points = 0;
    } else {
        fg.num_cb_points = r.f(4)?;
        for i in 0..fg.num_cb_points as usize {
            fg.point_cb_value[i] = r.f(8)?;
            fg.point_cb_scaling[i] = r.f(8)?;
        }
        fg.num_cr_points = r.f(4)?;
        for i in 0..fg.num_cr_points as usize {
            fg.point_cr_value[i] = r.f(8)?;
            fg.point_cr_scaling[i] = r.f(8)?;
        }
    }
    fg.grain_scaling_minus_8 = r.f(2)?;
    fg.ar_coeff_lag = r.f(2)?;
    let num_pos_luma = 2 * fg.ar_coeff_lag * (fg.ar_coeff_lag + 1);
    if fg.num_y_points != 0 {
        let num_pos_chroma = num_pos_luma + 1;
        let mut y = Vec::with_capacity(num_pos_luma as usize);
        for _ in 0..num_pos_luma {
            y.push(r.f(8)?);
        }
        fg.ar_coeffs_y_plus_128 = y;
        let mut cb = Vec::with_capacity(num_pos_chroma as usize);
        for _ in 0..num_pos_chroma {
            cb.push(r.f(8)?);
        }
        fg.ar_coeffs_cb_plus_128 = cb;
        let mut cr = Vec::with_capacity(num_pos_chroma as usize);
        for _ in 0..num_pos_chroma {
            cr.push(r.f(8)?);
        }
        fg.ar_coeffs_cr_plus_128 = cr;
    }
    fg.grain_scale_shift = r.f(2)?;
    if fg.num_cb_points != 0 {
        fg.cb_mult = r.f(8)?;
        fg.cb_luma_mult = r.f(8)?;
        fg.cb_offset = r.f(9)?;
    }
    if fg.num_cr_points != 0 {
        fg.cr_mult = r.f(8)?;
        fg.cr_luma_mult = r.f(8)?;
        fg.cr_offset = r.f(9)?;
    }
    Ok(Some(fg))
}

/// `frame_obu( sz )`: the frame header followed by one tile group.
///
/// Returns the byte length of the frame header inside the payload and the tiles.
pub fn frame_obu(
    payload: &[u8],
    header: &mut FrameHeader,
    seq: &SequenceHeader,
) -> Result<(usize, Vec<TileInfo>), Av1Error> {
    let mut r = BitReader::new(payload);
    let start = r.position();
    let parsed = frame_header_obu(&mut r, seq)?;
    *header = parsed;
    r.byte_align()?;
    let header_bytes = (r.position() - start) / 8;
    let tiles = tile_group_obu(payload, header_bytes, header)?;
    Ok((header_bytes, tiles))
}

/// `tile_group_obu( sz )` up to the start of the entropy coded tile data.
///
/// `offset` is the byte position of the tile group inside the OBU payload, so
/// the returned tile offsets stay relative to the OBU.
pub fn tile_group_obu(
    payload: &[u8],
    offset: usize,
    fh: &FrameHeader,
) -> Result<Vec<TileInfo>, Av1Error> {
    let num_tiles = fh.tile_cols * fh.tile_rows;
    if num_tiles == 0 {
        return Err(err("frame header without tiles"));
    }
    let mut r = BitReader::new(payload);
    r.set_position(offset * 8);
    let start = r.position();
    let present = if num_tiles > 1 { r.flag()? } else { false };
    let (tg_start, tg_end) = if num_tiles == 1 || !present {
        (0usize, num_tiles - 1)
    } else {
        let bits = fh.tile_cols_log2 as u32 + fh.tile_rows_log2 as u32;
        (r.f(bits)? as usize, r.f(bits)? as usize)
    };
    if tg_end >= num_tiles || tg_start > tg_end {
        return Err(err("tile group range outside the tile grid"));
    }
    r.byte_align()?;
    let header_bytes = (r.position() - start) / 8;
    let mut pos = offset + header_bytes;
    let mut tiles = Vec::with_capacity(tg_end - tg_start + 1);
    for tile_num in tg_start..=tg_end {
        let last = tile_num == tg_end;
        let size = if last {
            payload.len() - pos
        } else {
            if fh.tile_size_bytes == 0 {
                return Err(err("tiled frame without tile_size_bytes"));
            }
            let size_minus_1 = r.le(fh.tile_size_bytes)? as usize;
            pos += fh.tile_size_bytes as usize;
            size_minus_1 + 1
        };
        let row = tile_num / fh.tile_cols;
        let col = tile_num % fh.tile_cols;
        let tile = TileInfo {
            num: tile_num,
            row,
            col,
            size,
            offset: pos,
            mi_row_start: fh.mi_row_starts.get(row).copied().unwrap_or(0),
            mi_row_end: fh.mi_row_starts.get(row + 1).copied().unwrap_or(0),
            mi_col_start: fh.mi_col_starts.get(col).copied().unwrap_or(0),
            mi_col_end: fh.mi_col_starts.get(col + 1).copied().unwrap_or(0),
        };
        pos += size;
        tiles.push(tile);
    }
    Ok(tiles)
}