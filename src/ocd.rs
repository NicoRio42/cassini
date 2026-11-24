//! Reader and writer for OCAD 12 map files (`.ocd`).
//!
//! Cassini does not synthesize symbols: it copies the colours, symbols and
//! parameter strings of a template file and appends its own objects. Byte
//! layouts follow OpenOrienteering Mapper's OCD writer
//! (`src/fileformats/ocd_types_v12.h` and `ocd_file_export.cpp`,
//! GPL-3.0-or-later, Copyright Kai Pastor), which OCAD is known to accept.

use crate::error::{CassiniError, Result, ResultContext};
use log::warn;
use std::{
    borrow::Borrow,
    collections::HashMap,
    fs::File,
    io::{BufWriter, ErrorKind, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// OpenOrienteering Mapper's ISOM 2017-2 1:10 000 symbol set saved as OCD 12.
const DEFAULT_TEMPLATE: &[u8] = include_bytes!("../assets/ocad/isom-2017-2-10000.ocd");
const DEFAULT_TEMPLATE_NAME: &str = "<built-in ISOM 2017-2 template>";

const VENDOR_MARK: u16 = 0x0cad;
const VERSION: u16 = 12;

const HEADER_SIZE: usize = 60;
const HEADER_FIRST_SYMBOL_BLOCK: usize = 8;
const HEADER_FIRST_OBJECT_BLOCK: usize = 12;
const HEADER_FIRST_STRING_BLOCK: usize = 32;
const HEADER_FILE_NAME_POS: usize = 36;
const HEADER_FILE_NAME_SIZE: usize = 40;
const HEADER_FIRST_MULTI_REP_BLOCK: usize = 56;

const INDEX_BLOCK_ENTRIES: usize = 256;
const STRING_ENTRY_SIZE: usize = 16;
const SYMBOL_ENTRY_SIZE: usize = 4;
const OBJECT_ENTRY_SIZE: usize = 40;
const OBJECT_HEADER_SIZE: usize = 56;

const SYMBOL_TYPE: usize = 8;
const SYMBOL_NUM_COLORS: usize = 26;
const SYMBOL_COLORS: usize = 28;

const SYMBOL_TYPE_POINT: u8 = 1;
const SYMBOL_TYPE_LINE: u8 = 2;
const SYMBOL_TYPE_AREA: u8 = 3;

// Fields after the 796-byte base symbol
const AREA_BORDER_SYMBOL: usize = 796;
const AREA_FILL_COLOR: usize = 800;
const AREA_HATCH_MODE: usize = 802;
const AREA_HATCH_COLOR: usize = 804;
const AREA_FILL_ON: usize = 814;
const AREA_BORDER_ON: usize = 815;
const LINE_COLOR: usize = 796;
const LINE_WIDTH: usize = 798;
const AREA_SYMBOL_MIN_SIZE: usize = 832;
const LINE_SYMBOL_MIN_SIZE: usize = 872;

const GEOREFERENCING_STRING_TYPE: i32 = 1039;
/// OCAD grid "France" (7) and zone "Lambert-93" (9), combined as grid * 1000 + zone.
const LAMBERT_93_GRID_AND_ZONE: &str = "7009";

/// Coordinates are 24-bit signed values in 0.01 mm of paper.
const MAX_COORDINATE: i64 = 0x7f_ffff;
const OBJECT_STATUS_NORMAL: u8 = 1;
/// The object index entries hold file positions as signed 32-bit integers.
const MAX_FILE_SIZE: u64 = i32::MAX as u64;
/// The limit the OCD 12 format documents for the object index.
const MAX_OBJECTS: usize = 65_536 * INDEX_BLOCK_ENTRIES;

/// Margin added around point objects in the object index, in 0.01 mm of
/// paper. OCAD looks objects up by this bounding box, which should hold the
/// whole symbol, and a point symbol extends around its coordinate. 1 mm holds
/// the boulder symbols of the ISOM sets.
const POINT_BOUNDS_MARGIN: i32 = 100;

/// Marks the first point of a hole in an area object (set in the y coordinate).
pub const FLAG_HOLE: i32 = 0x02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Point = 1,
    Line = 2,
    Area = 3,
}

/// A point in OCD paper units (0.01 mm, Y up) with its y-coordinate flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcdPoint {
    pub x: i32,
    pub y: i32,
    pub y_flags: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcdObject {
    pub symbol: i32,
    pub kind: ObjectKind,
    pub points: Vec<OcdPoint>,
}

/// Maps projected coordinates to OCD paper coordinates around an integer
/// real-world offset.
#[derive(Debug, Clone, Copy)]
pub struct Georeferencing {
    pub offset_x: i64,
    pub offset_y: i64,
    pub scale: f64,
}

impl Georeferencing {
    /// Paper units (0.01 mm) per projected metre.
    pub fn units_per_meter(self) -> f64 {
        100_000.0 / self.scale
    }

    pub fn to_ocd(self, x: f64, y: f64) -> Result<(i32, i32)> {
        let units_per_meter = self.units_per_meter();
        let ocd_x = ((x - self.offset_x as f64) * units_per_meter).round();
        let ocd_y = ((y - self.offset_y as f64) * units_per_meter).round();

        if !(ocd_x.abs() <= MAX_COORDINATE as f64 && ocd_y.abs() <= MAX_COORDINATE as f64) {
            return Err(CassiniError::InvalidInput {
                message: format!(
                    "point ({x}, {y}) is too far from the OCAD map origin ({}, {})",
                    self.offset_x, self.offset_y
                ),
            });
        }

        Ok((ocd_x as i32, ocd_y as i32))
    }
}

struct ParameterString {
    string_type: i32,
    /// Raw bytes, including the terminating NUL.
    data: Vec<u8>,
}

struct TemplateSymbol {
    number: i32,
    /// The OCD symbol type: 1 for point symbols, 2 for lines, 3 for areas,
    /// and others.
    symbol_type: u8,
    data: Vec<u8>,
    dominant_color: i16,
    has_border: bool,
}

impl TemplateSymbol {
    fn parse(reader: &ByteReader, pos: usize) -> Result<Self> {
        let size = reader.u32(pos)? as usize;
        let data = reader.slice(pos, size)?.to_vec();
        let symbol = ByteReader {
            bytes: &data,
            source: reader.source,
        };

        let symbol_type = data.get(SYMBOL_TYPE).copied();
        let is_area = symbol_type == Some(SYMBOL_TYPE_AREA) && size >= AREA_SYMBOL_MIN_SIZE;
        let is_line = symbol_type == Some(SYMBOL_TYPE_LINE) && size >= LINE_SYMBOL_MIN_SIZE;

        // The main colour, as OpenOrienteering Mapper's export guesses it: the
        // fill or hatch of areas, the main line of lines, else the first
        // colour the symbol uses
        let main_color = if is_area && data[AREA_FILL_ON] != 0 {
            Some(symbol.u16(AREA_FILL_COLOR)?)
        } else if is_area && symbol.u16(AREA_HATCH_MODE)? != 0 {
            Some(symbol.u16(AREA_HATCH_COLOR)?)
        } else if is_line && symbol.u16(LINE_WIDTH)? > 0 {
            Some(symbol.u16(LINE_COLOR)?)
        } else if symbol.u16(SYMBOL_NUM_COLORS)? > 0 {
            Some(symbol.u16(SYMBOL_COLORS)?)
        } else {
            None
        };

        Ok(Self {
            number: symbol.i32(4)?,
            symbol_type: symbol_type.unwrap_or(0),
            dominant_color: main_color.map_or(-1, |color| color as i16),
            has_border: is_area
                && data[AREA_BORDER_ON] != 0
                && symbol.u32(AREA_BORDER_SYMBOL)? != 0,
            data,
        })
    }
}

/// The colours, symbols and parameter strings of an OCD 12 template file.
pub struct OcdTemplate {
    header: Vec<u8>,
    strings: Vec<ParameterString>,
    symbols: Vec<TemplateSymbol>,
    /// Index in `symbols` of each symbol number.
    symbol_indexes: HashMap<i32, usize>,
    georeferencing_string: String,
    scale: f64,
}

impl OcdTemplate {
    /// Loads `path`, or the built-in ISOM 2017-2 template when `path` is `None`.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) => {
                let bytes = std::fs::read(path)
                    .context(format!("could not read OCAD template `{}`", path.display()))?;
                Self::parse(&bytes, path)
            }
            None => Self::parse(DEFAULT_TEMPLATE, Path::new(DEFAULT_TEMPLATE_NAME)),
        }
    }

    fn parse(bytes: &[u8], source: &Path) -> Result<Self> {
        let reader = ByteReader { bytes, source };

        if reader.u16(0)? != VENDOR_MARK {
            return Err(reader.error("not an OCAD file"));
        }

        let version = reader.u16(4)?;
        if version != VERSION {
            return Err(reader.error(&format!(
                "OCAD format version {version} is not supported, save the template as OCAD 12"
            )));
        }

        let mut strings = vec![];
        for entry in reader.index_entries(HEADER_FIRST_STRING_BLOCK, STRING_ENTRY_SIZE)? {
            let pos = reader.u32(entry)? as usize;
            if pos == 0 {
                continue;
            }
            let object_index = reader.i32(entry + 12)?;
            // Strings of the template's objects, which are not copied
            if object_index != 0 {
                continue;
            }
            let len = reader.u32(entry + 4)? as usize;
            strings.push(ParameterString {
                string_type: reader.i32(entry + 8)?,
                data: reader.slice(pos, len)?.to_vec(),
            });
        }

        let mut symbols = vec![];
        let mut symbol_indexes = HashMap::new();
        for entry in reader.index_entries(HEADER_FIRST_SYMBOL_BLOCK, SYMBOL_ENTRY_SIZE)? {
            let pos = reader.u32(entry)? as usize;
            if pos == 0 {
                continue;
            }
            let symbol = TemplateSymbol::parse(&reader, pos)?;
            symbol_indexes.entry(symbol.number).or_insert(symbols.len());
            symbols.push(symbol);
        }

        let mut object_count = 0;
        for entry in reader.index_entries(HEADER_FIRST_OBJECT_BLOCK, OBJECT_ENTRY_SIZE)? {
            if reader.u32(entry + 16)? != 0 {
                object_count += 1;
            }
        }
        if object_count > 0 {
            warn!(
                "OCAD template `{}` contains {object_count} objects, they will not be copied",
                source.display()
            );
        }

        let georeferencing_string = strings
            .iter()
            .find(|string| string.string_type == GEOREFERENCING_STRING_TYPE)
            .map(|string| decode_parameter_string(&string.data))
            .ok_or_else(|| reader.error("missing georeferencing parameter string (1039)"))?;

        let scale = get_parameter_field(&georeferencing_string, 'm')
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|scale| *scale > 0.0)
            .ok_or_else(|| reader.error("georeferencing parameter string has no valid scale"))?;

        Ok(Self {
            header: reader.slice(0, HEADER_SIZE)?.to_vec(),
            strings,
            symbols,
            symbol_indexes,
            georeferencing_string,
            scale,
        })
    }

    fn symbol(&self, number: i32) -> Option<&TemplateSymbol> {
        self.symbol_indexes
            .get(&number)
            .map(|index| &self.symbols[*index])
    }

    /// Whether `number` is an area symbol drawn with a border line.
    pub fn has_border(&self, number: i32) -> bool {
        self.symbol(number).is_some_and(|symbol| symbol.has_border)
    }

    pub fn georeferencing(&self, offset_x: i64, offset_y: i64) -> Georeferencing {
        Georeferencing {
            offset_x,
            offset_y,
            scale: self.scale,
        }
    }

    /// Fails with the list of `symbols` that are not defined in the
    /// template, or that cannot draw objects of their kind.
    pub fn check_symbols(&self, symbols: &[(i32, ObjectKind)]) -> Result<()> {
        let type_name = |symbol_type: u8| match symbol_type {
            SYMBOL_TYPE_POINT => "point",
            SYMBOL_TYPE_LINE => "line",
            SYMBOL_TYPE_AREA => "area",
            _ => "other",
        };
        let mut missing: Vec<String> = vec![];
        let mut mistyped: Vec<String> = vec![];
        for (number, kind) in symbols {
            let expected_type = match kind {
                ObjectKind::Point => SYMBOL_TYPE_POINT,
                ObjectKind::Line => SYMBOL_TYPE_LINE,
                ObjectKind::Area => SYMBOL_TYPE_AREA,
            };
            match self.symbol(*number) {
                None => missing.push(format_symbol_number(*number)),
                Some(symbol) if symbol.symbol_type != expected_type => mistyped.push(format!(
                    "{} is a {} symbol instead of a {} symbol",
                    format_symbol_number(*number),
                    type_name(symbol.symbol_type),
                    type_name(expected_type)
                )),
                Some(_) => {}
            }
        }

        let mut problems = vec![];
        if !missing.is_empty() {
            problems.push(format!("is missing symbols {}", missing.join(", ")));
        }
        problems.extend(mistyped);
        if problems.is_empty() {
            Ok(())
        } else {
            Err(CassiniError::InvalidInput {
                message: format!("the OCAD template {}", problems.join(", and ")),
            })
        }
    }

    /// Serializes the template's strings and symbols followed by `objects`.
    #[cfg(test)]
    pub fn write(&self, georeferencing: &Georeferencing, objects: &[OcdObject]) -> Result<Vec<u8>> {
        let mut out = std::io::Cursor::new(vec![]);
        self.write_to(
            &mut out,
            Path::new("<memory>"),
            georeferencing,
            objects.iter().map(Ok),
        )?;
        Ok(out.into_inner())
    }

    /// Writes the template's strings and symbols followed by `objects` to a
    /// new file at `path`.
    pub fn write_file<O: Borrow<OcdObject>>(
        &self,
        path: &Path,
        georeferencing: &Georeferencing,
        objects: impl IntoIterator<Item = Result<O>>,
    ) -> Result<()> {
        let file = File::create(path).context(format!("could not create `{}`", path.display()))?;
        let mut out = BufWriter::new(file);
        self.write_to(&mut out, path, georeferencing, objects)?;
        out.flush()
            .context(format!("could not write `{}`", path.display()))
    }

    /// Streams the template's strings and symbols followed by `objects` to
    /// `out`, so that only one object and one index block are held at a time.
    /// `path` names `out` in errors.
    fn write_to<O: Borrow<OcdObject>>(
        &self,
        out: &mut (impl Write + Seek),
        path: &Path,
        georeferencing: &Georeferencing,
        objects: impl IntoIterator<Item = Result<O>>,
    ) -> Result<()> {
        let mut prefix = self.header.clone();
        for field in [
            HEADER_FILE_NAME_POS,
            HEADER_FILE_NAME_SIZE,
            HEADER_FIRST_MULTI_REP_BLOCK,
        ] {
            put_u32(&mut prefix, field, 0);
        }

        let string_index = append_index_blocks(&mut prefix, STRING_ENTRY_SIZE, self.strings.len());
        put_u32(
            &mut prefix,
            HEADER_FIRST_STRING_BLOCK,
            string_index.first_block,
        );

        for (string, entry) in self.strings.iter().zip(string_index.entries) {
            let data = if string.string_type == GEOREFERENCING_STRING_TYPE {
                encode_parameter_string(&update_georeferencing_string(
                    &self.georeferencing_string,
                    georeferencing,
                ))
            } else {
                string.data.clone()
            };

            let pos = append_block(&mut prefix, &data);
            put_u32(&mut prefix, entry, pos);
            put_u32(&mut prefix, entry + 4, data.len() as u32);
            put_i32(&mut prefix, entry + 8, string.string_type);
        }

        let symbol_index = append_index_blocks(&mut prefix, SYMBOL_ENTRY_SIZE, self.symbols.len());
        put_u32(
            &mut prefix,
            HEADER_FIRST_SYMBOL_BLOCK,
            symbol_index.first_block,
        );

        for (symbol, entry) in self.symbols.iter().zip(symbol_index.entries) {
            let pos = append_block(&mut prefix, &symbol.data);
            put_u32(&mut prefix, entry, pos);
        }

        // The object index blocks are interleaved with the objects: each one
        // is appended when the previous one is full, and written once the
        // objects it indexes are
        let first_object_block = prefix.len().next_multiple_of(8) as u32;
        put_u32(&mut prefix, HEADER_FIRST_OBJECT_BLOCK, first_object_block);
        let mut writer = BlockWriter { out, path, len: 0 };
        writer.append(&prefix)?;

        let empty_block = vec![0u8; 4 + INDEX_BLOCK_ENTRIES * OBJECT_ENTRY_SIZE];
        let mut block_pos = writer.append(&empty_block)?;
        let mut block = empty_block.clone();

        for (count, object) in objects.into_iter().enumerate() {
            let object = object?;
            let object = object.borrow();

            if count == MAX_OBJECTS {
                return Err(CassiniError::InvalidInput {
                    message: format!("an OCAD file holds at most {MAX_OBJECTS} objects"),
                });
            }
            if count > 0 && count % INDEX_BLOCK_ENTRIES == 0 {
                let next_block_pos = writer.append(&empty_block)?;
                put_u32(&mut block, 0, next_block_pos);
                writer.write_at(block_pos, &block)?;
                block_pos = next_block_pos;
                block.copy_from_slice(&empty_block);
            }

            let symbol = self
                .symbol(object.symbol)
                .ok_or_else(|| CassiniError::InvalidInput {
                    message: format!(
                        "the OCAD template is missing symbol {}",
                        format_symbol_number(object.symbol)
                    ),
                })?;

            let data = encode_object(object);
            let pos = writer.append(&data)?;

            let margin = match object.kind {
                ObjectKind::Point => POINT_BOUNDS_MARGIN,
                ObjectKind::Line | ObjectKind::Area => 0,
            };
            let min_x = object.points.iter().map(|point| point.x).min().unwrap_or(0) - margin;
            let min_y = object.points.iter().map(|point| point.y).min().unwrap_or(0) - margin;
            let max_x = object.points.iter().map(|point| point.x).max().unwrap_or(0) + margin;
            let max_y = object.points.iter().map(|point| point.y).max().unwrap_or(0) + margin;

            let entry = 4 + (count % INDEX_BLOCK_ENTRIES) * OBJECT_ENTRY_SIZE;
            put_i32(&mut block, entry, min_x << 8);
            put_i32(&mut block, entry + 4, min_y << 8);
            put_i32(&mut block, entry + 8, max_x << 8);
            put_i32(&mut block, entry + 12, max_y << 8);
            put_u32(&mut block, entry + 16, pos);
            // The size is in bytes. The OCAD documentation's `32 + 8 * n`
            // produces files that OCAD reports as damaged.
            put_u32(&mut block, entry + 20, data.len() as u32);
            put_i32(&mut block, entry + 24, object.symbol);
            block[entry + 28] = object.kind as u8;
            block[entry + 30] = OBJECT_STATUS_NORMAL;
            put_i16(&mut block, entry + 32, symbol.dominant_color);
        }

        writer.write_at(block_pos, &block)
    }
}

/// Appends 8-byte aligned blocks to an OCD file being written.
struct BlockWriter<'a, W> {
    out: &'a mut W,
    path: &'a Path,
    len: u64,
}

impl<W: Write + Seek> BlockWriter<'_, W> {
    /// Appends `data` at the next 8-byte boundary and returns its offset.
    fn append(&mut self, data: &[u8]) -> Result<u32> {
        let pos = self.len.next_multiple_of(8);
        let end = pos + data.len() as u64;
        if end > MAX_FILE_SIZE {
            return Err(CassiniError::InvalidInput {
                message: format!(
                    "`{}` would be larger than the {MAX_FILE_SIZE} bytes an OCAD file may hold",
                    self.path.display()
                ),
            });
        }

        let padding = [0u8; 8];
        self.out
            .write_all(&padding[..(pos - self.len) as usize])
            .and_then(|_| self.out.write_all(data))
            .context(format!("could not write `{}`", self.path.display()))?;
        self.len = end;
        Ok(pos as u32)
    }

    /// Overwrites the bytes at `pos`, written earlier.
    fn write_at(&mut self, pos: u32, data: &[u8]) -> Result<()> {
        self.out
            .seek(SeekFrom::Start(pos as u64))
            .and_then(|_| self.out.write_all(data))
            .and_then(|_| self.out.seek(SeekFrom::Start(self.len)))
            .map(|_| ())
            .context(format!("could not write `{}`", self.path.display()))
    }
}

/// Writes `object` as an OCD object record, which `read_object_record` reads
/// back.
pub fn write_object_record(out: &mut impl Write, object: &OcdObject) -> std::io::Result<()> {
    out.write_all(&encode_object(object))
}

/// Reads an object record written by `write_object_record`, or `None` at the
/// end of `input`.
pub fn read_object_record(input: &mut impl Read) -> std::io::Result<Option<OcdObject>> {
    // The end of `input` is only expected between records
    let mut header = [0u8; OBJECT_HEADER_SIZE];
    let mut filled = 0;
    while filled < header.len() {
        match input.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }

    let read_i32 =
        |bytes: &[u8], pos: usize| i32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
    let kind = match header[4] {
        1 => ObjectKind::Point,
        2 => ObjectKind::Line,
        3 => ObjectKind::Area,
        other => {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!("unexpected object type {other}"),
            ))
        }
    };
    let num_items = read_i32(&header, 44) as u32 as usize;

    let mut coordinates = vec![0u8; 8 * num_items];
    input.read_exact(&mut coordinates)?;
    let points = coordinates
        .as_chunks::<8>()
        .0
        .iter()
        .map(|coordinate| {
            let y = read_i32(coordinate, 4);
            OcdPoint {
                x: read_i32(coordinate, 0) >> 8,
                y: y >> 8,
                y_flags: y & 0xff,
            }
        })
        .collect();

    Ok(Some(OcdObject {
        symbol: read_i32(&header, 0),
        kind,
        points,
    }))
}

fn encode_object(object: &OcdObject) -> Vec<u8> {
    let mut data = vec![0u8; OBJECT_HEADER_SIZE + 8 * object.points.len()];
    put_i32(&mut data, 0, object.symbol);
    data[4] = object.kind as u8;
    put_u32(&mut data, 44, object.points.len() as u32);

    for (index, point) in object.points.iter().enumerate() {
        let offset = OBJECT_HEADER_SIZE + 8 * index;
        put_i32(&mut data, offset, point.x << 8);
        put_i32(&mut data, offset + 4, (point.y << 8) | point.y_flags);
    }

    data
}

struct IndexLayout {
    first_block: u32,
    entries: Vec<usize>,
}

/// Appends zeroed, chained index blocks holding at least one block and
/// `count` entries, and returns the offset of each entry.
fn append_index_blocks(out: &mut Vec<u8>, entry_size: usize, count: usize) -> IndexLayout {
    let block_count = count.div_ceil(INDEX_BLOCK_ENTRIES).max(1);
    let block_size = 4 + INDEX_BLOCK_ENTRIES * entry_size;
    let mut blocks: Vec<usize> = vec![];

    for _ in 0..block_count {
        let pos = append_block(out, &vec![0u8; block_size]) as usize;
        if let Some(previous) = blocks.last() {
            put_u32(out, *previous, pos as u32);
        }
        blocks.push(pos);
    }

    IndexLayout {
        first_block: blocks[0] as u32,
        entries: (0..count)
            .map(|index| {
                blocks[index / INDEX_BLOCK_ENTRIES] + 4 + (index % INDEX_BLOCK_ENTRIES) * entry_size
            })
            .collect(),
    }
}

/// Appends `data` at the next 8-byte boundary and returns its offset.
fn append_block(out: &mut Vec<u8>, data: &[u8]) -> u32 {
    out.resize(out.len().next_multiple_of(8), 0);
    let pos = out.len() as u32;
    out.extend_from_slice(data);
    pos
}

fn put_u32(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(out: &mut [u8], offset: usize, value: i32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i16(out: &mut [u8], offset: usize, value: i16) {
    out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub fn format_symbol_number(number: i32) -> String {
    format!("{}.{:03}", number / 1000, number % 1000)
}

/// Parameter strings are UTF-8 and NUL-terminated in OCD 12.
fn decode_parameter_string(data: &[u8]) -> String {
    let end = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

fn encode_parameter_string(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    data
}

/// Parameter strings are a first value followed by tab-separated fields,
/// each one a single-letter code and its value.
fn get_parameter_field(text: &str, code: char) -> Option<&str> {
    text.split('\t')
        .skip(1)
        .find_map(|field| field.strip_prefix(code))
}

fn set_parameter_field(fields: &mut Vec<String>, code: char, value: &str) {
    let field = format!("{code}{value}");
    match fields
        .iter_mut()
        .skip(1)
        .find(|existing| existing.starts_with(code))
    {
        Some(existing) => *existing = field,
        None => fields.push(field),
    }
}

/// Keeps the template's scale and grid settings, and sets the real-world
/// offset and Cassini's coordinate system (Lambert-93, grid north up).
fn update_georeferencing_string(template: &str, georeferencing: &Georeferencing) -> String {
    let mut fields: Vec<String> = template.split('\t').map(String::from).collect();
    set_parameter_field(&mut fields, 'r', "1");
    set_parameter_field(&mut fields, 'x', &georeferencing.offset_x.to_string());
    set_parameter_field(&mut fields, 'y', &georeferencing.offset_y.to_string());
    set_parameter_field(&mut fields, 'a', "0.00000000");
    set_parameter_field(&mut fields, 'i', LAMBERT_93_GRID_AND_ZONE);
    fields.join("\t")
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    source: &'a Path,
}

impl ByteReader<'_> {
    fn error(&self, message: &str) -> CassiniError {
        CassiniError::InvalidArtifact {
            path: PathBuf::from(self.source),
            message: message.to_owned(),
        }
    }

    fn slice(&self, pos: usize, len: usize) -> Result<&[u8]> {
        pos.checked_add(len)
            .and_then(|end| self.bytes.get(pos..end))
            .ok_or_else(|| self.error(&format!("file is truncated at offset {pos}")))
    }

    fn u16(&self, pos: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.slice(pos, 2)?.try_into().unwrap()))
    }

    fn u32(&self, pos: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.slice(pos, 4)?.try_into().unwrap()))
    }

    fn i32(&self, pos: usize) -> Result<i32> {
        Ok(i32::from_le_bytes(self.slice(pos, 4)?.try_into().unwrap()))
    }

    /// Returns the offsets of every entry, empty ones included, of the index
    /// chain whose first block is stored in the header at `header_field`.
    fn index_entries(&self, header_field: usize, entry_size: usize) -> Result<Vec<usize>> {
        let block_size = 4 + INDEX_BLOCK_ENTRIES * entry_size;
        let max_blocks = self.bytes.len() / block_size;
        let mut entries = vec![];
        let mut block = self.u32(header_field)? as usize;
        let mut block_count = 0;

        while block != 0 {
            block_count += 1;
            if block_count > max_blocks {
                return Err(self.error("index blocks form a cycle"));
            }

            self.slice(block, block_size)?;
            entries.extend((0..INDEX_BLOCK_ENTRIES).map(|index| block + 4 + index * entry_size));
            block = self.u32(block)? as usize;
        }

        Ok(entries)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct WrittenObject {
        pub(crate) object: OcdObject,
        bounds: [i32; 4],
        size: usize,
        status: u8,
        color: i16,
    }

    pub(crate) fn read_objects(bytes: &[u8]) -> Vec<WrittenObject> {
        let reader = ByteReader {
            bytes,
            source: Path::new("test.ocd"),
        };
        let mut objects = vec![];

        for entry in reader
            .index_entries(HEADER_FIRST_OBJECT_BLOCK, OBJECT_ENTRY_SIZE)
            .unwrap()
        {
            let pos = reader.u32(entry + 16).unwrap() as usize;
            if pos == 0 {
                continue;
            }

            let num_items = reader.u32(pos + 44).unwrap() as usize;
            let points = (0..num_items)
                .map(|index| {
                    let x = reader.i32(pos + OBJECT_HEADER_SIZE + 8 * index).unwrap();
                    let y = reader
                        .i32(pos + OBJECT_HEADER_SIZE + 8 * index + 4)
                        .unwrap();
                    OcdPoint {
                        x: x >> 8,
                        y: y >> 8,
                        y_flags: y & 0xff,
                    }
                })
                .collect();

            objects.push(WrittenObject {
                object: OcdObject {
                    symbol: reader.i32(pos).unwrap(),
                    kind: match bytes[pos + 4] {
                        1 => ObjectKind::Point,
                        2 => ObjectKind::Line,
                        3 => ObjectKind::Area,
                        other => panic!("unexpected object type {other}"),
                    },
                    points,
                },
                bounds: [0, 4, 8, 12].map(|offset| reader.i32(entry + offset).unwrap() >> 8),
                size: reader.u32(entry + 20).unwrap() as usize,
                status: bytes[entry + 30],
                color: i16::from_le_bytes([bytes[entry + 32], bytes[entry + 33]]),
            });
        }

        objects
    }

    fn point(x: i32, y: i32) -> OcdPoint {
        OcdPoint { x, y, y_flags: 0 }
    }

    #[test]
    fn parses_built_in_template() {
        let template = OcdTemplate::load(None).unwrap();

        assert_eq!(template.scale, 10_000.0);
        assert_eq!(template.symbols.len(), 170);
        template
            .check_symbols(&[
                (101000, ObjectKind::Line),
                (205000, ObjectKind::Point),
                (403000, ObjectKind::Area),
            ])
            .unwrap();
    }

    #[test]
    fn reads_main_colors_and_borders() {
        let template = OcdTemplate::load(None).unwrap();
        let color = |number| template.symbol(number).unwrap().dominant_color;

        // The fill of an area with a black bank line border, not the border
        assert_eq!(color(301000), color(301001));
        assert_ne!(color(301000), color(301004));
        // The hatch of an area without fill
        assert_eq!(color(308000), color(301001));
        // The main line of a line with side lines
        assert_eq!(color(509000), color(505000));

        assert!(template.has_border(301000));
        assert!(template.has_border(521002));
        assert!(!template.has_border(301001));
        assert!(!template.has_border(301004));
    }

    #[test]
    fn reports_missing_symbols() {
        let template = OcdTemplate::load(None).unwrap();

        let error = template
            .check_symbols(&[
                (101000, ObjectKind::Line),
                (123456, ObjectKind::Line),
                (999999, ObjectKind::Area),
            ])
            .unwrap_err();

        assert!(error.to_string().contains("123.456, 999.999"), "{error}");
    }

    #[test]
    fn reports_symbols_of_the_wrong_type() {
        let template = OcdTemplate::load(None).unwrap();

        let error = template
            .check_symbols(&[(205000, ObjectKind::Line), (202000, ObjectKind::Line)])
            .unwrap_err();

        assert!(
            error
                .to_string()
                .ends_with("the OCAD template 205.000 is a point symbol instead of a line symbol"),
            "{error}"
        );
    }

    #[test]
    fn rejects_other_versions() {
        let mut bytes = DEFAULT_TEMPLATE.to_vec();
        bytes[4] = 11;

        assert!(OcdTemplate::parse(&bytes, Path::new("v11.ocd")).is_err());
    }

    #[test]
    fn converts_projected_coordinates_to_paper_units() {
        let georeferencing = Georeferencing {
            offset_x: 907_500,
            offset_y: 6_454_500,
            scale: 10_000.0,
        };

        assert_eq!(
            georeferencing.to_ocd(907_000.0, 6_455_000.0).unwrap(),
            (-5_000, 5_000)
        );
        assert_eq!(
            georeferencing.to_ocd(907_500.04, 6_454_500.06).unwrap(),
            (0, 1)
        );
        assert!(georeferencing.to_ocd(2_000_000.0, 6_454_500.0).is_err());
        assert!(georeferencing.to_ocd(f64::NAN, 6_454_500.0).is_err());
    }

    #[test]
    fn rewrites_georeferencing_string() {
        let georeferencing = Georeferencing {
            offset_x: 907_500,
            offset_y: 6_454_500,
            scale: 10_000.0,
        };

        assert_eq!(
            update_georeferencing_string(
                "\tm10000\tg50.0000\tr0\tx0\ty0\ta1.50000000\td500.000000\ti1000\tb0.00\tc0.00",
                &georeferencing
            ),
            "\tm10000\tg50.0000\tr1\tx907500\ty6454500\ta0.00000000\td500.000000\ti7009\tb0.00\tc0.00"
        );
    }

    #[test]
    fn round_trips_template_and_objects() {
        let template = OcdTemplate::load(None).unwrap();
        let georeferencing = template.georeferencing(907_500, 6_454_500);
        let line = OcdObject {
            symbol: 101000,
            kind: ObjectKind::Line,
            points: vec![point(-100, -200), point(300, 400), point(-5_000, 5_000)],
        };
        let area = OcdObject {
            symbol: 403000,
            kind: ObjectKind::Area,
            points: vec![
                point(0, 0),
                point(1_000, 0),
                point(1_000, 1_000),
                point(0, 0),
                OcdPoint {
                    x: 100,
                    y: 100,
                    y_flags: FLAG_HOLE,
                },
                point(200, 100),
                point(200, 200),
                point(100, 100),
            ],
        };

        let boulder = OcdObject {
            symbol: 205000,
            kind: ObjectKind::Point,
            points: vec![point(500, -700)],
        };

        let bytes = template
            .write(
                &georeferencing,
                &[line.clone(), area.clone(), boulder.clone()],
            )
            .unwrap();
        let written = OcdTemplate::parse(&bytes, Path::new("test.ocd")).unwrap();

        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), VERSION);
        assert_eq!(written.scale, template.scale);
        assert_eq!(written.symbols.len(), template.symbols.len());
        for (written_symbol, template_symbol) in written.symbols.iter().zip(&template.symbols) {
            assert!(written_symbol.data == template_symbol.data);
        }
        assert_eq!(written.strings.len(), template.strings.len());
        assert_eq!(
            get_parameter_field(&written.georeferencing_string, 'x'),
            Some("907500")
        );
        assert_eq!(
            get_parameter_field(&written.georeferencing_string, 'i'),
            Some("7009")
        );

        let objects = read_objects(&bytes);
        assert_eq!(objects.len(), 3);
        assert_eq!(objects[0].object, line);
        assert_eq!(objects[0].bounds, [-5_000, -200, 300, 5_000]);
        assert_eq!(objects[0].size, OBJECT_HEADER_SIZE + 3 * 8);
        assert_eq!(objects[0].status, OBJECT_STATUS_NORMAL);
        // Brown 100%
        assert_eq!(objects[0].color, 7);
        assert_eq!(objects[1].object, area);
        assert_eq!(objects[2].object, boulder);
        // Point objects are indexed with room for their symbol
        assert_eq!(objects[2].bounds, [400, -800, 600, -600]);
    }

    #[test]
    fn chains_object_index_blocks() {
        let template = OcdTemplate::load(None).unwrap();
        let georeferencing = template.georeferencing(0, 0);
        let objects: Vec<OcdObject> = (0..600)
            .map(|index| OcdObject {
                symbol: 505000,
                kind: ObjectKind::Line,
                points: vec![point(index, 0), point(index, 10)],
            })
            .collect();

        let bytes = template.write(&georeferencing, &objects).unwrap();
        let written = read_objects(&bytes);

        assert_eq!(written.len(), 600);
        assert_eq!(written[599].object, objects[599]);
    }

    #[test]
    fn reads_back_object_records() {
        let objects = vec![
            OcdObject {
                symbol: 101000,
                kind: ObjectKind::Line,
                points: vec![point(-100, -200), point(300, 400)],
            },
            OcdObject {
                symbol: 403000,
                kind: ObjectKind::Area,
                points: vec![
                    point(0, 0),
                    point(1_000, 0),
                    point(0, 1_000),
                    OcdPoint {
                        x: 100,
                        y: 100,
                        y_flags: FLAG_HOLE,
                    },
                    point(200, 100),
                    point(100, 200),
                ],
            },
            OcdObject {
                symbol: 205000,
                kind: ObjectKind::Point,
                points: vec![point(50, 60)],
            },
        ];

        let mut bytes = vec![];
        for object in &objects {
            write_object_record(&mut bytes, object).unwrap();
        }
        let mut input = &bytes[..];
        let mut read = vec![];
        while let Some(object) = read_object_record(&mut input).unwrap() {
            read.push(object);
        }

        assert_eq!(read, objects);

        let mut truncated = &bytes[..OBJECT_HEADER_SIZE / 2];
        assert!(read_object_record(&mut truncated).is_err());
    }

    #[test]
    fn writes_object_index_blocks_between_the_objects() {
        let template = OcdTemplate::load(None).unwrap();
        let objects: Vec<OcdObject> = (0..600)
            .map(|index| OcdObject {
                symbol: 505000,
                kind: ObjectKind::Line,
                points: vec![point(index, 0), point(index, 10)],
            })
            .collect();

        let bytes = template
            .write(&template.georeferencing(0, 0), &objects)
            .unwrap();

        // The second block follows the objects the first one indexes
        let first_block = u32::from_le_bytes(
            bytes[HEADER_FIRST_OBJECT_BLOCK..HEADER_FIRST_OBJECT_BLOCK + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let second_block =
            u32::from_le_bytes(bytes[first_block..first_block + 4].try_into().unwrap()) as usize;
        let last_object_of_first_block = u32::from_le_bytes(
            bytes[first_block + 4 + 255 * OBJECT_ENTRY_SIZE + 16..][..4]
                .try_into()
                .unwrap(),
        ) as usize;
        assert!(second_block > last_object_of_first_block);
        assert_eq!(second_block % 8, 0);
    }

    #[test]
    fn rejects_objects_with_unknown_symbols() {
        let template = OcdTemplate::load(None).unwrap();
        let object = OcdObject {
            symbol: 123456,
            kind: ObjectKind::Line,
            points: vec![point(0, 0), point(1, 1)],
        };

        assert!(template
            .write(&template.georeferencing(0, 0), &[object])
            .is_err());
    }
}
