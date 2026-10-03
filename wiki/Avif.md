# AVIF

AVIF (AV1 in ISOBMFF) support without third-party code. The container parser
lives in [`avif.rs`](../src/codecs/avif.rs) and the AV1 bitstream parser in
[`av1/`](../src/codecs/av1). `ImageFormat::Avif` is routed through both in
[`io.rs`](../src/io.rs).

An AVIF file is an ISOBMFF file whose `meta` box describes one or more image
items. This page documents what CoreImage parses natively today and where the
AV1 decoder stands.

## Status

| Stage | State |
|---|---|
| Container (`ftyp`, `meta`, `iinf`, `iloc`, `iprp`, `iref`, `mdat`) | Parsed natively |
| `ispe`, `pixi`, `av1C`, `colr`, `auxC` properties | Parsed natively |
| AV1 OBU framing and `sequence_header_obu( )` | Parsed natively |
| AV1 `frame_header_obu( )` for intra still frames | Parsed natively |
| `frame_obu( )` and `tile_group_obu( )` headers, tile byte ranges | Parsed natively |
| Per tile CDF model (`init_symbol`, `init_coeff_cdfs( )`) | Implemented |
| Tile level entropy decoding (`decode_tile`, modes, residual) | Not implemented |
| Pixel reconstruction (prediction, transforms, filters) | Not implemented |
| Encoding | Not implemented |

`TiImage::from_bytes` on an AVIF file parses the container and the headers and
then returns a localized `Unsupported` error. `probe`, `dimensions` and
`probe_metadata` work for every AVIF file, including 10 bit, monochrome and
tiled ones.

## Formats

| Variant | Extension | Backend | Notes |
|---|---|---|---|
| `ImageFormat::Avif` | `avif`, `avis` | Pure Rust (`codecs::avif`) | Container and AV1 headers, no third-party code |

Detection uses the `ftyp` brand list (`avif` or `avis`). AVIF files start with
an ISOBMFF header, so they are never confused with HEIF or MP4.

## Functions

### `avif::probe`

```rust
pub fn probe(bytes: &[u8]) -> Result<AvifInfo, AvifError>
```

Container overview without decoding pixels. Returns `Err` when the file has no
`meta` box, no primary image item or no `av1C` property.

| Field | Type | Description |
|---|---|---|
| `width` | `u32` | Spatial extent from `ispe`, falling back to the AV1 sequence header |
| `height` | `u32` | Same |
| `depths` | `Vec<u8>` | Bit depth per component from `pixi`, empty when absent |
| `chroma_format` | `ChromaFormat` | `Cs420`, `Cs422`, `Cs444` or `Monochrome` |
| `alpha_item` | `Option<u32>` | Item id of the alpha auxiliary image |
| `depth_item` | `Option<u32>` | Item id of the depth auxiliary image |
| `colour` | `Option<ColourInfo>` | `colr` `nclx` primaries, transfer, matrix and range |

### `avif::dimensions`

```rust
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), AvifError>
```

Reads `ispe` when present, otherwise the AV1 sequence header. Never decodes
pixels.

### `avif::item_obus`

```rust
pub fn item_obus(bytes: &[u8]) -> Result<Vec<u8>, AvifError>
```

OBUs of the primary image item: the configuration OBUs of `av1C` followed by the
OBUs of the item payload. Encoders may leave the `av1C` OBU list empty and put
everything into `mdat`, so both parts are concatenated.

### `avif::aux_obus`

```rust
pub fn aux_obus(bytes: &[u8], item: u32) -> Result<Vec<u8>, AvifError>
```

Same for an auxiliary item such as the alpha or depth image, addressed by the
item id reported by [`avif::probe`](#avifprobe).

### `avif::decode`

```rust
pub fn decode(bytes: &[u8]) -> Result<DecodedAvif, AvifError>
```

Decodes the primary image item into RGBA8 pixels. Returns
`Unsupported("AV1 pixel reconstruction is not implemented (WxH frame parsed)")`
today: the container, the sequence header and the frame header are parsed, then
the missing tile decoder is reported.

### `avif::aux_payload`

```rust
pub fn aux_payload(bytes: &[u8], item: u32) -> Result<DecodedAvif, AvifError>
```

Alpha payload of the alpha auxiliary item as RGBA8 grey pixels. Same limitation
as [`avif::decode`](#avifdecode).

### `avif::tile_layout`

```rust
pub fn tile_layout(bytes: &[u8]) -> Result<av1::TileLayout, AvifError>
```

Tile layout of the primary image item without decoding pixels. Handles
`OBU_FRAME` (frame header plus one tile group) and separate `OBU_TILE_GROUP`
OBUs. Returns `Err` when the stream has no sequence header, no frame header or
no tile data.

| Field | Type | Description |
|---|---|---|
| `sequence` | `SequenceHeader` | Parsed sequence header |
| `frame` | `FrameHeader` | Parsed intra frame header |
| `tiles` | `Vec<TileInfo>` | Every tile with its byte range and mode info range |

`TileInfo` carries `num`, `row`, `col`, `size` (entropy coded bytes), `offset`
(inside the tile group OBU payload) and the mode info bounds `mi_row_start`,
`mi_row_end`, `mi_col_start`, `mi_col_end`, plus the `mi_width`, `mi_height`,
`row_start` and `col_start` helpers.

The tile ranges are validated against the byte alignment of the frame header:
the padding bits after an `OBU_FRAME` frame header must be zero, which catches
any drift in the header parse. That check is what pinned down the
`force_integer_mv` bit of `uncompressed_header( )`, which is present whenever
`allow_screen_content_tools` is set even though intra frames force it to 1.

## AV1 module

`coreimage::codecs::av1` implements the bitstream side. It is public so callers
can work with the parsed headers directly.

| Function | Description |
|---|---|
| `av1::parse_sequence_header(obus)` | Sequence header of an OBU stream, without touching the frame |
| `av1::parse_headers(obus)` | Sequence and frame headers of a coded still image |
| `av1::split_obus(obus)` | Splits a stream into OBUs with their payloads |
| `av1::tile_layout(obus)` | Headers plus the tile byte ranges of a coded still image |
| `av1::decode_tiles()` | Placeholder for the tile level entropy decoder |

`av1::parse_headers` returns a `Headers` struct with the `SequenceHeader` and an
optional `FrameHeader`. The frame header covers key frames and intra-only
frames; inter frames, `show_existing_frame`, switch frames and superres return
`Unsupported` with a specific message.

The frame header exposes what later stages need: tile grid
(`tile_cols`, `tile_rows`, `tile_size_bytes`, `mi_col_starts`, `mi_row_starts`),
quantization (`base_q_idx`, the five delta q values, `using_qmatrix`),
segmentation (`feature_enabled`, `feature_data`, `last_active_seg_id`),
deblocking (`loop_filter_level`, `loop_filter_ref_deltas`), CDEF
(`cdef_bits`, strengths, damping) and loop restoration
(`frame_restoration_type`, `loop_restoration_size`).

## CDF model

`src/codecs/av1/cdf.rs` owns the mutable copy of every default table that the
symbol decoder adapts, generated by
[`tools/gen_cdf_model.py`](../tools/gen_cdf_model.py) from `cdf_default.rs`.

```rust
pub struct Cdfs { /* one array per default table */ }
```

| Function | Description |
|---|---|
| `Cdfs::new()` | `init_symbol( sz )` plus `init_non_coeff_cdfs( )`: a copy of every default table |
| `Cdfs::init_coeff(base_q_idx)` | `init_coeff_cdfs( )`: the coefficient tables of the current frame |

Every table has an accessor named after the specification that returns the slice
of one context, for example `y_mode(group)`, `partition_w32(ctx)`,
`intra_frame_y_mode(above, left)` or `coeff_base(tx_class, tx_size, plane, ctx)`.
The accessors borrow one context at a time, which keeps the borrow checker out
of the way in the tile decoder.

Coefficient tables are indexed by `base_q_idx` in the specification
(`TxbSkipCdf[ idx ][ ... ]`). The model stores one slice: `Cdfs::new()` seeds it
with slice 0 and `init_coeff( base_q_idx )` replaces it with the slice of the
current frame:

| `base_q_idx` | Slice |
|---|---|
| 0-20 | 0 |
| 21-60 | 1 |
| 61-120 | 2 |
| 121-255 | 3 |

The symbol decoder (`av1/symbol.rs`) implements `init_symbol`, `read_symbol`
with CDF adaptation, `read_literal` (`L( n )`), `NS( n )` and `exit_symbol`,
including the conformance check that the trailing bit of a tile partition is 1.
That check is the first hard signal that a tile decode consumed exactly the
right number of symbols.

## Generated tables

`src/codecs/av1/spec_consts.rs` and `src/codecs/av1/cdf_default.rs` are
generated from the AV1 specification with
[`tools/gen_av1_spec_tables.py`](../tools/gen_av1_spec_tables.py):

| File | Content |
|---|---|
| `spec_consts.rs` | 184 numeric constants of the symbols table |
| `cdf_default.rs` | 96 default CDF tables with 19677 values (section 9.4) |

The AV1 Bitstream & Decoding Process Specification (AOMedia) is published under
CC0 1.0, so the reproduced tables carry no license obligation. Regenerate with:

```bash
python3 tools/gen_av1_spec_tables.py av1-spec.html src/codecs/av1
```

The specification has one known typo that the generator corrects: the declared
dimension of `Default_Restoration_Type_Cdf` is `RESTORE_SWITCHABLE + 1` while
the table holds `RESTORE_SWITCHABLE + 2` entries.

## Usage / Example

```rust
use coreimage::{ImageFormat, TiImage};

// Metadata works for every AVIF file.
let meta = TiImage::probe_metadata("photo.avif")?;
println!("{}x{} {}", meta.width, meta.height, meta.color_type);

// Pixel decode is reported as unsupported until the tile decoder lands.
match TiImage::load("photo.avif") {
    Ok(img) => println!("{}", img.width()),
    Err(e) => println!("{e}"),
}

// Headers are available for callers that need the AV1 details.
let bytes = std::fs::read("photo.avif")?;
let obus = coreimage::codecs::avif::item_obus(&bytes)?;
let headers = coreimage::codecs::av1::parse_headers(&obus)?;
println!("{} bit depth", headers.sequence.bit_depth);
```

## Fixtures

`tests/avif_codec.rs` runs against the files in `tests/data/`, generated by
`tests/gen_avif.py` with libavif 1.4 `avifenc` and the aom encoder:

| Fixture | Encoded as |
|---|---|
| `avif_420_q60` | 8 bit 4:2:0, quality 60 |
| `avif_444_q70` | 8 bit 4:4:4 |
| `avif_422_q70` | 8 bit 4:2:2 |
| `avif_mono_q70` | monochrome |
| `avif_lossless` | lossless (libavif forces 4:4:4) |
| `avif_10bit_420` | 10 bit 4:2:0 |
| `avif_tiles` | 4:2:0 with two tile columns |
| `avif_odd_37x23` | 37x23, 4:4:4 |
| `avif_screenc` | speed 10, screen content tools |
| `avif_alpha` | alpha auxiliary item |
| `avif_edge_q80` | hard edges, deblocking and CDEF |

## Adding pixel support

The remaining work is the tile level decoder, in this order:

1. `decode_tile`: partition tree, `mode_info( )`, transform tree, coefficients.
   Each tile starts at `TileInfo::offset` with `TileInfo::size` bytes, which
   `init_symbol( size )` consumes directly. The CDF model is ready.
3. Intra prediction: directional, smooth, palette, intrabc, filter intra, CFL
   and the intra edge filter.
4. Inverse transforms: DCT, ADST, identity, Hadamard and the flip variants.
5. Loop filters: deblocking, CDEF, loop restoration and film grain.
6. Colour conversion and chroma upsampling in `yuv.rs`.

Reference decoders for differential checks: `avifdec` (raw `y4m` planes),
`dav1d --muxer yuv` and `aomdec` from libavif and libaom. Comparing our planes
against the reference planes isolates entropy decoding bugs from filter bugs:
a wrong block mode garbles a whole block, a wrong filter shifts a few pixels.

## Cross References

- [LoadingSaving.md](LoadingSaving.md) - format table and metadata routing
- [Codecs.md](Codecs.md) - the other pure-Rust codecs and how codecs are added
- [Ffi.md](Ffi.md) - C ABI surface for loading and saving