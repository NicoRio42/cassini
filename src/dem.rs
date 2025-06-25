use crate::{
    buffer::create_raster_with_buffer,
    constants::BUFFER,
    fillnodata::fill_nodata,
    terrain_rgba::{decode_rgba_to_elevation, encode_elevation_to_rgba},
    tile::Tile,
};
use image::{ImageBuffer, Rgba, RgbaImage};
use log::{error, info};
use std::{
    fs::create_dir_all,
    path::PathBuf,
    process::{Command, ExitStatus},
    time::Instant,
};

pub fn create_dem_with_buffer_and_slopes_raster(tile: &Tile, neighbor_tiles: &Vec<PathBuf>) {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Generating dem with buffer",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    create_raster_with_buffer(tile, &neighbor_tiles, BUFFER as u32, "dem");
    let dem_with_buffer_path = tile.render_dir_path.join("dem-with-buffer.png");

    let dem_with_buffer_image = image::open(&dem_with_buffer_path)
        .expect("Failed to open image")
        .to_rgba8();

    let elevation_matrix = image_to_elevation_matrix(dem_with_buffer_image);

    let filled_elevation_matrix = fill_nodata(&elevation_matrix);

    let mut terrain_rgb_img = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(1400, 1400);

    for (y, row) in filled_elevation_matrix.iter().enumerate() {
        for (x, elevation) in row.iter().enumerate() {
            let rgba = encode_elevation_to_rgba(*elevation);
            terrain_rgb_img.put_pixel(x as u32, y as u32, Rgba(rgba));
        }
    }

    terrain_rgb_img
        .save(tile.render_dir_path.join("filled-dem-with-buffer.png"))
        .unwrap();

    // Filling holes
    let gdal_fillnodata_output = Command::new("gdal_fillnodata")
        .arg(&dem_with_buffer_path.to_str().unwrap())
        .arg(&dem_with_buffer_path.to_str().unwrap())
        .output()
        .expect("failed to execute gdal_fillnodata command");

    if !ExitStatus::success(&gdal_fillnodata_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Gdal_fillnodata command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(gdal_fillnodata_output.stderr).unwrap()
        );
    }

    let dem_low_resolution_with_buffer_path = tile.render_dir_path.join("dem-low-resolution-with-buffer.tif");
    create_raster_with_buffer(tile, &neighbor_tiles, BUFFER as u32, "dem-low-resolution");

    // Filling holes
    let gdal_fillnodata_output = Command::new("gdal_fillnodata")
        .arg(&dem_low_resolution_with_buffer_path.to_str().unwrap())
        .arg(&dem_low_resolution_with_buffer_path.to_str().unwrap())
        .output()
        .expect("failed to execute gdal_fillnodata command");

    if !ExitStatus::success(&gdal_fillnodata_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Gdal_fillnodata command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(gdal_fillnodata_output.stderr).unwrap()
        );
    }

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Dem with buffer generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Generating contours shapefiles",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

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
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Gdal_contour command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(gdal_contours_output.stderr).unwrap()
        );
    }

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Contours shapefiles generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Generating slopes tif image",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

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
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Gdaldem command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(gdaldem_output.stderr).unwrap()
        );
    }

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Slopes tif image generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}

fn image_to_elevation_matrix(image: RgbaImage) -> Vec<Vec<f32>> {
    let (width, height) = image.dimensions();

    let mut elevation_matrix = Vec::with_capacity(height as usize);

    for y in 0..height {
        let mut row = Vec::with_capacity(width as usize);
        for x in 0..width {
            let pixel = image.get_pixel(x, y).0; // [u8; 4]
            let mut elevation = decode_rgba_to_elevation(pixel);

            row.push(elevation);
        }
        elevation_matrix.push(row);
    }

    elevation_matrix
}
