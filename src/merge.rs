use log::{info, warn};
use std::{path::Path, time::Instant};

use crate::{
    canvas::Canvas,
    config::get_config,
    constants::{INCH, MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT},
    tile::TileWithNeighbors,
    world_file::create_world_file,
};

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

    let total_width = ((max_x - min_x) as f32 * config.dpi_resolution / INCH).ceil() as i32;
    let total_height = ((max_y - min_y) as f32 * config.dpi_resolution / INCH).ceil() as i32;

    let cols = (total_width + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT - 1) / MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
    let rows = (total_height + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT - 1) / MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
    let num_chunks = cols * rows;
    let is_single_chunk = num_chunks == 1;

    let resolution = INCH / config.dpi_resolution;
    let geo_width = (max_x - min_x) as f32;
    let geo_height = (max_y - min_y) as f32;

    info!(
        "Merged image: {}x{} px, split into {} chunk(s) ({}x{} grid)",
        total_width, total_height, num_chunks, cols, rows
    );

    let mut chunk_index = 0;

    for row in 0..rows {
        for col in 0..cols {
            chunk_index += 1;

            // Pixel bounds of this chunk within the full merged image
            let chunk_px_x0 = col * MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
            let chunk_px_y0 = row * MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
            let chunk_px_x1 = (chunk_px_x0 + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT).min(total_width);
            let chunk_px_y1 = (chunk_px_y0 + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT).min(total_height);
            let chunk_w = chunk_px_x1 - chunk_px_x0;
            let chunk_h = chunk_px_y1 - chunk_px_y0;

            // Geographic bounds of this chunk
            let chunk_geo_min_x = min_x as f32 + (chunk_px_x0 as f32 / total_width as f32) * geo_width;
            let chunk_geo_max_x = min_x as f32 + (chunk_px_x1 as f32 / total_width as f32) * geo_width;
            // Image Y=0 is the top (max_y in geo), Y increases downward
            let chunk_geo_max_y = max_y as f32 - (chunk_px_y0 as f32 / total_height as f32) * geo_height;
            let chunk_geo_min_y = max_y as f32 - (chunk_px_y1 as f32 / total_height as f32) * geo_height;

            let mut chunk_canvas = Canvas::new(chunk_w, chunk_h);

            for twn in &tiles_with_neighbors {
                let t = &twn.tile;

                // Skip tiles that don't overlap this chunk geographically
                if (t.max_x as f32) <= chunk_geo_min_x
                    || (t.min_x as f32) >= chunk_geo_max_x
                    || (t.max_y as f32) <= chunk_geo_min_y
                    || (t.min_y as f32) >= chunk_geo_max_y
                {
                    continue;
                }

                let mut map = Canvas::load_from(t.render_dir_path.join("full-map.png").to_str().unwrap());

                // Tile pixel offset in the full merged image
                let tile_full_px_x = ((t.min_x - min_x) as f32 * config.dpi_resolution / INCH).floor();
                let tile_full_px_y = (((max_y - t.max_y) as f32) * config.dpi_resolution / INCH).floor();

                // Offset relative to this chunk's origin
                let overlay_x = tile_full_px_x - chunk_px_x0 as f32;
                let overlay_y = tile_full_px_y - chunk_px_y0 as f32;

                chunk_canvas.overlay(&mut map, overlay_x, overlay_y);
            }

            let (png_name, pgw_name) = if is_single_chunk {
                ("merged-map.png".to_string(), "merged-map.pgw".to_string())
            } else {
                (
                    format!("merged-map-{}.png", chunk_index),
                    format!("merged-map-{}.pgw", chunk_index),
                )
            };

            chunk_canvas.save_as(Path::new(output_dir).join(&png_name).to_str().unwrap());

            let world_file_path = Path::new(output_dir).join(&pgw_name);
            create_world_file(chunk_geo_min_x, chunk_geo_max_y, resolution, &world_file_path)
                .expect("Could not create world file");

            info!(
                "Saved chunk {}/{}: {} ({}x{} px)",
                chunk_index, num_chunks, png_name, chunk_w, chunk_h
            );
        }
    }

    let duration = start.elapsed();
    info!("Map merged in {:.1?}", duration);
}
