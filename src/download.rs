use std::{
    fs::File,
    io::{copy, stdout, Write},
    path::Path,
    process::{Command, Stdio},
    time::Instant,
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

    print!("Downloading osm file");
    let _ = stdout().flush();
    let start = Instant::now();

    let (min_lon, min_lat) = convert_coords_from_lambert_93_to_gps(
        (min_x - BUFFER as i64) as f64,
        (min_y - BUFFER as i64) as f64,
    );

    let (max_lon, max_lat) = convert_coords_from_lambert_93_to_gps(
        (max_x + BUFFER as i64) as f64,
        (max_y + BUFFER as i64) as f64,
    );

    let mut response = reqwest::blocking::get(&format!(
        "https://www.openstreetmap.org/api/0.6/map?bbox={}%2C{}%2C{}%2C{}",
        min_lon, min_lat, max_lon, max_lat,
    ))
    .expect("Could not download osm file.");

    let mut file = File::create(&osm_file_path).expect("Could not create file for osm download.");
    copy(&mut response, &mut file).expect("Could not copy file content.");

    let duration = start.elapsed();
    println!(" -> Done in {:.1?}", duration);
}

fn convert_coords_from_lambert_93_to_gps(x: f64, y: f64) -> (f64, f64) {
    let mut cs2cs = Command::new("cs2cs")
        .args(["+init=epsg:2154", "+to", "+init=epsg:4326", "-f", "%.8f"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start cs2cs");

    if let Some(mut stdin) = cs2cs.stdin.take() {
        writeln!(stdin, "{:.1} {:.1}", x, y).expect("Failed to write to stdin");
    }

    let output = cs2cs
        .wait_with_output()
        .expect("Failed to read cs2cs output");

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
