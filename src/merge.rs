use log::{info, warn};
use std::{path::Path, time::Instant};

use crate::{canvas::Canvas, config::get_config, constants::INCH, tile::TileWithNeighbors};

pub fn merge_maps(output_dir: &str, tiles_with_neighbors: Vec<TileWithNeighbors>) {
    info!("Merging maps");
    let start = Instant::now();

    let config = get_config();

    if tiles_with_neighbors.len() == 0 {
        warn!("No map to merge.");
        return;
    }

    let first_tile = &tiles_with_neighbors[0].tile;

    let mut min_x = first_tile.min_x;
    let mut min_y = first_tile.min_y;
    let mut max_x = first_tile.max_x;
    let mut max_y = first_tile.max_y;

    for tile in tiles_with_neighbors.clone() {
        if tile.tile.min_x < min_x {
            min_x = tile.tile.min_x;
        }
        if tile.tile.min_y < min_y {
            min_y = tile.tile.min_y;
        }
        if tile.tile.max_x > max_x {
            max_x = tile.tile.max_x;
        }
        if tile.tile.max_y > max_y {
            max_y = tile.tile.max_y;
        }
    }

    let merge_image_width = (max_x - min_x) as f32 * config.dpi_resolution / INCH;
    let merge_image_height = (max_y - min_y) as f32 * config.dpi_resolution / INCH;
    let mut merge_image = Canvas::new(merge_image_width as i32, merge_image_height as i32);

    for tile in tiles_with_neighbors {
        let mut map = Canvas::load_from(
            tile.tile
                .render_dir_path
                .join("full-map.png")
                .to_str()
                .unwrap(),
        );

        merge_image.overlay(
            &mut map,
            ((tile.tile.min_x - min_x) as f32 * config.dpi_resolution / INCH).floor(),
            (((max_y - min_y) - (tile.tile.max_y - min_y)) as f32 * config.dpi_resolution / INCH)
                .floor(),
        )
    }

    merge_image.save_as(
        Path::new(output_dir)
            .join("merged-map.png")
            .to_str()
            .unwrap(),
    );

    let duration = start.elapsed();
    info!("Map merged in {:.1?}", duration);
}
