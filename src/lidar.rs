use crate::utils::delete_dir_contents;
use std::fs;
use std::process::{Command, ExitStatus};

pub fn process_lidar() {
    let out_dir = fs::read_dir("out");
    delete_dir_contents(out_dir);

    println!("Executing PDAL pipeline.");

    let pdal_output = Command::new("pdal")
        .args([
            "-v",
            "4",
            "pipeline",
            "./src/pipeline.json",
            "--metadata",
            "./out/metadata.json",
        ])
        .output()
        .expect("failed to execute pdal pipeline command");

    if ExitStatus::success(&pdal_output.status) {
        println!("{}", String::from_utf8(pdal_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(pdal_output.stderr).unwrap());
    }

    println!("Generating countours shapefiles.");

    let gdal_contours_output = Command::new("gdal_contour")
        .args(["-a", "elev", "out/dem.tif", "out/contours.shp", "-i", "2.5"])
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

    println!("Generating slopes tif image.");

    let gdaldem_output = Command::new("gdaldem")
        .args(["slope", "out/dem.tif", "out/slopes.tif"])
        .output()
        .expect("failed to execute gdaldem command");

    if ExitStatus::success(&gdaldem_output.status) {
        println!("{}", String::from_utf8(gdaldem_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(gdaldem_output.stderr).unwrap());
    }
}
