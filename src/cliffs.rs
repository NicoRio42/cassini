use image::RgbaImage;
use imageproc::drawing::draw_filled_ellipse_mut;
use log::info;
use std::time::Instant;

use crate::{
    config::Config,
    constants::{
        BLACK, BUFFER, CLIFF_THICKNESS_1, CLIFF_THICKNESS_2, DEM_BLOCK_SIZE, INCH, TRANSPARENT,
    },
    error::{Result, ResultContext},
    grid::Grid,
    tile::Tile,
};

/// Renders `slopes`, the tile's `slopes.tif`, to `cliffs.png`.
pub fn render_cliffs(
    tile: &Tile,
    slopes: &Grid<f32>,
    image_width: u32,
    image_height: u32,
    config: &Config,
) -> Result<()> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering cliffs",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let dem_block_size_pixel = DEM_BLOCK_SIZE as f32 * config.dpi_resolution / INCH;

    let mut cliffs_layer_canvas = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);

    for (index, slope) in slopes.values.iter().copied().enumerate() {
        let x = index % slopes.width;
        let y = index / slopes.width;

        let x_pixel = ((x as i64 - (BUFFER as f64 / DEM_BLOCK_SIZE) as i64) as f32
            * dem_block_size_pixel) as i32;
        let y_pixel = ((y as i64 - (BUFFER as f64 / DEM_BLOCK_SIZE) as i64) as f32
            * dem_block_size_pixel) as i32;

        if x_pixel < 0
            || x_pixel > image_width as i32
            || y_pixel < 0
            || y_pixel > image_height as i32
        {
            continue;
        }

        let mut cliff_thickness: Option<f32> = None;

        if slope > config.cliff_threshold_2 {
            cliff_thickness = Some(CLIFF_THICKNESS_2);
        } else if slope > config.cliff_threshold_1 {
            cliff_thickness = Some(CLIFF_THICKNESS_1);
        }

        if let Some(thickness) = cliff_thickness {
            draw_filled_ellipse_mut(
                &mut cliffs_layer_canvas,
                (x_pixel, y_pixel),
                (thickness * config.dpi_resolution * 10.0 / INCH / 2.0) as i32,
                (thickness * config.dpi_resolution * 10.0 / INCH / 2.0) as i32,
                BLACK,
            );
        }
    }

    let cliffs_path = tile.render_dir_path.join("cliffs.png");

    cliffs_layer_canvas
        .save(&cliffs_path)
        .context(format!("could not save `{}`", cliffs_path.display()))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Cliffs rendering done in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    Ok(())
}
