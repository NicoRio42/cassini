use crate::contours::generate_contours_with_pullautin_algorithme;
use crate::vectors::render_osm_vector_shapes;
use crate::INCH;
use crate::{
    cliffs::render_cliffs,
    config::get_config,
    dem::create_dem_with_buffer_and_slopes_tiff,
    full_map::render_full_map_to_png,
    tile::{NeighborTiles, Tile},
    vegetation::render_vegetation,
};

pub fn generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
    tile: Tile,
    neighbor_tiles: NeighborTiles,
    skip_vector: bool,
) {
    let config = get_config();
    let image_width = ((tile.max_x - tile.min_x) as f32 * config.dpi_resolution / INCH) as u32;
    let image_height = ((tile.max_y - tile.min_y) as f32 * config.dpi_resolution / INCH) as u32;

    render_vegetation(&tile, &neighbor_tiles, image_width, image_height, &config);
    create_dem_with_buffer_and_slopes_tiff(&tile, &neighbor_tiles);
    generate_contours_with_pullautin_algorithme(&tile, image_width, image_height, &config);
    render_cliffs(&tile, image_width, image_height, &config);

    if !skip_vector {
        render_osm_vector_shapes(&tile, image_width, image_height, &config);
    }

    render_full_map_to_png(&tile, image_width, image_height, skip_vector);
}
