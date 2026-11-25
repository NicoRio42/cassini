use image::RgbaImage;
use imageproc::drawing::draw_filled_ellipse_mut;
use log::info;
use std::time::Instant;

use crate::{
    config::Config,
    constants::{BLACK, CLIFF_THICKNESS_1, CLIFF_THICKNESS_2, TRANSPARENT},
    error::{Result, ResultContext},
    grid::Grid,
    render_transform::RenderTransform,
    tile::Tile,
};

/// Renders `slopes`, the tile's `slopes.tif`, to `cliffs.png`.
pub fn render_cliffs(
    tile: &Tile,
    slopes: &Grid<f32>,
    transform: &RenderTransform,
    config: &Config,
) -> Result<()> {
    info!("Tile {}. Rendering cliffs", tile);

    let start = Instant::now();

    let (image_width, image_height) = (transform.width(), transform.height());
    let mut cliffs_layer_canvas = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);
    let radius = |thickness: f32| (transform.mm_to_px(thickness) / 2.0) as i32;
    let margin = radius(CLIFF_THICKNESS_1.max(CLIFF_THICKNESS_2));
    let extent = slopes.extent();

    for (index, slope) in slopes.values.iter().copied().enumerate() {
        let thickness = if slope > config.cliff_threshold_2 {
            CLIFF_THICKNESS_2
        } else if slope > config.cliff_threshold_1 {
            CLIFF_THICKNESS_1
        } else {
            continue;
        };

        let (x_pixel, y_pixel) =
            transform.cell_center_pixel(&extent, index % slopes.width, index / slopes.width);
        // The dots of the buffer that overlap the image are drawn, for the
        // cliffs across tile edges to be whole in the merged map.
        if x_pixel < -margin
            || x_pixel > image_width as i32 + margin
            || y_pixel < -margin
            || y_pixel > image_height as i32 + margin
        {
            continue;
        }

        draw_filled_ellipse_mut(
            &mut cliffs_layer_canvas,
            (x_pixel, y_pixel),
            radius(thickness),
            radius(thickness),
            BLACK,
        );
    }

    let cliffs_path = tile.render_dir_path.join("cliffs.png");

    cliffs_layer_canvas
        .save(&cliffs_path)
        .context(format!("could not save `{}`", cliffs_path.display()))?;

    let duration = start.elapsed();

    info!("Tile {}. Cliffs rendering done in {:.1?}", tile, duration);

    Ok(())
}
