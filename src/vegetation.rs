use crate::{
    config::Config,
    constants::{GREEN_1, GREEN_2, GREEN_3, INCH, WHITE, YELLOW},
    tile::{NeighborTiles, Tile},
};
use image::{Rgba, RgbaImage};
use imageproc::{drawing::draw_filled_rect_mut, rect::Rect};
use std::{fs::File, path::PathBuf, time::Instant};
use tiff::decoder::{Decoder, DecodingResult};

pub fn render_vegetation(
    tile: &Tile,
    neighbor_tiles: &NeighborTiles,
    image_width: u32,
    image_height: u32,
    buffer: usize,
    config: &Config,
) {
    println!("Rendering vegetation");

    let vegetation_block_size_pixel =
        config.vegetation_block_size as f32 * config.dpi_resolution / INCH;

    let casted_vegetation_block_size_pixel = vegetation_block_size_pixel.ceil() as u32;

    let high_vegetation_pixels_with_buffer =
        get_pixels_with_buffer(&tile, &neighbor_tiles, buffer, "high-vegetation.tif");

    let medium_vegetation_pixels_with_buffer =
        get_pixels_with_buffer(&tile, &neighbor_tiles, buffer, "medium-vegetation.tif");

    let mut vegetation_layer_img = RgbaImage::from_pixel(image_width, image_height, WHITE);

    for x_index in buffer..((tile.max_x + buffer as i64 - tile.min_x) as usize) {
        for y_index in buffer..((tile.max_y + buffer as i64 - tile.min_y) as usize) {
            let x_pixel = ((x_index - buffer) as f32 * vegetation_block_size_pixel) as i32;
            let y_pixel = ((y_index - buffer) as f32 * vegetation_block_size_pixel) as i32;

            let high_vegetation_density =
                get_average_pixel_value(&high_vegetation_pixels_with_buffer, x_index, y_index, 3);

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

            let medium_vegetation_density =
                get_average_pixel_value(&medium_vegetation_pixels_with_buffer, x_index, y_index, 3);

            let mut green_color: Option<Rgba<u8>> = None;

            // println!("{}", medium_vegetation_density);

            if medium_vegetation_density > config.green_3_threshold {
                green_color = Some(GREEN_3);
            } else if medium_vegetation_density > config.green_2_threshold {
                green_color = Some(GREEN_2);
            } else if medium_vegetation_density > config.green_1_threshold {
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

    let vegetation_output_path = tile.dir_path.join("vegetation.png");

    vegetation_layer_img
        .save(vegetation_output_path)
        .expect("could not save output png");
}

fn get_average_pixel_value(
    pixels: &Vec<Vec<f64>>,
    x_index: usize,
    y_index: usize,
    distance: usize,
) -> f64 {
    // TODO: fix this naive averaging function
    let mut count = 0.0;
    let mut sum = 0.0;

    if pixels.len() == 0 {
        panic!("Image with null width")
    }

    let pixels_y_len = pixels[0].len() - 1;

    let min_x = if distance > x_index {
        0
    } else {
        x_index - distance
    };
    let max_x = if x_index + distance > pixels.len() {
        pixels.len()
    } else {
        x_index + distance + 1
    };
    let min_y = if distance > y_index {
        0
    } else {
        y_index - distance
    };
    let max_y = if y_index + distance > pixels_y_len {
        pixels_y_len
    } else {
        y_index + distance + 1
    };

    for x in min_x..max_x {
        for y in min_y..max_y {
            count += 1.0;
            sum += pixels[x][y];
        }
    }

    return sum / count;
}

fn get_pixels_with_buffer(
    tile: &Tile,
    neighbor_tiles: &NeighborTiles,
    buffer_pixels: usize,
    tif_filename: &str,
) -> Vec<Vec<f64>> {
    // Because 1pixel = 1 metter
    let tile_width_pixels = (tile.max_x - tile.min_x) as usize;
    let tile_height_pixels = (tile.max_y - tile.min_y) as usize;

    // bottom left to top right
    let mut pixels: Vec<Vec<f64>> = vec![
        vec![0.0; buffer_pixels * 2 + tile_height_pixels];
        buffer_pixels * 2 + tile_height_pixels
    ];

    let tile_image_data = get_image_data_from_tif(&tile.dir_path.join(tif_filename));

    for index in 0..tile_image_data.len() {
        let y = index / tile_width_pixels;
        let x = index % tile_width_pixels;

        pixels[buffer_pixels + x][buffer_pixels + y] = tile_image_data[index];
    }

    // TODO: find a way to skip iteration of out of bound points if perf issues
    if neighbor_tiles.top.is_some() {
        let neighbor_tile = neighbor_tiles.top.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let y = index / neighbor_tile_width_pixels;

            if y > buffer_pixels - 1 {
                continue;
            }

            let x = index % neighbor_tile_width_pixels;
            pixels[buffer_pixels + x][buffer_pixels + tile_height_pixels + y] = image_data[index];
        }
    }

    if neighbor_tiles.top_right.is_some() {
        let neighbor_tile = neighbor_tiles.top_right.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % neighbor_tile_width_pixels;
            let y = index / neighbor_tile_width_pixels;

            if y > buffer_pixels - 1 || x > buffer_pixels - 1 {
                continue;
            }

            pixels[buffer_pixels + tile_width_pixels + x][buffer_pixels + tile_height_pixels + y] =
                image_data[index];
        }
    }

    if neighbor_tiles.right.is_some() {
        let neighbor_tile = neighbor_tiles.right.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % neighbor_tile_width_pixels;

            if x > buffer_pixels - 1 {
                continue;
            }

            let y = index / neighbor_tile_width_pixels;
            pixels[buffer_pixels + tile_width_pixels + x][buffer_pixels + y] = image_data[index];
        }
    }

    if neighbor_tiles.bottom_right.is_some() {
        let neighbor_tile = neighbor_tiles.bottom_right.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let neighbor_tile_height_pixels = (neighbor_tile.max_y - neighbor_tile.min_y) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % neighbor_tile_width_pixels;
            let y = index / neighbor_tile_width_pixels;

            if x > buffer_pixels - 1 || y < neighbor_tile_height_pixels - buffer_pixels {
                continue;
            }

            pixels[buffer_pixels + tile_width_pixels + x]
                [y + buffer_pixels - neighbor_tile_height_pixels] = image_data[index];
        }
    }

    if neighbor_tiles.bottom.is_some() {
        let neighbor_tile = neighbor_tiles.bottom.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let neighbor_tile_height_pixels = (neighbor_tile.max_y - neighbor_tile.min_y) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let y = index / neighbor_tile_width_pixels;

            if y < neighbor_tile_height_pixels - buffer_pixels {
                continue;
            }

            let x = index % neighbor_tile_width_pixels;

            pixels[buffer_pixels + x][y + buffer_pixels - neighbor_tile_height_pixels] =
                image_data[index];
        }
    }

    if neighbor_tiles.bottom_left.is_some() {
        let neighbor_tile = neighbor_tiles.bottom_left.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let neighbor_tile_height_pixels = (neighbor_tile.max_y - neighbor_tile.min_y) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % neighbor_tile_width_pixels;
            let y = index / neighbor_tile_width_pixels;

            if x < neighbor_tile_width_pixels - buffer_pixels
                || y < neighbor_tile_height_pixels - buffer_pixels
            {
                continue;
            }

            pixels[x + buffer_pixels - neighbor_tile_width_pixels]
                [y + buffer_pixels - neighbor_tile_height_pixels] = image_data[index];
        }
    }

    if neighbor_tiles.left.is_some() {
        let neighbor_tile = neighbor_tiles.left.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % neighbor_tile_width_pixels;

            if x < neighbor_tile_width_pixels - buffer_pixels {
                continue;
            }

            let y = index / neighbor_tile_width_pixels;

            pixels[x + buffer_pixels - neighbor_tile_width_pixels][buffer_pixels + y] =
                image_data[index];
        }
    }

    if neighbor_tiles.top_left.is_some() {
        let neighbor_tile = neighbor_tiles.top_left.as_ref().unwrap();
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % neighbor_tile_width_pixels;
            let y = index / neighbor_tile_width_pixels;

            if x < neighbor_tile_width_pixels - buffer_pixels || y > buffer_pixels - 1 {
                continue;
            }

            pixels[x + buffer_pixels - neighbor_tile_width_pixels]
                [buffer_pixels + tile_height_pixels + y] = image_data[index];
        }
    }

    return pixels;
}

fn get_image_data_from_tif(path: &PathBuf) -> Vec<f64> {
    let tif_file = File::open(path).expect("Cannot find high vegetation tif image!");
    let mut img_decoder = Decoder::new(tif_file).expect("Cannot create decoder");
    img_decoder = img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let DecodingResult::F64(image_data) = img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    return image_data;
}
