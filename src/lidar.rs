use las::raw::Header;
use std::fs::{create_dir_all, read_dir, remove_dir_all, write, File};
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

use crate::utils::delete_dir_contents;

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
    let vegetation_temp_path = output_dir_path.join("vegetation");
    let low_vegetation_path = vegetation_temp_path.join("low-vegetation-#.tif");
    let medium_vegetation_path = vegetation_temp_path.join("medium-vegetation-#.tif");
    let high_vegetation_path = vegetation_temp_path.join("high-vegetation-#.tif");
    let pipeline_path = output_dir_path.join("pipeline.json");
    create_dir_all(&output_dir_path.join("vegetation")).expect("Could not create out dir");

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
        "value": "Classification = 5",
        "where": "HeightAboveGround > 2 && HeightAboveGround <= 30"
    }},
    {{
        "type":"filters.expression",
        "expression":"ScanAngleRank > -12 && ScanAngleRank < 12"
    }},
    {{
        "type":"filters.groupby",
        "dimension":"PointSourceId"
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

    create_vegetation_rasters(&output_dir_path);
    // delete_dir_contents(read_dir(&vegetation_temp_path));
    // remove_dir_all(vegetation_temp_path).expect("Could not delete directory");
}

fn create_vegetation_rasters(output_dir_path: &PathBuf) {
    for raster_name in ["low-vegetation", "medium-vegetation", "high-vegetation"] {
        create_single_vegetation_raster(output_dir_path, raster_name);
    }
}

fn create_single_vegetation_raster(output_dir_path: &PathBuf, raster_name: &str) {
    let mut input_files_args: Vec<String> = vec![];
    let paths = read_dir(output_dir_path.join("vegetation")).unwrap();

    for path in paths {
        let image_path = path.unwrap().path().to_str().unwrap().to_string();

        if !image_path.contains(raster_name) {
            continue;
        }

        input_files_args.push(image_path);
    }

    let mut binding = Command::new("gdal_calc");
    let gdal_command = binding
        .arg("-A")
        .args(&input_files_args)
        .args([
            "--outfile",
            output_dir_path
                .join(format!("{}.tif", raster_name))
                .to_str()
                .unwrap(),
        ])
        .args(["--calc", "numpy.max(A,axis=0)"])
        .arg("--quiet");

    let gdal_output = gdal_command
        .output()
        .expect("failed to execute pdal pipeline command");

    if ExitStatus::success(&gdal_output.status) {
        println!("{}", String::from_utf8(gdal_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(gdal_output.stderr).unwrap());
    }
}
