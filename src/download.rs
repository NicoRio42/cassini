use log::{info, warn};
use reqwest::blocking::Client;
use std::{
    fs::File,
    io::{copy, BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
    process::{Command, Stdio},
    thread::sleep,
    time::Instant,
};

use crate::constants::BUFFER;

pub fn download_osm_file(min_x: i64, min_y: i64, max_x: i64, max_y: i64, output_dir_path: &PathBuf) {
    let raw_osm_file_path = output_dir_path.join(format!("{:0>7}_{:0>7}_raw.osm", min_x, max_y));
    let osm_file_path = output_dir_path.join(format!("{:0>7}_{:0>7}.osm", min_x, max_y));

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Downloading osm file",
        min_x, min_y, max_x, max_y
    );

    let start = Instant::now();

    let (min_lon, min_lat) =
        convert_coords_from_lambert_93_to_gps((min_x - BUFFER as i64) as f64, (min_y - BUFFER as i64) as f64);

    let (max_lon, max_lat) =
        convert_coords_from_lambert_93_to_gps((max_x + BUFFER as i64) as f64, (max_y + BUFFER as i64) as f64);

    // Overpass Query
    let query = r#"
[out:xml][timeout:25];
(
  way["building"]({{bbox}});
  relation["building"]({{bbox}});
  way["natural"="water"]({{bbox}});
  relation["natural"="water"]({{bbox}});
  way["natural"="wetland"]({{bbox}});
  relation["natural"="wetland"]({{bbox}});
  way["landuse"="residential"]({{bbox}});
  relation["landuse"="residential"]({{bbox}});
  way["landuse"="railway"]({{bbox}});
  relation["landuse"="railway"]({{bbox}});
  way["landuse"="industrial"]({{bbox}});
  relation["landuse"="industrial"]({{bbox}});
  way["natural"="coastline"]({{bbox}});
  way["highway"]({{bbox}});
  way["waterway"]({{bbox}});
  way["railway"]({{bbox}});
  way["power"]({{bbox}});
  way["aerialway"]({{bbox}});
);
out body;
>;
out skel qt;
"#;

    // Replace {{bbox}} with your bounding box (south, west, north, east)
    let bbox = format!("{},{},{},{}", min_lat, min_lon, max_lat, max_lon);
    let formatted_query = query.replace("{{bbox}}", &bbox);
    let client = Client::new();

    let mut retries_left = 5;

    let mut response = loop {
        let response_result = client
            .post("https://overpass-api.de/api/interpreter")
            .body(formatted_query.clone())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send();

        match response_result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    break response;
                }

                if retries_left == 0 {
                    panic!("Overpass API returned error status: {}", status);
                }

                warn!(
                    "Overpass API returned error status {}. Retrying in 2s ({} retries left)",
                    status, retries_left
                );
            }
            Err(error) => {
                if retries_left == 0 {
                    panic!("Could not get osm data from Overpass API: {}", error);
                }

                warn!(
                    "Overpass API request failed: {}. Retrying in 2s ({} retries left)",
                    error, retries_left
                );
            }
        }

        retries_left -= 1;
        sleep(std::time::Duration::from_secs(2));
    };

    let mut file = File::create(&raw_osm_file_path).expect("Could not create file for osm download.");
    copy(&mut response, &mut file).expect("Could not copy file content.");

    fix_osm_file(&raw_osm_file_path, &osm_file_path);
    std::fs::remove_file(&raw_osm_file_path).expect("Could not remove raw osm file");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Osm files downloaded in {:.1?}",
        min_x, min_y, max_x, max_y, duration
    );
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

    let output = cs2cs.wait_with_output().expect("Failed to read cs2cs output");

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

fn fix_osm_file(input: &PathBuf, output: &PathBuf) {
    let reader = BufReader::new(File::open(&input).unwrap());
    let mut writer = BufWriter::new(File::create(&output).unwrap());
    let mut relations_lines: Vec<String> = vec![];

    let mut is_inside_relation = false;

    for line in reader.lines() {
        let line = line.unwrap();

        if line.contains("</osm>") {
            for relations_line in &relations_lines {
                writeln!(writer, "{}", relations_line).unwrap();
            }

            writeln!(writer, "{}", line).unwrap();
            break;
        }

        if line.contains("</relation>") {
            is_inside_relation = false;
            relations_lines.push(line);
            continue;
        }

        if line.contains("<relation") {
            is_inside_relation = true;
            relations_lines.push(line);
            continue;
        }

        if is_inside_relation {
            relations_lines.push(line);
            continue;
        }

        writeln!(writer, "{}", line).unwrap();
    }
}
