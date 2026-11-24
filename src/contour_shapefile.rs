//! The contour shapefiles: the layer `gdal_contour -a elev` writes, whose
//! fields the smoothed contours and the form lines keep.

use std::{io::Write, path::Path};

use cassini::shapefile_output::write_shapefile;
use shapefile::{
    dbase::{FieldIOError, FieldName, FieldWriter, TableWriterBuilder, WritableRecord},
    Point, Polyline, ShapeType,
};

use crate::{error::Result, marching_squares::ContourLine};

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
    let features = lines.iter().enumerate().map(|(id, line)| {
        let points = line.points.iter().map(|&(x, y)| Point::new(x, y)).collect();
        let record = ContourRecord {
            id: id as f64,
            elevation: line.elevation,
        };
        (Polyline::new(points), record)
    });
    write_shapefile(path, ShapeType::Polyline, contour_table(), features)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cassini::shapefile_output::LAMBERT_93_ESRI_WKT;
    use shapefile::{dbase::FieldValue, Reader};
    use std::fs;

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
}
