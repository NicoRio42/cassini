//! Shapefile output shared by the contour and OpenStreetMap layers: the
//! files OGR writes for an EPSG:2154 layer.

use std::{fs, io::Cursor, path::Path};

use shapefile::{
    dbase::{TableWriterBuilder, WritableRecord},
    record::EsriShape,
    ShapeWriter,
};

use crate::error::{Result, ResultContext};

/// The `.prj` OGR writes for EPSG:2154.
pub const LAMBERT_93_ESRI_WKT: &str = r#"PROJCS["RGF_1993_Lambert_93",GEOGCS["GCS_RGF_1993",DATUM["D_RGF_1993",SPHEROID["GRS_1980",6378137.0,298.257222101]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Lambert_Conformal_Conic"],PARAMETER["False_Easting",700000.0],PARAMETER["False_Northing",6600000.0],PARAMETER["Central_Meridian",3.0],PARAMETER["Standard_Parallel_1",49.0],PARAMETER["Standard_Parallel_2",44.0],PARAMETER["Latitude_Of_Origin",46.5],UNIT["Meter",1.0]]"#;

/// The byte ranges of the shape type and the extent in the `.shp` and `.shx`
/// headers.
const SHAPE_TYPE_BYTES: std::ops::Range<usize> = 32..36;
const EXTENT_BYTES: std::ops::Range<usize> = 36..100;

/// Writes `features` to the shapefile at `path` (`.shp`, `.shx`, `.dbf`)
/// with the fields of `table`, and an EPSG:2154 `.prj`.
///
/// An empty layer is written as OGR writes it: with the shape type of `S`,
/// and a zero extent.
pub fn write_shapefile<S: EsriShape, R: WritableRecord>(
    path: &Path,
    table: TableWriterBuilder,
    features: impl IntoIterator<Item = (S, R)>,
) -> Result<()> {
    let context = || format!("could not write shapefile `{}`", path.display());

    // The `shapefile` and `dbase` writers write their headers when dropped,
    // and drop the errors, so they write to memory, where it cannot fail.
    let (mut shp, mut shx, mut dbf) = (Vec::new(), Vec::new(), Vec::new());
    let mut is_empty = true;
    {
        let mut writer = shapefile::Writer::new(
            ShapeWriter::with_shx(Cursor::new(&mut shp), Cursor::new(&mut shx)),
            table.build_with_dest(Cursor::new(&mut dbf)),
        );
        for (shape, record) in features {
            writer
                .write_shape_and_record(&shape, &record)
                .context(context())?;
            is_empty = false;
        }
    }
    // The shape type is set by the first shape, so an empty layer would be a
    // null shape one, with the no-data M range.
    if is_empty {
        for header in [&mut shp, &mut shx] {
            header[SHAPE_TYPE_BYTES].copy_from_slice(&(S::shapetype() as i32).to_le_bytes());
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
    use shapefile::{dbase::Record, Polyline, Reader, ShapeType};

    #[test]
    fn writes_an_empty_layer_with_its_shape_type() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.shp");

        write_shapefile::<Polyline, Record>(&path, TableWriterBuilder::new(), []).unwrap();
        let shp = fs::read(&path).unwrap();
        let shx = fs::read(path.with_extension("shx")).unwrap();
        let prj = fs::read_to_string(path.with_extension("prj")).unwrap();
        let reader = Reader::from_path(&path).unwrap();

        // As OGR writes it: a 100-byte header, polyline type, empty extent.
        for file in [&shp, &shx] {
            assert_eq!(file.len(), 100);
            assert_eq!(file[SHAPE_TYPE_BYTES], 3i32.to_le_bytes());
            assert!(file[EXTENT_BYTES].iter().all(|&byte| byte == 0));
        }
        assert_eq!(reader.header().shape_type, ShapeType::Polyline);
        assert_eq!(prj, LAMBERT_93_ESRI_WKT);
    }
}
