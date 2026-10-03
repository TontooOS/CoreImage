//! OBU framing, sequence header parsing and the AV1 decoder entry points.

pub mod bit;
pub mod cdf_default;
pub mod spec_consts;
pub mod symbol;
pub mod tables;
pub mod yuv;

mod frame;

pub use frame::FilmGrainParams;
pub use frame::FrameHeader;
pub use frame::TileInfo;

use bit::BitReader;

/// Errors of the AV1 decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Av1Error {
    Decode(String),
    Unsupported(String),
}

impl std::fmt::Display for Av1Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "av1 decode error: {m}"),
            Self::Unsupported(m) => write!(f, "av1 unsupported: {m}"),
        }
    }
}

impl std::error::Error for Av1Error {}

fn err(m: impl Into<String>) -> Av1Error {
    Av1Error::Decode(m.into())
}

fn unsupported(m: impl Into<String>) -> Av1Error {
    Av1Error::Unsupported(m.into())
}

/// OBU types from section 5.2.1.
pub const OBU_SEQUENCE_HEADER: u8 = 1;
pub const OBU_TEMPORAL_DELIMITER: u8 = 2;
pub const OBU_FRAME_HEADER: u8 = 3;
pub const OBU_TILE_GROUP: u8 = 4;
pub const OBU_METADATA: u8 = 5;
pub const OBU_FRAME: u8 = 6;
pub const OBU_REDUNDANT_FRAME_HEADER: u8 = 7;
pub const OBU_TILE_LIST: u8 = 8;
pub const OBU_PADDING: u8 = 15;

/// Chroma subsampling as coded in the sequence header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChromaFormat {
    /// 4:2:0 (subsampling_x = 1, subsampling_y = 1).
    #[default]
    Cs420,
    /// 4:2:2 (subsampling_x = 1, subsampling_y = 0).
    Cs422,
    /// 4:4:4 (subsampling_x = 0, subsampling_y = 0).
    Cs444,
    /// Monochrome, no chroma planes.
    Monochrome,
}

impl ChromaFormat {
    pub fn subsampling_x(self) -> u32 {
        match self {
            Self::Cs444 => 0,
            _ => 1,
        }
    }

    pub fn subsampling_y(self) -> u32 {
        match self {
            Self::Cs420 | Self::Monochrome => 1,
            _ => 0,
        }
    }

    pub fn planes(self) -> u32 {
        match self {
            Self::Monochrome => 1,
            _ => 3,
        }
    }
}

/// Parsed sequence header, `sequence_header_obu( )`.
#[derive(Debug, Clone, Default)]
pub struct SequenceHeader {
    pub seq_profile: u8,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,
    pub frame_width_bits_minus_1: u8,
    pub frame_height_bits_minus_1: u8,
    pub max_frame_width_minus_1: u32,
    pub max_frame_height_minus_1: u32,
    pub frame_id_numbers_present_flag: bool,
    pub delta_frame_id_length_minus_2: u8,
    pub additional_frame_id_length_minus_1: u8,
    pub use_128x128_superblock: bool,
    pub enable_filter_intra: bool,
    pub enable_intra_edge_filter: bool,
    pub enable_interintra_compound: bool,
    pub enable_masked_compound: bool,
    pub enable_warped_motion: bool,
    pub enable_dual_filter: bool,
    pub enable_order_hint: bool,
    pub enable_jnt_comp: bool,
    pub enable_ref_frame_mvs: bool,
    pub seq_force_screen_content_tools: u8,
    pub seq_force_integer_mv: u8,
    pub order_hint_bits: u8,
    pub enable_superres: bool,
    pub enable_cdef: bool,
    pub enable_restoration: bool,
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub bit_depth: u32,
    pub mono_chrome: bool,
    pub color_primaries: u32,
    pub transfer_characteristics: u32,
    pub matrix_coefficients: u32,
    pub color_range: bool,
    pub subsampling_x: u8,
    pub subsampling_y: u8,
    pub chroma_sample_position: u8,
    pub separate_uv_delta_q: bool,
    pub film_grain_params_present: bool,
}

impl SequenceHeader {
    pub fn max_width(&self) -> u32 {
        self.max_frame_width_minus_1 + 1
    }

    pub fn max_height(&self) -> u32 {
        self.max_frame_height_minus_1 + 1
    }

    pub fn chroma_format(&self) -> ChromaFormat {
        if self.mono_chrome {
            ChromaFormat::Monochrome
        } else if self.subsampling_x == 1 && self.subsampling_y == 1 {
            ChromaFormat::Cs420
        } else if self.subsampling_x == 1 && self.subsampling_y == 0 {
            ChromaFormat::Cs422
        } else {
            ChromaFormat::Cs444
        }
    }
}

const CP_BT_709: u32 = 1;
const TC_SRGB: u32 = 13;
const MC_IDENTITY: u32 = 0;
const CP_UNSPECIFIED: u32 = 2;
const TC_UNSPECIFIED: u32 = 2;
const MC_UNSPECIFIED: u32 = 2;

/// `sequence_header_obu( )`.
pub fn sequence_header_obu(r: &mut BitReader<'_>) -> Result<SequenceHeader, Av1Error> {
    let mut seq = SequenceHeader {
        color_primaries: CP_UNSPECIFIED,
        transfer_characteristics: TC_UNSPECIFIED,
        matrix_coefficients: MC_UNSPECIFIED,
        ..SequenceHeader::default()
    };
    seq.seq_profile = r.f(3)? as u8;
    seq.still_picture = r.flag()?;
    seq.reduced_still_picture_header = r.flag()?;
    let mut decoder_model_info_present_flag = false;
    let mut buffer_delay_length_minus_1 = 0u32;
    if seq.reduced_still_picture_header {
        // The level is read but unused for still pictures.
        let _level = r.f(5)?;
    } else {
        let timing_info_present_flag = r.flag()?;
        if timing_info_present_flag {
            // timing_info( )
            let _num_units_in_display_tick = r.f(32)?;
            let _time_scale = r.f(32)?;
            let equal_picture_interval = r.flag()?;
            if equal_picture_interval {
                let _ = r.uvlc()?;
            }
            decoder_model_info_present_flag = r.flag()?;
            if decoder_model_info_present_flag {
                buffer_delay_length_minus_1 = decoder_model_info(r)?;
            }
        }
        let initial_display_delay_present_flag = r.flag()?;
        let operating_points_cnt_minus_1 = r.f(5)?;
        for _ in 0..=operating_points_cnt_minus_1 {
            let _operating_point_idc = r.f(12)?;
            let seq_level_idx = r.f(5)?;
            let _seq_tier = if seq_level_idx > 7 { r.f(1)? } else { 0 };
            let decoder_model_present_for_this_op = if decoder_model_info_present_flag {
                r.flag()?
            } else {
                false
            };
            if decoder_model_present_for_this_op {
                let _decoder_buffer_delay = r.f(buffer_delay_length_minus_1 + 1)?;
                let _encoder_buffer_delay = r.f(buffer_delay_length_minus_1 + 1)?;
                let _low_delay_mode_flag = r.f(1)?;
            }
            if initial_display_delay_present_flag && r.flag()? {
                let _ = r.f(4)?;
            }
        }
    }
    seq.frame_width_bits_minus_1 = r.f(4)? as u8;
    seq.frame_height_bits_minus_1 = r.f(4)? as u8;
    seq.max_frame_width_minus_1 = r.f(seq.frame_width_bits_minus_1 as u32 + 1)?;
    seq.max_frame_height_minus_1 = r.f(seq.frame_height_bits_minus_1 as u32 + 1)?;
    seq.frame_id_numbers_present_flag = if seq.reduced_still_picture_header {
        false
    } else {
        r.flag()?
    };
    if seq.frame_id_numbers_present_flag {
        seq.delta_frame_id_length_minus_2 = r.f(4)? as u8;
        seq.additional_frame_id_length_minus_1 = r.f(3)? as u8;
    }
    seq.use_128x128_superblock = r.flag()?;
    seq.enable_filter_intra = r.flag()?;
    seq.enable_intra_edge_filter = r.flag()?;
    if seq.reduced_still_picture_header {
        seq.seq_force_screen_content_tools = 2;
        seq.seq_force_integer_mv = 2;
        seq.order_hint_bits = 0;
    } else {
        seq.enable_interintra_compound = r.flag()?;
        seq.enable_masked_compound = r.flag()?;
        seq.enable_warped_motion = r.flag()?;
        seq.enable_dual_filter = r.flag()?;
        seq.enable_order_hint = r.flag()?;
        if seq.enable_order_hint {
            seq.enable_jnt_comp = r.flag()?;
            seq.enable_ref_frame_mvs = r.flag()?;
            seq.order_hint_bits = r.f(3)? as u8 + 1;
        }
        let seq_choose_screen_content_tools = r.flag()?;
        if seq_choose_screen_content_tools {
            seq.seq_force_screen_content_tools = 2;
        } else {
            seq.seq_force_screen_content_tools = r.f(1)? as u8;
        }
        if seq.seq_force_screen_content_tools > 0 {
            if r.flag()? {
                seq.seq_force_integer_mv = 2;
            } else {
                seq.seq_force_integer_mv = r.f(1)? as u8;
            }
        } else {
            seq.seq_force_integer_mv = 2;
        }
    }
    seq.enable_superres = r.flag()?;
    seq.enable_cdef = r.flag()?;
    seq.enable_restoration = r.flag()?;
    color_config(r, &mut seq)?;
    seq.film_grain_params_present = r.flag()?;
    Ok(seq)
}

/// `decoder_model_info( )`, returning `buffer_delay_length_minus_1`.
fn decoder_model_info(r: &mut BitReader<'_>) -> Result<u32, Av1Error> {
    let buffer_delay_length_minus_1 = r.f(5)?;
    let _num_units_in_decoding_tick = r.f(32)?;
    let _buffer_removal_time_length_minus_1 = r.f(5)?;
    let _frame_presentation_time_length_minus_1 = r.f(5)?;
    Ok(buffer_delay_length_minus_1)
}

/// `color_config( )`.
fn color_config(r: &mut BitReader<'_>, seq: &mut SequenceHeader) -> Result<(), Av1Error> {
    seq.high_bitdepth = r.flag()?;
    if seq.seq_profile == 2 && seq.high_bitdepth {
        seq.twelve_bit = r.flag()?;
        seq.bit_depth = if seq.twelve_bit { 12 } else { 10 };
    } else if seq.seq_profile <= 2 {
        seq.bit_depth = if seq.high_bitdepth { 10 } else { 8 };
    } else {
        return Err(unsupported("invalid seq_profile"));
    }
    if seq.seq_profile == 1 {
        seq.mono_chrome = false;
    } else {
        seq.mono_chrome = r.flag()?;
    }
    let mut color_description_present_flag = r.flag()?;
    if color_description_present_flag {
        seq.color_primaries = r.f(8)?;
        seq.transfer_characteristics = r.f(8)?;
        seq.matrix_coefficients = r.f(8)?;
    } else {
        color_description_present_flag = false;
    }
    if seq.mono_chrome {
        seq.color_range = r.flag()?;
        seq.subsampling_x = 1;
        seq.subsampling_y = 1;
        seq.chroma_sample_position = 0;
        seq.separate_uv_delta_q = false;
        return Ok(());
    } else if seq.color_primaries == CP_BT_709
        && seq.transfer_characteristics == TC_SRGB
        && seq.matrix_coefficients == MC_IDENTITY
    {
        seq.color_range = true;
        seq.subsampling_x = 0;
        seq.subsampling_y = 0;
    } else {
        seq.color_range = r.flag()?;
        if seq.seq_profile == 0 {
            seq.subsampling_x = 1;
            seq.subsampling_y = 1;
        } else if seq.seq_profile == 1 {
            seq.subsampling_x = 0;
            seq.subsampling_y = 0;
        } else if seq.bit_depth == 12 {
            seq.subsampling_x = r.f(1)? as u8;
            seq.subsampling_y = if seq.subsampling_x == 1 { r.f(1)? as u8 } else { 0 };
        } else {
            seq.subsampling_x = 1;
            seq.subsampling_y = 0;
        }
        if seq.subsampling_x == 1 && seq.subsampling_y == 1 {
            seq.chroma_sample_position = r.f(2)? as u8;
        }
    }
    seq.separate_uv_delta_q = r.flag()?;
    Ok(())
}

/// A parsed OBU with its payload.
pub struct Obu<'a> {
    pub kind: u8,
    pub has_size_field: bool,
    pub payload: &'a [u8],
}

/// Split a byte stream into OBUs. `obu_has_size_field` must be set for every
/// OBU except possibly the last one.
pub fn split_obus(mut data: &[u8]) -> Result<Vec<Obu<'_>>, Av1Error> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let mut r = BitReader::new(data);
        let _forbidden = r.f(1)?;
        if _forbidden != 0 {
            return Err(err("obu_forbidden_bit set"));
        }
        let kind = r.f(4)? as u8;
        let extension_flag = r.flag()?;
        let has_size_field = r.flag()?;
        let _reserved = r.flag()?;
        if extension_flag {
            let _ = r.f(8)?;
        }
        let header_bits = r.position();
        let size = if has_size_field {
            r.leb128()? as usize
        } else {
            data.len() - header_bits / 8
        };
        let start = r.position() / 8;
        let payload = data
            .get(start..start + size)
            .ok_or_else(|| err("obu_size beyond end of stream"))?;
        out.push(Obu {
            kind,
            has_size_field,
            payload,
        });
        data = &data[start + size..];
    }
    Ok(out)
}

/// Parse the OBUs of an `av1C` configuration record.
pub fn parse_config_obus(obus: &[u8]) -> Result<SequenceHeader, Av1Error> {
    for obu in split_obus(obus)? {
        if obu.kind == OBU_SEQUENCE_HEADER {
            let mut r = BitReader::new(obu.payload);
            return sequence_header_obu(&mut r);
        }
    }
    Err(err("no sequence header in av1C configuration"))
}

/// A decoded AV1 frame: planar samples at the sequence bit depth.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub chroma_format: ChromaFormat,
    pub color_range: bool,
    pub matrix_coefficients: u32,
    pub planes: Vec<Plane>,
}

/// One decoded plane. `data` is row major with `stride` samples per row.
pub struct Plane {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u16>,
}

impl Plane {
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width as usize;
        Self {
            width,
            height,
            stride,
            data: vec![0u16; stride * height as usize],
        }
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> u16 {
        self.data[y as usize * self.stride + x as usize]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, v: u16) {
        let stride = self.stride;
        self.data[y as usize * stride + x as usize] = v;
    }

    #[inline]
    pub fn row(&self, y: u32) -> &[u16] {
        &self.data[y as usize * self.stride..(y as usize + 1) * self.stride]
    }

    #[inline]
    pub fn row_mut(&mut self, y: u32) -> &mut [u16] {
        let stride = self.stride;
        let start = y as usize * stride;
        &mut self.data[start..start + stride]
    }
}

/// Sequence and frame headers of a coded AV1 still image.
pub struct Headers {
    pub sequence: SequenceHeader,
    /// `None` when the stream carries the frame inside an `OBU_FRAME`.
    pub frame: Option<FrameHeader>,
}

/// Parse only the sequence header of an OBU stream.
///
/// Used by the container overview, which must work even when the frame header
/// uses features this decoder does not reconstruct yet.
pub fn parse_sequence_header(obus: &[u8]) -> Result<SequenceHeader, Av1Error> {
    for obu in split_obus(obus)? {
        if obu.kind == OBU_SEQUENCE_HEADER {
            let mut r = BitReader::new(obu.payload);
            return sequence_header_obu(&mut r);
        }
    }
    Err(err("no sequence header in OBU stream"))
}

/// Parse the sequence and frame headers of an OBU stream.
pub fn parse_headers(obus: &[u8]) -> Result<Headers, Av1Error> {
    let parsed = split_obus(obus)?;
    let mut sequence: Option<SequenceHeader> = None;
    let mut frame: Option<FrameHeader> = None;
    for obu in &parsed {
        match obu.kind {
            OBU_SEQUENCE_HEADER => {
                let mut r = BitReader::new(obu.payload);
                sequence = Some(sequence_header_obu(&mut r)?);
            }
            OBU_FRAME | OBU_FRAME_HEADER | OBU_REDUNDANT_FRAME_HEADER => {
                let seq = sequence
                    .as_ref()
                    .ok_or_else(|| err("frame OBU before sequence header"))?;
                let mut r = BitReader::new(obu.payload);
                frame = Some(frame::frame_header_obu(&mut r, seq)?);
                if obu.kind == OBU_FRAME {
                    // An OBU_FRAME carries the frame header in its first bytes;
                    // the tile data follows in the same payload.
                    return finish_headers(sequence, frame);
                }
            }
            _ => {}
        }
    }
    finish_headers(sequence, frame)
}

fn finish_headers(
    sequence: Option<SequenceHeader>,
    frame: Option<FrameHeader>,
) -> Result<Headers, Av1Error> {
    let sequence = sequence.ok_or_else(|| err("no sequence header"))?;
    Ok(Headers { sequence, frame })
}

/// Tile layout of a coded still image: the headers plus every tile with its
/// byte range and mode info range.
pub struct TileLayout {
    pub sequence: SequenceHeader,
    pub frame: FrameHeader,
    pub tiles: Vec<TileInfo>,
}

impl TileLayout {
    /// Number of tiles in the frame.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Total entropy coded bytes of all tiles.
    pub fn coded_bytes(&self) -> usize {
        self.tiles.iter().map(|t| t.size).sum()
    }
}

/// Parse the tile layout of a coded still image without decoding pixels.
///
/// Handles `OBU_FRAME` (frame header plus one tile group) and separate
/// `OBU_TILE_GROUP` OBUs, which is what AVIF containers and elementary streams
/// use.
pub fn tile_layout(obus: &[u8]) -> Result<TileLayout, Av1Error> {
    let parsed = split_obus(obus)?;
    let mut sequence: Option<SequenceHeader> = None;
    let mut frame: Option<FrameHeader> = None;
    let mut tiles: Vec<TileInfo> = Vec::new();
    for obu in &parsed {
        match obu.kind {
            OBU_SEQUENCE_HEADER => {
                let mut r = BitReader::new(obu.payload);
                sequence = Some(sequence_header_obu(&mut r)?);
            }
            OBU_FRAME => {
                let seq = sequence
                    .as_ref()
                    .ok_or_else(|| err("frame OBU before sequence header"))?;
                let mut header = frame.take().unwrap_or_default();
                let (_header_bytes, group) = frame::frame_obu(obu.payload, &mut header, seq)?;
                tiles.extend(group);
                frame = Some(header);
            }
            OBU_TILE_GROUP => {
                let header = frame
                    .as_ref()
                    .ok_or_else(|| err("tile group before frame header"))?;
                tiles.extend(frame::tile_group_obu(obu.payload, 0, header)?);
            }
            _ => {}
        }
    }
    let sequence = sequence.ok_or_else(|| err("no sequence header"))?;
    let frame = frame.ok_or_else(|| err("no frame header"))?;
    if tiles.is_empty() {
        return Err(err("no tile data"));
    }
    Ok(TileLayout { sequence, frame, tiles })
}

/// Decode an AV1 still image from a stream of OBUs.
///
/// The OBUs are those of the `av1C` configuration record followed by the OBUs
/// of the image item payload, which is how AVIF stores a coded image.
///
/// Pixel reconstruction is not part of this decoder yet: the sequence header,
/// frame header, tile layout and loop filter parameters are parsed, and the
/// tile level entropy decoding of [`decode_tiles`] is what is still missing.
pub fn decode(obus: &[u8], _fallback_width: u32, _fallback_height: u32) -> Result<Frame, Av1Error> {
    let headers = parse_headers(obus)?;
    Err(unsupported(format!(
        "AV1 pixel reconstruction is not implemented ({}x{} frame parsed)",
        headers
            .frame
            .as_ref()
            .map(|f| f.frame_width)
            .unwrap_or(headers.sequence.max_width()),
        headers
            .frame
            .as_ref()
            .map(|f| f.frame_height)
            .unwrap_or(headers.sequence.max_height())
    )))
}

/// Decode tile groups: the tile level entropy decoder is not implemented yet.
///
/// The sequence header, frame header, tile layout and loop filter parameters
/// are parsed by [`parse_headers`]; pixel reconstruction starts here.
pub fn decode_tiles() -> Result<(), Av1Error> {
    Err(unsupported("AV1 tile decoding is not implemented"))
}

/// Convert a decoded frame to RGBA8 using the given colour description.
pub fn to_rgba8(frame: &Frame, colour: Option<crate::codecs::avif::ColourInfo>) -> Vec<u8> {
    yuv::frame_to_rgba(frame, colour)
}

/// Apply an alpha frame (decoded from the alpha auxiliary item) in place.
pub fn apply_alpha(rgba: &mut [u8], alpha: &Frame) {
    let (aw, ah) = (alpha.width, alpha.height);
    let (w, h) = (aw as usize, ah as usize);
    for y in 0..h {
        for x in 0..w {
            let a = alpha.planes[0].get(x as u32, y as u32) as u32;
            let scale = |v: u8| ((v as u32 * a + 127) / 255) as u8;
            let i = (y * w + x) * 4;
            if i + 3 >= rgba.len() {
                break;
            }
            rgba[i] = scale(rgba[i]);
            rgba[i + 1] = scale(rgba[i + 1]);
            rgba[i + 2] = scale(rgba[i + 2]);
            rgba[i + 3] = a as u8;
        }
    }
}