mod batch;
mod bezier;
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
mod png;
mod tile;
mod utils;
mod vectors;
mod vegetation;

use crate::lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file;
use batch::batch;
use clap::Parser;
use cli::Args;
use config::get_config;
use constants::INCH;
use png::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};
use tile::{NeighborTiles, Tile};

fn main() {
    let args = Args::parse();
    let config = get_config();

    if args.batch {
        batch(3);
        return;
    }

    let start = Instant::now();

    let tile = "LHD_FXX_0616_6164_PTS_C_LAMB93_IGN69.copc.laz";
    let top = "LHD_FXX_0616_6165_PTS_C_LAMB93_IGN69.copc.laz";
    let top_right = "LHD_FXX_0617_6165_PTS_C_LAMB93_IGN69.copc.laz";
    let right = "LHD_FXX_0617_6164_PTS_C_LAMB93_IGN69.copc.laz";
    let bottom_right = "LHD_FXX_0617_6163_PTS_C_LAMB93_IGN69.copc.laz";
    let bottom = "LHD_FXX_0616_6163_PTS_C_LAMB93_IGN69.copc.laz";
    let bottom_left = "LHD_FXX_0615_6163_PTS_C_LAMB93_IGN69.copc.laz";
    let left = "LHD_FXX_0615_6164_PTS_C_LAMB93_IGN69.copc.laz";
    let top_left = "LHD_FXX_0615_6165_PTS_C_LAMB93_IGN69.copc.laz";

    let tiles_first_batch = [tile, top, top_right, right, bottom_right];

    let tiles_second_batch = [bottom, bottom_left, left, top_left];

    let mut handles: Vec<thread::JoinHandle<()>> = Vec::with_capacity(2usize);

    handles.push(thread::spawn(move || {
        for tile_filename in tiles_first_batch {
            println!("{}", tile_filename);
            let (min_x, min_y, max_x, max_y) =
                get_bounds_from_lidar_hd_tile_filename(&tile_filename);

            generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                &Path::new("in").join(tile_filename),
                &Path::new("out").join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)),
            );
        }
        thread::sleep(Duration::from_millis(1));
    }));

    thread::sleep(Duration::from_millis(1));

    handles.push(thread::spawn(move || {
        for tile_filename in tiles_second_batch {
            println!("{}", tile_filename);
            let (min_x, min_y, max_x, max_y) =
                get_bounds_from_lidar_hd_tile_filename(&tile_filename);

            generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                &Path::new("in").join(tile_filename),
                &Path::new("out").join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)),
            );
        }
        thread::sleep(Duration::from_millis(1));
    }));

    thread::sleep(Duration::from_millis(1));

    for handle in handles {
        handle.join().unwrap();
    }

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
        get_tile_struct_from_lidar_hd_tile_filename(&tile),
        NeighborTiles {
            top: Some(get_tile_struct_from_lidar_hd_tile_filename(&top)),
            top_right: Some(get_tile_struct_from_lidar_hd_tile_filename(&top_right)),
            right: Some(get_tile_struct_from_lidar_hd_tile_filename(&right)),
            bottom_right: Some(get_tile_struct_from_lidar_hd_tile_filename(&bottom_right)),
            bottom: Some(get_tile_struct_from_lidar_hd_tile_filename(&bottom)),
            bottom_left: Some(get_tile_struct_from_lidar_hd_tile_filename(&bottom_left)),
            left: Some(get_tile_struct_from_lidar_hd_tile_filename(&left)),
            top_left: Some(get_tile_struct_from_lidar_hd_tile_filename(&top_left)),
        },
        &config,
    );

    let duration = start.elapsed();
    println!("Tiles generated in {:.1?}", duration);
}

fn get_tile_struct_from_lidar_hd_tile_filename(filename: &str) -> Tile {
    let (min_x, min_y, max_x, max_y) = get_bounds_from_lidar_hd_tile_filename(filename);
    return Tile {
        laz_path: Path::new("in").join(filename),
        dir_path: Path::new("out").join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)),
        min_x: min_x * 1000,
        min_y: min_y * 1000,
        max_x: max_x * 1000,
        max_y: max_y * 1000,
    };
}

fn get_bounds_from_lidar_hd_tile_filename(filename: &str) -> (i64, i64, i64, i64) {
    let parts = filename.split("_");
    let collection: Vec<&str> = parts.collect();
    let min_x_km: i64 = collection[2].parse().unwrap();
    let max_y_km: i64 = collection[3].parse().unwrap();
    return (min_x_km, max_y_km - 1, min_x_km + 1, max_y_km);
}
