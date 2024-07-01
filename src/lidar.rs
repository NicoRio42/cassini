use las::{raw::Header, Read, Reader};
use std::fs::{create_dir_all, write, File};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: PathBuf,
    output_dir_path: PathBuf,
) {
    let mut file = File::open(&laz_path).unwrap();
    let header = Header::read_from(&mut file).unwrap();
    let min_x = header.min_x;
    let min_y = header.min_y;
    let max_x = header.max_x;
    let max_y = header.max_y;

    println!("{} {} {} {}", min_x, min_y, max_x, max_y);

    println!("Generating PDAL pipeline json file for tile");

    let dem_path = output_dir_path.join("dem.tif");
    let low_vegetation_path = output_dir_path.join("low-vegetation.tif");
    let medium_vegetation_path = output_dir_path.join("medium-vegetation.tif");
    let high_vegetation_path = output_dir_path.join("high-vegetation.tif");
    let pipeline_path = output_dir_path.join("pipeline.json");

    let gdal_common_options = format!(
        r#""resolution": 1,
        "binmode": true,
        "origin_x": {},
        "origin_y": {},
        "width": {},
        "height": {},"#,
        min_x,
        min_y,
        max_x - min_x,
        max_y - min_y
    );

    let pdal_pipeline = format!(
        r#"[
    {:?},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        {}
        "where": "Classification == 2",
        "output_type": "mean"
    }},
    {{
        "type": "filters.hag_dem",
        "raster": {:?}
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 3",
        "where": "HeightAboveGround > 0 && HeightAboveGround <= 0.4"
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 4",
        "where": "HeightAboveGround > 0.4 && HeightAboveGround <= 2"
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 3",
        "where": "HeightAboveGround > 2 && HeightAboveGround <= 30"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        {}
        "where": "Classification == 3",
        "output_type": "count"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        {}
        "where": "Classification == 4",
        "output_type": "count"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        {}
        "where": "Classification == 5",
        "output_type": "count"
    }}
]"#,
        laz_path,
        dem_path,
        gdal_common_options,
        dem_path,
        low_vegetation_path,
        gdal_common_options,
        medium_vegetation_path,
        gdal_common_options,
        high_vegetation_path,
        gdal_common_options,
    );

    create_dir_all(&output_dir_path).expect("Could not create out dir");
    write(&pipeline_path, pdal_pipeline).expect("Unable to write pipeline file");

    println!("Executing PDAL pipeline.");

    let pdal_output = Command::new("pdal")
        .args(["pipeline", &pipeline_path.to_str().unwrap()])
        .output()
        .expect("failed to execute pdal pipeline command");

    if ExitStatus::success(&pdal_output.status) {
        println!("{}", String::from_utf8(pdal_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(pdal_output.stderr).unwrap());
    }
}

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
