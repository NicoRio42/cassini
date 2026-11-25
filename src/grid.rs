//! Georeferenced single-band rasters in Lambert-93 (EPSG:2154), read from
//! and written to GeoTIFF files.

use crate::error::{CassiniError, Result, ResultContext};
use std::{
    fmt,
    fs::File,
    io::{BufWriter, Seek, Write},
    path::Path,
};
use tiff::{
    decoder::{Decoder, DecodingResult, Limits},
    encoder::{colortype, DeflateLevel, ImageEncoder, TiffEncoder, TiffKind},
    tags::{Predictor, Tag},
    ColorType, TiffResult,
};

/// The EPSG code of Lambert-93, the only CRS Cassini reads and writes.
const LAMBERT_93_EPSG: u16 = 2154;

/// GeoKey ids and values from the GeoTIFF specification.
const GT_MODEL_TYPE_GEO_KEY: u16 = 1024;
const GT_RASTER_TYPE_GEO_KEY: u16 = 1025;
const PROJECTED_CRS_GEO_KEY: u16 = 3072;
const PROJ_COORD_TRANS_GEO_KEY: u16 = 3075;
const PROJ_LINEAR_UNITS_GEO_KEY: u16 = 3076;
const MODEL_TYPE_PROJECTED: u16 = 1;
const MODEL_TYPE_GEOGRAPHIC: u16 = 2;
const RASTER_PIXEL_IS_AREA: u16 = 1;
const RASTER_PIXEL_IS_POINT: u16 = 2;
const USER_DEFINED: u16 = 32767;
const CT_LAMBERT_CONF_CONIC_2SP: u16 = 8;
const LINEAR_METER: u16 = 9001;

/// The parameters of Lambert-93, as GeoKeys: each with the keys a
/// writer may store it under, and its value.
const LAMBERT_93_PARAMETERS: [(&str, &[u16], f64); 6] = [
    ("first standard parallel", &[3078], 49.0),
    ("second standard parallel", &[3079], 44.0),
    // The false origin keys, or the natural origin ones some writers use.
    ("longitude of origin", &[3084, 3080], 3.0),
    ("latitude of origin", &[3085, 3081], 46.5),
    ("false easting", &[3086, 3082], 700_000.0),
    ("false northing", &[3087, 3083], 6_600_000.0),
];

/// A cell value type that can be read from and written to a GeoTIFF band.
pub trait Sample: Copy + PartialEq + 'static {
    type Color: colortype::ColorType<Inner = Self>;

    /// Whether the integer predictor applies when compressing this type.
    const HORIZONTAL_PREDICTOR: bool;

    /// The decoded values as `Self`, or `None` if they cannot be converted.
    fn from_decoded(decoded: DecodingResult) -> Option<Vec<Self>>;

    fn format_nodata(self) -> String;

    fn is_nan(self) -> bool;

    /// Writes `values` as the data of `image`.
    fn write_values<W: Write + Seek, K: TiffKind>(
        image: ImageEncoder<'_, W, Self::Color, K>,
        values: &[Self],
    ) -> TiffResult<()>;
}

macro_rules! float_sample {
    ($type:ty, $color:ty) => {
        impl Sample for $type {
            type Color = $color;
            const HORIZONTAL_PREDICTOR: bool = false;

            fn from_decoded(decoded: DecodingResult) -> Option<Vec<Self>> {
                Some(match decoded {
                    DecodingResult::U8(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::U16(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::U32(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::U64(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::I8(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::I16(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::I32(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::I64(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::F32(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::F64(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::F16(_) => return None,
                })
            }

            fn format_nodata(self) -> String {
                self.to_string()
            }

            fn is_nan(self) -> bool {
                <$type>::is_nan(self)
            }

            fn write_values<W: Write + Seek, K: TiffKind>(
                image: ImageEncoder<'_, W, Self::Color, K>,
                values: &[Self],
            ) -> TiffResult<()> {
                image.write_data(values)
            }
        }
    };
}

float_sample!(f32, colortype::Gray32Float);
float_sample!(f64, colortype::Gray64Float);

impl Sample for u8 {
    type Color = colortype::Gray8;
    const HORIZONTAL_PREDICTOR: bool = true;

    fn from_decoded(decoded: DecodingResult) -> Option<Vec<Self>> {
        match decoded {
            DecodingResult::U8(values) => Some(values),
            _ => None,
        }
    }

    fn format_nodata(self) -> String {
        self.to_string()
    }

    fn is_nan(self) -> bool {
        false
    }

    fn write_values<W: Write + Seek, K: TiffKind>(
        image: ImageEncoder<'_, W, Self::Color, K>,
        values: &[Self],
    ) -> TiffResult<()> {
        image.write_data(values)
    }
}

/// The type a GeoTIFF band stores its values as, named as GDAL does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    Byte,
    Int8,
    UInt16,
    Int16,
    UInt32,
    Int32,
    UInt64,
    Int64,
    Float16,
    Float32,
    Float64,
}

impl SampleType {
    fn of(decoded: &DecodingResult) -> Self {
        match decoded {
            DecodingResult::U8(_) => Self::Byte,
            DecodingResult::I8(_) => Self::Int8,
            DecodingResult::U16(_) => Self::UInt16,
            DecodingResult::I16(_) => Self::Int16,
            DecodingResult::U32(_) => Self::UInt32,
            DecodingResult::I32(_) => Self::Int32,
            DecodingResult::U64(_) => Self::UInt64,
            DecodingResult::I64(_) => Self::Int64,
            DecodingResult::F16(_) => Self::Float16,
            DecodingResult::F32(_) => Self::Float32,
            DecodingResult::F64(_) => Self::Float64,
        }
    }

    /// `value` parsed as this type, as a single decoded value, so that it
    /// converts to another type exactly as the cells do.
    fn parse(self, value: &str) -> Option<DecodingResult> {
        let value = value.trim();
        Some(match self {
            Self::Byte => DecodingResult::U8(vec![value.parse().ok()?]),
            Self::Int8 => DecodingResult::I8(vec![value.parse().ok()?]),
            Self::UInt16 => DecodingResult::U16(vec![value.parse().ok()?]),
            Self::Int16 => DecodingResult::I16(vec![value.parse().ok()?]),
            Self::UInt32 => DecodingResult::U32(vec![value.parse().ok()?]),
            Self::Int32 => DecodingResult::I32(vec![value.parse().ok()?]),
            Self::UInt64 => DecodingResult::U64(vec![value.parse().ok()?]),
            Self::Int64 => DecodingResult::I64(vec![value.parse().ok()?]),
            Self::Float16 => return None,
            Self::Float32 => DecodingResult::F32(vec![value.parse().ok()?]),
            Self::Float64 => DecodingResult::F64(vec![value.parse().ok()?]),
        })
    }
}

impl fmt::Display for SampleType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, formatter)
    }
}

/// How `Grid::write_geotiff` compresses the band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    /// Deflate, with the horizontal predictor for integer types. Floats have
    /// no predictor, as the `tiff` crate cannot write the floating-point one.
    Deflate,
}

/// Where the cells of a GeoTIFF band are, without their values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridExtent {
    pub min_x: f64,
    pub max_y: f64,
    /// The cell size in metres.
    pub resolution: f64,
    pub width: usize,
    pub height: usize,
}

impl GridExtent {
    pub fn max_x(&self) -> f64 {
        self.min_x + self.width as f64 * self.resolution
    }

    pub fn min_y(&self) -> f64 {
        self.max_y - self.height as f64 * self.resolution
    }

    /// Reads the extent of the single-band GeoTIFF file at `path`, with the
    /// same checks as `Grid::read_geotiff`, without decoding its cells.
    pub fn read_geotiff(path: &Path) -> Result<Self> {
        open_geotiff(path).map(|(_, extent)| extent)
    }
}

/// A north-up raster of square cells, row-major from the north-west corner.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid<T> {
    pub min_x: f64,
    pub max_y: f64,
    /// The cell size in metres.
    pub resolution: f64,
    pub width: usize,
    pub height: usize,
    /// The value of the cells without data, from the `GDAL_NODATA` tag.
    pub nodata: Option<T>,
    /// `width * height` values.
    pub values: Vec<T>,
}

impl<T: Sample> Grid<T> {
    /// A grid of `width` x `height` cells set to `fill`.
    pub fn new(
        min_x: f64,
        max_y: f64,
        resolution: f64,
        width: usize,
        height: usize,
        nodata: Option<T>,
        fill: T,
    ) -> Self {
        Self {
            min_x,
            max_y,
            resolution,
            width,
            height,
            nodata,
            values: vec![fill; width * height],
        }
    }

    pub fn max_x(&self) -> f64 {
        self.min_x + self.width as f64 * self.resolution
    }

    pub fn min_y(&self) -> f64 {
        self.max_y - self.height as f64 * self.resolution
    }

    /// The index in `values` of cell (`col`, `row`).
    pub fn index(&self, col: usize, row: usize) -> usize {
        debug_assert!(
            col < self.width && row < self.height,
            "cell ({col}, {row}) is outside the {} x {} grid",
            self.width,
            self.height
        );
        row * self.width + col
    }

    pub fn get(&self, col: usize, row: usize) -> T {
        self.values[self.index(col, row)]
    }

    pub fn set(&mut self, col: usize, row: usize, value: T) {
        let index = self.index(col, row);
        self.values[index] = value;
    }

    /// Whether `value` is a cell with data: neither NaN nor the nodata value.
    pub fn has_data(&self, value: T) -> bool {
        !value.is_nan() && self.nodata != Some(value)
    }

    /// The projected coordinates of the centre of cell (`col`, `row`).
    pub fn cell_center(&self, col: usize, row: usize) -> (f64, f64) {
        (
            self.min_x + (col as f64 + 0.5) * self.resolution,
            self.max_y - (row as f64 + 0.5) * self.resolution,
        )
    }

    /// The cell (`col`, `row`) containing the point (`x`, `y`), or `None`
    /// outside the grid or for a NaN coordinate. A point on the edge between
    /// two cells belongs to the east or south one, so the east and south
    /// edges of the grid are out.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let col = ((x - self.min_x) / self.resolution).floor();
        let row = ((self.max_y - y) / self.resolution).floor();
        if !(0.0..self.width as f64).contains(&col) || !(0.0..self.height as f64).contains(&row) {
            return None;
        }
        Some((col as usize, row as usize))
    }

    /// Reads the band of the single-band GeoTIFF file at `path`. Values of
    /// another numeric type are converted to `T` with `as`, which may lose
    /// precision, as `gdal_translate -ot` does.
    pub fn read_geotiff(path: &Path) -> Result<Self> {
        Self::read_geotiff_with_type(path).map(|(grid, _)| grid)
    }

    /// Like `read_geotiff`, also returning the type the file stores.
    pub fn read_geotiff_with_type(path: &Path) -> Result<(Self, SampleType)> {
        let invalid = |message: String| CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message,
        };
        let (mut decoder, extent) = open_geotiff(path)?;
        let GridExtent {
            min_x,
            max_y,
            resolution,
            width,
            height,
        } = extent;

        let nodata = decoder.get_tag_ascii_string(Tag::GdalNodata).ok();

        let decoded = decoder
            .read_image()
            .context(format!("could not decode `{}`", path.display()))?;
        let sample_type = SampleType::of(&decoded);
        let cannot_convert = || {
            invalid(format!(
                "cannot convert {sample_type} cells to {}",
                std::any::type_name::<T>()
            ))
        };
        let values = T::from_decoded(decoded).ok_or_else(cannot_convert)?;
        if values.len() != width * height {
            return Err(invalid(format!(
                "expected {} values for {width} x {height} cells, found {}",
                width * height,
                values.len()
            )));
        }
        // Parsed as the stored type, then converted like the cells, so that
        // the nodata cells still equal it.
        let nodata = nodata
            .and_then(|value| sample_type.parse(&value))
            .and_then(T::from_decoded)
            .map(|value| value[0]);

        let grid = Self {
            min_x,
            max_y,
            resolution,
            width,
            height,
            nodata,
            values,
        };
        Ok((grid, sample_type))
    }

    /// Writes the grid to `path` as a GeoTIFF in Lambert-93.
    pub fn write_geotiff(&self, path: &Path, compression: Compression) -> Result<()> {
        let file = File::create(path).context(format!("could not create `{}`", path.display()))?;
        let mut writer = BufWriter::new(file);
        self.encode(&mut writer, compression)
            .context(format!("could not write `{}`", path.display()))?;
        writer
            .flush()
            .context(format!("could not write `{}`", path.display()))
    }

    fn encode<W: Write + Seek>(&self, writer: W, compression: Compression) -> TiffResult<()> {
        let mut encoder = TiffEncoder::new(writer)?;
        if compression == Compression::Deflate {
            encoder = encoder
                .with_compression(tiff::encoder::Compression::Deflate(DeflateLevel::Balanced));
            if T::HORIZONTAL_PREDICTOR {
                encoder = encoder.with_predictor(Predictor::Horizontal);
            }
        }

        let mut image =
            encoder.new_image::<T::Color>(u32::try_from(self.width)?, u32::try_from(self.height)?)?;
        let directory = image.encoder();
        directory.write_tag(
            Tag::ModelPixelScaleTag,
            &[self.resolution, self.resolution, 0.0][..],
        )?;
        directory.write_tag(
            Tag::ModelTiepointTag,
            &[0.0, 0.0, 0.0, self.min_x, self.max_y, 0.0][..],
        )?;
        directory.write_tag(
            Tag::GeoKeyDirectoryTag,
            &[
                1,
                1,
                0,
                3,
                GT_MODEL_TYPE_GEO_KEY,
                0,
                1,
                MODEL_TYPE_PROJECTED,
                GT_RASTER_TYPE_GEO_KEY,
                0,
                1,
                RASTER_PIXEL_IS_AREA,
                PROJECTED_CRS_GEO_KEY,
                0,
                1,
                LAMBERT_93_EPSG,
            ][..],
        )?;
        if let Some(nodata) = self.nodata {
            directory.write_tag(Tag::GdalNodata, nodata.format_nodata().as_str())?;
        }
        T::write_values(image, &self.values)
    }
}

/// A decoder of the single-band GeoTIFF file at `path`, positioned before its
/// cells, and their extent. Checks that the file is in Lambert-93.
fn open_geotiff(path: &Path) -> Result<(Decoder<File>, GridExtent)> {
    let invalid = |message: String| CassiniError::InvalidArtifact {
        path: path.to_path_buf(),
        message,
    };
    let file = File::open(path).context(format!("could not open `{}`", path.display()))?;
    let mut decoder = Decoder::new(file)
        .context(format!("could not create decoder for `{}`", path.display()))?
        .with_limits(Limits::unlimited());

    let (width, height) = decoder.dimensions().context(format!(
        "could not read dimensions from `{}`",
        path.display()
    ))?;
    let color_type = decoder
        .colortype()
        .context(format!("could not read color type of `{}`", path.display()))?;
    if !matches!(color_type, ColorType::Gray(_)) {
        return Err(invalid(format!(
            "expected a single band, found {color_type:?}"
        )));
    }

    let geo_keys = decoder
        .get_tag_u16_vec(Tag::GeoKeyDirectoryTag)
        .unwrap_or_default();
    let geo_doubles = decoder
        .get_tag_f64_vec(Tag::GeoDoubleParamsTag)
        .unwrap_or_default();
    check_lambert_93(&geo_keys, &geo_doubles).map_err(invalid)?;

    let scale = decoder
        .get_tag_f64_vec(Tag::ModelPixelScaleTag)
        .map_err(|_| invalid("missing ModelPixelScaleTag".to_owned()))?;
    let tiepoint = decoder
        .get_tag_f64_vec(Tag::ModelTiepointTag)
        .map_err(|_| invalid("missing ModelTiepointTag".to_owned()))?;
    if scale.len() < 2 || tiepoint.len() < 6 {
        return Err(invalid("malformed georeferencing tags".to_owned()));
    }
    if scale[0] != scale[1] {
        return Err(invalid(format!(
            "expected square cells, found {} x {}",
            scale[0], scale[1]
        )));
    }
    let resolution = scale[0];
    let mut min_x = tiepoint[3] - tiepoint[0] * resolution;
    let mut max_y = tiepoint[4] + tiepoint[1] * resolution;
    // As GDAL does, a PixelIsPoint tiepoint is the centre of its cell.
    if geo_key(&geo_keys, GT_RASTER_TYPE_GEO_KEY) == Some(RASTER_PIXEL_IS_POINT) {
        min_x -= resolution / 2.0;
        max_y += resolution / 2.0;
    }

    let extent = GridExtent {
        min_x,
        max_y,
        resolution,
        width: width as usize,
        height: height as usize,
    };
    Ok((decoder, extent))
}

/// Checks that the GeoKeys `directory`, with the double parameters
/// `doubles`, describe Lambert-93.
///
/// A user-defined CRS is accepted if its projection is Lambert-93's, as in
/// the IGN DEMs, whose GeoKeys spell out Lambert-93 instead of naming
/// EPSG:2154. A user-defined CRS without a projection, or no CRS at all, is
/// assumed to be Lambert-93.
fn check_lambert_93(directory: &[u16], doubles: &[f64]) -> std::result::Result<(), String> {
    if geo_key(directory, GT_MODEL_TYPE_GEO_KEY) == Some(MODEL_TYPE_GEOGRAPHIC) {
        return Err("expected Lambert-93 (EPSG:2154), found geographic coordinates".to_owned());
    }
    match geo_key(directory, PROJECTED_CRS_GEO_KEY) {
        None | Some(LAMBERT_93_EPSG) => return Ok(()),
        Some(USER_DEFINED) => {}
        Some(code) => {
            return Err(format!(
                "expected Lambert-93 (EPSG:2154), found EPSG:{code}"
            ));
        }
    }

    let user_defined = |found: String| {
        format!("expected Lambert-93 (EPSG:2154), found a user-defined CRS with {found}")
    };
    if let Some(units) =
        geo_key(directory, PROJ_LINEAR_UNITS_GEO_KEY).filter(|&units| units != LINEAR_METER)
    {
        return Err(user_defined(format!("linear units {units}")));
    }
    match geo_key(directory, PROJ_COORD_TRANS_GEO_KEY) {
        None => return Ok(()),
        Some(CT_LAMBERT_CONF_CONIC_2SP) => {}
        Some(transformation) => {
            return Err(user_defined(format!(
                "coordinate transformation {transformation}"
            )));
        }
    }
    for (name, keys, expected) in LAMBERT_93_PARAMETERS {
        let value = keys
            .iter()
            .find_map(|&key| geo_double(directory, doubles, key))
            .ok_or_else(|| user_defined(format!("no {name}")))?;
        if (value - expected).abs() > 1e-9 * expected.abs() {
            return Err(user_defined(format!(
                "a {name} of {value} instead of {expected}"
            )));
        }
    }
    Ok(())
}

/// The value of `key` in a GeoKeyDirectoryTag, if stored in the
/// GeoDoubleParamsTag `doubles`.
fn geo_double(directory: &[u16], doubles: &[f64], key: u16) -> Option<f64> {
    match geo_key_entry(directory, key)? {
        [location, 1, offset] if location == Tag::GeoDoubleParamsTag.to_u16() => {
            doubles.get(offset as usize).copied()
        }
        _ => None,
    }
}

/// The value of `key` in a GeoKeyDirectoryTag, if stored inline.
fn geo_key(directory: &[u16], key: u16) -> Option<u16> {
    match geo_key_entry(directory, key)? {
        [0, _, value] => Some(value),
        _ => None,
    }
}

/// The location, count and value or offset of `key` in a
/// GeoKeyDirectoryTag.
fn geo_key_entry(directory: &[u16], key: u16) -> Option<[u16; 3]> {
    let count = *directory.get(3)? as usize;
    directory
        .get(4..4 + 4 * count)?
        .as_chunks::<4>()
        .0
        .iter()
        .find(|&&[id, ..]| id == key)
        .map(|&[_, location, count, value]| [location, count, value])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    /// A temporary file path, the file being removed on drop.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(std::env::temp_dir().join(format!("cassini-grid-{unique}-{name}")))
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn str(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn sample_grid<T: Sample>(nodata: T, value: impl Fn(usize) -> T) -> Grid<T> {
        let mut grid = Grid::new(907_000.0, 6_455_000.0, 0.5, 7, 5, Some(nodata), nodata);
        for (index, cell) in grid.values.iter_mut().enumerate() {
            *cell = if index % 4 == 0 { nodata } else { value(index) };
        }
        grid
    }

    fn assert_round_trip<T: Sample + std::fmt::Debug>(grid: &Grid<T>, compression: Compression) {
        let file = TempFile::new("round-trip.tif");
        grid.write_geotiff(file.path(), compression).unwrap();
        assert_eq!(&Grid::<T>::read_geotiff(file.path()).unwrap(), grid);
    }

    #[test]
    fn round_trips_every_sample_type() {
        for compression in [Compression::None, Compression::Deflate] {
            assert_round_trip(
                &sample_grid(-9999.0f64, |i| 500.0 + i as f64 / 3.0),
                compression,
            );
            assert_round_trip(
                &sample_grid(-9999.0f32, |i| 500.0 + i as f32 / 3.0),
                compression,
            );
            assert_round_trip(&sample_grid(255u8, |i| (i * 7 % 255) as u8), compression);
        }
    }

    #[test]
    fn round_trips_without_nodata() {
        let mut grid = sample_grid(0u8, |i| i as u8);
        grid.nodata = None;
        assert_round_trip(&grid, Compression::Deflate);
    }

    #[test]
    fn converts_to_wider_types_on_read() {
        let grid = sample_grid(255u8, |i| i as u8);
        let file = TempFile::new("convert.tif");
        grid.write_geotiff(file.path(), Compression::Deflate).unwrap();
        let (read, sample_type) = Grid::<f64>::read_geotiff_with_type(file.path()).unwrap();
        assert_eq!(sample_type, SampleType::Byte);
        assert_eq!(read.nodata, Some(255.0));
        assert_eq!(
            read.values,
            grid.values.iter().map(|&v| v as f64).collect::<Vec<_>>()
        );
    }

    #[test]
    fn converts_nodata_like_the_cells() {
        // -9999.9 is not a float32, so the tag and the f64 cells differ unless
        // the tag is parsed as a float32 too.
        let grid = sample_grid(-9999.9f32, |i| i as f32);
        let file = TempFile::new("nodata.tif");
        grid.write_geotiff(file.path(), Compression::None).unwrap();
        let (read, sample_type) = Grid::<f64>::read_geotiff_with_type(file.path()).unwrap();
        assert_eq!(sample_type, SampleType::Float32);
        assert_eq!(read.nodata, Some(-9999.9f32 as f64));
        assert!(!read.has_data(read.get(0, 0)));
    }

    #[test]
    fn maps_between_cells_and_coordinates() {
        let grid = Grid::new(1000.0, 2000.0, 0.5, 4, 2, None, 0.0f32);
        assert_eq!(grid.max_x(), 1002.0);
        assert_eq!(grid.min_y(), 1999.0);
        assert_eq!(grid.cell_center(0, 0), (1000.25, 1999.75));
        assert_eq!(grid.cell_center(3, 1), (1001.75, 1999.25));
        assert_eq!(grid.cell_at(1000.25, 1999.75), Some((0, 0)));
        assert_eq!(grid.cell_at(1000.0, 2000.0), Some((0, 0)));
        assert_eq!(grid.cell_at(1000.5, 1999.5), Some((1, 1)));
        assert_eq!(grid.cell_at(1001.99, 1999.01), Some((3, 1)));
        assert_eq!(grid.cell_at(1002.0, 1999.5), None);
        assert_eq!(grid.cell_at(1001.0, 1999.0), None);
        assert_eq!(grid.cell_at(999.99, 1999.5), None);
        assert_eq!(grid.cell_at(1001.0, 2000.01), None);
        assert_eq!(grid.cell_at(f64::NAN, 1999.5), None);
        assert_eq!(grid.cell_at(1001.0, f64::NAN), None);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "outside the 4 x 2 grid")]
    fn rejects_columns_past_the_east_edge() {
        let grid = Grid::new(1000.0, 2000.0, 0.5, 4, 2, None, 0.0f32);
        grid.get(4, 0);
    }

    /// Runs `program` with `args`, or returns `None` if it is not installed.
    fn run(program: &str, args: &[&str]) -> Option<String> {
        match Command::new(program).args(args).output() {
            Ok(output) => {
                assert!(output.status.success(), "{program} failed: {output:?}");
                Some(String::from_utf8(output.stdout).unwrap())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("`{program}` not found, skipping");
                None
            }
            Err(error) => panic!("could not run {program}: {error}"),
        }
    }

    /// Runs `gdal_translate -q` with `args` from `source` to `target`, or
    /// returns `false` if it is not installed.
    fn gdal_translate(source: &TempFile, target: &TempFile, args: &[&str]) -> bool {
        let mut all_args = vec!["-q"];
        all_args.extend(args);
        all_args.extend([source.str(), target.str()]);
        run("gdal_translate", &all_args).is_some()
    }

    #[test]
    fn gdalinfo_reads_the_georeferencing() {
        let grid = sample_grid(-9999.0f32, |i| i as f32);
        let file = TempFile::new("gdalinfo.tif");
        grid.write_geotiff(file.path(), Compression::Deflate).unwrap();
        let Some(output) = run("gdalinfo", &["-json", file.str()]) else {
            return;
        };

        let info: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(info["size"], serde_json::json!([7, 5]));
        assert_eq!(
            info["geoTransform"],
            serde_json::json!([907000.0, 0.5, 0.0, 6455000.0, 0.0, -0.5])
        );
        // WKT2 since GDAL 3, WKT1 before.
        let wkt = info["coordinateSystem"]["wkt"].as_str().unwrap();
        assert!(
            wkt.contains(r#"ID["EPSG",2154]"#) || wkt.contains(r#"AUTHORITY["EPSG","2154"]"#),
            "{wkt}"
        );
        assert_eq!(info["bands"][0]["type"], "Float32");
        assert_eq!(info["bands"][0]["noDataValue"], -9999.0);
        assert_eq!(
            info["metadata"]["IMAGE_STRUCTURE"]["COMPRESSION"],
            "DEFLATE"
        );
        assert_eq!(info["metadata"][""]["AREA_OR_POINT"], "Area", "{output}");
    }

    #[test]
    fn reads_gdal_float_predictor_files() {
        let grid = sample_grid(-9999.0f64, |i| 500.0 + i as f64 / 3.0);
        let source = TempFile::new("predictor-source.tif");
        let translated = TempFile::new("predictor-translated.tif");
        grid.write_geotiff(source.path(), Compression::None).unwrap();
        let args = ["-co", "COMPRESS=DEFLATE", "-co", "PREDICTOR=3"];
        if gdal_translate(&source, &translated, &args) {
            assert_eq!(Grid::<f64>::read_geotiff(translated.path()).unwrap(), grid);
        }
    }

    #[test]
    fn reads_pixel_is_point_files_like_gdal() {
        let grid = sample_grid(-9999.0f32, |i| i as f32);
        let source = TempFile::new("point-source.tif");
        let translated = TempFile::new("point-translated.tif");
        grid.write_geotiff(source.path(), Compression::None).unwrap();
        // GDAL keeps the extent and writes the tiepoint at the cell centre.
        if gdal_translate(&source, &translated, &["-mo", "AREA_OR_POINT=Point"]) {
            assert_eq!(Grid::<f32>::read_geotiff(translated.path()).unwrap(), grid);
        }
    }

    /// The GeoKeys of the IGN DEMs (MNT LiDAR HD from the Géoplateforme WMS):
    /// a user-defined CRS spelling out Lambert-93, and its double parameters.
    #[rustfmt::skip]
    const IGN_GEO_KEYS: [u16; 80] = [
        1, 1, 0, 19,
        1024, 0, 1, 1,
        1025, 0, 1, 1,
        1026, 34737, 10, 0,
        2048, 0, 1, 32767,
        2050, 0, 1, 32767,
        2051, 0, 1, 32767,
        2052, 0, 1, 9001,
        2054, 0, 1, 9102,
        2056, 0, 1, 32767,
        3072, 0, 1, 32767,
        3074, 0, 1, 32767,
        3075, 0, 1, 8,
        3076, 0, 1, 9001,
        3078, 34736, 1, 0,
        3079, 34736, 1, 1,
        3084, 34736, 1, 2,
        3085, 34736, 1, 3,
        3086, 34736, 1, 4,
        3087, 34736, 1, 5,
    ];
    const IGN_GEO_DOUBLES: [f64; 6] = [49.0, 44.0, 3.0, 46.5, 700000.0, 6600000.0];

    /// Writes a 2 x 2 Float32 GeoTIFF of 0.5 m cells to `path`, its
    /// tiepoint at (`x`, `y`), with `geo_keys` and `geo_doubles`.
    fn write_with_geo_keys(path: &Path, x: f64, y: f64, geo_keys: &[u16], geo_doubles: &[f64]) {
        let mut encoder = TiffEncoder::new(File::create(path).unwrap()).unwrap();
        let mut image = encoder.new_image::<colortype::Gray32Float>(2, 2).unwrap();
        let directory = image.encoder();
        directory
            .write_tag(Tag::ModelPixelScaleTag, &[0.5, 0.5, 0.0][..])
            .unwrap();
        directory
            .write_tag(Tag::ModelTiepointTag, &[0.0, 0.0, 0.0, x, y, 0.0][..])
            .unwrap();
        directory
            .write_tag(Tag::GeoKeyDirectoryTag, geo_keys)
            .unwrap();
        directory
            .write_tag(Tag::GeoDoubleParamsTag, geo_doubles)
            .unwrap();
        directory
            .write_tag(Tag::GeoAsciiParamsTag, "EPSG:2154|")
            .unwrap();
        directory.write_tag(Tag::GdalNodata, " -9999").unwrap();
        image.write_data(&[1.0f32, 2.0, -9999.0, 4.0]).unwrap();
    }

    #[test]
    fn reads_the_user_defined_lambert_93_of_ign_dems() {
        let file = TempFile::new("ign.tif");
        write_with_geo_keys(
            file.path(),
            906999.75,
            6455000.25,
            &IGN_GEO_KEYS,
            &IGN_GEO_DOUBLES,
        );
        let grid = Grid::<f32>::read_geotiff(file.path()).unwrap();
        assert_eq!(
            (
                grid.min_x,
                grid.max_y,
                grid.resolution,
                grid.width,
                grid.height
            ),
            (906999.75, 6455000.25, 0.5, 2, 2)
        );
        assert_eq!(grid.nodata, Some(-9999.0));
        assert_eq!(grid.values, [1.0, 2.0, -9999.0, 4.0]);
        assert_eq!(
            GridExtent::read_geotiff(file.path()).unwrap(),
            GridExtent {
                min_x: 906999.75,
                max_y: 6455000.25,
                resolution: 0.5,
                width: 2,
                height: 2,
            }
        );
    }

    #[test]
    fn rejects_user_defined_crs_other_than_lambert_93() {
        let file = TempFile::new("user-defined.tif");
        let with = |key: u16, value: u16| {
            let mut keys = IGN_GEO_KEYS;
            let entry = keys.chunks_mut(4).find(|entry| entry[0] == key).unwrap();
            entry[3] = value;
            keys
        };
        // Lambert-93 parameters but another projection: Transverse Mercator,
        // and feet.
        for (keys, message) in [
            (with(3075, 1), "coordinate transformation 1"),
            (with(3076, 9002), "linear units 9002"),
        ] {
            write_with_geo_keys(file.path(), 0.0, 0.0, &keys, &IGN_GEO_DOUBLES);
            let error = Grid::<f32>::read_geotiff(file.path()).unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
        }
        // The parameters of CC46 (EPSG:3946), a Lambert conformal conic
        // projection of the same family.
        let cc46 = [45.25, 46.75, 3.0, 46.0, 1700000.0, 5200000.0];
        write_with_geo_keys(file.path(), 0.0, 0.0, &IGN_GEO_KEYS, &cc46);
        let error = GridExtent::read_geotiff(file.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("first standard parallel of 45.25 instead of 49"),
            "{error}"
        );
        // A missing parameter.
        write_with_geo_keys(file.path(), 0.0, 0.0, &IGN_GEO_KEYS, &IGN_GEO_DOUBLES[..5]);
        let error = Grid::<f32>::read_geotiff(file.path()).unwrap_err();
        assert!(error.to_string().contains("no false northing"), "{error}");
    }

    #[test]
    fn rejects_other_crs() {
        let grid = sample_grid(-9999.0f32, |i| i as f32);
        let source = TempFile::new("crs-source.tif");
        let translated = TempFile::new("crs-translated.tif");
        grid.write_geotiff(source.path(), Compression::None).unwrap();
        for (srs, message) in [
            ("EPSG:2056", "found EPSG:2056"),
            ("EPSG:4326", "found geographic coordinates"),
        ] {
            if !gdal_translate(&source, &translated, &["-a_srs", srs]) {
                return;
            }
            let error = Grid::<f32>::read_geotiff(translated.path()).unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
        }
    }
}
