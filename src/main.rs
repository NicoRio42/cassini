mod canvas;
mod cli;
mod cliffs;
mod config;
mod constants;
mod contours;
mod full_map;
mod lidar;
mod metadata;
mod utils;
mod vegetation;

use clap::Parser;
use cli::Args;
use cliffs::render_cliffs;
use config::get_config;
use contours::render_contours_to_png;
use full_map::render_full_map_to_png;
use lidar::process_lidar;
use metadata::get_metadata;
use vegetation::render_vegetation;

fn main() {
    let args = Args::parse();

    if !args.skip_lidar_processing {
        process_lidar()
    } else {
        println!("Skipping LiDAR processing.");
    }

    let metadata = get_metadata();
    println!("{}", metadata.stages.filters_info.bbox.maxx);

    let config = get_config();
    println!("{}", config.dem_resolution);

    render_vegetation();
    render_cliffs();
    render_contours_to_png();
    render_full_map_to_png();
}
