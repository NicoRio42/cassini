use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn generate_pipeline_for_single_tile(
    min_x_tile: u64,
    min_y_tile: u64,
    max_x_tile: u64,
    max_y_tile: u64,
    buffer: u64,
    out_dir: &PathBuf,
) {
    println!("Generating PDAL pipeline json file for tile");

    let mut tiles_paths: [PathBuf; 4] = core::array::from_fn(|_| PathBuf::new());

    if min_x_tile % 500 != 0
        || min_x_tile % 500 != 0
        || min_x_tile % 500 != 0
        || min_x_tile % 500 != 0
    {
        panic!("Tile should be a 1km square with a 500m offset");
    }

    let mut index = 0;

    for x in [min_x_tile / 1000, max_x_tile / 1000] {
        for y in [min_y_tile / 1000 + 1, max_y_tile / 1000 + 1] {
            let file_name = format!("LHD_FXX_{:0>4}_{:0>4}_PTS_C_LAMB93_IGN69.copc.laz", x, y);

            let path = Path::new("in").join(&file_name);

            if !path.exists() {
                panic!("Tile {} is missing in the in folder.", file_name)
            }

            tiles_paths[index] = path;

            index += 1;
        }
    }

    let min_x_with_buffer = min_x_tile - buffer;
    let min_y_with_buffer = min_y_tile - buffer;
    let max_x_with_buffer = max_x_tile + buffer;
    let max_y_with_buffer = max_y_tile + buffer;

    let dem_path = out_dir.join("dem.tif");
    let low_vegetation_path = out_dir.join("low-vegetation.tif");
    let medium_vegetation_path = out_dir.join("medium-vegetation.tif");
    let high_vegetation_path = out_dir.join("high-vegetation.tif");
    let pipeline_path = out_dir.join("pipeline.json");

    let pdal_pipeline = format!(
        r#"[
    {:?},
    {:?},
    {:?},
    {:?},
    {{
        "type":"filters.crop",
        "bounds":"([{},{}],[{},{}])"
    }},
    {{
        "type": "filters.info"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "window_size": 1,
        "resolution": 1,
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
        "window_size": 50,
        "resolution": 2,
        "where": "Classification == 3",
        "output_type": "count"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "window_size": 50,
        "resolution": 2,
        "where": "Classification == 4",
        "output_type": "count"
    }},
    {{
        "type": "writers.gdal",
        "filename": {:?},
        "window_size": 50,
        "resolution": 2,
        "where": "Classification == 5",
        "output_type": "count"
    }}
]"#,
        tiles_paths[0],
        tiles_paths[1],
        tiles_paths[2],
        tiles_paths[3],
        min_x_with_buffer,
        max_x_with_buffer,
        min_y_with_buffer,
        max_y_with_buffer,
        dem_path,
        dem_path,
        low_vegetation_path,
        medium_vegetation_path,
        high_vegetation_path,
    );

    fs::write(pipeline_path, pdal_pipeline).expect("Unable to write pipeline file");
}
