use crate::tile::{NeighborTiles, Tile};
use std::process::{Command, ExitStatus};

pub fn create_dem_with_buffer(tile: &Tile, neighbor_tiles: &NeighborTiles, buffer: i64) {
    println!("Generating dem with buffer.");

    let dem_vrt_with_buffer_path = tile.dir_path.join("dem-with-buffer.vrt");
    let dem_with_buffer_path = tile.dir_path.join("dem-with-buffer.tif");
    let tile_dem_path = tile.dir_path.join("dem.tif");

    let mut dems_paths: Vec<String> = vec![tile_dem_path
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
            let path = neighbor_tile.dir_path.join("dem.tif");
            if let Some(path_str) = path.to_str() {
                dems_paths.push(path_str.to_string());
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
        .arg(&dem_vrt_with_buffer_path.to_str().unwrap())
        .args(&dems_paths)
        .output()
        .expect("failed to execute gdal_contour command");

    if ExitStatus::success(&gdalbuildvrt_output.status) {
        println!("{}", String::from_utf8(gdalbuildvrt_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(gdalbuildvrt_output.stderr).unwrap());
    }

    // Then outpouting croped dem with buffer
    let gdal_translate_output = Command::new("gdal_translate")
        .args([
            "-projwin",
            &(tile.min_x - buffer).to_string(),
            &(tile.max_y + buffer).to_string(),
            &(tile.max_x + buffer).to_string(),
            &(tile.min_y - buffer).to_string(),
        ])
        .args(["-of", "GTiff"])
        .arg(&dem_vrt_with_buffer_path.to_str().unwrap())
        .arg(&dem_with_buffer_path.to_str().unwrap())
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

    // Then outpouting croped dem with buffer
    let gdal_fillnodata_output = Command::new("gdal_fillnodata")
        .arg(&dem_with_buffer_path.to_str().unwrap())
        .arg(&dem_with_buffer_path.to_str().unwrap())
        .output()
        .expect("failed to execute gdal_contour command");

    if ExitStatus::success(&gdal_fillnodata_output.status) {
        println!(
            "{}",
            String::from_utf8(gdal_fillnodata_output.stdout).unwrap()
        );
    } else {
        println!(
            "{}",
            String::from_utf8(gdal_fillnodata_output.stderr).unwrap()
        );
    }

    println!("Generating countours shapefiles.");

    let contours_raw_path = tile.dir_path.join("contours-raw.shp");

    let gdal_contours_output = Command::new("gdal_contour")
        .args([
            "-a",
            "elev",
            &dem_with_buffer_path.to_str().unwrap(),
            &contours_raw_path.to_str().unwrap(),
            "-i",
            "2.5",
        ])
        .output()
        .expect("failed to execute gdal_contour command");

    if ExitStatus::success(&gdal_contours_output.status) {
        println!(
            "{}",
            String::from_utf8(gdal_contours_output.stdout).unwrap()
        );
    } else {
        println!(
            "{}",
            String::from_utf8(gdal_contours_output.stderr).unwrap()
        );
    }

    println!("Simplifying countours shapefiles.");

    let contours_path = tile.dir_path.join("contours.shp");

    let contours_simplify_output = Command::new("ogr2ogr")
        .args([
            "-simplify",
            "0.5",
            &contours_path.to_str().unwrap(),
            &contours_raw_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute ogr2ogr command");

    if ExitStatus::success(&contours_simplify_output.status) {
        println!(
            "{}",
            String::from_utf8(contours_simplify_output.stdout).unwrap()
        );
    } else {
        println!(
            "{}",
            String::from_utf8(contours_simplify_output.stderr).unwrap()
        );
    }

    println!("Generating slopes tif image.");

    let slopes_path = tile.dir_path.join("slopes.tif");

    let gdaldem_output = Command::new("gdaldem")
        .args([
            "slope",
            &dem_with_buffer_path.to_str().unwrap(),
            &slopes_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute gdaldem command");

    if ExitStatus::success(&gdaldem_output.status) {
        println!("{}", String::from_utf8(gdaldem_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(gdaldem_output.stderr).unwrap());
    }
}
