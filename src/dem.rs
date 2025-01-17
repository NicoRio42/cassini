use crate::{buffer::create_tif_with_buffer, constants::BUFFER, tile::Tile};
use std::{
    fs::create_dir_all,
    io::{stdout, Write},
    path::PathBuf,
    process::{Command, ExitStatus},
    time::Instant,
};

pub fn create_dem_with_buffer_and_slopes_tiff(tile: &Tile, neighbor_tiles: &Vec<PathBuf>) {
    print!("Generating dem with buffer");
    let _ = stdout().flush();
    let start = Instant::now();

    let dem_with_buffer_path = tile.render_dir_path.join("dem-with-buffer.tif");
    create_tif_with_buffer(tile, &neighbor_tiles, BUFFER as i64, "dem");

    // Filling holes
    let gdal_fillnodata_output = Command::new("gdal_fillnodata")
        .arg(&dem_with_buffer_path.to_str().unwrap())
        .arg(&dem_with_buffer_path.to_str().unwrap())
        .output()
        .expect("failed to execute gdal_fillnodata command");

    if !ExitStatus::success(&gdal_fillnodata_output.status) {
        println!(
            "{}",
            String::from_utf8(gdal_fillnodata_output.stderr).unwrap()
        );
    }

    let dem_low_resolution_with_buffer_path = tile
        .render_dir_path
        .join("dem-low-resolution-with-buffer.tif");

    create_tif_with_buffer(tile, &neighbor_tiles, BUFFER as i64, "dem-low-resolution");

    // Filling holes
    let gdal_fillnodata_output = Command::new("gdal_fillnodata")
        .arg(&dem_low_resolution_with_buffer_path.to_str().unwrap())
        .arg(&dem_low_resolution_with_buffer_path.to_str().unwrap())
        .output()
        .expect("failed to execute gdal_fillnodata command");

    if !ExitStatus::success(&gdal_fillnodata_output.status) {
        println!(
            "{}",
            String::from_utf8(gdal_fillnodata_output.stderr).unwrap()
        );
    }

    let duration = start.elapsed();
    println!(" -> Done in {:.1?}", duration);

    print!("Generating countours shapefiles");
    let _ = stdout().flush();
    let start = Instant::now();

    let contours_raw_dir = tile.render_dir_path.join("contours-raw");
    create_dir_all(&contours_raw_dir).expect("Could not create contours-raw dir");
    let contours_raw_path = contours_raw_dir.join("contours-raw.shp");

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

    if !ExitStatus::success(&gdal_contours_output.status) {
        println!(
            "{}",
            String::from_utf8(gdal_contours_output.stderr).unwrap()
        );
    }

    let duration = start.elapsed();
    println!(" -> Done in {:.1?}", duration);

    print!("Generating slopes tif image");
    let _ = stdout().flush();
    let start = Instant::now();

    let slopes_path = tile.render_dir_path.join("slopes.tif");

    let gdaldem_output = Command::new("gdaldem")
        .args([
            "slope",
            &dem_with_buffer_path.to_str().unwrap(),
            &slopes_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute gdaldem command");

    if !ExitStatus::success(&gdaldem_output.status) {
        println!("{}", String::from_utf8(gdaldem_output.stderr).unwrap());
    }

    let duration = start.elapsed();
    println!(" -> Done in {:.1?}", duration);
}
