use crate::constants::INCH;
use crate::contours::generate_contours_with_pullautin_algorithme;
use crate::download::download_osm_file;
use crate::helpers::{remove_dir_content, remove_if_exists};
use crate::tile::TileWithNeighbors;
use crate::vectors::render_map_with_osm_vector_shapes;
use crate::world_file::create_world_file;
use crate::UndergrowthMode;
use crate::{
    cliffs::render_cliffs, config::get_config, dem::create_dem_with_buffer_and_slopes_tiff, tile::Tile,
    vegetation::render_vegetation,
};
use log::{error, info};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use std::time::Instant;

pub fn generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
    tile: Tile,
    neighbor_tiles: Vec<PathBuf>,
    skip_vector: bool,
    skip_520: bool,
    undergrowth_mode: &UndergrowthMode,
    shapefiles_dir: Option<PathBuf>,
) {
    let config = get_config();
    let image_width = ((tile.max_x - tile.min_x) as f32 * config.dpi_resolution / INCH) as u32;
    let image_height = ((tile.max_y - tile.min_y) as f32 * config.dpi_resolution / INCH) as u32;

    render_vegetation(
        &tile,
        &neighbor_tiles,
        image_width,
        image_height,
        &config,
        undergrowth_mode,
    );

    create_dem_with_buffer_and_slopes_tiff(&tile, &neighbor_tiles);
    generate_contours_with_pullautin_algorithme(&tile, image_width, image_height, &config);
    render_cliffs(&tile, image_width, image_height, &config);

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map to png",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let cliffs_path = tile.render_dir_path.join("cliffs.png");
    let vegetation_path = tile.render_dir_path.join("vegetation.png");
    let undergrowth_path = tile.render_dir_path.join("undergrowth.png");
    let contours_path = tile.render_dir_path.join("contours.png");

    let shapes_path: Option<PathBuf> = if shapefiles_dir.is_some() {
        shapefiles_dir
    } else if !skip_vector {
        let shapes_output_path = tile.render_dir_path.join("shapes");

        if shapes_output_path.exists() {
            remove_dir_content(&shapes_output_path).unwrap();
        }

        let osm_path = tile
            .render_dir_path
            .join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));

        if !osm_path.exists() {
            download_osm_file(
                tile.min_x,
                tile.min_y,
                tile.max_x,
                tile.max_y,
                &tile.render_dir_path.to_path_buf(),
            );
        }

        info!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Transforming osm file to shapefiles",
            tile.min_x, tile.min_y, tile.max_x, tile.max_y
        );

        let ogr2ogr_output = Command::new("ogr2ogr")
            .args([
                "--config",
                "OSM_USE_CUSTOM_INDEXING",
                "NO",
                "-f",
                "ESRI Shapefile",
                &shapes_output_path.to_str().unwrap(),
                &osm_path.to_str().unwrap(),
                "-t_srs",
                "EPSG:2154",
                "-nlt",
                "MULTIPOLYGON",
                "-sql",
                "SELECT * FROM multipolygons",
            ])
            .arg("--quiet")
            .output()
            .expect("failed to execute ogr2ogr command");

        if !ExitStatus::success(&ogr2ogr_output.status) {
            error!(
                "Tile min_x={} min_y={} max_x={} max_y={}. Ogr2ogr command failed {:?}",
                tile.min_x,
                tile.min_y,
                tile.max_x,
                tile.max_y,
                String::from_utf8(ogr2ogr_output.stderr).unwrap()
            );
        }

        let ogr2ogr_output = Command::new("ogr2ogr")
            .args([
                "--config",
                "OSM_USE_CUSTOM_INDEXING",
                "NO",
                "-f",
                "ESRI Shapefile",
                &shapes_output_path.to_str().unwrap(),
                &osm_path.to_str().unwrap(),
                "-t_srs",
                "EPSG:2154",
                "-nlt",
                "LINESTRING",
                "-sql",
                "SELECT * FROM lines",
            ])
            .arg("--quiet")
            .output()
            .expect("failed to execute ogr2ogr command");

        if !ExitStatus::success(&ogr2ogr_output.status) {
            error!(
                "Tile min_x={} min_y={} max_x={} max_y={}. Ogr2ogr command failed {:?}",
                tile.min_x,
                tile.min_y,
                tile.max_x,
                tile.max_y,
                String::from_utf8(ogr2ogr_output.stderr).unwrap()
            );
        }

        Some(shapes_output_path)
    } else {
        None
    };

    render_map_with_osm_vector_shapes(
        &tile,
        image_width,
        image_height,
        &config,
        &vegetation_path,
        &undergrowth_path,
        &contours_path,
        &cliffs_path,
        skip_520,
        shapes_path,
    );

    let resolution = INCH / (config.dpi_resolution);
    let world_file_path = tile.render_dir_path.join("full-map.pgw");

    create_world_file(tile.min_x as f32, tile.max_y as f32, resolution, &world_file_path)
        .expect("Could not create world file");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Map rendered to png in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}

const RENDER_STEP_FILES: [&str; 16] = [
    "cliffs.png",
    "contours",
    "contours.png",
    "contours-raw",
    "dem-low-resolution-with-buffer.tif",
    "dem-with-buffer.tif",
    "formlines",
    "full-map.pgw",
    "full-map.png",
    "high-vegetation-with-buffer.tif",
    "low-vegetation-with-buffer.tif",
    "medium-vegetation-with-buffer.tif",
    "shapes",
    "slopes.tif",
    "undergrowth.png",
    "vegetation.png",
];

pub fn cleanup_render_step_files(tiles: &Vec<TileWithNeighbors>, output_dir: &str) {
    for tile in tiles {
        cleanup_render_step_files_for_single_tile(tile);
    }

    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("merged-map") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub fn cleanup_render_step_files_for_single_tile(tile: &TileWithNeighbors) {
    for path in RENDER_STEP_FILES {
        let _ = remove_if_exists(tile.tile.render_dir_path.join(path));
    }
}
