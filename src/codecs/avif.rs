//! Pure-Rust AVIF (AV1 in ISOBMFF) container parsing, no third-party code.
//!
//! An AVIF file is an ISOBMFF file whose `meta` box describes one or more
//! image items. Only the primary, non-grid, non-hidden item is decoded: the
//! pixel data lives in `mdat` at the offsets given by the `iloc` box, and the
//! codec configuration lives in the `av1C` property, which carries the AV1
//! sequence header OBU (and the frame OBU when it is not in `mdat`).
//!
//! Auxiliary items (alpha, depth) are resolved so that callers can pick them
//! up with [`aux_payload`]; the alpha item in particular is a second AV1 image
//! with its own configuration.

use crate::codecs::av1;

/// Errors of the AVIF container parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvifError {
    Decode(String),
    Unsupported(String),
}

impl std::fmt::Display for AvifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "avif decode error: {m}"),
            Self::Unsupported(m) => write!(f, "avif unsupported: {m}"),
        }
    }
}

impl std::error::Error for AvifError {}

impl From<av1::Av1Error> for AvifError {
    fn from(e: av1::Av1Error) -> Self {
        match e {
            av1::Av1Error::Decode(m) => Self::Decode(m),
            av1::Av1Error::Unsupported(m) => Self::Unsupported(m),
        }
    }
}

/// Container-level image information.
#[derive(Debug, Clone)]
pub struct AvifInfo {
    pub width: u32,
    pub height: u32,
    /// Bit depth per component from the `pixi` property, empty when absent.
    pub depths: Vec<u8>,
    pub chroma_format: av1::ChromaFormat,
    /// Item id of the alpha auxiliary image, when present.
    pub alpha_item: Option<u32>,
    /// Item id of the depth auxiliary image, when present.
    pub depth_item: Option<u32>,
    /// `nclx` colour primaries, transfer function and matrix coefficients.
    pub colour: Option<ColourInfo>,
}

/// `colr` box payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColourInfo {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

fn unsupported(m: impl Into<String>) -> AvifError {
    AvifError::Unsupported(m.into())
}

fn decode_err(m: impl Into<String>) -> AvifError {
    AvifError::Decode(m.into())
}

/// True for ISOBMFF files whose brand list includes `avif` or `avis`.
pub fn is_avif(bytes: &[u8]) -> bool {
    let Some(ftyp) = find_box(bytes, 0, bytes.len(), b"ftyp") else {
        return false;
    };
    let body = &bytes[ftyp.0..ftyp.1];
    if body.len() < 8 {
        return false;
    }
    body[4..].chunks(4).any(|b| b == b"avif" || b == b"avis")
}

/// Locate a top level box by type. Returns `(start, end)` of the payload.
fn find_box(bytes: &[u8], _from: usize, end: usize, want: &[u8; 4]) -> Option<(usize, usize)> {
    let mut off = 0usize;
    while off + 8 <= end {
        let size = u32::from_be_bytes(bytes[off..off + 4].try_into().ok()?) as usize;
        let typ = &bytes[off + 4..off + 8];
        let (hdr, size) = if size == 1 {
            let big = u64::from_be_bytes(bytes[off + 8..off + 16].try_into().ok()?) as usize;
            (16usize, big)
        } else if size == 0 {
            (8usize, end - off)
        } else {
            (8usize, size)
        };
        if size < hdr || off + size > end {
            return None;
        }
        if typ == want {
            return Some((off + hdr, off + size));
        }
        off += size;
    }
    None
}

struct Boxes<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Iterator for Boxes<'a> {
    type Item = (usize, usize, [u8; 4]);

    fn next(&mut self) -> Option<Self::Item> {
        while self.pos + 8 <= self.end {
            let head = &self.data[self.pos..self.pos + 8];
            let size = u32::from_be_bytes(head[0..4].try_into().ok()?) as usize;
            let mut typ = [0u8; 4];
            typ.copy_from_slice(&head[4..8]);
            let hdr = if size == 1 {
                let big = u64::from_be_bytes(
                    self.data
                        .get(self.pos + 8..self.pos + 16)?
                        .try_into()
                        .ok()?,
                ) as usize;
                16usize.max(0).max(if big > 0 { 16 } else { 16 })
            } else if size == 0 {
                8
            } else {
                8
            };
            let size = if size == 1 {
                u64::from_be_bytes(
                    self.data
                        .get(self.pos + 8..self.pos + 16)?
                        .try_into()
                        .ok()?,
                ) as usize
            } else if size == 0 {
                self.end - self.pos
            } else {
                size
            };
            if size < hdr || self.pos + size > self.end {
                return None;
            }
            let start = self.pos + hdr;
            let stop = self.pos + size;
            self.pos += size;
            return Some((start, stop, typ));
        }
        None
    }
}

fn boxes(data: &[u8], start: usize, end: usize) -> Boxes<'_> {
    Boxes { data, pos: start, end }
}

/// Parsed item table of the `meta` box.
struct ItemTable {
    /// (item id, item type, name, content type, hidden, primary)
    items: Vec<ItemEntry>,
    /// item id to index in `items`
    index: std::collections::HashMap<u32, usize>,
    /// item id to `(extent_count, [(offset, length)])`
    locations: std::collections::HashMap<u32, Vec<(u64, u64)>>,
    /// item id to property indices (`ipma`)
    properties: std::collections::HashMap<u32, Vec<(bool, u8)>>,
    /// property index to (type, payload)
    props: Vec<([u8; 4], Vec<u8>)>,
    primary: Option<u32>,
    /// item id to the item it references through `dimg`/`auxl`
    references: std::collections::HashMap<u32, Vec<u32>>,
}

struct ItemEntry {
    id: u32,
    kind: String,
    hidden: bool,
    primary: bool,
}

impl ItemTable {
    /// Property payload of `item`, for example `ispe` or `av1C`.
    fn prop(&self, item: u32, want: &[u8; 4]) -> Option<Vec<u8>> {
        let list = self.properties.get(&item)?;
        for (_essential, index) in list {
            let i = *index as usize;
            if i == 0 || i > self.props.len() {
                continue;
            }
            if self.props[i - 1].0 == *want {
                return Some(self.props[i - 1].1.clone());
            }
        }
        None
    }

    /// Bytes of `item` inside `mdat`, located through the `iloc` table.
    fn payload<'a>(&self, bytes: &'a [u8], item: u32) -> Result<&'a [u8], AvifError> {
        let extents = self
            .locations
            .get(&item)
            .ok_or_else(|| decode_err(format!("item {item} has no iloc extent")))?;
        if extents.is_empty() {
            return Err(decode_err(format!("item {item} has an empty iloc extent list")));
        }
        let (offset, length) = extents[0];
        let start = offset as usize;
        let stop = start
            .checked_add(length as usize)
            .ok_or_else(|| decode_err("iloc extent overflows"))?;
        bytes
            .get(start..stop)
            .ok_or_else(|| decode_err(format!("iloc extent {start}..{stop} outside file")))
    }
}

/// Parse the fields of an `infe` box that matter here: item id and item type.
///
/// The rest of the box (protection index, name, content type, extensions) is
/// not needed to locate the coded image item.
fn parse_infe(data: &[u8]) -> Option<ItemEntry> {
    let version = data[0];
    let (id, mut pos) = match version {
        0 | 1 => (u16::from_be_bytes(data.get(4..6)?.try_into().ok()?) as u32, 6usize),
        2 => (u16::from_be_bytes(data.get(4..6)?.try_into().ok()?) as u32, 6usize),
        _ => (u32::from_be_bytes(data.get(4..8)?.try_into().ok()?), 8usize),
    };
    let mut kind = String::new();
    if version >= 2 {
        let protection = be16(data, pos).ok()?;
        pos += 2;
        if protection == 0 {
            if let Some(item_type) = data.get(pos..pos + 4) {
                kind = String::from_utf8_lossy(item_type).into_owned();
            }
        }
    }
    Some(ItemEntry { id, kind, hidden: false, primary: false })
}

fn parse_meta(bytes: &[u8], meta: (usize, usize)) -> Result<ItemTable, AvifError> {
    let (start, end) = meta;
    // meta is a FullBox: skip version and flags.
    let body = bytes
        .get(start + 4..end)
        .ok_or_else(|| decode_err("truncated meta box"))?;
    let mut table = ItemTable {
        items: Vec::new(),
        index: std::collections::HashMap::new(),
        locations: std::collections::HashMap::new(),
        properties: std::collections::HashMap::new(),
        props: Vec::new(),
        primary: None,
        references: std::collections::HashMap::new(),
    };
    for (b0, b1, typ) in boxes(body, 0, body.len()) {
        match &typ {
            b"pitm" => {
                let v = body[b0];
                let id = if v == 0 {
                    u16::from_be_bytes(body.get(b0 + 4..b0 + 6).ok_or_else(|| decode_err("bad pitm"))?
                        .try_into()
                        .map_err(|_| decode_err("bad pitm"))?) as u32
                } else {
                    u32::from_be_bytes(
                        body.get(b0 + 4..b0 + 8)
                            .ok_or_else(|| decode_err("bad pitm"))?
                            .try_into()
                            .map_err(|_| decode_err("bad pitm"))?,
                    )
                };
                table.primary = Some(id);
            }
            b"iinf" => {
                let v = body[b0];
                let count = if v == 0 {
                    u16::from_be_bytes(
                        body.get(b0 + 4..b0 + 6)
                            .ok_or_else(|| decode_err("bad iinf"))?
                            .try_into()
                            .map_err(|_| decode_err("bad iinf"))?,
                    ) as usize
                } else {
                    u32::from_be_bytes(
                        body.get(b0 + 4..b0 + 8)
                            .ok_or_else(|| decode_err("bad iinf"))?
                            .try_into()
                            .map_err(|_| decode_err("bad iinf"))?,
                    ) as usize
                };
                let mut seen = 0usize;
                for (c0, c1, ctyp) in boxes(body, b0 + if v == 0 { 6 } else { 8 }, b1) {
                    if &ctyp != b"infe" {
                        continue;
                    }
                    let payload = &body[c0..c1];
                    if payload.len() < 4 {
                        continue;
                    }
                    let version = payload[0];
                    // item_id / item_protection_index / item_type / item_name
                    let entry = parse_infe(payload);
                    // The hidden flag lives in the 12 bit item_flags field.
                    let hidden = if payload.len() >= 2 {
                        let flags = u16::from_be_bytes([payload[2], payload[3]]) & 0x0001;
                        flags == 1
                    } else {
                        false
                    };
                    let _ = version;
                    if let Some(mut e) = entry {
                        e.hidden = hidden;
                        if !e.kind.is_empty() {
                            table.index.insert(e.id, table.items.len());
                            table.items.push(e);
                        }
                    }
                    seen += 1;
                    if seen >= count {
                        break;
                    }
                }
            }
            b"iloc" => {
                parse_iloc(&body[b0..b1], &mut table)?;
            }
            b"iprp" => {
                for (p0, p1, ptyp) in boxes(body, b0, b1) {
                    if &ptyp == b"ipco" {
                        for (q0, q1, qtyp) in boxes(body, p0, p1) {
                            table.props.push((qtyp, body[q0..q1].to_vec()));
                        }
                    } else if &ptyp == b"ipma" {
                        parse_ipma(&body[p0..p1], &mut table)?;
                    }
                }
            }
            b"iref" => {
                parse_iref(&body[b0..b1], &mut table)?;
            }
            _ => {}
        }
    }
    for e in &mut table.items {
        e.primary = table.primary == Some(e.id);
    }
    if table.primary.is_none() {
        return Err(decode_err("meta box without pitm"));
    }
    if table.items.is_empty() {
        return Err(decode_err("meta box without image items"));
    }
    Ok(table)
}

fn parse_iloc(data: &[u8], table: &mut ItemTable) -> Result<(), AvifError> {
    let version = data[0];
    // Two size bytes follow the four byte FullBox header: the first holds
    // offset_size and length_size, the second base_offset_size and index_size.
    let sizes = data
        .get(4..6)
        .ok_or_else(|| decode_err("truncated iloc"))?;
    let offset_size = (sizes[0] >> 4) as usize;
    let length_size = (sizes[0] & 0x0f) as usize;
    let base_offset_size = (sizes[1] >> 4) as usize;
    let index_size = (sizes[1] & 0x0f) as usize;
    let mut pos = 6usize;
    let item_count = if version < 2 {
        let v = be16(data, pos)?;
        pos += 2;
        v as usize
    } else {
        let v = be32(data, pos)?;
        pos += 4;
        v as usize
    };
    for _ in 0..item_count {
        let id = if version < 2 {
            let v = be16(data, pos)?;
            pos += 2;
            v as u32
        } else {
            let v = be32(data, pos)?;
            pos += 4;
            v
        };
        if version == 1 || version == 2 {
            pos += 2; // construction_method
        }
        let data_ref = be16(data, pos)?;
        pos += 2;
        let base_offset = read_uint(data, pos, base_offset_size);
        pos += base_offset_size;
        let extent_count = be16(data, pos)?;
        pos += 2;
        let mut extents = Vec::with_capacity(extent_count as usize);
        for _ in 0..extent_count {
            if index_size > 0 {
                let _ = read_uint(data, pos, index_size);
                pos += index_size;
            }
            let offset = read_uint(data, pos, offset_size);
            pos += offset_size;
            let length = read_uint(data, pos, length_size);
            pos += length_size;
            extents.push((offset + base_offset, length));
        }
        if data_ref == 0 {
            table.locations.insert(id, extents);
        }
    }
    Ok(())
}


/// Big endian `u16` at `pos`.
fn be16(data: &[u8], pos: usize) -> Result<u16, AvifError> {
    let bytes = data
        .get(pos..pos + 2)
        .ok_or_else(|| decode_err("truncated box"))?;
    Ok(u16::from_be_bytes(
        bytes.try_into().map_err(|_| decode_err("truncated box"))?,
    ))
}

/// Big endian `u32` at `pos`.
fn be32(data: &[u8], pos: usize) -> Result<u32, AvifError> {
    let bytes = data
        .get(pos..pos + 4)
        .ok_or_else(|| decode_err("truncated box"))?;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| decode_err("truncated box"))?,
    ))
}

fn read_uint(data: &[u8], pos: usize, size: usize) -> u64 {
    if size == 0 {
        return 0;
    }
    let mut v = 0u64;
    for i in 0..size {
        let b = data.get(pos + i).copied().unwrap_or(0) as u64;
        v = (v << 8) | b;
    }
    v
}

fn parse_ipma(data: &[u8], table: &mut ItemTable) -> Result<(), AvifError> {
    let version = data[0];
    let flags = u32::from_be_bytes([0, data[1], data[2], data[3]]);
    let wide = flags & 1 == 1;
    let mut pos = 4usize;
    let entry_count = be32(data, pos)?;
    pos += 4;
    for _ in 0..entry_count {
        let id = if version < 1 {
            let v = be16(data, pos)?;
            pos += 2;
            v as u32
        } else {
            let v = be32(data, pos)?;
            pos += 4;
            v
        };
        let count = *data.get(pos).ok_or_else(|| decode_err("bad ipma count"))?;
        pos += 1;
        let mut list = Vec::with_capacity(count as usize);
        for _ in 0..count {
            if wide {
                let v = be16(data, pos)?;
                pos += 2;
                list.push((v & 0x8000 != 0, (v & 0x7fff) as u8));
            } else {
                let v = *data.get(pos).ok_or_else(|| decode_err("bad ipma"))?;
                pos += 1;
                list.push((v & 0x80 != 0, (v & 0x7f) as u8));
            }
        }
        table.properties.insert(id, list);
    }
    Ok(())
}

/// Parse an `iref` box into `from item id -> referenced item ids`.
///
/// Two layouts appear in the wild: the ISO one starts the child box with a
/// `reference_count`, while libavif omits it. The layout is chosen by the
/// length of the child payload.
fn parse_iref(data: &[u8], table: &mut ItemTable) -> Result<(), AvifError> {
    let version = data[0];
    let mut pos = 4usize;
    while pos + 8 <= data.len() {
        let size = be32(data, pos)? as usize;
        let mut typ = [0u8; 4];
        typ.copy_from_slice(&data[pos + 4..pos + 8]);
        if size < 8 || pos + size > data.len() {
            return Err(decode_err("bad iref child size"));
        }
        let stop = pos + size;
        let mut inner = pos + 8;
        let id_size = if version == 0 { 2 } else { 4 };
        // The ISO layout repeats the item id and its own count per reference.
        let iso_fits = match be16(data, inner) {
            Ok(outer) => {
                let mut probe = inner + 2;
                let mut fits = true;
                for _ in 0..outer {
                    if probe + id_size + 2 > stop {
                        fits = false;
                        break;
                    }
                    probe += id_size;
                    let count = match be16(data, probe) {
                        Ok(c) => c as usize,
                        Err(_) => {
                            fits = false;
                            break;
                        }
                    };
                    probe += 2 + count * id_size;
                }
                fits && probe <= stop
            }
            Err(_) => false,
        };
        if !iso_fits {
            inner = pos + 8;
        }
        while inner + id_size + 2 <= stop {
            let from = read_id(data, inner, id_size);
            inner += id_size;
            let count = be16(data, inner)? as usize;
            inner += 2;
            let mut targets = Vec::with_capacity(count);
            for _ in 0..count {
                if inner + id_size > stop {
                    return Err(decode_err("truncated iref reference"));
                }
                targets.push(read_id(data, inner, id_size));
                inner += id_size;
            }
            if typ == *b"dimg" || typ == *b"auxl" {
                table.references.insert(from, targets);
            }
        }
        pos = stop;
    }
    Ok(())
}

/// Read a 16 or 32 bit item id.
fn read_id(data: &[u8], pos: usize, size: usize) -> u32 {
    if size == 2 {
        be16(data, pos).unwrap_or(0) as u32
    } else {
        be32(data, pos).unwrap_or(0)
    }
}

/// True for coded AV1 image items. AVIF files use the item type `av01`.
fn is_image_item(entry: &ItemEntry) -> bool {
    entry.kind == "av01" || entry.kind == "avif"
}

/// Pick the item to decode: the primary visible still image.
fn select_item(table: &ItemTable) -> Result<u32, AvifError> {
    let primary = table.primary.unwrap_or(0);
    if let Some(i) = table.index.get(&primary) {
        let e = &table.items[*i];
        if is_image_item(e) && !e.hidden {
            return Ok(primary);
        }
    }
    for e in &table.items {
        if is_image_item(e) && !e.hidden {
            return Ok(e.id);
        }
    }
    Err(unsupported("no primary avif image item"))
}

/// AV1 configuration of an image item.
struct Av1Config {
    /// Configuration OBUs of `av1C` followed by the OBUs of the item payload.
    obus: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub chroma_format: av1::ChromaFormat,
}

/// Concatenated OBUs of an item: the `av1C` configuration OBUs followed by the
/// OBUs of the item payload.
fn concatenated_obus(table: &ItemTable, item: u32, payload: &[u8]) -> Result<Vec<u8>, AvifError> {
    let raw = table
        .prop(item, b"av1C")
        .ok_or_else(|| decode_err(format!("item {item} has no av1C property")))?;
    if raw.len() < 4 {
        return Err(decode_err("truncated av1C property"));
    }
    // The first four bytes are marker, version, seq profile/level and the size
    // of the configuration OBUs. Encoders may leave that list empty and put
    // every OBU into `mdat`, so the payload is always appended.
    let mut obus = raw[4..].to_vec();
    obus.extend_from_slice(payload);
    Ok(obus)
}

/// AV1 configuration of an item: its OBUs and the parsed sequence header.
fn av1_config(table: &ItemTable, item: u32, payload: &[u8]) -> Result<Av1Config, AvifError> {
    let obus = concatenated_obus(table, item, payload)?;
    let seq = av1::parse_sequence_header(&obus)?;
    Ok(Av1Config {
        obus,
        width: seq.max_width(),
        height: seq.max_height(),
        chroma_format: seq.chroma_format(),
    })
}

fn parse_ispe(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 12 {
        return None;
    }
    let w = u32::from_be_bytes(data[4..8].try_into().ok()?);
    let h = u32::from_be_bytes(data[8..12].try_into().ok()?);
    Some((w, h))
}

fn parse_pixi(data: &[u8]) -> Option<Vec<u8>> {
    let n = *data.get(4)? as usize;
    data.get(5..5 + n).map(|s| s.to_vec())
}

fn parse_colr(data: &[u8]) -> Option<ColourInfo> {
    if data.len() < 4 || &data[0..4] != b"nclx" {
        return None;
    }
    if data.len() < 11 {
        return None;
    }
    Some(ColourInfo {
        primaries: u16::from_be_bytes(data[4..6].try_into().ok()?),
        transfer: u16::from_be_bytes(data[6..8].try_into().ok()?),
        matrix: u16::from_be_bytes(data[8..10].try_into().ok()?),
        full_range: data[10] & 0x80 != 0,
    })
}

/// Auxiliary image type of an `auxC` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuxKind {
    Alpha,
    Depth,
    Other,
}

/// Auxiliary image type of an `auxC` property payload.
///
/// The box is a FullBox followed by a null terminated URN such as
/// `urn:mpeg:mpegB:cicp:systems:auxiliary:alpha`.
fn aux_kind(payload: &[u8]) -> Option<AuxKind> {
    let text = payload.get(4..)?;
    let end = text.iter().position(|b| *b == 0).unwrap_or(text.len());
    let name = std::str::from_utf8(&text[..end]).ok()?;
    let last = name.rsplit(':').next().unwrap_or(name);
    match last {
        "alpha" => Some(AuxKind::Alpha),
        "depth" => Some(AuxKind::Depth),
        _ => Some(AuxKind::Other),
    }
}

/// Container overview of an AVIF file: sizes, depth and auxiliary items.
pub fn probe(bytes: &[u8]) -> Result<AvifInfo, AvifError> {
    let meta = find_box(bytes, 0, bytes.len(), b"meta")
        .ok_or_else(|| decode_err("no meta box"))?;
    let table = parse_meta(bytes, meta)?;
    let item = select_item(&table)?;
    let payload = table.payload(bytes, item)?;
    let cfg = av1_config(&table, item, payload)?;
    let (width, height) = table
        .prop(item, b"ispe")
        .and_then(|d| parse_ispe(&d))
        .unwrap_or((cfg.width, cfg.height));
    let depths = table.prop(item, b"pixi").and_then(|d| parse_pixi(&d)).unwrap_or_default();
    let colour = table.prop(item, b"colr").and_then(|d| parse_colr(&d));
    let mut alpha_item = None;
    let mut depth_item = None;
    for (from, _targets) in &table.references {
        // The auxiliary type lives in the `auxC` property of the item that holds
        // the auxiliary image, which is the source of the `auxl` reference.
        if let Some(kind) = table.prop(*from, b"auxC").as_deref().and_then(aux_kind) {
            match kind {
                AuxKind::Alpha => alpha_item = Some(*from),
                AuxKind::Depth => depth_item = Some(*from),
                AuxKind::Other => {}
            }
        }
    }
    Ok(AvifInfo {
        width,
        height,
        depths,
        chroma_format: cfg.chroma_format,
        alpha_item,
        depth_item,
        colour,
    })
}

/// OBUs of the primary image item: the `av1C` configuration OBUs followed by
/// the OBUs of the item payload, which is how AVIF stores a coded image.
pub fn item_obus(bytes: &[u8]) -> Result<Vec<u8>, AvifError> {
    let meta = find_box(bytes, 0, bytes.len(), b"meta")
        .ok_or_else(|| decode_err("no meta box"))?;
    let table = parse_meta(bytes, meta)?;
    let item = select_item(&table)?;
    let payload = table.payload(bytes, item)?;
    concatenated_obus(&table, item, payload)
}

/// Tile layout of the primary image item.
pub fn tile_layout(bytes: &[u8]) -> Result<av1::TileLayout, AvifError> {
    let obus = item_obus(bytes)?;
    Ok(av1::tile_layout(&obus)?)
}

/// Decode the primary image item of an AVIF file into RGBA8 pixels.
pub fn decode(bytes: &[u8]) -> Result<DecodedAvif, AvifError> {
    let meta = find_box(bytes, 0, bytes.len(), b"meta")
        .ok_or_else(|| decode_err("no meta box"))?;
    let table = parse_meta(bytes, meta)?;
    let item = select_item(&table)?;
    let payload = table.payload(bytes, item)?;
    let cfg = av1_config(&table, item, payload)?;
    let obus = cfg.obus;
    let info = probe(bytes)?;
    let frame = av1::decode(&obus, cfg.width, cfg.height)?;
    let mut rgba = av1::to_rgba8(&frame, info.colour);
    if let Some(alpha_item) = info.alpha_item {
        if let Ok(apayload) = table.payload(bytes, alpha_item) {
            if let Ok(acfg) = av1_config(&table, alpha_item, apayload) {
                if let Ok(aframe) = av1::decode(&acfg.obus, acfg.width, acfg.height) {
                    av1::apply_alpha(&mut rgba, &aframe);
                }
            }
        }
    }
    Ok(DecodedAvif {
        width: info.width,
        height: info.height,
        pixels: rgba,
    })
}

/// Decoded AVIF image: always RGBA8 pixels.
#[derive(Debug, Clone)]
pub struct DecodedAvif {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// OBUs of an auxiliary image item such as alpha or depth.
pub fn aux_obus(bytes: &[u8], item: u32) -> Result<Vec<u8>, AvifError> {
    let meta = find_box(bytes, 0, bytes.len(), b"meta")
        .ok_or_else(|| decode_err("no meta box"))?;
    let table = parse_meta(bytes, meta)?;
    let payload = table.payload(bytes, item)?;
    concatenated_obus(&table, item, payload)
}

/// Alpha payload of the alpha auxiliary item, as RGBA8 grey pixels.
pub fn aux_payload(bytes: &[u8], item: u32) -> Result<DecodedAvif, AvifError> {
    let meta = find_box(bytes, 0, bytes.len(), b"meta")
        .ok_or_else(|| decode_err("no meta box"))?;
    let table = parse_meta(bytes, meta)?;
    let payload = table.payload(bytes, item)?;
    let cfg = av1_config(&table, item, payload)?;
    let frame = av1::decode(&cfg.obus, cfg.width, cfg.height)?;
    let mut rgba = av1::to_rgba8(&frame, None);
    for px in rgba.chunks_mut(4) {
        px[1] = px[0];
        px[2] = px[0];
    }
    Ok(DecodedAvif {
        width: cfg.width,
        height: cfg.height,
        pixels: rgba,
    })
}

/// Fast dimension probe: reads `ispe` or the AV1 sequence header only.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), AvifError> {
    let meta = find_box(bytes, 0, bytes.len(), b"meta")
        .ok_or_else(|| decode_err("no meta box"))?;
    let table = parse_meta(bytes, meta)?;
    let item = select_item(&table)?;
    if let Some((w, h)) = table.prop(item, b"ispe").and_then(|d| parse_ispe(&d)) {
        return Ok((w, h));
    }
    let payload = table.payload(bytes, item)?;
    let cfg = av1_config(&table, item, payload)?;
    Ok((cfg.width, cfg.height))
}