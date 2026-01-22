use las::raw::Header;
use log::{error, info};
use std::fs::{create_dir_all, write, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::Instant;

use crate::helpers::remove_dir_content;

pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: &PathBuf,
    output_dir_path: &PathBuf,
) {
    let start = Instant::now();

    let mut file = File::open(&laz_path).unwrap();
    let header = Header::read_from(&mut file).unwrap();
    let min_x = header.min_x.round() as i64;
    let min_y = header.min_y.round() as i64;
    let max_x = header.max_x.round() as i64;
    let max_y = header.max_y.round() as i64;

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Executing PDAL pipeline",
        min_x, min_y, max_x, max_y
    );

    // Cleaning up output directory to fix https://github.com/NicoRio42/cassini/issues/7
    if output_dir_path.exists() {
        remove_dir_content(output_dir_path).unwrap();
    } else {
        create_dir_all(&output_dir_path).expect("Could not create out dir");
    }

    let dem_path = output_dir_path.join("dem.tif");
    let dem_low_resolution_path = output_dir_path.join("dem-low-resolution.tif");
    let low_vegetation_path = output_dir_path.join("low-vegetation.tif");
    let medium_vegetation_path = output_dir_path.join("medium-vegetation.tif");
    let high_vegetation_path = output_dir_path.join("high-vegetation.tif");

    let gdal_dem_options = format!(
        r#""origin_x": {},
        "origin_y": {},
        "width": {},
        "height": {},"#,
        min_x,
        min_y,
        max_x - min_x,
        max_y - min_y
    );

    let gdal_dem_low_resolution_options = format!(
        r#""origin_x": {},
        "origin_y": {},
        "width": {},
        "height": {},"#,
        min_x,
        min_y,
        ((max_x - min_x) as f64 / 2.).ceil() as i64,
        ((max_y - min_y) as f64 / 2.).ceil() as i64
    );

    let gdal_vegetation_options = format!(
        r#""binmode": true,
        "resolution": 1,
        "output_type": "count",
        "data_type": "uint8",
        "gdalopts": "COMPRESS=DEFLATE,PREDICTOR=2,ZLEVEL=9",
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
        "binmode": true,
        "resolution": 1,
        "gdalopts": "COMPRESS=DEFLATE,PREDICTOR=3,ZLEVEL=9",
        {}
        "where": "Classification == 2",
        "output_type": "mean"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "binmode": true,
        "resolution": 2,
        "gdalopts": "COMPRESS=DEFLATE,PREDICTOR=3,ZLEVEL=9",
        {}
        "where": "Classification == 2",
        "output_type": "mean"
    }},
    {{
        "type": "filters.hag_dem",
        "raster": {:?}
    }},
    {{
        "type":"filters.voxeldownsize",
        "cell": 0.5,
        "mode": "first"
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 3",
        "where": "Classification != 2 && HeightAboveGround > 0 && HeightAboveGround <= 1"
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 4",
        "where": "Classification != 2 && HeightAboveGround > 1 && HeightAboveGround <= 4"
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 5",
        "where": "Classification != 2 && HeightAboveGround > 4 && HeightAboveGround <= 30"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        {}
        "where": "Classification == 3"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        {}
        "where": "Classification == 4"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        {}
        "where": "Classification == 5"
    }}
]"#,
        laz_path,
        dem_path,
        gdal_dem_options,
        dem_low_resolution_path,
        gdal_dem_low_resolution_options,
        dem_path,
        low_vegetation_path,
        gdal_vegetation_options,
        medium_vegetation_path,
        gdal_vegetation_options,
        high_vegetation_path,
        gdal_vegetation_options,
    );

    let mut pdal_process = Command::new("pdal")
        .args(["pipeline", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to execute pdal command");

    if let Some(mut stdin) = pdal_process.stdin.take() {
        stdin
            .write_all(pdal_pipeline.as_bytes())
            .expect("Failed to write to pdal stdin");
    }

    let pdal_output = pdal_process
        .wait_with_output()
        .expect("Failed to wait on pdal process");

    if !ExitStatus::success(&pdal_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Pdal command failed {:?}",
            min_x,
            min_y,
            max_x,
            max_y,
            String::from_utf8(pdal_output.stderr).unwrap()
        );
    } else {
        // The existence of the extent.txt file is used as a proof of right execution of lidar pipeline by mapant-fr-worker

        let mut extent_file =
            File::create(&output_dir_path.join("extent.txt")).expect("Could not create extent.txt file");

        extent_file
            .write_all(format!("{}|{}|{}|{}", min_x, min_y, max_x, max_y).as_bytes())
            .expect("Could not write to the extent.txt file");
    }

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. PDAL pipeline executed in {:.1?}",
        min_x, min_y, max_x, max_y, duration
    );
}
