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
mod pipeline;
mod tile;
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
use dem::create_dem_with_buffer;
use download::{download_laz_files_if_needed, download_osm_file_if_needed};
use full_map::render_full_map_to_png;
use lidar::{generate_dem_and_vegetation_density_tiff_images_from_laz_file, process_lidar};
use pipeline::generate_pipeline_for_single_tile;
use std::{
    fs::{self, create_dir_all, File},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
use tile::{NeighborTiles, Tile};
use tile_list::get_tile_list_from_extent;
use utils::delete_dir_contents;
use vectors::render_vector_shapes;
use vegetation::render_vegetation;

fn main() {
    let args = Args::parse();

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

    // let tiles_first_batch = [tile, top, top_right, right, bottom_right];

    // let tiles_second_batch = [bottom, bottom_left, left, top_left];

    // let mut handles: Vec<thread::JoinHandle<()>> = Vec::with_capacity(2usize);

    // handles.push(thread::spawn(move || {
    //     for t in tiles_first_batch {
    //         let tile_filename = t.unwrap();
    //         println!("{}", tile_filename);
    //         let (min_x, min_y, max_x, max_y) =
    //             get_bounds_from_lidar_hd_tile_filename(&tile_filename);

    //         generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    //             Path::new("in").join(tile_filename),
    //             Path::new("out").join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)),
    //         );
    //     }
    //     thread::sleep(Duration::from_millis(1));
    // }));

    // thread::sleep(Duration::from_millis(1));

    // handles.push(thread::spawn(move || {
    //     for t in tiles_second_batch {
    //         let tile_filename = t.unwrap();
    //         println!("{}", tile_filename);
    //         let (min_x, min_y, max_x, max_y) =
    //             get_bounds_from_lidar_hd_tile_filename(&tile_filename);

    //         generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    //             Path::new("in").join(tile_filename),
    //             Path::new("out").join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)),
    //         );
    //     }
    //     thread::sleep(Duration::from_millis(1));
    // }));

    // thread::sleep(Duration::from_millis(1));

    // for handle in handles {
    //     handle.join().unwrap();
    // }

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
    );

    let duration = start.elapsed();
    println!("Tiles generated in {:.1?}", duration);
    // let tile_list = get_tile_list_from_extent(616112, 6163204, 616692, 6163693);

    // // TODO: multithreading
    // for (min_x, min_y, max_x, max_y) in tile_list {
    //     let start = Instant::now();
    //     download_laz_files_if_needed(min_x, min_y, max_x, max_y, "JS".to_owned());
    //     download_osm_file_if_needed(min_x, min_y, max_x, max_y);
    //     process_sigle_tile(min_x, min_y, max_x, max_y, args.skip_lidar);
    //     let duration = start.elapsed();
    //     println!("Tile {} {} generated in {:.1?}", min_x, max_y, duration);
    // }
}

fn get_tile_struct_from_lidar_hd_tile_filename(filename: &str) -> Tile {
    let (min_x, min_y, max_x, max_y) = get_bounds_from_lidar_hd_tile_filename(filename);
    return Tile {
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

fn generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
    tile: Tile,
    neighbor_tiles: NeighborTiles,
) {
    let buffer = 200;
    let config = get_config();
    let image_width = ((tile.max_x - tile.min_x) as f32 * config.dpi_resolution / INCH) as u32;
    let image_height = ((tile.max_y - tile.min_y) as f32 * config.dpi_resolution / INCH) as u32;

    render_vegetation(
        &tile,
        &neighbor_tiles,
        image_width,
        image_height,
        buffer,
        &config,
    );

    create_dem_with_buffer(&tile, &neighbor_tiles, buffer as i64)

    // render_contours_to_png(tile, neighbor_tiles, image_width, image_height, &config);
}

// fn process_sigle_tile(min_x: u64, min_y: u64, max_x: u64, max_y: u64, skip_lidar: bool) {
//     let buffer = 200;
//     let out_dir = Path::new("out").join(format!("{:0>7}_{:0>7}", min_x, max_y));

//     if !skip_lidar {
//         delete_dir_contents(fs::read_dir(&out_dir));
//         create_dir_all(&out_dir).expect("Could not create out dir");
//         generate_pipeline_for_single_tile(min_x, min_y, max_x, max_y, buffer, &out_dir);
//         process_lidar(&out_dir)
//     } else {
//         println!("Skipping LiDAR processing.");
//     }

//     let config = get_config();

//     let image_width = ((max_x - min_x) as f32 * config.dpi_resolution / INCH) as u32;
//     let image_height = ((max_y - min_y) as f32 * config.dpi_resolution / INCH) as u32;

//     render_vegetation(image_width, image_height, buffer, &config, &out_dir);
//     render_cliffs(image_width, image_height, buffer, &config, &out_dir);
//     render_contours_to_png(image_width, image_height, &config, min_x, min_y, &out_dir);

//     let osm_path = Path::new("in").join(format!("{:0>7}_{:0>7}.osm", min_x, max_y));

//     render_vector_shapes(
//         image_width,
//         image_height,
//         &config,
//         min_x,
//         min_y,
//         &out_dir,
//         &osm_path,
//     );

//     render_full_map_to_png(image_width, image_height, &out_dir);
// }
