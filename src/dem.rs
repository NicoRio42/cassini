use crate::{
    buffer::create_tif_with_buffer,
    tile::{NeighborTiles, Tile},
};
use std::process::{Command, ExitStatus};

pub fn create_dem_with_buffer_contours_shapefiles_and_slopes_tiff(
    tile: &Tile,
    neighbor_tiles: &NeighborTiles,
    buffer: i64,
) {
    println!("Generating dem with buffer.");

    let dem_with_buffer_path = tile.dir_path.join("dem-with-buffer.tif");
    create_tif_with_buffer(tile, neighbor_tiles, buffer, "dem");

    // Filling holes
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

    let dem_low_resolution_with_buffer_path =
        tile.dir_path.join("dem-low-resolution-with-buffer.tif");
    create_tif_with_buffer(tile, neighbor_tiles, buffer, "dem-low-resolution");

    // Filling holes
    let gdal_fillnodata_output = Command::new("gdal_fillnodata")
        .arg(&dem_low_resolution_with_buffer_path.to_str().unwrap())
        .arg(&dem_low_resolution_with_buffer_path.to_str().unwrap())
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
            &dem_low_resolution_with_buffer_path.to_str().unwrap(),
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

    // TODO: find a better simplifying solution
    println!("Simplifying countours shapefiles.");

    let contours_path = tile.dir_path.join("contours.shp");

    let contours_simplify_output = Command::new("ogr2ogr")
        .args([
            "-simplify",
            "2",
            &contours_path.to_str().unwrap(),
            &contours_raw_path.to_str().unwrap(),
        ])
        .arg("--quiet")
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
            &dem_low_resolution_with_buffer_path.to_str().unwrap(),
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
