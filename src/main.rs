mod batch;
mod buffer;
mod canvas;
mod cli;
mod cliffs;
mod config;
mod constants;
mod contours;
mod dem;
mod download;
mod fill_nodata;
mod full_map;
mod lidar;
mod merge;
mod png;
mod pullautin_contours_render;
mod pullautin_smooth_contours;
mod tile;
mod vectors;
mod vegetation;

use batch::batch;
use clap::Parser;
use cli::Args;
use config::generate_default_config;
use constants::INCH;
use las::raw::Header;
use lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file;
use png::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file;
use std::{fs::File, path::Path, time::Instant};
use tile::{NeighborTiles, Tile};

fn main() {
    let args = Args::parse();

    if args.default_config {
        generate_default_config();
        return;
    }

    if args.batch {
        let start = Instant::now();
        let number_of_threads = args.threads.unwrap_or(3);
        batch(number_of_threads, args.skip_lidar);
        let duration = start.elapsed();
        println!("Tiles generated in {:.1?}", duration);

        return;
    }

    if let Some(file_name) = args.file_path.as_deref() {
        let start = Instant::now();
        let laz_path = Path::new(file_name);
        let dir_path = Path::new("out").join("tile");

        if !args.skip_lidar {
            generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                &laz_path.to_path_buf(),
                &dir_path,
            );
        }

        let mut file = File::open(&laz_path).expect("Cound not open laz file");
        let header = Header::read_from(&mut file).unwrap();

        let tile = Tile {
            dir_path,
            laz_path: laz_path.to_path_buf(),
            min_x: header.min_x.round() as i64,
            min_y: header.min_y.round() as i64,
            max_x: header.max_x.round() as i64,
            max_y: header.max_y.round() as i64,
        };

        let neighbor_tiles = NeighborTiles {
            top: None,
            top_right: None,
            right: None,
            bottom_right: None,
            bottom: None,
            bottom_left: None,
            left: None,
            top_left: None,
        };

        generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(tile, neighbor_tiles);

        let duration = start.elapsed();
        println!("Tile generated in {:.1?}", duration);
    }
}
