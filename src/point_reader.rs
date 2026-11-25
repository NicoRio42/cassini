//! Reads the points of a LAS or LAZ file in file order, decoding only the
//! fields Cassini uses.
//!
//! LiDAR HD's LAZ files use the layered point formats of LAS 1.4, whose
//! fields are compressed separately: only X, Y, Z and the classification are
//! decompressed, the other fields of the records are left at zero. The
//! legacy formats 0 to 5 are compressed point by point, so all their fields
//! are decompressed. Uncompressed LAS files are read with `las::Reader`.
//!
//! The records are those `las::Reader` reads, in the same order, so the
//! points are the same: they are read with `las::PointData`'s accessors.

use las::{Header, PointData, PointDataBuilder, Reader};
use laz::{DecompressionSelection, ParLasZipDecompressor};
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::{CassiniError, Result, ResultContext};

/// The fields decompressed from layered LAZ records. X and Y are always
/// decompressed.
fn selection() -> DecompressionSelection {
    DecompressionSelection::xy_returns_channel()
        .decompress_z()
        .decompress_classification()
}

/// A reader of the points of a LAS or LAZ file.
pub struct PointReader {
    path: PathBuf,
    header: Header,
    source: PointSource,
}

enum PointSource {
    /// LAZ chunks are decompressed on rayon's global thread pool, which the
    /// tiles of a batch share, so the batch threads do not oversubscribe the
    /// CPU.
    Laz {
        decompressor: Box<ParLasZipDecompressor<BufReader<File>>>,
        /// The number of points not read yet. The decompressor must not be
        /// asked for more.
        remaining: u64,
    },
    Las(Box<Reader>),
}

impl PointReader {
    pub fn open(path: &Path) -> Result<Self> {
        let header_context = || format!("could not read LiDAR header `{}`", path.display());
        let file =
            File::open(path).context(format!("could not open LiDAR file `{}`", path.display()))?;
        let mut read = BufReader::new(file);
        // Leaves `read` at the start of the point records.
        let header = Header::new(&mut read).context(header_context())?;

        let source = if header.point_format().is_compressed {
            let vlr = header.laz_vlr().context(header_context())?;
            let record_len = PointDataBuilder::new()
                .for_header(&header)
                .build()
                .record_len();
            if vlr.items_size() != record_len as u64 {
                return Err(CassiniError::InvalidInput {
                    message: format!(
                        "LiDAR file `{}` has {}-byte compressed records for a {}-byte point \
                         format",
                        path.display(),
                        vlr.items_size(),
                        record_len
                    ),
                });
            }
            let decompressor = ParLasZipDecompressor::selective(read, vlr, selection())
                .context(header_context())?;
            PointSource::Laz {
                decompressor: Box::new(decompressor),
                remaining: header.number_of_points(),
            }
        } else {
            read.seek(SeekFrom::Start(0)).context(header_context())?;
            PointSource::Las(Box::new(Reader::new(read).context(header_context())?))
        };

        Ok(Self {
            path: path.to_path_buf(),
            header,
            source,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// An empty `PointData` for the points of this file.
    pub fn point_data(&self) -> PointData {
        PointDataBuilder::new().for_header(&self.header).build()
    }

    /// Replaces `points`, which must come from `point_data`, with the next
    /// `n` points at most, returning how many were read, 0 at the end of the
    /// file.
    pub fn fill_points(&mut self, n: u64, points: &mut PointData) -> Result<u64> {
        let context = || format!("could not read LiDAR points `{}`", self.path.display());
        match &mut self.source {
            PointSource::Laz {
                decompressor,
                remaining,
            } => {
                let count = n.min(*remaining);
                let len = usize::try_from(count).context(context())?;
                decompressor
                    .decompress_many(points.resize_for(len))
                    .context(context())?;
                *remaining -= count;
                Ok(count)
            }
            PointSource::Las(reader) => reader.fill_points(n, points).context(context()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use las::{
        point::{Classification, Format},
        Builder, Color, Point, Transform, Vector, Writer,
    };

    /// Writes `points` to `path` in point format `format`, compressed if
    /// `path` ends with `.laz`, with LiDAR HD's 1 cm scale.
    fn write_points(path: &Path, format: u8, points: &[Point]) {
        let mut builder = Builder::from((1, 4));
        builder.point_format = Format::new(format).unwrap();
        builder.point_format.is_compressed = path.extension().unwrap() == "laz";
        let transform = |offset| Transform {
            scale: 0.01,
            offset,
        };
        builder.transforms = Vector {
            x: transform(900_000.0),
            y: transform(6_400_000.0),
            z: transform(0.0),
        };
        let header = builder.into_header().unwrap();
        let mut writer = Writer::from_path(path, header).unwrap();
        for point in points {
            writer.write_point(point.clone()).unwrap();
        }
        writer.close().unwrap();
    }

    /// 1234 points with every field set, so that a field decoded from the
    /// wrong bytes shows.
    fn points(format: &Format) -> Vec<Point> {
        (0..1234)
            .map(|i| {
                let mut point = Point {
                    x: 900_000.0 + (i * 37 % 1000) as f64 * 0.01,
                    y: 6_400_000.0 + (i * 53 % 1000) as f64 * 0.01,
                    z: 100.0 + (i % 97) as f64 * 0.01,
                    intensity: (i * 7) as u16,
                    return_number: (i % 4 + 1) as u8,
                    number_of_returns: 4,
                    classification: Classification::new((i % 7 + 1) as u8).unwrap(),
                    user_data: (i % 251) as u8,
                    point_source_id: (i % 13) as u16,
                    scan_angle: (i % 30) as f32,
                    ..Default::default()
                };
                if format.has_gps_time {
                    point.gps_time = Some(i as f64 * 0.5);
                }
                if format.has_color {
                    point.color = Some(Color::new(i as u16, 2 * i as u16, 3 * i as u16));
                }
                if format.has_nir {
                    point.nir = Some(i as u16);
                }
                point
            })
            .collect()
    }

    type Fields = Vec<(f64, f64, f64, u8)>;

    fn fields(points: &PointData) -> Fields {
        points
            .x()
            .zip(points.y())
            .zip(points.z())
            .zip(points.classification())
            .map(|(((x, y), z), classification)| (x, y, z, classification))
            .collect()
    }

    /// X, Y, Z and the classification of every point of `path`, read `n` at a
    /// time.
    fn read_fields(path: &Path, n: u64) -> Fields {
        let mut reader = PointReader::open(path).unwrap();
        let mut points = reader.point_data();
        let mut all = vec![];
        while reader.fill_points(n, &mut points).unwrap() > 0 {
            all.extend(fields(&points));
        }
        all
    }

    #[test]
    fn reads_the_same_points_as_las_reader() {
        let dir = tempfile::tempdir().unwrap();
        for format in [1, 3, 6, 7, 8] {
            let points = points(&Format::new(format).unwrap());
            for extension in ["laz", "las"] {
                let path = dir.path().join(format!("points-{format}.{extension}"));
                write_points(&path, format, &points);
                let mut reader = Reader::from_path(&path).unwrap();
                let mut expected = PointDataBuilder::new().for_header(reader.header()).build();
                assert_eq!(reader.fill_points(5000, &mut expected).unwrap(), 1234);
                let expected = fields(&expected);
                assert!(expected
                    .iter()
                    .any(|&(_, _, _, classification)| classification == 6));

                // Batches smaller than the file, and larger.
                for n in [7, 1000, 5000] {
                    assert_eq!(read_fields(&path, n), expected, "{} by {n}", path.display());
                }

                // The other fields of layered records are not decompressed.
                let mut reader = PointReader::open(&path).unwrap();
                let mut points = reader.point_data();
                reader.fill_points(5000, &mut points).unwrap();
                let skipped = extension == "laz" && format >= 6;
                assert_eq!(
                    points.intensity().all(|intensity| intensity == 0),
                    skipped,
                    "{}",
                    path.display()
                );
            }
        }
    }
}
