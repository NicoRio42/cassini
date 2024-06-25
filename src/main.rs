mod canvas;
mod cli;
mod cliffs;
mod config;
mod constants;
mod contours;
mod full_map;
mod lidar;
mod metadata;
mod process_tile;
mod utils;
mod vectors;
mod vegetation;

use clap::Parser;
use cli::Args;
use cliffs::render_cliffs;
use config::get_config;
use constants::INCH;
use contours::render_contours_to_png;
use full_map::render_full_map_to_png;
use lidar::process_lidar;
use metadata::get_metadata;
use process_tile::process_single_tile;
use vectors::render_vector_shapes;
use vegetation::render_vegetation;

fn main() {
    process_single_tile(615500, 6163500, 616500, 6164500);
    // let args = Args::parse();

    // if !args.skip_lidar_processing {
    //     process_lidar()
    // } else {
    //     println!("Skipping LiDAR processing.");
    // }

    // let metadata = get_metadata();
    // let config = get_config();

    // let image_width =
    //     ((metadata.stages.filters_info.bbox.maxx - metadata.stages.filters_info.bbox.minx).round()
    //         * config.dpi_resolution
    //         / INCH) as u32;

    // let image_height =
    //     ((metadata.stages.filters_info.bbox.maxy - metadata.stages.filters_info.bbox.miny).round()
    //         * config.dpi_resolution
    //         / INCH) as u32;

    // render_vegetation(image_width, image_height, &config);
    // render_cliffs(image_width, image_height, &config);
    // render_contours_to_png(image_width, image_height, &config, &metadata);
    // render_vector_shapes(image_width, image_height, &config, &metadata);
    // render_full_map_to_png(image_width, image_height);
}
