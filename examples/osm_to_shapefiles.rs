//! Converts an OpenStreetMap XML file to the `lines.shp` and
//! `multipolygons.shp` shapefiles the render step reads.
//!
//! ```text
//! cargo run --release --example osm_to_shapefiles -- tile.osm output_dir
//! ```

use cassini::osm_import::convert_osm_file_to_shapefiles;
use clap::Parser;
use std::{path::PathBuf, process::ExitCode, time::Instant};

#[derive(Parser)]
struct Args {
    osm: PathBuf,
    output_dir: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let start = Instant::now();
    match convert_osm_file_to_shapefiles(&args.osm, &args.output_dir) {
        Ok(()) => {
            println!("converted in {:.1?}", start.elapsed());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}
