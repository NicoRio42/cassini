use crate::{
    config::Config, pullautin_contours_render::pullautin_render_contours,
    pullautin_raw_contours::xyz2contours, pullautin_smooth_contours::smoothjoin, tile::Tile,
};

pub fn generate_contours_with_pullautin_algorithme(
    tile: &Tile,
    image_width: u32,
    image_height: u32,
    config: &Config,
) {
    let avg_alt = xyz2contours(&tile);
    let avg_alt = smoothjoin(&tile, avg_alt);

    pullautin_render_contours(&tile, image_width, image_height, &config, avg_alt);
}
