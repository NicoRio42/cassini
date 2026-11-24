//! Copies the points of a LAS or LAZ file that fall in a rectangle to a new
//! file, for small test fixtures. A point is kept when
//! `min_x <= x < max_x` and `min_y <= y < max_y`, so the output's header
//! bounds round to the rectangle. The output keeps the point format, scales,
//! offsets and CRS records of the input, but not its COPC records: it is a
//! plain LAZ file (or LAS, from the output's extension).
//!
//! ```text
//! cargo run --release --example crop_laz -- in.copc.laz out.laz --bbox min_x,min_y,max_x,max_y
//! ```

use clap::Parser;
use las::{Builder, PointDataBuilder, Reader, Writer};
use std::{path::PathBuf, process::ExitCode, time::Instant};

/// The number of points decompressed at a time.
const POINTS_PER_BATCH: u64 = 1 << 21;

#[derive(Parser)]
struct Args {
    input: PathBuf,
    output: PathBuf,
    /// The rectangle to keep, `min_x,min_y,max_x,max_y`.
    #[arg(long, value_parser = parse_bbox)]
    bbox: [f64; 4],
}

fn parse_bbox(value: &str) -> Result<[f64; 4], String> {
    let bounds = value
        .split(',')
        .map(|bound| {
            bound
                .trim()
                .parse::<f64>()
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bbox: [f64; 4] = bounds
        .try_into()
        .map_err(|_| "expected min_x,min_y,max_x,max_y".to_owned())?;
    if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
        return Err("the rectangle is empty".to_owned());
    }
    Ok(bbox)
}

fn main() -> ExitCode {
    let args = Args::parse();
    let start = Instant::now();
    match crop(&args) {
        Ok((kept, read)) => {
            println!("kept {kept} of {read} points in {:.1?}", start.elapsed());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn crop(args: &Args) -> las::Result<(u64, u64)> {
    let [min_x, min_y, max_x, max_y] = args.bbox;
    let mut reader = Reader::from_path(&args.input)?;
    let input_header = reader.header();

    let mut builder = Builder::from((1, 4));
    builder.point_format = *input_header.point_format();
    builder.transforms = *input_header.transforms();
    builder.system_identifier = input_header.system_identifier().to_owned();
    builder.generating_software = "cassini crop_laz example".to_owned();
    builder.date = input_header.date();
    builder.file_source_id = input_header.file_source_id();
    builder.gps_time_type = input_header.gps_time_type();
    builder.has_wkt_crs = input_header.has_wkt_crs();
    builder.vlrs = input_header
        .vlrs()
        .iter()
        .chain(input_header.evlrs())
        .filter(|vlr| vlr.user_id == "LASF_Projection")
        .cloned()
        .collect();
    let mut writer = Writer::from_path(&args.output, builder.into_header()?)?;

    let (mut kept, mut read) = (0, 0);
    let mut points = PointDataBuilder::new().for_header(reader.header()).build();
    loop {
        let count = reader.fill_points(POINTS_PER_BATCH, &mut points)?;
        if count == 0 {
            break;
        }
        read += count;
        // The records are copied as they are, so every field is kept.
        let record_len = points.record_len();
        let mut bytes = Vec::new();
        for ((x, y), record) in points
            .x()
            .zip(points.y())
            .zip(points.raw_bytes().chunks_exact(record_len))
        {
            if (min_x..max_x).contains(&x) && (min_y..max_y).contains(&y) {
                bytes.extend_from_slice(record);
            }
        }
        kept += (bytes.len() / record_len) as u64;
        let kept_points = PointDataBuilder::new()
            .for_header(reader.header())
            .build_from_bytes(bytes)?;
        writer.write_points(&kept_points)?;
    }
    writer.close()?;
    Ok((kept, read))
}
