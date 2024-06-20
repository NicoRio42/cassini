use image::{Rgba, RgbaImage};
use imageproc::{drawing::draw_filled_rect_mut, rect::Rect};
use std::fs::File;
use tiff::decoder::{Decoder, DecodingResult};

const VEGETATION_RESOLUTION: u32 = 2;
const DEM_RESOLUTION: u32 = 1;
const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const TRANSPARENT: Rgba<u8> = Rgba([255, 255, 255, 0]);
const YELLOW: Rgba<u8> = Rgba([255, 221, 154, 255]);
const GREEN_1: Rgba<u8> = Rgba([197, 255, 185, 255]);
const GREEN_2: Rgba<u8> = Rgba([139, 255, 116, 255]);
const GREEN_3: Rgba<u8> = Rgba([61, 255, 23, 255]);
const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);
const YELLOW_THRESHOLD: f64 = 700.0;
const GREEN_1_THRESHOLD: f64 = 100.0;
const GREEN_2_THRESHOLD: f64 = 200.0;
const GREEN_3_THRESHOLD: f64 = 300.0;
const SLOPE_THRESHOLD: f32 = 40.0;

fn main() {
    render_vegetation();
    render_cliffs();
}

fn render_vegetation() {
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
                (forest_height as i32 - y as i32) * VEGETATION_RESOLUTION as i32,
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

    // if green_width != forest_width || green_height != forest_height {
    //     panic!(
    //         "Forest tif image and green tif image should be the same size. {} {} {} {}",
    //         green_width, forest_width, green_height, forest_height
    //     )
    // }

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
                        (green_height as i32 - y as i32) * VEGETATION_RESOLUTION as i32,
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

fn render_cliffs() {
    let slopes_tif_file = File::open("../out/slopes.tif").expect("Cannot find slopes tif image!");

    let mut slopes_img_decoder = Decoder::new(slopes_tif_file).expect("Cannot create decoder");
    slopes_img_decoder = slopes_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (slopes_width, slopes_height) = slopes_img_decoder.dimensions().unwrap();

    let mut cliffs_layer_img = RgbaImage::from_pixel(
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

        draw_filled_rect_mut(
            &mut cliffs_layer_img,
            Rect::at(
                x as i32 * DEM_RESOLUTION as i32,
                (slopes_height as i32 - y as i32) * DEM_RESOLUTION as i32,
            )
            .of_size(DEM_RESOLUTION, DEM_RESOLUTION),
            BLACK,
        );
    }

    cliffs_layer_img
        .save("../out/cliffs.png")
        .expect("could not save cliffs png");
}
