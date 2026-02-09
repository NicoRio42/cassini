use crate::constants::INCH;
use crate::contours::generate_contours_with_pullautin_algorithme;
use crate::vectors::render_map_with_osm_vector_shapes;
use crate::world_file::create_world_file;
use crate::UndergrowthMode;
use crate::{
    cliffs::render_cliffs, config::get_config, dem::create_dem_with_buffer_and_slopes_tiff, tile::Tile,
    vegetation::render_vegetation,
};
use log::info;
use std::path::PathBuf;
use std::time::Instant;

pub fn generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
    tile: Tile,
    neighbor_tiles: Vec<PathBuf>,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
) {
    let config = get_config();
    let image_width = ((tile.max_x - tile.min_x) as f32 * config.dpi_resolution / INCH) as u32;
    let image_height = ((tile.max_y - tile.min_y) as f32 * config.dpi_resolution / INCH) as u32;

    render_vegetation(
        &tile,
        &neighbor_tiles,
        image_width,
        image_height,
        &config,
        undergrowth_mode,
    );

    create_dem_with_buffer_and_slopes_tiff(&tile, &neighbor_tiles);
    generate_contours_with_pullautin_algorithme(&tile, image_width, image_height, &config);
    render_cliffs(&tile, image_width, image_height, &config);

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map to png",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let cliffs_path = tile.render_dir_path.join("cliffs.png");
    let vegetation_path = tile.render_dir_path.join("vegetation.png");
    let undergrowth_path = tile.render_dir_path.join("undergrowth.png");
    let contours_path = tile.render_dir_path.join("contours.png");

    render_map_with_osm_vector_shapes(
        &tile,
        image_width,
        image_height,
        &config,
        &vegetation_path,
        &undergrowth_path,
        &contours_path,
        &cliffs_path,
        skip_520,
        skip_vector,
        undergrowth_mode,
    );

    let resolution = INCH / (config.dpi_resolution);
    let world_file_path = tile.render_dir_path.join("full-map.pgw");

    create_world_file(tile.min_x as f32, tile.max_y as f32, resolution, &world_file_path)
        .expect("Could not create world file");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Map rendered to png in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}
