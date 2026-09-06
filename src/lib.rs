mod batch;
mod buffer;
mod canvas;
mod cliffs;
mod coastlines;
mod config;
mod constants;
mod contours;
mod dem;
mod download;
mod helpers;
mod lidar;
mod map_renderer;
mod merge;
mod pullautin_contours_render;
mod pullautin_smooth_contours;
mod render;
mod tile;
mod vectors;
mod vegetation;
mod world_file;

pub use vegetation::UndergrowthMode;

use batch::batch;
use config::{default_config, get_config};
use download::download_osm_file;
use las::raw::Header;
use lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file;
use render::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file;
use std::{
    fs::{create_dir_all, File},
    path::{Path, PathBuf},
};
use tile::Tile;

pub use tile::get_extent_from_lidar_dir_path;

pub fn process_single_tile(
    file_path: &PathBuf,
    output_dir_path: &PathBuf,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
    shapefiles_dir: Option<PathBuf>,
) {
    let config = get_config(None);

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

    if shapefiles_dir.is_none() && !skip_vector {
        download_osm_file_if_needed(&tile);
    }

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
        tile,
        vec![],
        skip_vector,
        skip_520,
        undergrowth_mode,
        shapefiles_dir,
        &config,
    );
}

pub fn process_single_tile_lidar_step(file_path: &PathBuf, output_dir_path: &PathBuf) {
    generate_dem_and_vegetation_density_tiff_images_from_laz_file(&file_path, &output_dir_path);
}

pub fn process_single_tile_lidar_step_with_config(
    file_path: &PathBuf,
    output_dir_path: &PathBuf,
    config_path: Option<&Path>,
) {
    if config_path.is_some() {
        get_config(config_path);
    }

    process_single_tile_lidar_step(file_path, output_dir_path);
}

pub fn process_single_tile_render_step(
    input_dir_path: &PathBuf,
    output_dir_path: &PathBuf,
    neighbor_tiles: Vec<PathBuf>,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
    shapefiles_dir: Option<PathBuf>,
) {
    process_single_tile_render_step_with_config(
        input_dir_path,
        output_dir_path,
        neighbor_tiles,
        skip_vector,
        skip_520,
        undergrowth_mode,
        shapefiles_dir,
        None,
    );
}

pub fn process_single_tile_render_step_with_config(
    input_dir_path: &PathBuf,
    output_dir_path: &PathBuf,
    neighbor_tiles: Vec<PathBuf>,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
    shapefiles_dir: Option<PathBuf>,
    config_path: Option<&Path>,
) {
    let config = get_config(config_path);
    create_dir_all(&output_dir_path).expect("Could not create out dir");

    let (min_x, min_y, max_x, max_y) = get_extent_from_lidar_dir_path(&input_dir_path.to_path_buf());

    let tile = Tile {
        lidar_dir_path: input_dir_path.to_path_buf(),
        render_dir_path: output_dir_path.to_path_buf(),
        min_x,
        min_y,
        max_x,
        max_y,
    };

    if shapefiles_dir.is_none() && !skip_vector {
        download_osm_file_if_needed(&tile);
    }

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
        tile,
        neighbor_tiles,
        skip_vector,
        skip_520,
        undergrowth_mode,
        shapefiles_dir,
        &config,
    );
}

fn download_osm_file_if_needed(tile: &Tile) {
    let osm_path = tile
        .render_dir_path
        .join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));

    if !osm_path.exists() {
        download_osm_file(tile.min_x, tile.min_y, tile.max_x, tile.max_y, &tile.render_dir_path);
    }
}

pub fn batch_process_tiles(
    input_dir: &str,
    output_dir: &str,
    number_of_threads: usize,
    skip_lidar: bool,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
) {
    batch_process_tiles_with_config(
        input_dir,
        output_dir,
        number_of_threads,
        skip_lidar,
        skip_vector,
        skip_520,
        undergrowth_mode,
        None,
    );
}

pub fn batch_process_tiles_with_config(
    input_dir: &str,
    output_dir: &str,
    number_of_threads: usize,
    skip_lidar: bool,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
    config_path: Option<&Path>,
) {
    let config = get_config(config_path);
    batch(
        &input_dir,
        &output_dir,
        number_of_threads,
        skip_lidar,
        skip_vector,
        skip_520,
        undergrowth_mode,
        config,
    );
}

pub fn generate_default_config() {
    default_config();
}
