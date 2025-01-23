use las::raw::Header;
use log::{error, info};
use std::fs::{create_dir_all, read_dir, remove_dir_all, remove_file, write, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Instant;

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
    let medium_vegetation_path = output_dir_path.join("medium-vegetation.tif");
    let high_vegetation_path = output_dir_path.join("high-vegetation.tif");
    let pipeline_path = output_dir_path.join("pipeline.json");

    let gdal_common_options = format!(
        r#""binmode": true,
        "origin_x": {},
        "origin_y": {},
        "width": {},
        "height": {},"#,
        min_x,
        min_y,
        max_x - min_x,
        max_y - min_y
    );

    let gdal_dem_low_resolution_options = format!(
        r#""binmode": true,
        "origin_x": {},
        "origin_y": {},
        "width": {},
        "height": {},"#,
        min_x,
        min_y,
        ((max_x - min_x) as f64 / 2.).ceil() as i64,
        ((max_y - min_y) as f64 / 2.).ceil() as i64
    );

    let pdal_pipeline = format!(
        r#"[
    {:?},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "resolution": 1,
        {}
        "where": "Classification == 2",
        "output_type": "mean"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "resolution": 2,
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
        "value": "Classification = 4",
        "where": "Classification != 2 && HeightAboveGround > 0.3 && HeightAboveGround <= 4"
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 5",
        "where": "Classification != 2 && HeightAboveGround > 4 && HeightAboveGround <= 30"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "resolution": 1,
        {}
        "where": "Classification == 4",
        "output_type": "count"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "resolution": 1,
        {}
        "where": "Classification == 5",
        "output_type": "count"
    }}
]"#,
        laz_path,
        dem_path,
        gdal_common_options,
        dem_low_resolution_path,
        gdal_dem_low_resolution_options,
        dem_path,
        medium_vegetation_path,
        gdal_common_options,
        high_vegetation_path,
        gdal_common_options,
    );

    write(&pipeline_path, pdal_pipeline).expect("Unable to write pipeline file");

    let mut extent_file = File::create(&output_dir_path.join("extent.txt"))
        .expect("Could not create extent.txt file");

    extent_file
        .write_all(format!("{}|{}|{}|{}", min_x, min_y, max_x, max_y).as_bytes())
        .expect("Could not write to the extent.txt file");

    let pdal_output = Command::new("pdal")
        .args(["pipeline", &pipeline_path.to_str().unwrap()])
        .output()
        .expect("failed to execute pdal pipeline command");

    if !ExitStatus::success(&pdal_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Pdal command failed {:?}",
            min_x,
            min_y,
            max_x,
            max_y,
            String::from_utf8(pdal_output.stderr).unwrap()
        );
    }

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. PDAL pipeline executed in {:.1?}",
        min_x, min_y, max_x, max_y, duration
    );
}

fn remove_dir_content<P: AsRef<Path>>(path: P) -> io::Result<()> {
    for entry in read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            remove_dir_all(&path)?;
        } else {
            remove_file(path)?;
        }
    }

    Ok(())
}
