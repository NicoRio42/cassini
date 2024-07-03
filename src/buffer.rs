use crate::tile::{NeighborTiles, Tile};
use std::process::{Command, ExitStatus};

pub fn create_tif_with_buffer(
    tile: &Tile,
    neighbor_tiles: &NeighborTiles,
    buffer: i64,
    tif_filename_without_extension: &str,
) {
    let vrt_with_buffer_path = tile.dir_path.join(format!(
        "{}-with-buffer.vrt",
        tif_filename_without_extension
    ));

    let raster_with_buffer_path = tile.dir_path.join(format!(
        "{}-with-buffer.tif",
        tif_filename_without_extension
    ));

    let tile_raster_path = tile
        .dir_path
        .join(format!("{}.tif", tif_filename_without_extension));

    let mut rasters_paths: Vec<String> = vec![tile_raster_path
        .to_str()
        .expect("Failed to convert path to string")
        .to_string()];

    let neighbors = vec![
        neighbor_tiles.top.as_ref(),
        neighbor_tiles.top_right.as_ref(),
        neighbor_tiles.right.as_ref(),
        neighbor_tiles.bottom_right.as_ref(),
        neighbor_tiles.bottom.as_ref(),
        neighbor_tiles.bottom_left.as_ref(),
        neighbor_tiles.left.as_ref(),
        neighbor_tiles.top_left.as_ref(),
    ];

    for neighbor in neighbors {
        if let Some(neighbor_tile) = neighbor {
            let path = neighbor_tile
                .dir_path
                .join(format!("{}.tif", tif_filename_without_extension));

            if let Some(path_str) = path.to_str() {
                rasters_paths.push(path_str.to_string());
            } else {
                eprintln!(
                    "Failed to convert path to string for {:?}",
                    neighbor_tile.dir_path
                );
            }
        }
    }

    // First creating a GDAL Virtual Dataset
    let gdalbuildvrt_output = Command::new("gdalbuildvrt")
        .arg(&vrt_with_buffer_path.to_str().unwrap())
        .args(&rasters_paths)
        .output()
        .expect("failed to execute gdal_contour command");

    if ExitStatus::success(&gdalbuildvrt_output.status) {
        println!("{}", String::from_utf8(gdalbuildvrt_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(gdalbuildvrt_output.stderr).unwrap());
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
        .output()
        .expect("failed to execute gdal_contour command");

    if ExitStatus::success(&gdal_translate_output.status) {
        println!(
            "{}",
            String::from_utf8(gdal_translate_output.stdout).unwrap()
        );
    } else {
        println!(
            "{}",
            String::from_utf8(gdal_translate_output.stderr).unwrap()
        );
    }
}
