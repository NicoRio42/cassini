use image::RgbaImage;
use imageproc::drawing::draw_filled_ellipse_mut;
use log::info;
use std::{fs::File, time::Instant};
use tiff::decoder::{Decoder, DecodingResult};

use crate::{
    config::Config,
    constants::{BLACK, BUFFER, CLIFF_THICKNESS_1, CLIFF_THICKNESS_2, DEM_BLOCK_SIZE, INCH, TRANSPARENT},
    tile::Tile,
};

pub fn render_cliffs(tile: &Tile, image_width: u32, image_height: u32, config: &Config) {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering cliffs",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let dem_block_size_pixel = DEM_BLOCK_SIZE * config.dpi_resolution / INCH;

    let slopes_path = tile.render_dir_path.join("slopes.tif");
    let slopes_tif_file = File::open(slopes_path).expect("Cannot find slopes tif image!");

    let mut slopes_img_decoder = Decoder::new(slopes_tif_file).expect("Cannot create decoder");
    slopes_img_decoder = slopes_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (slopes_width, _) = slopes_img_decoder.dimensions().unwrap();
    let mut cliffs_layer_canvas = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);

    let DecodingResult::F32(image_data) = slopes_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    for index in 0..image_data.len() {
        let x = index % slopes_width as usize;
        let y = index / slopes_width as usize;

        let x_pixel =
            ((x as i64 - (BUFFER as f32 / DEM_BLOCK_SIZE) as i64) as f32 * dem_block_size_pixel) as i32;
        let y_pixel =
            ((y as i64 - (BUFFER as f32 / DEM_BLOCK_SIZE) as i64) as f32 * dem_block_size_pixel) as i32;

        if x_pixel < 0 || x_pixel > image_width as i32 || y_pixel < 0 || y_pixel > image_height as i32 {
            continue;
        }

        let slope = image_data[index];

        let mut cliff_thickness: Option<f32> = None;

        if slope > config.cliff_threshold_2 {
            cliff_thickness = Some(CLIFF_THICKNESS_2);
        } else if slope > config.cliff_threshold_1 {
            cliff_thickness = Some(CLIFF_THICKNESS_1);
        }

        match cliff_thickness {
            Some(thickness) => {
                draw_filled_ellipse_mut(
                    &mut cliffs_layer_canvas,
                    (x_pixel, y_pixel),
                    (thickness * config.dpi_resolution * 10.0 / INCH / 2.0) as i32,
                    (thickness * config.dpi_resolution * 10.0 / INCH / 2.0) as i32,
                    BLACK,
                );
            }
            _ => (),
        }
    }

    let cliffs_path = tile.render_dir_path.join("cliffs.png");

    cliffs_layer_canvas
        .save(cliffs_path)
        .expect("could not save cliffs png");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Cliffs rendering done in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}
