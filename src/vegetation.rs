use crate::{
    config::Config,
    constants::{GREEN_1, GREEN_2, GREEN_3, INCH, WHITE, YELLOW},
};
use image::{Rgba, RgbaImage};
use imageproc::{drawing::draw_filled_rect_mut, rect::Rect};
use std::fs::File;
use tiff::decoder::{Decoder, DecodingResult};

pub fn render_vegetation(image_width: u32, image_height: u32, config: &Config) {
    println!("Rendering vegetation");

    let vegetation_block_size_pixel =
        (config.vegetation_block_size as f32 * config.dpi_resolution / INCH) as u32;

    let forest_density_tif_file =
        File::open("./out/high-vegetation.tif").expect("Cannot find high vegetation tif image!");

    let mut forest_img_decoder =
        Decoder::new(forest_density_tif_file).expect("Cannot create decoder");

    forest_img_decoder = forest_img_decoder.with_limits(tiff::decoder::Limits::unlimited());
    let forest_width = forest_img_decoder.dimensions().unwrap().0;
    let mut vegetation_layer_img = RgbaImage::from_pixel(image_width, image_height, WHITE);

    let DecodingResult::F64(image_data) = forest_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    for index in 0..image_data.len() {
        let x = index % usize::try_from(forest_width).unwrap();
        let y = index / usize::try_from(forest_width).unwrap();

        let forest_density = image_data[index];

        if forest_density > config.yellow_threshold {
            continue;
        }

        draw_filled_rect_mut(
            &mut vegetation_layer_img,
            Rect::at(
                x as i32 * vegetation_block_size_pixel as i32,
                y as i32 * vegetation_block_size_pixel as i32,
            )
            .of_size(vegetation_block_size_pixel, vegetation_block_size_pixel),
            YELLOW,
        );
    }

    let green_density_tif_file = File::open("./out/middle-vegetation.tif")
        .expect("Cannot find middle vegetation tif image!");

    let mut green_img_decoder =
        Decoder::new(green_density_tif_file).expect("Cannot create decoder");

    green_img_decoder = green_img_decoder.with_limits(tiff::decoder::Limits::unlimited());
    let (green_width, _) = green_img_decoder.dimensions().unwrap();

    let DecodingResult::F64(image_data) = green_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    // Todo: group the two for loops into one
    for index in 0..image_data.len() {
        let x = index % usize::try_from(green_width).unwrap();
        let y = index / usize::try_from(green_width).unwrap();

        let green_density = image_data[index];

        let mut green_color: Option<Rgba<u8>> = None;

        if green_density > config.green_3_threshold {
            green_color = Some(GREEN_3);
        } else if green_density > config.green_2_threshold {
            green_color = Some(GREEN_2);
        } else if green_density > config.green_1_threshold {
            green_color = Some(GREEN_1);
        }

        match green_color {
            Some(color) => {
                draw_filled_rect_mut(
                    &mut vegetation_layer_img,
                    Rect::at(
                        x as i32 * vegetation_block_size_pixel as i32,
                        y as i32 * vegetation_block_size_pixel as i32,
                    )
                    .of_size(vegetation_block_size_pixel, vegetation_block_size_pixel),
                    color,
                );
            }
            _ => (),
        }
    }

    vegetation_layer_img
        .save("./out/vegetation.png")
        .expect("could not save output png");
}
