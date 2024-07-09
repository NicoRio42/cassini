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
use las::raw::Header;
use lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file;
use png::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file;
use std::{fs::File, path::Path, time::Instant};
use tile::{NeighborTiles, Tile};

fn main() {
    let args = Args::parse();

    if args.batch {
        let start = Instant::now();
        let number_of_threads = args.threads.unwrap_or(1);
        batch(number_of_threads, args.skip_lidar);
        let duration = start.elapsed();
        println!("Tiles generated in {:.1?}", duration);
        return;
    }

    let start = Instant::now();
    let laz_path = Path::new("in").join("LHD_FXX_0615_6163_PTS_C_LAMB93_IGN69.copc.laz");
    let dir_path = Path::new("out").join("test");

    generate_dem_and_vegetation_density_tiff_images_from_laz_file(&laz_path, &dir_path);

    let mut file = File::open(&laz_path).expect("Cound not open laz file");
    let header = Header::read_from(&mut file).unwrap();

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
        Tile {
            dir_path,
            laz_path,
            min_x: header.min_x as i64,
            min_y: header.min_y as i64,
            max_x: header.max_x as i64,
            max_y: header.max_y as i64,
        },
        NeighborTiles {
            top: None,
            top_right: None,
            right: None,
            bottom_right: None,
            bottom: None,
            bottom_left: None,
            left: None,
            top_left: None,
        },
    );

    let duration = start.elapsed();
    println!("Tiles generated in {:.1?}", duration);
    // TODO implement single tile
}
