use crate::{
    config::Config, pullautin_contours_render::pullautin_render_contours,
    pullautin_smooth_contours::pullautin_smooth_contours, tile::Tile,
};

pub fn generate_contours_with_pullautin_algorithme(
    tile: &Tile,
    image_width: u32,
    image_height: u32,
    config: &Config,
) {
    let avg_alt = pullautin_smooth_contours(&tile);
    pullautin_render_contours(&tile, image_width, image_height, &config, avg_alt);
}
