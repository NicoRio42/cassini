use crate::{
    error::{Result, ResultContext, Stage, TileId},
    process::{checked_output, ensure_file},
    tile::Tile,
};
use std::{path::PathBuf, process::Command};

pub fn create_tif_with_buffer(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
    buffer: i64,
    tif_filename_without_extension: &str,
    resolution: f32,
) -> Result<()> {
    let vrt_with_buffer_path = tile.render_dir_path.join(format!(
        "{}_with_buffer.vrt",
        tif_filename_without_extension
    ));

    let raster_with_buffer_path = tile.render_dir_path.join(format!(
        "{}_with_buffer.tif",
        tif_filename_without_extension
    ));

    let tile_raster_path = tile
        .lidar_dir_path
        .join(format!("{}.tif", tif_filename_without_extension));

    let mut rasters_paths = vec![tile_raster_path];

    for neighbor_tile in neighbor_tiles {
        let path = neighbor_tile.join(format!("{}.tif", tif_filename_without_extension));

        rasters_paths.push(path);
    }

    let tile_id = Some(TileId::from(tile));

    // First creating a GDAL Virtual Dataset
    checked_output(
        Command::new("gdalbuildvrt")
            .arg(&vrt_with_buffer_path)
            .args(&rasters_paths)
            .arg("--quiet"),
        Stage::Buffer,
        tile_id,
    )?;
    ensure_file(&vrt_with_buffer_path)?;

    // Then outpouting croped tif with buffer
    checked_output(
        Command::new("gdal_translate")
            .args([
                "-projwin",
                &(tile.min_x - buffer).to_string(),
                &(tile.max_y + buffer).to_string(),
                &(tile.max_x + buffer).to_string(),
                &(tile.min_y - buffer).to_string(),
            ])
            .args(["-of", "GTiff"])
            .args(["-tr", &resolution.to_string(), &resolution.to_string()])
            .arg(&vrt_with_buffer_path)
            .arg(&raster_with_buffer_path)
            .arg("--quiet"),
        Stage::Buffer,
        tile_id,
    )?;
    ensure_file(&raster_with_buffer_path)?;

    // Finally removing the vrt file
    std::fs::remove_file(&vrt_with_buffer_path).context(format!(
        "could not remove temporary VRT `{}`",
        vrt_with_buffer_path.display()
    ))?;
    Ok(())
}
