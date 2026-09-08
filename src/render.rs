use crate::config::Config;
use crate::constants::INCH;
use crate::contours::generate_contours_with_pullautin_algorithme;
use crate::error::{CassiniError, Result, ResultContext, Stage, TileId};
use crate::helpers::{remove_dir_content, remove_if_exists};
use crate::process::{checked_output, ensure_file};
use crate::tile::TileWithNeighbors;
use crate::vectors::render_map_with_osm_vector_shapes;
use crate::world_file::create_world_file;
use crate::{
    cliffs::render_cliffs,
    dem::create_dem_with_buffer_and_slopes_tiff,
    tile::Tile,
    vegetation::{render_vegetation, UndergrowthMode},
};
use log::info;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

pub fn generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
    tile: Tile,
    neighbor_tiles: Vec<PathBuf>,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
    shapefiles_dir: Option<PathBuf>,
    config: &Config,
) -> Result<()> {
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
    remove_if_exists(&undergrowth_path).context(format!(
        "could not remove stale optional layer `{}`",
        undergrowth_path.display()
    ))?;

    render_vegetation(
        &tile,
        &neighbor_tiles,
        image_width,
        image_height,
        config,
        undergrowth_mode,
    )?;

    create_dem_with_buffer_and_slopes_tiff(&tile, &neighbor_tiles)?;
    generate_contours_with_pullautin_algorithme(&tile, image_width, image_height, config)?;
    render_cliffs(&tile, image_width, image_height, config)?;

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map to png",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let cliffs_path = tile.render_dir_path.join("cliffs.png");
    let vegetation_path = tile.render_dir_path.join("vegetation.png");
    let contours_path = tile.render_dir_path.join("contours.png");

    let shapes_path: Option<PathBuf> = if shapefiles_dir.is_some() {
        shapefiles_dir
    } else if !skip_vector {
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

    render_map_with_osm_vector_shapes(
        &tile,
        image_width,
        image_height,
        config,
        &vegetation_path,
        &undergrowth_path,
        &contours_path,
        &cliffs_path,
        skip_520,
        shapes_path,
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
        "Tile min_x={} min_y={} max_x={} max_y={}. Map rendered to png in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    Ok(())
}

const RENDER_STEP_FILES: [&str; 16] = [
    "cliffs.png",
    "contours",
    "contours.png",
    "contours-raw",
    "dem_2m_with_buffer.tif",
    "dem_with_buffer.tif",
    "formlines",
    "full-map.pgw",
    "full-map.png",
    "high_vegetation_with_buffer.tif",
    "low_vegetation_with_buffer.tif",
    "medium_vegetation_with_buffer.tif",
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
