use std::{
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

pub fn download_laz_files_if_needed(
    min_x: u64,
    min_y: u64,
    max_x: u64,
    max_y: u64,
    macro_tile_id: String,
) {
    for x in [min_x / 1000, max_x / 1000] {
        for y in [min_y / 1000 + 1, max_y / 1000 + 1] {
            let file_name = format!("LHD_FXX_{:0>4}_{:0>4}_PTS_C_LAMB93_IGN69.copc.laz", x, y);
            let path = Path::new("in").join(&file_name);

            if path.exists() {
                continue;
            }

            println!("Downloading {}", file_name);

            let download_output = Command::new("wget")
                .args(["-P", "in" , &format!("https://storage.sbg.cloud.ovh.net/v1/AUTH_63234f509d6048bca3c9fd7928720ca1/ppk-lidar/{}/{}", macro_tile_id, file_name)])
                .output()
                .expect("failed to execute gdal_contour command");

            if ExitStatus::success(&download_output.status) {
                println!("{}", String::from_utf8(download_output.stdout).unwrap());
            } else {
                println!("{}", String::from_utf8(download_output.stderr).unwrap());
            }
        }
    }
}

pub fn download_osm_file_if_needed(min_x: u64, min_y: u64, max_x: u64, max_y: u64) {
    println!("Downloading osm file");

    let osm_file_path = Path::new("in").join(format!("{:0>7}_{:0>7}.osm", min_x, max_y));

    if osm_file_path.exists() {
        return;
    }

    let buffer = 300;

    let (min_lon, min_lat) =
        convert_coords_from_lambert_93_to_gps((min_x - buffer) as f64, (min_y - buffer) as f64);

    let (max_lon, max_lat) =
        convert_coords_from_lambert_93_to_gps((max_x + buffer) as f64, (max_y + buffer) as f64);

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
