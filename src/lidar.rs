use las::raw::Header;
use log::info;
use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::{
    error::{Result, ResultContext, Stage, TileId},
    helpers::remove_dir_content,
    process::{checked_output_with_input, ensure_file},
};

pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: &Path,
    output_dir_path: &Path,
) -> Result<()> {
    let start = Instant::now();

    let mut file = File::open(laz_path).context(format!(
        "could not open LiDAR file `{}`",
        laz_path.display()
    ))?;
    let header = Header::read_from(&mut file).context(format!(
        "could not read LiDAR header `{}`",
        laz_path.display()
    ))?;
    let min_x = header.min_x.round() as i64;
    let min_y = header.min_y.round() as i64;
    let max_x = header.max_x.round() as i64;
    let max_y = header.max_y.round() as i64;
    let tile_id = TileId {
        min_x,
        min_y,
        max_x,
        max_y,
    };

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Executing PDAL pipeline",
        min_x, min_y, max_x, max_y
    );

    // Cleaning up output directory to fix https://github.com/NicoRio42/cassini/issues/7
    if output_dir_path.exists() {
        remove_dir_content(output_dir_path).context(format!(
            "could not clean LiDAR output directory `{}`",
            output_dir_path.display()
        ))?;
    } else {
        create_dir_all(output_dir_path).context(format!(
            "could not create LiDAR output directory `{}`",
            output_dir_path.display()
        ))?;
    }

    let dem_path = output_dir_path.join("dem.tif");
    let low_vegetation_path = output_dir_path.join("low_vegetation.tif");
    let medium_vegetation_path = output_dir_path.join("medium_vegetation.tif");
    let high_vegetation_path = output_dir_path.join("high_vegetation.tif");

    let gdal_dem_options = format!(
        r#""origin_x": {},
        "origin_y": {},
        "width": {},
        "height": {},"#,
        min_x,
        min_y,
        (max_x - min_x) * 2,
        (max_y - min_y) * 2
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
        dem_path,
        low_vegetation_path,
        gdal_vegetation_options,
        medium_vegetation_path,
        gdal_vegetation_options,
        high_vegetation_path,
        gdal_vegetation_options,
    );

    checked_output_with_input(
        Command::new("pdal").args(["pipeline", "-s"]),
        pdal_pipeline.as_bytes(),
        Stage::Lidar,
        Some(tile_id),
    )?;

    for output in [
        &dem_path,
        &low_vegetation_path,
        &medium_vegetation_path,
        &high_vegetation_path,
    ] {
        ensure_file(output)?;
    }

    // Downstream tools use extent.txt as the completion marker, so write it last.
    let extent_path = output_dir_path.join("extent.txt");
    let mut extent_file = File::create(&extent_path)
        .context(format!("could not create `{}`", extent_path.display()))?;
    extent_file
        .write_all(format!("{min_x}|{min_y}|{max_x}|{max_y}").as_bytes())
        .context(format!("could not write `{}`", extent_path.display()))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. PDAL pipeline executed in {:.1?}",
        min_x, min_y, max_x, max_y, duration
    );

    Ok(())
}
