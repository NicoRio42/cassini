use crate::{
    config::Config,
    constants::{GREEN_1, GREEN_2, GREEN_3, INCH, WHITE, YELLOW},
    tile::{NeighborTiles, Tile},
};
use image::{Rgba, RgbaImage};
use imageproc::{drawing::draw_filled_rect_mut, rect::Rect};
use std::{fs::File, path::PathBuf};
use tiff::decoder::{Decoder, DecodingResult};

pub fn render_vegetation(
    tile: Tile,
    neighbor_tiles: NeighborTiles,
    image_width: u32,
    image_height: u32,
    buffer: u64,
    config: &Config,
) {
    println!("Rendering vegetation");

    let vegetation_block_size_pixel =
        config.vegetation_block_size as f32 * config.dpi_resolution / INCH;

    let casted_vegetation_block_size_pixel = vegetation_block_size_pixel.ceil() as u32;

    let medium_vegetation_path = tile.dir_path.join("medium-vegetation.tif");
    let high_vegetation_path = tile.dir_path.join("high-vegetation.tif");

    let forest_density_tif_file =
        File::open(high_vegetation_path).expect("Cannot find high vegetation tif image!");

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

        let x_pixel = ((x as i64 - buffer as i64) as f32 * vegetation_block_size_pixel) as i32;
        let y_pixel = ((y as i64 - buffer as i64) as f32 * vegetation_block_size_pixel) as i32;

        if x_pixel < 0
            || x_pixel > image_width as i32
            || y_pixel < 0
            || y_pixel > image_height as i32
        {
            continue;
        }

        let forest_density = image_data[index];

        if forest_density > config.yellow_threshold {
            continue;
        }

        draw_filled_rect_mut(
            &mut vegetation_layer_img,
            Rect::at(x_pixel, y_pixel).of_size(
                casted_vegetation_block_size_pixel,
                casted_vegetation_block_size_pixel,
            ),
            YELLOW,
        );
    }

    let green_density_tif_file =
        File::open(medium_vegetation_path).expect("Cannot find medium vegetation tif image!");

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

        let x_pixel = ((x as i64 - buffer as i64) as f32 * vegetation_block_size_pixel) as i32;
        let y_pixel = ((y as i64 - buffer as i64) as f32 * vegetation_block_size_pixel) as i32;

        if x_pixel < 0
            || x_pixel > image_width as i32
            || y_pixel < 0
            || y_pixel > image_height as i32
        {
            continue;
        }

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

    let vegetation_output_path = out_dir.join("vegetation.png");

    vegetation_layer_img
        .save(vegetation_output_path)
        .expect("could not save output png");
}

fn get_pixels_with_buffer(
    tile: Tile,
    neighbor_tiles: NeighborTiles,
    buffer_pixels: usize,
    tif_filename: &str,
) {
    // Because 1pixel = 1 metter
    let tile_width_pixels = (tile.max_x - tile.min_x) as usize;
    let tile_height_pixels = (tile.max_y - tile.min_y) as usize;

    // bottom left to top right
    let mut pixels: Vec<Vec<f64>> = vec![
        vec![0.0; buffer_pixels * 2 + tile_height_pixels];
        buffer_pixels * 2 + tile_height_pixels
    ];

    // TODO: find a way to skip iteration of out of bound points if perf issues
    if neighbor_tiles.top.is_some() {
        let neighbor_tile = neighbor_tiles.top.unwrap();
        // Because 1pixel = 1 metter
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let neighbor_tile_height_pixels = (neighbor_tile.max_y - neighbor_tile.min_y) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let y = index / neighbor_tile_width_pixels;

            if y < neighbor_tile_height_pixels - buffer_pixels {
                continue;
            }

            let x = index % neighbor_tile_width_pixels;

            pixels[x + buffer_pixels]
                [buffer_pixels * 2 + tile_height_pixels + y - neighbor_tile_height_pixels] =
                image_data[index];
        }
    }

    if neighbor_tiles.top_right.is_some() {
        let neighbor_tile = neighbor_tiles.top_right.unwrap();
        // Because 1pixel = 1 metter
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let neighbor_tile_height_pixels = (neighbor_tile.max_y - neighbor_tile.min_y) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % usize::try_from(neighbor_tile_width_pixels).unwrap();
            let y = index / usize::try_from(neighbor_tile_width_pixels).unwrap();

            if y < neighbor_tile_height_pixels - buffer_pixels || x > buffer_pixels {
                continue;
            }

            pixels[buffer_pixels + tile_width_pixels + x]
                [buffer_pixels * 2 + tile_height_pixels + y - neighbor_tile_height_pixels] =
                image_data[index];
        }
    }

    if neighbor_tiles.right.is_some() {
        let neighbor_tile = neighbor_tiles.right.unwrap();
        // Because 1pixel = 1 metter
        let neighbor_tile_width_pixels = (neighbor_tile.max_x - neighbor_tile.min_x) as usize;
        let neighbor_tile_height_pixels = (neighbor_tile.max_y - neighbor_tile.min_y) as usize;
        let image_data = get_image_data_from_tif(&neighbor_tile.dir_path.join(tif_filename));

        for index in 0..image_data.len() {
            let x = index % usize::try_from(neighbor_tile_width_pixels).unwrap();
            let y = index / usize::try_from(neighbor_tile_width_pixels).unwrap();

            if x > buffer_pixels {
                continue;
            }

            pixels[buffer_pixels + tile_width_pixels + x]
                [buffer_pixels * 2 + tile_height_pixels + y - neighbor_tile_height_pixels] =
                image_data[index];
        }
    }
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
