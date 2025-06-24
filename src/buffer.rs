use log::error;

use crate::tile::Tile;
use std::{
    path::PathBuf,
    process::{Command, ExitStatus},
};

pub fn create_raster_with_buffer(
    tile: &Tile,
    neighbor_tiles: &Vec<PathBuf>,
    buffer: i64,
    tif_filename_without_extension: &str,
) {
    let vrt_with_buffer_path = tile
        .render_dir_path
        .join(format!("{}-with-buffer.vrt", tif_filename_without_extension));

    let raster_with_buffer_path = tile
        .render_dir_path
        .join(format!("{}-with-buffer.tif", tif_filename_without_extension));

    let tile_raster_path = tile
        .lidar_dir_path
        .join(format!("{}.tif", tif_filename_without_extension));

    let mut rasters_paths: Vec<String> = vec![tile_raster_path
        .to_str()
        .expect("Failed to convert path to string")
        .to_string()];

    for neighbor_tile in neighbor_tiles {
        let path = neighbor_tile.join(format!("{}.tif", tif_filename_without_extension));

        if let Some(path_str) = path.to_str() {
            rasters_paths.push(path_str.to_string());
        } else {
            error!(
                "Tile min_x={} min_y={} max_x={} max_y={}. Failed to convert path to string for {:?}",
                tile.min_x, tile.min_y, tile.max_x, tile.max_y, neighbor_tile
            );
        }
    }

    // First creating a GDAL Virtual Dataset
    let gdalbuildvrt_output = Command::new("gdalbuildvrt")
        .arg(&vrt_with_buffer_path.to_str().unwrap())
        .args(&rasters_paths)
        .arg("--quiet")
        .output()
        .expect("failed to execute gdalbuildvrt command");

    if !ExitStatus::success(&gdalbuildvrt_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Gdalbuildvrt command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(gdalbuildvrt_output.stderr).unwrap()
        );
    }

    // Then outpouting croped tif with buffer
    let gdal_translate_output = Command::new("gdal_translate")
        .args([
            "-projwin",
            &(tile.min_x - buffer).to_string(),
            &(tile.max_y + buffer).to_string(),
            &(tile.max_x + buffer).to_string(),
            &(tile.min_y - buffer).to_string(),
        ])
        .args(["-of", "GTiff"])
        .arg(&vrt_with_buffer_path.to_str().unwrap())
        .arg(&raster_with_buffer_path.to_str().unwrap())
        .arg("--quiet")
        .output()
        .expect("failed to execute gdal_translate command");

    if !ExitStatus::success(&gdal_translate_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Gdal_translate command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(gdal_translate_output.stderr).unwrap()
        );
    }
}
