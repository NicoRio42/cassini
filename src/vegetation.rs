use crate::{
    buffer::create_tif_with_buffer,
    canvas::Canvas,
    config::Config,
    constants::{BUFFER, GREEN_1, GREEN_2, GREEN_3, INCH, TRANSPARENT, VEGETATION_BLOCK_SIZE, WHITE, YELLOW},
    tile::Tile,
};
use image::{imageops, Rgba, RgbaImage};
use imageproc::drawing::draw_filled_ellipse_mut;
use log::info;
use skia_safe::{Data, Image};
use std::{fs::File, path::PathBuf, time::Instant, u8};
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
    let casted_vegetation_block_size_pixel = (vegetation_block_size_pixel * 2.).ceil() as i32;

    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "high-vegetation", 1.0);
    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "medium-vegetation", 1.0);

    let high_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("high-vegetation-with-buffer.tif"));

    let medium_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("medium-vegetation-with-buffer.tif"));

    let mut base_vegetation_img = RgbaImage::from_pixel(image_width, image_height, YELLOW);
    let mut green_1_vegetation_img = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);

    for x_index in BUFFER..((tile.max_x + BUFFER as i64 - tile.min_x) as usize) {
        for y_index in BUFFER..((tile.max_y + BUFFER as i64 - tile.min_y) as usize) {
            let x_pixel = ((x_index - BUFFER) as f32 * vegetation_block_size_pixel) as i32;
            let y_pixel = ((y_index - BUFFER) as f32 * vegetation_block_size_pixel) as i32;

            let high_vegetation_density = get_min_value_in_circle(&high_vegetation, x_index, y_index);

            if high_vegetation_density > config.yellow_threshold as u8 {
                draw_filled_ellipse_mut(
                    &mut base_vegetation_img,
                    (x_pixel, y_pixel),
                    casted_vegetation_block_size_pixel,
                    casted_vegetation_block_size_pixel,
                    WHITE,
                );
            }

            let medium_vegetation_density = get_min_value_in_circle(&medium_vegetation, x_index, y_index);

            let mut green_color: Option<Rgba<u8>> = None;

            // if medium_vegetation_density > config.green_threshold_3 as u8 {
            //     green_color = Some(GREEN_3);
            // } else if medium_vegetation_density > config.green_threshold_2 as u8 {
            //     green_color = Some(GREEN_2);
            // } else
            if medium_vegetation_density > config.green_threshold_1 as u8 {
                green_color = Some(GREEN_1);
            }

            match green_color {
                Some(color) => {
                    draw_filled_ellipse_mut(
                        &mut green_1_vegetation_img,
                        (x_pixel, y_pixel),
                        casted_vegetation_block_size_pixel,
                        casted_vegetation_block_size_pixel,
                        color,
                    );
                }
                _ => (),
            }
        }
    }

    imageops::overlay(&mut base_vegetation_img, &green_1_vegetation_img, 0, 0);
    let vegetation_output_path = tile.render_dir_path.join("vegetation.png");

    base_vegetation_img
        .save(vegetation_output_path)
        .expect("could not save output png");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vegetation rendered in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}

fn get_min_value_in_circle(tif_image: &TifImage, x_index: usize, y_index: usize) -> u8 {
    let mut min = u8::MAX;
    let width = tif_image.width as usize;
    let height = tif_image.height as usize;

    if tif_image.pixels.len() == 0 {
        panic!("Image with no pixels")
    }

    for y_matrix in 0..5 {
        for x_matrix in 0..5 {
            if x_index + x_matrix < 2
                || y_index + y_matrix < 2
                || y_matrix == 0
                || x_matrix == 0
                || y_matrix == 4
                || x_matrix == 4
            {
                continue;
            }

            let x = x_index + x_matrix - 2;
            let y = y_index + y_matrix - 2;

            if x > width || y > height {
                continue;
            }

            let pixel_value = tif_image.pixels[y * width + x];

            if pixel_value < min {
                min = pixel_value;
            }
        }
    }

    return min;
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
