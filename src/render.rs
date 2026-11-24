use crate::config::Config;
use crate::constants::INCH;
use crate::contours::generate_contours_with_pullautin_algorithme;
use crate::error::{CassiniError, Result, ResultContext, Stage, TileId};
use crate::helpers::{remove_dir_content, remove_if_exists};
use crate::map_features::MapFeature;
use crate::ocad_export::{SPILLED_OBJECTS_FILE_NAME, TILE_OCD_FILE_NAME};
use crate::ocd::OcdTemplate;
use crate::process::{checked_output, ensure_file};
use crate::raster::F32Raster;
use crate::tile::TileWithNeighbors;
use crate::vectors::{get_map_features_from_osm_shapes, render_map_with_osm_vector_shapes};
use crate::vegetation_areas::get_vegetation_features;
use crate::world_file::create_world_file;
use crate::{
    cliff_features::get_cliff_features,
    cliffs::render_cliffs,
    dem::create_dem_with_buffer_and_slopes_tiff,
    tile::Tile,
    vegetation::{render_vegetation, UndergrowthMode},
};
use log::info;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

/// Options of the render step, shared by the `process`, `render` and `batch`
/// commands.
#[derive(Clone)]
pub struct RenderOptions {
    pub skip_vector: bool,
    pub skip_520: bool,
    pub undergrowth_mode: UndergrowthMode,
    /// Shapefiles to use instead of the tile's OpenStreetMap data.
    pub shapefiles_dir: Option<PathBuf>,
    /// Also export the map to OCAD.
    pub ocad: Option<OcadOptions>,
}

#[derive(Clone)]
pub struct OcadOptions {
    pub template: Arc<OcdTemplate>,
    /// Douglas–Peucker tolerance in metres for the vegetation areas and the
    /// cliffs, 0 to keep every traced vertex.
    pub simplify_tolerance: f64,
}

/// Renders the tile to `full-map.png` from its DEM, vegetation density
/// rasters and vector data. Returns the map features to export when the OCAD
/// export is requested.
pub fn render_tile_map(
    tile: Tile,
    neighbor_tiles: Vec<PathBuf>,
    options: &RenderOptions,
    config: &Config,
) -> Result<Option<Vec<MapFeature>>> {
    let width = tile.max_x - tile.min_x;
    let height = tile.max_y - tile.min_y;
    if width <= 0 || height <= 0 {
        return Err(CassiniError::InvalidInput {
            message: format!("tile {} has a non-positive extent", TileId::from(&tile)),
        });
    }

    let image_width = (width as f32 * config.dpi_resolution / INCH) as u32;
    let image_height = (height as f32 * config.dpi_resolution / INCH) as u32;

    // Optional layers must belong to this run, not a previous render in the same directory.
    let undergrowth_path = tile.render_dir_path.join("undergrowth.png");
    let ocd_path = tile.render_dir_path.join(TILE_OCD_FILE_NAME);
    let spilled_objects_path = tile.render_dir_path.join(SPILLED_OBJECTS_FILE_NAME);
    for optional_path in [&undergrowth_path, &ocd_path, &spilled_objects_path] {
        remove_if_exists(optional_path).context(format!(
            "could not remove stale optional layer `{}`",
            optional_path.display()
        ))?;
    }

    let vegetation_rasters = render_vegetation(
        &tile,
        &neighbor_tiles,
        image_width,
        image_height,
        config,
        &options.undergrowth_mode,
    )?;
    let vegetation_features = match &options.ocad {
        Some(ocad) => get_vegetation_features(
            &tile,
            &vegetation_rasters,
            config,
            &options.undergrowth_mode,
            ocad.simplify_tolerance,
        )?,
        None => vec![],
    };
    drop(vegetation_rasters);

    create_dem_with_buffer_and_slopes_tiff(&tile, &neighbor_tiles)?;
    let contour_features =
        generate_contours_with_pullautin_algorithme(&tile, image_width, image_height, config)?;
    let slopes = F32Raster::read(&tile.render_dir_path.join("slopes.tif"))?;
    render_cliffs(&tile, &slopes, image_width, image_height, config)?;
    let cliff_features = match &options.ocad {
        Some(ocad) => get_cliff_features(&tile, &slopes, config, ocad.simplify_tolerance)?,
        None => vec![],
    };
    drop(slopes);

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map to PNG",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let cliffs_path = tile.render_dir_path.join("cliffs.png");
    let vegetation_path = tile.render_dir_path.join("vegetation.png");
    let contours_path = tile.render_dir_path.join("contours.png");

    let shapes_path: Option<PathBuf> = if options.shapefiles_dir.is_some() {
        options.shapefiles_dir.clone()
    } else if !options.skip_vector {
        let shapes_output_path = tile.render_dir_path.join("shapes");

        if shapes_output_path.exists() {
            remove_dir_content(&shapes_output_path).context(format!(
                "could not clean shapefile directory `{}`",
                shapes_output_path.display()
            ))?;
        }

        let osm_path = tile
            .render_dir_path
            .join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));

        info!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Transforming osm file to shapefiles",
            tile.min_x, tile.min_y, tile.max_x, tile.max_y
        );

        checked_output(
            Command::new("ogr2ogr")
                .args([
                    "--config",
                    "OSM_USE_CUSTOM_INDEXING",
                    "NO",
                    "-f",
                    "ESRI Shapefile",
                ])
                .arg(&shapes_output_path)
                .arg(&osm_path)
                .args([
                    "-t_srs",
                    "EPSG:2154",
                    "-nlt",
                    "MULTIPOLYGON",
                    "-sql",
                    "SELECT * FROM multipolygons",
                ])
                .arg("--quiet"),
            Stage::Vectors,
            Some(TileId::from(&tile)),
        )?;
        ensure_file(&shapes_output_path.join("multipolygons.shp"))?;

        checked_output(
            Command::new("ogr2ogr")
                .args([
                    "--config",
                    "OSM_USE_CUSTOM_INDEXING",
                    "NO",
                    "-f",
                    "ESRI Shapefile",
                ])
                .arg(&shapes_output_path)
                .arg(&osm_path)
                .args([
                    "-t_srs",
                    "EPSG:2154",
                    "-nlt",
                    "LINESTRING",
                    "-sql",
                    "SELECT * FROM lines",
                ])
                .arg("--quiet"),
            Stage::Vectors,
            Some(TileId::from(&tile)),
        )?;
        ensure_file(&shapes_output_path.join("lines.shp"))?;

        Some(shapes_output_path)
    } else {
        None
    };

    let osm_features = match shapes_path {
        Some(shapes_path) => {
            get_map_features_from_osm_shapes(&tile, &shapes_path, options.skip_520)?
        }
        None => vec![],
    };

    render_map_with_osm_vector_shapes(
        &tile,
        image_width,
        image_height,
        config,
        &vegetation_path,
        &undergrowth_path,
        &contours_path,
        &cliffs_path,
        &osm_features,
    )?;

    let resolution = INCH / (config.dpi_resolution);
    let world_file_path = tile.render_dir_path.join("full-map.pgw");

    create_world_file(
        tile.min_x as f32,
        tile.max_y as f32,
        resolution,
        &world_file_path,
    )
    .context(format!("could not create `{}`", world_file_path.display()))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Map rendered to PNG in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    if options.ocad.is_none() {
        return Ok(None);
    }

    let mut features = vegetation_features;
    features.extend(cliff_features);
    features.extend(contour_features);
    features.extend(osm_features);
    Ok(Some(features))
}

const RENDER_STEP_FILES: [&str; 18] = [
    "cliffs.png",
    "contours",
    "contours.png",
    "contours-raw",
    "dem_2m_with_buffer.tif",
    "dem_with_buffer.tif",
    "formlines",
    TILE_OCD_FILE_NAME,
    "full-map.pgw",
    "full-map.png",
    "high_vegetation_with_buffer.tif",
    "low_vegetation_with_buffer.tif",
    "medium_vegetation_with_buffer.tif",
    SPILLED_OBJECTS_FILE_NAME,
    "shapes",
    "slopes.tif",
    "undergrowth.png",
    "vegetation.png",
];

pub fn cleanup_render_step_files(tiles: &[TileWithNeighbors], output_dir: &str) -> Result<()> {
    for tile in tiles {
        cleanup_render_step_files_for_single_tile(tile)?;
    }

    let entries = std::fs::read_dir(output_dir)
        .context(format!("could not read output directory `{output_dir}`"))?;
    for entry in entries {
        let entry = entry.context(format!("could not read an entry in `{output_dir}`"))?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("merged-map")
        {
            std::fs::remove_file(entry.path()).context(format!(
                "could not remove stale merged map `{}`",
                entry.path().display()
            ))?;
        }
    }
    Ok(())
}

pub fn cleanup_render_step_files_for_single_tile(tile: &TileWithNeighbors) -> Result<()> {
    for path in RENDER_STEP_FILES {
        let artifact = tile.tile.render_dir_path.join(path);
        remove_if_exists(&artifact).context(format!(
            "could not remove stale artifact `{}`",
            artifact.display()
        ))?;
    }
    Ok(())
}
