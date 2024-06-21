use image::RgbaImage;
use imageproc::drawing::draw_filled_ellipse_mut;
use std::fs::File;
use tiff::decoder::{Decoder, DecodingResult};

use crate::constants::{BLACK, DEM_RESOLUTION, SLOPE_THRESHOLD, TRANSPARENT};

pub fn render_cliffs() {
    println!("Rendering cliffs");

    let slopes_tif_file = File::open("./out/slopes.tif").expect("Cannot find slopes tif image!");

    let mut slopes_img_decoder = Decoder::new(slopes_tif_file).expect("Cannot create decoder");
    slopes_img_decoder = slopes_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (slopes_width, slopes_height) = slopes_img_decoder.dimensions().unwrap();

    let mut cliffs_layer_canvas = RgbaImage::from_pixel(
        (slopes_width * DEM_RESOLUTION) as u32,
        (slopes_height * DEM_RESOLUTION) as u32,
        TRANSPARENT,
    );

    let DecodingResult::F32(image_data) = slopes_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    for index in 0..image_data.len() {
        let x = index % usize::try_from(slopes_width).unwrap();
        let y = index / usize::try_from(slopes_height).unwrap();

        let slope = image_data[index];

        if slope < SLOPE_THRESHOLD {
            continue;
        }

        draw_filled_ellipse_mut(
            &mut cliffs_layer_canvas,
            (
                x as i32 * DEM_RESOLUTION as i32,
                y as i32 * DEM_RESOLUTION as i32,
            ),
            2,
            2,
            BLACK,
        );
    }

    cliffs_layer_canvas
        .save("./out/cliffs.png")
        .expect("could not save cliffs png");
}
