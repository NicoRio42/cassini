use crate::{
    buffer::create_tif_with_buffer,
    config::Config,
    constants::{BUFFER, GREEN_1, GREEN_2, GREEN_3, INCH, VEGETATION_BLOCK_SIZE, WHITE, YELLOW},
    tile::Tile,
};
use image::{Rgba, RgbaImage};
use imageproc::{drawing::draw_filled_rect_mut, rect::Rect};
use log::info;
use std::{fs::File, path::PathBuf, time::Instant};
use tiff::decoder::{Decoder, DecodingResult};

pub fn render_vegetation(
    tile: &Tile,
    neighbor_tiles: &Vec<PathBuf>,
    image_width: u32,
    image_height: u32,
    config: &Config,
) {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering vegetation",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let vegetation_block_size_pixel = VEGETATION_BLOCK_SIZE as f32 * config.dpi_resolution / INCH;
    let casted_vegetation_block_size_pixel = vegetation_block_size_pixel.ceil() as u32;

    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "high-vegetation");
    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "medium-vegetation");

    let high_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("high-vegetation-with-buffer.tif"));

    let medium_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("medium-vegetation-with-buffer.tif"));

    let mut vegetation_layer_img = RgbaImage::from_pixel(image_width, image_height, WHITE);

    for x_index in BUFFER..((tile.max_x + BUFFER as i64 - tile.min_x) as usize) {
        for y_index in BUFFER..((tile.max_y + BUFFER as i64 - tile.min_y) as usize) {
            let x_pixel = ((x_index - BUFFER) as f32 * vegetation_block_size_pixel) as i32;
            let y_pixel = ((y_index - BUFFER) as f32 * vegetation_block_size_pixel) as i32;

            let high_vegetation_density = get_average_pixel_value(&high_vegetation, x_index, y_index);

            if high_vegetation_density < config.yellow_threshold {
                draw_filled_rect_mut(
                    &mut vegetation_layer_img,
                    Rect::at(x_pixel, y_pixel).of_size(
                        casted_vegetation_block_size_pixel,
                        casted_vegetation_block_size_pixel,
                    ),
                    YELLOW,
                );
            }

            let medium_vegetation_density = get_average_pixel_value(&medium_vegetation, x_index, y_index);

            let mut green_color: Option<Rgba<u8>> = None;

            if medium_vegetation_density > config.green_threshold_3 {
                green_color = Some(GREEN_3);
            } else if medium_vegetation_density > config.green_threshold_2 {
                green_color = Some(GREEN_2);
            } else if medium_vegetation_density > config.green_threshold_1 {
                green_color = Some(GREEN_1);
            }

            match green_color {
                Some(color) => {
                    draw_filled_rect_mut(
                        &mut vegetation_layer_img,
                        Rect::at(x_pixel, y_pixel).of_size(
                            casted_vegetation_block_size_pixel,
                            casted_vegetation_block_size_pixel,
                        ),
                        color,
                    );
                }
                _ => (),
            }
        }
    }

    let vegetation_output_path = tile.render_dir_path.join("vegetation.png");

    vegetation_layer_img
        .save(vegetation_output_path)
        .expect("could not save output png");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vegetation rendered in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}

const CONVOLUTION_MATRIX_7_LINEAR: [[f64; 7]; 7] = [
    [
        0.,
        0.1501634144012025,
        0.25464400750007,
        0.2928932188134524,
        0.25464400750007,
        0.1501634144012025,
        0.,
    ],
    [
        0.1501634144012025,
        0.33333333333333326,
        0.4729537233052701,
        0.5285954792089682,
        0.4729537233052701,
        0.33333333333333326,
        0.1501634144012025,
    ],
    [
        0.25464400750007,
        0.4729537233052701,
        0.6666666666666666,
        0.7642977396044841,
        0.6666666666666666,
        0.4729537233052701,
        0.25464400750007,
    ],
    [
        0.2928932188134524,
        0.5285954792089682,
        0.7642977396044841,
        1.,
        0.7642977396044841,
        0.5285954792089682,
        0.2928932188134524,
    ],
    [
        0.25464400750007,
        0.4729537233052701,
        0.6666666666666666,
        0.7642977396044841,
        0.6666666666666666,
        0.4729537233052701,
        0.25464400750007,
    ],
    [
        0.1501634144012025,
        0.33333333333333326,
        0.4729537233052701,
        0.5285954792089682,
        0.4729537233052701,
        0.33333333333333326,
        0.1501634144012025,
    ],
    [
        0.,
        0.1501634144012025,
        0.25464400750007,
        0.2928932188134524,
        0.25464400750007,
        0.1501634144012025,
        0.,
    ],
];

fn get_average_pixel_value(tif_image: &TifImage, x_index: usize, y_index: usize) -> f64 {
    let mut count = 0.0;
    let mut sum = 0.0;
    let width = tif_image.width as usize;
    let height = tif_image.height as usize;

    if tif_image.pixels.len() == 0 {
        panic!("Image with no pixels")
    }

    for (y_matrix, row) in CONVOLUTION_MATRIX_7_LINEAR.iter().enumerate() {
        for (x_matrix, coef) in row.iter().enumerate() {
            if x_index + x_matrix < 3 || y_index + y_matrix < 3 {
                continue;
            }

            let x = x_index + x_matrix - 3;
            let y = y_index + y_matrix - 3;

            if x > width || y > height {
                continue;
            }

            count += coef;
            sum += tif_image.pixels[y * width + x] as f64 * coef;
        }
    }

    return sum / count;
}

struct TifImage {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

fn get_image_data_from_tif(path: &PathBuf) -> TifImage {
    let tif_file = File::open(path).expect("Cannot find high vegetation tif image!");
    let mut img_decoder = Decoder::new(tif_file).expect("Cannot create decoder");
    img_decoder = img_decoder.with_limits(tiff::decoder::Limits::unlimited());
    let (width, height) = img_decoder.dimensions().unwrap();

    let DecodingResult::U8(image_data) = img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    return TifImage {
        pixels: image_data,
        width,
        height,
    };
}
