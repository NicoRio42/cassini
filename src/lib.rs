mod batch;
mod buffer;
mod canvas;
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
use config::default_config;
use download::download_osm_file_if_needed;
use las::raw::Header;
use lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file;
use png::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file;
use std::{
    fs::{create_dir_all, File},
    path::PathBuf,
};
use tile::Tile;

pub use tile::get_extent_from_lidar_dir_path;

pub fn process_single_tile(file_path: &PathBuf, output_dir_path: &PathBuf, skip_vector: bool) {
    generate_dem_and_vegetation_density_tiff_images_from_laz_file(
        &file_path.to_path_buf(),
        &output_dir_path.to_path_buf(),
    );

    let mut file = File::open(&file_path).expect("Cound not open laz file");
    let header = Header::read_from(&mut file).unwrap();

    let tile = Tile {
        lidar_dir_path: output_dir_path.to_path_buf(),
        render_dir_path: output_dir_path.to_path_buf(),
        min_x: header.min_x.round() as i64,
        min_y: header.min_y.round() as i64,
        max_x: header.max_x.round() as i64,
        max_y: header.max_y.round() as i64,
    };

    if !skip_vector {
        download_osm_file_if_needed(tile.min_x, tile.min_y, tile.max_x, tile.max_y);
    }

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(tile, vec![], skip_vector);
}

pub fn process_single_tile_lidar_step(file_path: &PathBuf, output_dir_path: &PathBuf) {
    generate_dem_and_vegetation_density_tiff_images_from_laz_file(&file_path, &output_dir_path);
}

pub fn process_single_tile_render_step(
    input_dir_path: &PathBuf,
    output_dir_path: &PathBuf,
    neighbor_tiles: Vec<PathBuf>,
    skip_vector: bool,
) {
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

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
        tile,
        neighbor_tiles,
        skip_vector,
    );
}

pub fn batch_process_tiles(
    input_dir: &str,
    output_dir: &str,
    number_of_threads: usize,
    skip_lidar: bool,
    skip_vector: bool,
) {
    batch(
        &input_dir,
        &output_dir,
        number_of_threads,
        skip_lidar,
        skip_vector,
    );
}

pub fn generate_default_config() {
    default_config();
}
