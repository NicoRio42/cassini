use crate::{
    config::Config,
    error::{CassiniError, Result, ResultContext},
    map_features::{LineSymbol, MapFeature},
    pullautin_contours_render::{get_contour_symbol, pullautin_cull_formlines_render_contours},
    pullautin_smooth_contours::{get_elevation_matrix_from_dem, pullautin_smooth_contours},
    tile::Tile,
};
use shapefile::{
    dbase::{FieldValue, Record},
    read_as, Polyline,
};

pub fn generate_contours_with_pullautin_algorithme(
    tile: &Tile,
    image_width: u32,
    image_height: u32,
    config: &Config,
) -> Result<()> {
    let avg_alt = get_elevation_matrix_from_dem(tile)?;

    let smoothed_contours = pullautin_smooth_contours(tile, &avg_alt)?;

    pullautin_cull_formlines_render_contours(
        tile,
        image_width,
        image_height,
        config,
        &avg_alt,
        smoothed_contours,
    )
}

/// Reads the contours and culled form lines written by
/// `generate_contours_with_pullautin_algorithme` as map features.
pub fn get_contour_map_features(tile: &Tile) -> Result<Vec<MapFeature>> {
    let mut features: Vec<MapFeature> = vec![];

    let contours_path = tile.render_dir_path.join("contours").join("contours.shp");
    let contours = read_as::<_, Polyline, Record>(&contours_path).context(format!(
        "could not read contour shapefile `{}`",
        contours_path.display()
    ))?;

    for (line, record) in contours {
        let elevation = match record.get("elev") {
            Some(FieldValue::Numeric(Some(x))) => *x,
            Some(_) => continue,
            None => {
                return Err(CassiniError::InvalidArtifact {
                    path: contours_path.clone(),
                    message: "required `elev` field is missing".to_owned(),
                });
            }
        };

        match get_contour_symbol(elevation) {
            // Only the culled parts of form lines are mapped, they are read below
            LineSymbol::FormLine103 => {}
            symbol => features.push(MapFeature::Line(symbol, line)),
        }
    }

    let formlines_path = tile.render_dir_path.join("formlines").join("formlines.shp");
    let formlines = read_as::<_, Polyline, Record>(&formlines_path).context(format!(
        "could not read formline shapefile `{}`",
        formlines_path.display()
    ))?;

    for (line, _) in formlines {
        features.push(MapFeature::Line(LineSymbol::FormLine103, line));
    }

    Ok(features)
}
