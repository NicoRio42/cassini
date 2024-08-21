use std::{
    path::Path,
    process::{Command, ExitStatus, Stdio},
};

use crate::{constants::BUFFER, tile::TileWithNeighbors};

pub fn download_osm_files_for_all_tiles_if_needed(tiles: &Vec<TileWithNeighbors>) {
    for tile in tiles {
        download_osm_file_if_needed(
            tile.tile.min_x,
            tile.tile.min_y,
            tile.tile.max_x,
            tile.tile.max_y,
        );
    }
}

pub fn download_osm_file_if_needed(min_x: i64, min_y: i64, max_x: i64, max_y: i64) {
    let osm_file_path = Path::new("in").join(format!("{:0>7}_{:0>7}.osm", min_x, max_y));

    if osm_file_path.exists() {
        println!("Osm file already downloaded");
        return;
    }

    println!("Downloading osm file");

    let (min_lon, min_lat) = convert_coords_from_lambert_93_to_gps(
        (min_x - BUFFER as i64) as f64,
        (min_y - BUFFER as i64) as f64,
    );

    let (max_lon, max_lat) = convert_coords_from_lambert_93_to_gps(
        (max_x + BUFFER as i64) as f64,
        (max_y + BUFFER as i64) as f64,
    );

    let download_output = Command::new("wget")
        .args([
            "-O",
            &osm_file_path.to_str().unwrap(),
            &format!(
                "https://www.openstreetmap.org/api/0.6/map?bbox={}%2C{}%2C{}%2C{}",
                min_lon, min_lat, max_lon, max_lat,
            ),
        ])
        .output()
        .expect("failed to execute gdal_contour command");

    if ExitStatus::success(&download_output.status) {
        println!("{}", String::from_utf8(download_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(download_output.stderr).unwrap());
    }
}

fn convert_coords_from_lambert_93_to_gps(x: f64, y: f64) -> (f64, f64) {
    let echo = Command::new("echo")
        .arg(format!("{:.1} {:.1}", x, y))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let output = Command::new("cs2cs")
        .args(["+init=epsg:2154", "+to", "+init=epsg:4326", "-f", "%.8f"])
        .stdin(Stdio::from(echo.stdout.unwrap()))
        .output()
        .expect("failed to execute proj command");

    if !output.status.success() {
        panic!("Proj conversion failed.")
    }

    let result = String::from_utf8(output.stdout).unwrap();
    let coords: Vec<&str> = result.trim().split_whitespace().collect();

    if coords.len() < 2 {
        panic!("Wrong Proj conversion result format")
    }

    let lon: f64 = coords[0].parse().expect("Failed to parse longitude");
    let lat: f64 = coords[1].parse().expect("Failed to parse latitude");

    return (lon, lat);
}
