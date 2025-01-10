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
use clap::{CommandFactory, Parser};
use cli::{Args, Commands};
use config::generate_default_config;
use constants::INCH;
use download::download_osm_file_if_needed;
use las::raw::Header;
use lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file;
use png::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file;
use std::{fs::File, path::Path, time::Instant};
use tile::{NeighborTiles, Tile};

fn main() {
    let args = Args::parse();

    if std::env::args().len() == 1 {
        Args::command().print_help().unwrap();
        return;
    }

    if let Some(command) = args.command {
        match command {
            Commands::DefaultConfig {} => {
                generate_default_config();
            }

            Commands::Process {
                file_path,
                output_dir,
                skip_vector,
            } => {
                let start = Instant::now();
                let laz_path = Path::new(&file_path);
                let dir_path = Path::new(&output_dir);

                generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                    &laz_path.to_path_buf(),
                    &dir_path.to_path_buf(),
                );

                let mut file = File::open(&laz_path).expect("Cound not open laz file");
                let header = Header::read_from(&mut file).unwrap();

                let tile = Tile {
                    dir_path: dir_path.to_path_buf(),
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

                if skip_vector {
                    download_osm_file_if_needed(tile.min_x, tile.min_y, tile.max_x, tile.max_y);
                }

                generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
                    tile,
                    neighbor_tiles,
                    skip_vector,
                );

                let duration = start.elapsed();
                println!("Tile generated in {:.1?}", duration);
            }

            Commands::Lidar {
                file_path,
                output_dir,
            } => {}

            Commands::Render {
                input_dir,
                output_dir,
                neighbors,
                skip_vector,
            } => {}

            Commands::Batch {
                input_dir,
                output_dir,
                threads,
                skip_lidar,
                skip_vector,
            } => {
                let start = Instant::now();
                batch(&input_dir, &output_dir, threads, skip_lidar, skip_vector);
                let duration = start.elapsed();
                println!("Tiles generated in {:.1?}", duration);
            }
        }
    }
}
