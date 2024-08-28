use crate::{
    buffer::create_tif_with_buffer,
    constants::BUFFER,
    fill_nodata::fill_nodata_in_raster,
    tile::{NeighborTiles, Tile},
};
use std::{
    fs::create_dir_all,
    io::{stdout, Write},
    process::{Command, ExitStatus},
    time::Instant,
};

pub fn create_dem_with_buffer_and_slopes_tiff(tile: &Tile, neighbor_tiles: &NeighborTiles) {
    print!("Generating dem with buffer");
    let _ = stdout().flush();
    let start = Instant::now();

    let dem_with_buffer_path = tile.dir_path.join("dem-with-buffer.tif");
    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "dem");
    fill_nodata_in_raster(
        &dem_with_buffer_path,
        &tile.dir_path.join("dem-with-buffer-filled.tif"),
    );

    let dem_low_resolution_with_buffer_path =
        tile.dir_path.join("dem-low-resolution-with-buffer.tif");

    create_tif_with_buffer(tile, neighbor_tiles, BUFFER as i64, "dem-low-resolution");

    fill_nodata_in_raster(
        &dem_low_resolution_with_buffer_path,
        &dem_low_resolution_with_buffer_path,
    );

    let duration = start.elapsed();
    println!(" -> Done in {:.1?}", duration);

    print!("Generating countours shapefiles");
    let _ = stdout().flush();
    let start = Instant::now();

    let contours_raw_dir = tile.dir_path.join("contours-raw");
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

    let slopes_path = tile.dir_path.join("slopes.tif");

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
