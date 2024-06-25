use image::RgbaImage;
use imageproc::drawing::draw_filled_ellipse_mut;
use std::fs::File;
use tiff::decoder::{Decoder, DecodingResult};

use crate::{
    config::Config,
    constants::{BLACK, CLIFF_THICKNESS, INCH, TRANSPARENT},
};

pub fn render_cliffs(image_width: u32, image_height: u32, config: &Config) {
    println!("Rendering cliffs");

    let dem_block_size_pixel = (config.dem_block_size as f32 * config.dpi_resolution / INCH);

    let slopes_tif_file = File::open("./out/slopes.tif").expect("Cannot find slopes tif image!");

    let mut slopes_img_decoder = Decoder::new(slopes_tif_file).expect("Cannot create decoder");
    slopes_img_decoder = slopes_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (slopes_width, slopes_height) = slopes_img_decoder.dimensions().unwrap();
    let mut cliffs_layer_canvas = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);

    let DecodingResult::F32(image_data) = slopes_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    for index in 0..image_data.len() {
        let x = index % usize::try_from(slopes_width).unwrap();
        let y = index / usize::try_from(slopes_height).unwrap();

        let slope = image_data[index];

        if slope < config.slope_threshold {
            continue;
        }

        draw_filled_ellipse_mut(
            &mut cliffs_layer_canvas,
            (
                (x as f32 * dem_block_size_pixel) as i32,
                (y as f32 * dem_block_size_pixel) as i32,
            ),
            (CLIFF_THICKNESS * config.dpi_resolution * 10.0 / INCH / 2.0) as i32,
            (CLIFF_THICKNESS * config.dpi_resolution * 10.0 / INCH / 2.0) as i32,
            BLACK,
        );
    }

    cliffs_layer_canvas
        .save("./out/cliffs.png")
        .expect("could not save cliffs png");
}
