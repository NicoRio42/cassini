mod bezier;
mod canvas;
mod cli;
mod cliffs;
mod config;
mod constants;
mod contours;
mod download;
mod full_map;
mod lidar;
mod pipeline;
mod tile_list;
mod utils;
mod vectors;
mod vegetation;

use clap::Parser;
use cli::Args;
use cliffs::render_cliffs;
use config::get_config;
use constants::INCH;
use contours::render_contours_to_png;
use download::{download_laz_files_if_needed, download_osm_file_if_needed};
use full_map::render_full_map_to_png;
use lidar::process_lidar;
use pipeline::generate_pipeline_for_single_tile;
use std::{
    fs::{self, create_dir_all},
    path::Path,
    time::Instant,
};
use tile_list::get_tile_list_from_extent;
use utils::delete_dir_contents;
use vectors::render_vector_shapes;
use vegetation::render_vegetation;

fn main() {
    let args = Args::parse();

    let tile_list = get_tile_list_from_extent(616112, 6163204, 616692, 6163693);

    // TODO: multithreading
    for (min_x, min_y, max_x, max_y) in tile_list {
        let start = Instant::now();
        download_laz_files_if_needed(min_x, min_y, max_x, max_y, "JS".to_owned());
        download_osm_file_if_needed(min_x, min_y, max_x, max_y);
        process_sigle_tile(min_x, min_y, max_x, max_y, args.skip_lidar);
        let duration = start.elapsed();
        println!("Tile {} {} generated in {:.1?}", min_x, max_y, duration);
    }
}

fn process_sigle_tile(min_x: u64, min_y: u64, max_x: u64, max_y: u64, skip_lidar: bool) {
    let buffer = 200;
    let out_dir = Path::new("out").join(format!("{:0>7}_{:0>7}", min_x, max_y));

    if !skip_lidar {
        delete_dir_contents(fs::read_dir(&out_dir));
        create_dir_all(&out_dir).expect("Could not create out dir");
        generate_pipeline_for_single_tile(min_x, min_y, max_x, max_y, buffer, &out_dir);
        process_lidar(&out_dir)
    } else {
        println!("Skipping LiDAR processing.");
    }

    let config = get_config();

    let image_width = ((max_x - min_x) as f32 * config.dpi_resolution / INCH) as u32;
    let image_height = ((max_y - min_y) as f32 * config.dpi_resolution / INCH) as u32;

    render_vegetation(image_width, image_height, buffer, &config, &out_dir);
    render_cliffs(image_width, image_height, buffer, &config, &out_dir);
    render_contours_to_png(image_width, image_height, &config, min_x, min_y, &out_dir);

    let osm_path = Path::new("in").join(format!("{:0>7}_{:0>7}.osm", min_x, max_y));

    render_vector_shapes(
        image_width,
        image_height,
        &config,
        min_x,
        min_y,
        &out_dir,
        &osm_path,
    );

    render_full_map_to_png(image_width, image_height, &out_dir);
}
