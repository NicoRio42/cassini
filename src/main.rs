mod batch;
mod bezier;
mod buffer;
mod canvas;
mod cli;
mod cliffs;
mod config;
mod constants;
mod contours;
mod dem;
mod download;
mod full_map;
mod lidar;
mod merge;
mod png;
mod tile;
mod utils;
mod vectors;
mod vegetation;

use batch::batch;
use clap::Parser;
use cli::Args;
use constants::INCH;
use std::time::Instant;

fn main() {
    let args = Args::parse();

    if args.batch {
        let start = Instant::now();
        let number_of_threads = args.threads.unwrap_or(3);
        batch(number_of_threads, args.skip_lidar);
        let duration = start.elapsed();
        println!("Tiles generated in {:.1?}", duration);
        return;
    }

    // TODO implement single tile
}
