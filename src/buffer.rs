use crate::{get_extent_from_lidar_dir_path, tile::Tile};
use image::{ImageBuffer, RgbaImage};
use std::path::PathBuf;

pub fn create_raster_with_buffer(
    tile: &Tile,
    neighbor_tiles: &Vec<PathBuf>,
    buffer_in_pixels: u32,
    raster_filename_without_extension: &str,
) {
    let tile_raster_path = tile
        .lidar_dir_path
        .join(format!("{}.png", raster_filename_without_extension));

    let tile_raster_file = image::open(tile_raster_path)
        .expect("Failed to open image")
        .to_rgba8();

    let (tile_raster_width, tile_raster_height) = tile_raster_file.dimensions();
    let output_width = tile_raster_width + buffer_in_pixels * 2;
    let output_height = tile_raster_height + buffer_in_pixels * 2;
    let mut output: RgbaImage = ImageBuffer::new(output_width, output_height);

    image::imageops::overlay(
        &mut output,
        &tile_raster_file,
        buffer_in_pixels as i64,
        buffer_in_pixels as i64,
    );

    for neighbor_tile in neighbor_tiles {
        let neighbor_tile_raster_path =
            neighbor_tile.join(format!("{}.png", raster_filename_without_extension));

        let (min_x, min_y, _, _) = get_extent_from_lidar_dir_path(&neighbor_tile);

        let neighbor_tile_raster_file = image::open(neighbor_tile_raster_path)
            .expect("Failed to open neighbor tile")
            .to_rgba8();

        let (neighbor_width, neighbor_height) = tile_raster_file.dimensions();

        let x_sign = min_x - tile.min_x;
        let mut x_offset = buffer_in_pixels as i64;

        if x_sign < 0 {
            x_offset = -(neighbor_width as i64) + buffer_in_pixels as i64
        } else if x_sign > 0 {
            x_offset = (tile_raster_width + buffer_in_pixels) as i64
        }

        let y_sign = min_y - tile.min_y;
        let mut y_offset = buffer_in_pixels as i64;

        if y_sign < 0 {
            y_offset = (tile_raster_height + buffer_in_pixels) as i64;
        } else if y_sign > 0 {
            y_offset = -(neighbor_height as i64) + buffer_in_pixels as i64;
        }

        println!("{} {}", x_offset, y_offset);

        image::imageops::overlay(&mut output, &neighbor_tile_raster_file, x_offset, y_offset);
    }

    let raster_with_buffer_path = tile
        .render_dir_path
        .join(format!("{}-with-buffer.png", raster_filename_without_extension));

    output
        .save(raster_with_buffer_path)
        .expect("Failed to save image");
}
