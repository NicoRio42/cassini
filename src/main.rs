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
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::{fs::File, path::Path, time::Instant};
use tile::{get_extent_from_lidar_dir_path, Tile};

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
                output_dir: maybe_output_dir,
                skip_vector,
            } => {
                let start = Instant::now();
                let output_dir = maybe_output_dir.unwrap_or("tile".to_owned());
                let laz_path = Path::new(&file_path);
                let dir_path = Path::new(&output_dir);

                generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                    &laz_path.to_path_buf(),
                    &dir_path.to_path_buf(),
                );

                let mut file = File::open(&laz_path).expect("Cound not open laz file");
                let header = Header::read_from(&mut file).unwrap();

                let tile = Tile {
                    lidar_dir_path: dir_path.to_path_buf(),
                    render_dir_path: dir_path.to_path_buf(),
                    min_x: header.min_x.round() as i64,
                    min_y: header.min_y.round() as i64,
                    max_x: header.max_x.round() as i64,
                    max_y: header.max_y.round() as i64,
                };

                if !skip_vector {
                    download_osm_file_if_needed(tile.min_x, tile.min_y, tile.max_x, tile.max_y);
                }

                generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
                    tile,
                    vec![],
                    skip_vector,
                );

                let duration = start.elapsed();
                println!("Tile generated in {:.1?}", duration);
            }

            Commands::Lidar {
                file_path,
                output_dir: maybe_output_dir,
            } => {
                let start = Instant::now();
                let output_dir = maybe_output_dir.unwrap_or("lidar".to_owned());
                let laz_path = Path::new(&file_path);
                let dir_path = Path::new(&output_dir);

                generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                    &laz_path.to_path_buf(),
                    &dir_path.to_path_buf(),
                );

                let duration = start.elapsed();
                println!("LiDAR file processed in {:.1?}", duration);
            }

            Commands::Render {
                input_dir,
                output_dir: maybe_output_dir,
                neighbors,
                skip_vector,
            } => {
                let start = Instant::now();
                let output_dir = maybe_output_dir.unwrap_or("tile".to_owned());
                let input_dir_path = Path::new(&input_dir);
                let output_dir_path = Path::new(&output_dir);
                create_dir_all(&output_dir_path).expect("Could not create out dir");

                let (min_x, min_y, max_x, max_y) =
                    get_extent_from_lidar_dir_path(&input_dir_path.to_path_buf());

                let tile = Tile {
                    lidar_dir_path: input_dir_path.to_path_buf(),
                    render_dir_path: output_dir_path.to_path_buf(),
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                };

                if !skip_vector {
                    download_osm_file_if_needed(tile.min_x, tile.min_y, tile.max_x, tile.max_y);
                }

                let mut neighbor_tiles: Vec<PathBuf> = vec![];

                for neighbor in neighbors {
                    let neighbor_path = Path::new(&neighbor).to_path_buf();

                    if !neighbor_path.exists() {
                        panic!("{} does not exist", neighbor)
                    }

                    neighbor_tiles.push(neighbor_path);
                }

                generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
                    tile,
                    neighbor_tiles,
                    skip_vector,
                );

                let duration = start.elapsed();
                println!("Tile generated in {:.1?}", duration);
            }

            Commands::Batch {
                input_dir: maybe_input_dir,
                output_dir: maybe_output_dir,
                threads: maybe_threads,
                skip_lidar,
                skip_vector,
            } => {
                let start = Instant::now();
                let input_dir = maybe_input_dir.unwrap_or("in".to_owned());
                let output_dir = maybe_output_dir.unwrap_or("out".to_owned());
                let threads = maybe_threads.unwrap_or(3);
                batch(&input_dir, &output_dir, threads, skip_lidar, skip_vector);
                let duration = start.elapsed();
                println!("Tiles generated in {:.1?}", duration);
            }
        }
    }
}
