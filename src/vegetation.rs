use crate::{
    buffer::create_tif_with_buffer,
    config::Config,
    constants::{BUFFER, GREEN_1, GREEN_2, GREEN_3, INCH, TRANSPARENT, VEGETATION_BLOCK_SIZE, WHITE, YELLOW},
    tile::Tile,
};
use image::{imageops, Rgba, RgbaImage};
use imageproc::{
    drawing::{draw_filled_ellipse_mut, draw_filled_rect_mut},
    rect::Rect,
};
use log::info;
use std::{f32::consts::E, fs::File, path::PathBuf, time::Instant, u8};
use tiff::decoder::{Decoder, DecodingResult};

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum UndergrowthMode {
    None,
    Merge,
    #[value(name = "406")]
    Symbol406,
    #[value(name = "409")]
    Symbol409,
}

pub fn render_vegetation(
    tile: &Tile,
    neighbor_tiles: &Vec<PathBuf>,
    image_width: u32,
    image_height: u32,
    config: &Config,
    undergrowth_mode: &UndergrowthMode,
) {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering vegetation",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let vegetation_block_size_pixel = VEGETATION_BLOCK_SIZE as f32 * config.dpi_resolution / INCH;
    let casted_base_vegetation_block_size_pixel = (vegetation_block_size_pixel * 2.).ceil() as i32;
    let casted_green_block_size_pixel = (vegetation_block_size_pixel).ceil() as u32;

    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "high-vegetation", 1.0);
    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "medium-vegetation", 1.0);
    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "low-vegetation", 1.0);

    let high_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("high-vegetation-with-buffer.tif"));

    let medium_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("medium-vegetation-with-buffer.tif"));

    let low_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("low-vegetation-with-buffer.tif"));

    let mut base_vegetation_img = RgbaImage::from_pixel(image_width, image_height, YELLOW);
    let mut green_vegetation_img = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);
    let mut undergrowth_vegetation_img = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);

    let medium_vegetation_kernel_radius = 2;
    let medium_vegetation_kernel = get_convolution_kernel_matrix(medium_vegetation_kernel_radius);
    let low_vegetation_kernel_radius = 4;
    let low_vegetation_kernel = get_convolution_kernel_matrix(low_vegetation_kernel_radius);

    for x_index in BUFFER..((tile.max_x + BUFFER as i64 - tile.min_x) as usize) {
        for y_index in BUFFER..((tile.max_y + BUFFER as i64 - tile.min_y) as usize) {
            let x_pixel = ((x_index - BUFFER) as f32 * vegetation_block_size_pixel) as i32;
            let y_pixel = ((y_index - BUFFER) as f32 * vegetation_block_size_pixel) as i32;

            let high_vegetation_density = get_min_value_in_circle(&high_vegetation, x_index, y_index);

            if high_vegetation_density > config.yellow_threshold as u8 {
                draw_filled_ellipse_mut(
                    &mut base_vegetation_img,
                    (x_pixel, y_pixel),
                    casted_base_vegetation_block_size_pixel,
                    casted_base_vegetation_block_size_pixel,
                    WHITE,
                );
            }

            let mut medium_vegetation_density = get_average_pixel_value(
                &medium_vegetation,
                x_index,
                y_index,
                &medium_vegetation_kernel,
                medium_vegetation_kernel_radius,
            );

            match undergrowth_mode {
                UndergrowthMode::Merge => {
                    medium_vegetation_density += get_average_pixel_value(
                        &low_vegetation,
                        x_index,
                        y_index,
                        &medium_vegetation_kernel,
                        medium_vegetation_kernel_radius,
                    );
                }
                UndergrowthMode::Symbol406 | UndergrowthMode::Symbol409 => {
                    let low_vegetation_density = get_average_pixel_value(
                        &low_vegetation,
                        x_index,
                        y_index,
                        &low_vegetation_kernel,
                        low_vegetation_kernel_radius,
                    );

                    let undergrowth_color = match undergrowth_mode {
                        UndergrowthMode::Symbol406 => Some(GREEN_1),
                        UndergrowthMode::Symbol409 => Some(GREEN_3),
                        _ => None,
                    };

                    if low_vegetation_density > 1.0 {
                        match undergrowth_color {
                            Some(color) => {
                                draw_filled_rect_mut(
                                    &mut undergrowth_vegetation_img,
                                    Rect::at(x_pixel, y_pixel).of_size(
                                        casted_green_block_size_pixel,
                                        casted_green_block_size_pixel,
                                    ),
                                    color,
                                );
                            }
                            _ => (),
                        }
                    }
                }
                UndergrowthMode::None => {}
            }

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
                        &mut green_vegetation_img,
                        Rect::at(x_pixel, y_pixel)
                            .of_size(casted_green_block_size_pixel, casted_green_block_size_pixel),
                        color,
                    );
                }
                _ => (),
            }
        }
    }

    match undergrowth_mode {
        UndergrowthMode::Symbol406 => {
            imageops::overlay(&mut base_vegetation_img, &undergrowth_vegetation_img, 0, 0);
        }
        UndergrowthMode::Symbol409 => {
            let undergrowth_output_path = tile.render_dir_path.join("undergrowth.png");

            undergrowth_vegetation_img
                .save(undergrowth_output_path)
                .expect("could not save undergrowth output png");
        }
        UndergrowthMode::None | UndergrowthMode::Merge => {}
    }

    imageops::overlay(&mut base_vegetation_img, &green_vegetation_img, 0, 0);
    let vegetation_output_path = tile.render_dir_path.join("vegetation.png");

    base_vegetation_img
        .save(vegetation_output_path)
        .expect("could not save vegetation output png");

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

pub fn get_convolution_kernel_matrix(radius: usize) -> Vec<Vec<f32>> {
    let size = 2 * radius + 1;
    let sigma = radius as f32 / 2.0_f32; // avoid sigma = 0
    let two_sigma_sq = 2.0 * sigma * sigma;

    let mut kernel = vec![vec![0.0; size]; size];
    let mut sum = 0.0;

    let center = radius as isize;

    for y in 0..size {
        for x in 0..size {
            let dx = x as isize - center;
            let dy = y as isize - center;

            let value = E.powf(-((dx * dx + dy * dy) as f32) / two_sigma_sq);
            kernel[y][x] = value;
            sum += value;
        }
    }

    // Normalize so the kernel sums to 1.0
    for row in kernel.iter_mut() {
        for v in row.iter_mut() {
            *v /= sum;
        }
    }

    kernel
}

fn get_average_pixel_value(
    tif_image: &TifImage,
    x: usize,
    y: usize,
    kernel: &Vec<Vec<f32>>,
    kernel_radius: usize,
) -> f32 {
    if kernel.len() <= 1 || kernel[0].len() <= 1 {
        panic!("kernel should be a square matrix of size 2 at least")
    }

    let width = tif_image.width as usize;
    let height = tif_image.height as usize;
    let size = kernel.len();
    let radius_i = kernel_radius as isize;
    let mut weighted_sum = 0.0f32;
    let mut weight_total = 0.0f32;

    for ky in 0..size {
        for kx in 0..size {
            let nx = x as isize + kx as isize - radius_i;
            let ny = y as isize + ky as isize - radius_i;

            if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                continue;
            }

            let nxi = nx as usize;
            let nyi = ny as usize;
            let pixel = tif_image.pixels[nyi * width + nxi] as f32;
            let weight = kernel[ky][kx];

            weighted_sum += pixel * weight;
            weight_total += weight;
        }
    }

    if weight_total > 0.0 {
        return weighted_sum / weight_total;
    }

    return 0.;
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
