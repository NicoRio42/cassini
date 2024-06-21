mod canvas;
mod cli;
mod cliffs;
mod constants;
mod contours;
mod full_map;
mod utils;
mod vegetation;

use clap::Parser;
use cli::Args;
use cliffs::render_cliffs;
use contours::render_contours_to_png;
use full_map::render_full_map_to_png;
use std::fs;
use std::process::Command;
use utils::delete_dir_contents;
use vegetation::render_vegetation;

fn main() {
    let args = Args::parse();

    if !args.skip_lidar_processing {
        let out_dir = fs::read_dir("out");
        delete_dir_contents(out_dir);

        println!("Executing PDAL pipeline.");

        let pdal_output = Command::new("pdal")
            .args(["-v", "4", "pipeline", "pipeline.json"])
            .output()
            .expect("failed to execute pdal pipeline command");

        println!("{}", String::from_utf8(pdal_output.stdout).unwrap());

        println!("Generating countours shapefiles.");

        let gdal_contours_output = Command::new("gdal_contour")
            .args(["-a", "elev", "out/dem.tif", "out/contours.shp", "-i", "2.5"])
            .output()
            .expect("failed to execute gdal_contour command");

        println!(
            "{}",
            String::from_utf8(gdal_contours_output.stdout).unwrap()
        );

        println!("Generating slopes tif image.");

        let gdaldem_output = Command::new("gdaldem")
            .args(["slope", "out/dem.tif", "out/slopes.tif"])
            .output()
            .expect("failed to execute gdaldem command");

        println!("{}", String::from_utf8(gdaldem_output.stdout).unwrap());
    } else {
        println!("Skipping LiDAR processing.");
    }

    render_vegetation();
    render_cliffs();
    render_contours_to_png();
    render_full_map_to_png();
}
