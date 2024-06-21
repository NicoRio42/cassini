use crate::constants::{
    GREEN_1, GREEN_1_THRESHOLD, GREEN_2, GREEN_2_THRESHOLD, GREEN_3, GREEN_3_THRESHOLD,
    VEGETATION_RESOLUTION, WHITE, YELLOW, YELLOW_THRESHOLD,
};
use image::{Rgba, RgbaImage};
use imageproc::{drawing::draw_filled_rect_mut, rect::Rect};
use std::fs::File;
use tiff::decoder::{Decoder, DecodingResult};

pub fn render_vegetation() {
    let forest_density_tif_file =
        File::open("../out/high-vegetation.tif").expect("Cannot find high vegetation tif image!");

    let mut forest_img_decoder =
        Decoder::new(forest_density_tif_file).expect("Cannot create decoder");
    forest_img_decoder = forest_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (forest_width, forest_height) = forest_img_decoder.dimensions().unwrap();

    let mut vegetation_layer_img = RgbaImage::from_pixel(
        (forest_width * VEGETATION_RESOLUTION) as u32,
        (forest_height * VEGETATION_RESOLUTION) as u32,
        WHITE,
    );

    let DecodingResult::F64(image_data) = forest_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    for index in 0..image_data.len() {
        let x = index % usize::try_from(forest_width).unwrap();
        let y = index / usize::try_from(forest_width).unwrap();

        let forest_density = image_data[index];

        if forest_density > YELLOW_THRESHOLD {
            continue;
        }

        draw_filled_rect_mut(
            &mut vegetation_layer_img,
            Rect::at(
                x as i32 * VEGETATION_RESOLUTION as i32,
                y as i32 * VEGETATION_RESOLUTION as i32,
            )
            .of_size(VEGETATION_RESOLUTION, VEGETATION_RESOLUTION),
            YELLOW,
        );
    }

    let green_density_tif_file = File::open("../out/middle-vegetation.tif")
        .expect("Cannot find middle vegetation tif image!");

    let mut green_img_decoder =
        Decoder::new(green_density_tif_file).expect("Cannot create decoder");

    green_img_decoder = green_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (green_width, green_height) = green_img_decoder.dimensions().unwrap();

    let DecodingResult::F64(image_data) = green_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    for index in 0..image_data.len() {
        let x = index % usize::try_from(green_width).unwrap();
        let y = index / usize::try_from(green_width).unwrap();

        let green_density = image_data[index];

        let mut green_color: Option<Rgba<u8>> = None;

        if green_density > GREEN_3_THRESHOLD {
            green_color = Some(GREEN_3);
        } else if green_density > GREEN_2_THRESHOLD {
            green_color = Some(GREEN_2);
        } else if green_density > GREEN_1_THRESHOLD {
            green_color = Some(GREEN_1);
        }

        match green_color {
            Some(color) => {
                draw_filled_rect_mut(
                    &mut vegetation_layer_img,
                    Rect::at(
                        x as i32 * VEGETATION_RESOLUTION as i32,
                        y as i32 * VEGETATION_RESOLUTION as i32,
                    )
                    .of_size(VEGETATION_RESOLUTION, VEGETATION_RESOLUTION),
                    color,
                );
            }
            _ => (),
        }
    }

    vegetation_layer_img
        .save("../out/vegetation.png")
        .expect("could not save output png");
}
