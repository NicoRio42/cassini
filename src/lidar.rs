use las::raw::Header;
use std::fs::{create_dir_all, write, File};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: &PathBuf,
    output_dir_path: &PathBuf,
) {
    let mut file = File::open(&laz_path).unwrap();
    let header = Header::read_from(&mut file).unwrap();
    let min_x = header.min_x;
    let min_y = header.min_y;
    let max_x = header.max_x;
    let max_y = header.max_y;

    println!("Generating PDAL pipeline json file for tile");

    let dem_path = output_dir_path.join("dem.tif");
    let dem_low_resolution_path = output_dir_path.join("dem-low-resolution.tif");
    let medium_vegetation_path = output_dir_path.join("medium-vegetation.tif");
    let high_vegetation_path = output_dir_path.join("high-vegetation.tif");
    let pipeline_path = output_dir_path.join("pipeline.json");
    create_dir_all(&output_dir_path).expect("Could not create out dir");

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
        "type":"filters.sample",
        "cell": 0.5
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 4",
        "where": "HeightAboveGround > 0.3 && HeightAboveGround <= 4"
    }},
    {{
        "type": "filters.assign",
        "value": "Classification = 5",
        "where": "HeightAboveGround > 4 && HeightAboveGround <= 30"
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
        gdal_common_options,
        dem_path,
        medium_vegetation_path,
        gdal_common_options,
        high_vegetation_path,
        gdal_common_options,
    );

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
