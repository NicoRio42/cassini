//! Georeferenced single-band rasters in Lambert-93 (EPSG:2154), read from
//! and written to GeoTIFF files.

use crate::error::{CassiniError, Result, ResultContext};
use std::{
    fs::File,
    io::{BufWriter, Seek, Write},
    path::Path,
};
use tiff::{
    decoder::{Decoder, DecodingResult, Limits},
    encoder::{colortype, DeflateLevel, TiffEncoder},
    tags::{Predictor, Tag},
    ColorType,
};

/// The EPSG code of Lambert-93, the only CRS Cassini writes.
const LAMBERT_93_EPSG: u16 = 2154;

/// GeoKey ids and values from the GeoTIFF specification.
const GT_MODEL_TYPE_GEO_KEY: u16 = 1024;
const GT_RASTER_TYPE_GEO_KEY: u16 = 1025;
const PROJECTED_CRS_GEO_KEY: u16 = 3072;
const MODEL_TYPE_PROJECTED: u16 = 1;
const RASTER_PIXEL_IS_AREA: u16 = 1;
const RASTER_PIXEL_IS_POINT: u16 = 2;

/// A cell value type that can be read from and written to a GeoTIFF band.
pub trait Sample: Copy + PartialEq + 'static {
    #[doc(hidden)]
    type Color: colortype::ColorType<Inner = Self>;

    /// Whether the integer predictor applies when compressing this type.
    #[doc(hidden)]
    const HORIZONTAL_PREDICTOR: bool;

    /// The decoded values as `Self`, or `None` if they cannot be converted.
    #[doc(hidden)]
    fn from_decoded(decoded: DecodingResult) -> Option<Vec<Self>>;

    #[doc(hidden)]
    fn parse_nodata(value: &str) -> Option<Self>;

    #[doc(hidden)]
    fn format_nodata(self) -> String;

    #[doc(hidden)]
    fn is_nan(self) -> bool;
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
                    DecodingResult::I8(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::I16(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::I32(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::F32(values) => values.into_iter().map(|v| v as _).collect(),
                    DecodingResult::F64(values) => values.into_iter().map(|v| v as _).collect(),
                    _ => return None,
                })
            }

            fn parse_nodata(value: &str) -> Option<Self> {
                value.trim().parse().ok()
            }

            fn format_nodata(self) -> String {
                self.to_string()
            }

            fn is_nan(self) -> bool {
                <$type>::is_nan(self)
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

    fn parse_nodata(value: &str) -> Option<Self> {
        value.trim().parse().ok()
    }

    fn format_nodata(self) -> String {
        self.to_string()
    }

    fn is_nan(self) -> bool {
        false
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
    /// outside the grid. A point on the edge between two cells belongs to the
    /// east or south one, so the east and south edges of the grid are out.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let col = ((x - self.min_x) / self.resolution).floor();
        let row = ((self.max_y - y) / self.resolution).floor();
        if col < 0.0 || row < 0.0 || col >= self.width as f64 || row >= self.height as f64 {
            return None;
        }
        Some((col as usize, row as usize))
    }

    /// Reads the first band of the GeoTIFF file at `path`, converting the
    /// values to `T` if the file stores another numeric type.
    pub fn read_geotiff(path: &Path) -> Result<Self> {
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
        let geo_keys = decoder
            .get_tag_u16_vec(Tag::GeoKeyDirectoryTag)
            .unwrap_or_default();
        if geo_key(&geo_keys, GT_RASTER_TYPE_GEO_KEY) == Some(RASTER_PIXEL_IS_POINT) {
            min_x -= resolution / 2.0;
            max_y += resolution / 2.0;
        }

        let nodata = decoder
            .get_tag_ascii_string(Tag::GdalNodata)
            .ok()
            .and_then(|value| T::parse_nodata(&value));

        let decoded = decoder
            .read_image()
            .context(format!("could not decode `{}`", path.display()))?;
        let values = T::from_decoded(decoded).ok_or_else(|| {
            invalid(format!(
                "cannot convert {color_type:?} cells to {}",
                std::any::type_name::<T>()
            ))
        })?;
        let (width, height) = (width as usize, height as usize);
        if values.len() != width * height {
            return Err(invalid(format!(
                "expected {} values for {width} x {height} cells, found {}",
                width * height,
                values.len()
            )));
        }

        Ok(Self {
            min_x,
            max_y,
            resolution,
            width,
            height,
            nodata,
            values,
        })
    }

    /// Writes the grid to `path` as a GeoTIFF in Lambert-93.
    pub fn write_geotiff(&self, path: &Path, compression: Compression) -> Result<()>
    where
        [T]: tiff::encoder::TiffValue,
    {
        let file = File::create(path).context(format!("could not create `{}`", path.display()))?;
        let mut writer = BufWriter::new(file);
        self.encode(&mut writer, compression)
            .context(format!("could not write `{}`", path.display()))?;
        writer
            .flush()
            .context(format!("could not write `{}`", path.display()))
    }

    fn encode<W: Write + Seek>(&self, writer: W, compression: Compression) -> tiff::TiffResult<()>
    where
        [T]: tiff::encoder::TiffValue,
    {
        let mut encoder = TiffEncoder::new(writer)?;
        if compression == Compression::Deflate {
            encoder = encoder
                .with_compression(tiff::encoder::Compression::Deflate(DeflateLevel::Balanced));
            if T::HORIZONTAL_PREDICTOR {
                encoder = encoder.with_predictor(Predictor::Horizontal);
            }
        }

        let mut image = encoder.new_image::<T::Color>(self.width as u32, self.height as u32)?;
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
        image.write_data(&self.values)
    }
}

/// The value of `key` in a GeoKeyDirectoryTag, if stored inline.
fn geo_key(directory: &[u16], key: u16) -> Option<u16> {
    let count = *directory.get(3)? as usize;
    directory
        .get(4..4 + 4 * count)?
        .chunks_exact(4)
        .find(|entry| entry[0] == key && entry[1] == 0)
        .map(|entry| entry[3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("cassini-grid-{unique}-{name}"))
    }

    fn sample_grid<T: Sample>(nodata: T, value: impl Fn(usize) -> T) -> Grid<T> {
        let mut grid = Grid::new(907_000.0, 6_455_000.0, 0.5, 7, 5, Some(nodata), nodata);
        for (index, cell) in grid.values.iter_mut().enumerate() {
            *cell = if index % 4 == 0 { nodata } else { value(index) };
        }
        grid
    }

    fn assert_round_trip<T: Sample + std::fmt::Debug>(grid: &Grid<T>, compression: Compression)
    where
        [T]: tiff::encoder::TiffValue,
    {
        let path = temp_path("round-trip.tif");
        grid.write_geotiff(&path, compression).unwrap();
        let read = Grid::<T>::read_geotiff(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(&read, grid);
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
    fn converts_to_wider_types_on_read() {
        let grid = sample_grid(255u8, |i| i as u8);
        let path = temp_path("convert.tif");
        grid.write_geotiff(&path, Compression::Deflate).unwrap();
        let read = Grid::<f64>::read_geotiff(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(read.nodata, Some(255.0));
        assert_eq!(
            read.values,
            grid.values.iter().map(|&v| v as f64).collect::<Vec<_>>()
        );
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

    #[test]
    fn gdalinfo_reads_the_georeferencing() {
        let grid = sample_grid(-9999.0f32, |i| i as f32);
        let path = temp_path("gdalinfo.tif");
        grid.write_geotiff(&path, Compression::Deflate).unwrap();
        let output = run("gdalinfo", &["-json", path.to_str().unwrap()]);
        std::fs::remove_file(&path).unwrap();
        let Some(output) = output else { return };

        let info: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(info["size"], serde_json::json!([7, 5]));
        assert_eq!(
            info["geoTransform"],
            serde_json::json!([907000.0, 0.5, 0.0, 6455000.0, 0.0, -0.5])
        );
        assert_eq!(info["stac"]["proj:epsg"], 2154);
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
        let source = temp_path("gdal-source.tif");
        let translated = temp_path("gdal-translated.tif");
        grid.write_geotiff(&source, Compression::None).unwrap();
        let output = run(
            "gdal_translate",
            &[
                "-q",
                "-co",
                "COMPRESS=DEFLATE",
                "-co",
                "PREDICTOR=3",
                source.to_str().unwrap(),
                translated.to_str().unwrap(),
            ],
        );
        std::fs::remove_file(&source).unwrap();
        if output.is_none() {
            return;
        }
        let read = Grid::<f64>::read_geotiff(&translated).unwrap();
        std::fs::remove_file(&translated).unwrap();
        assert_eq!(read, grid);
    }
}
