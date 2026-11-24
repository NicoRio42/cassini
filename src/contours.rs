use crate::{
    config::Config,
    error::Result,
    map_features::MapFeature,
    marching_squares::ContourLine,
    pullautin_contours_render::pullautin_cull_formlines_render_contours,
    pullautin_smooth_contours::{get_elevation_matrix_from_dem, pullautin_smooth_contours},
    tile::Tile,
};

/// Smooths `raw_contours`, renders them to `contours.png` and returns them,
/// with the culled form lines, as map features.
pub fn generate_contours_with_pullautin_algorithme(
    tile: &Tile,
    raw_contours: Vec<ContourLine>,
    image_width: u32,
    image_height: u32,
    config: &Config,
) -> Result<Vec<MapFeature>> {
    let avg_alt = get_elevation_matrix_from_dem(tile)?;

    let smoothed_contours = pullautin_smooth_contours(tile, &avg_alt, raw_contours)?;

    pullautin_cull_formlines_render_contours(
        tile,
        image_width,
        image_height,
        config,
        &avg_alt,
        smoothed_contours,
    )
}
