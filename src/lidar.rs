use std::path::PathBuf;
use std::process::{Command, ExitStatus};

pub fn process_lidar(out_dir: &PathBuf) {
    println!("Executing PDAL pipeline.");

    let pipeline_path = out_dir.join("pipeline.json");

    let pdal_output = Command::new("pdal")
        .args(["-v", "4", "pipeline", &pipeline_path.to_str().unwrap()])
        .output()
        .expect("failed to execute pdal pipeline command");

    if ExitStatus::success(&pdal_output.status) {
        println!("{}", String::from_utf8(pdal_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(pdal_output.stderr).unwrap());
    }

    println!("Generating countours shapefiles.");

    let dem_path = out_dir.join("dem.tif");
    let contours_raw_path = out_dir.join("contours-raw.shp");

    let gdal_contours_output = Command::new("gdal_contour")
        .args([
            "-a",
            "elev",
            &dem_path.to_str().unwrap(),
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

    println!("Simplifying countours shapefiles.");

    let contours_path = out_dir.join("contours.shp");

    let contours_simplify_output = Command::new("ogr2ogr")
        .args([
            "-simplify",
            "0.5",
            &contours_path.to_str().unwrap(),
            &contours_raw_path.to_str().unwrap(),
        ])
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

    let slopes_path = out_dir.join("slopes.tif");

    let gdaldem_output = Command::new("gdaldem")
        .args([
            "slope",
            &dem_path.to_str().unwrap(),
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
