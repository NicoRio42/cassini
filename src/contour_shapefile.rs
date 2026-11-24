//! The contour shapefiles: the layer `gdal_contour -a elev` writes, whose
//! fields the smoothed contours and the form lines keep.

use std::{fs, io::Cursor, io::Write, path::Path};

use shapefile::{
    dbase::{FieldIOError, FieldName, FieldWriter, TableWriterBuilder, WritableRecord},
    Point, Polyline, ShapeType, ShapeWriter,
};

use crate::{
    error::{Result, ResultContext},
    marching_squares::ContourLine,
};

/// The `.prj` OGR writes for EPSG:2154.
const LAMBERT_93_ESRI_WKT: &str = r#"PROJCS["RGF_1993_Lambert_93",GEOGCS["GCS_RGF_1993",DATUM["D_RGF_1993",SPHEROID["GRS_1980",6378137.0,298.257222101]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Lambert_Conformal_Conic"],PARAMETER["False_Easting",700000.0],PARAMETER["False_Northing",6600000.0],PARAMETER["Central_Meridian",3.0],PARAMETER["Standard_Parallel_1",49.0],PARAMETER["Standard_Parallel_2",44.0],PARAMETER["Latitude_Of_Origin",46.5],UNIT["Meter",1.0]]"#;

/// The byte ranges of the shape type and the extent in the `.shp` and `.shx`
/// headers.
const SHAPE_TYPE_BYTES: std::ops::Range<usize> = 32..36;
const EXTENT_BYTES: std::ops::Range<usize> = 36..100;

/// The fields of a contour shapefile: `ID` and `elev`, as `gdal_contour -a
/// elev` defines them.
pub fn contour_table() -> TableWriterBuilder {
    TableWriterBuilder::new()
        .add_numeric_field(FieldName::try_from("ID").expect("valid field name"), 8, 0)
        .add_numeric_field(
            FieldName::try_from("elev").expect("valid field name"),
            24,
            15,
        )
}

/// A record of [`contour_table`].
pub struct ContourRecord {
    pub id: f64,
    pub elevation: f64,
}

impl WritableRecord for ContourRecord {
    fn write_using<W: Write>(
        &self,
        field_writer: &mut FieldWriter<'_, W>,
    ) -> std::result::Result<(), FieldIOError> {
        field_writer.write_next_field_value(&self.id)?;
        field_writer.write_next_field_value(&self.elevation)
    }
}

/// Writes `lines` to the shapefile at `path`, as `gdal_contour -a elev`
/// does: a `LineString` layer with fields `ID` (from 0) and `elev`, and an
/// EPSG:2154 `.prj`.
pub fn write_contour_shapefile(path: &Path, lines: &[ContourLine]) -> Result<()> {
    let context = || format!("could not write contour shapefile `{}`", path.display());

    // The `shapefile` and `dbase` writers write their headers when dropped,
    // and drop the errors, so they write to memory, where it cannot fail.
    let (mut shp, mut shx, mut dbf) = (Vec::new(), Vec::new(), Vec::new());
    {
        let mut writer = shapefile::Writer::new(
            ShapeWriter::with_shx(Cursor::new(&mut shp), Cursor::new(&mut shx)),
            contour_table().build_with_dest(Cursor::new(&mut dbf)),
        );
        for (id, line) in lines.iter().enumerate() {
            let points = line.points.iter().map(|&(x, y)| Point::new(x, y)).collect();
            let record = ContourRecord {
                id: id as f64,
                elevation: line.elevation,
            };
            writer
                .write_shape_and_record(&Polyline::new(points), &record)
                .context(context())?;
        }
    }
    // The shape type is set by the first shape, so an empty layer would be a
    // null shape one, with the no-data M range. OGR writes a polyline layer
    // with a zero extent.
    if lines.is_empty() {
        for header in [&mut shp, &mut shx] {
            header[SHAPE_TYPE_BYTES].copy_from_slice(&(ShapeType::Polyline as i32).to_le_bytes());
            header[EXTENT_BYTES].fill(0);
        }
    }

    for (extension, contents) in [
        ("shp", &shp[..]),
        ("shx", &shx[..]),
        ("dbf", &dbf[..]),
        ("prj", LAMBERT_93_ESRI_WKT.as_bytes()),
    ] {
        let file_path = path.with_extension(extension);
        fs::write(&file_path, contents)
            .context(format!("could not write `{}`", file_path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shapefile::{dbase::FieldValue, Reader};

    fn temp_shapefile(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cassini-contour-shapefile-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn writes_lines_with_id_and_elevation() {
        let path = temp_shapefile("contours.shp");
        let lines = [
            ContourLine {
                elevation: 2.5,
                points: vec![(0.0, 1.0), (2.0, 3.0)],
            },
            ContourLine {
                elevation: 5.0,
                points: vec![(4.0, 5.0), (6.0, 7.0), (8.0, 9.0)],
            },
        ];
        write_contour_shapefile(&path, &lines).unwrap();

        let mut reader = Reader::from_path(&path).unwrap();
        let read: Vec<(Polyline, shapefile::dbase::Record)> = reader
            .iter_shapes_and_records_as::<Polyline, shapefile::dbase::Record>()
            .map(|shape_record| shape_record.unwrap())
            .collect();
        let prj = fs::read_to_string(path.with_extension("prj")).unwrap();
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        assert_eq!(read.len(), 2);
        for (index, ((polyline, record), line)) in read.iter().zip(&lines).enumerate() {
            let points: Vec<(f64, f64)> = polyline.parts()[0].iter().map(|p| (p.x, p.y)).collect();
            assert_eq!(points, line.points);
            assert_eq!(
                record.get("ID"),
                Some(&FieldValue::Numeric(Some(index as f64)))
            );
            assert_eq!(
                record.get("elev"),
                Some(&FieldValue::Numeric(Some(line.elevation)))
            );
        }
        assert_eq!(prj, LAMBERT_93_ESRI_WKT);
    }

    #[test]
    fn writes_an_empty_layer_as_polylines() {
        let path = temp_shapefile("empty.shp");
        write_contour_shapefile(&path, &[]).unwrap();
        let shp = fs::read(&path).unwrap();
        let shx = fs::read(path.with_extension("shx")).unwrap();
        let reader = Reader::from_path(&path).unwrap();
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        // As OGR writes it: a 100-byte header, polyline type, empty extent.
        for file in [&shp, &shx] {
            assert_eq!(file.len(), 100);
            assert_eq!(file[SHAPE_TYPE_BYTES], 3i32.to_le_bytes());
            assert!(file[EXTENT_BYTES].iter().all(|&byte| byte == 0));
        }
        assert_eq!(reader.header().shape_type, ShapeType::Polyline);
    }
}
