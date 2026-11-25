use log::{info, warn};
use std::{path::Path, time::Instant};

use crate::{
    canvas::Canvas,
    config::Config,
    constants::MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT,
    error::{Result, ResultContext},
    render_transform::{PixelGrid, PixelRect, RenderTransform},
    tile::TileWithNeighbors,
};

pub fn merge_maps(
    output_dir: &str,
    tiles_with_neighbors: Vec<TileWithNeighbors>,
    config: &Config,
) -> Result<()> {
    info!("Merging maps");
    let start = Instant::now();

    if tiles_with_neighbors.is_empty() {
        warn!("No map to merge.");
        return Ok(());
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

    let grid = PixelGrid::from_config(config);
    let merged_rect = grid.rect(min_x as f64, min_y as f64, max_x as f64, max_y as f64);
    let total_width = merged_rect.width();
    let total_height = merged_rect.height();

    let cols =
        (total_width + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT - 1) / MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
    let rows =
        (total_height + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT - 1) / MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
    let num_chunks = cols * rows;
    let is_single_chunk = num_chunks == 1;

    info!(
        "Merged image: {}x{} px, split into {} chunk(s) ({}x{} grid)",
        total_width, total_height, num_chunks, cols, rows
    );

    let mut chunk_index = 0;

    for row in 0..rows {
        for col in 0..cols {
            chunk_index += 1;

            // The pixels of this chunk on the grid of the tiles.
            let left = merged_rect.left + col * MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
            let top = merged_rect.top + row * MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT;
            let chunk = RenderTransform::new(
                grid,
                PixelRect {
                    left,
                    top,
                    right: (left + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT).min(merged_rect.right),
                    bottom: (top + MAX_MERGED_PIXEL_WIDTH_AND_HEIGHT).min(merged_rect.bottom),
                },
            );
            let chunk_rect = chunk.pixel_rect();
            let chunk_w = chunk_rect.width();
            let chunk_h = chunk_rect.height();

            let mut chunk_canvas = Canvas::new(chunk_w as i32, chunk_h as i32)?;

            for twn in &tiles_with_neighbors {
                let tile_rect = RenderTransform::for_tile(&twn.tile, config).pixel_rect();
                if !tile_rect.intersects(&chunk_rect) {
                    continue;
                }

                let mut map = Canvas::load_from(&twn.tile.render_dir_path.join("full-map.png"))?;
                chunk_canvas.overlay(
                    &mut map,
                    (tile_rect.left - chunk_rect.left) as f32,
                    (tile_rect.top - chunk_rect.top) as f32,
                );
            }

            let (png_name, pgw_name) = if is_single_chunk {
                ("merged-map.png".to_string(), "merged-map.pgw".to_string())
            } else {
                (
                    format!("merged-map-{}.png", chunk_index),
                    format!("merged-map-{}.pgw", chunk_index),
                )
            };

            let png_path = Path::new(output_dir).join(&png_name);
            chunk_canvas.save_as(&png_path)?;

            let world_file_path = Path::new(output_dir).join(&pgw_name);
            chunk
                .write_world_file(&world_file_path)
                .context(format!("could not create `{}`", world_file_path.display()))?;

            info!(
                "Saved chunk {}/{}: {} ({}x{} px)",
                chunk_index, num_chunks, png_name, chunk_w, chunk_h
            );
        }
    }

    let duration = start.elapsed();
    info!("Map merged in {:.1?}", duration);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::merge_maps;
    use crate::{
        config::Config,
        render_transform::{PixelGrid, RenderTransform},
        tile::{Tile, TileWithNeighbors},
    };
    use image::{Rgba, RgbaImage};
    use std::path::PathBuf;

    #[test]
    fn merges_tiles_without_seams_and_georeferences_the_merged_map() {
        let dir = tempfile::tempdir().unwrap();
        // 125.98 pixels per 1 km tile, which truncated sizes and offsets
        // left a transparent column after.
        let config = Config {
            dpi_resolution: 32.0,
            ..Config::default()
        };
        let mut tiles = vec![];
        for column in 0..6 {
            for row in 0..2 {
                let (min_x, min_y) = (900_000 + column * 1000, 6_400_000 + row * 1000);
                let dir_path = dir.path().join(format!("{min_x}_{min_y}"));
                std::fs::create_dir(&dir_path).unwrap();
                let tile = Tile {
                    lidar_dir_path: dir_path.clone(),
                    render_dir_path: dir_path.clone(),
                    min_x,
                    min_y,
                    max_x: min_x + 1000,
                    max_y: min_y + 1000,
                };
                let transform = RenderTransform::for_tile(&tile, &config);
                RgbaImage::from_pixel(transform.width(), transform.height(), Rgba([0, 0, 0, 255]))
                    .save(dir_path.join("full-map.png"))
                    .unwrap();
                tiles.push(TileWithNeighbors {
                    laz_path: PathBuf::new(),
                    dem_path: None,
                    tile,
                    neighbors: vec![],
                });
            }
        }

        merge_maps(dir.path().to_str().unwrap(), tiles, &config).unwrap();

        let merged = image::open(dir.path().join("merged-map.png"))
            .unwrap()
            .into_rgba8();
        let grid = PixelGrid::from_config(&config);
        let rect = grid.rect(900_000.0, 6_400_000.0, 906_000.0, 6_402_000.0);
        assert_eq!(
            (i64::from(merged.width()), i64::from(merged.height())),
            (rect.width(), rect.height())
        );
        let transparent = merged.pixels().filter(|pixel| pixel[3] == 0).count();
        assert_eq!(transparent, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("merged-map.pgw")).unwrap(),
            RenderTransform::new(grid, rect).world_file().to_string()
        );
    }
}
