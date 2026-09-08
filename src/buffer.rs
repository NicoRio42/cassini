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
    output_type: Option<&str>,
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

    let tile_id = Some(TileId::from(tile));

    let translate_input_path = if neighbor_tiles.is_empty() {
        tile_raster_path
    } else {
        let mut rasters_paths = vec![tile_raster_path];
        rasters_paths.extend(neighbor_tiles.iter().map(|neighbor_tile| {
            neighbor_tile.join(format!("{}.tif", tif_filename_without_extension))
        }));

        // A VRT is only needed to mosaic the tile with its neighbors.
        checked_output(
            Command::new("gdalbuildvrt")
                .arg(&vrt_with_buffer_path)
                .args(&rasters_paths)
                .arg("-q"),
            Stage::Buffer,
            tile_id,
        )?;
        ensure_file(&vrt_with_buffer_path)?;
        vrt_with_buffer_path.clone()
    };

    // Output the cropped TIFF with a buffer.
    let mut translate_command = Command::new("gdal_translate");
    translate_command
        .args([
            "-projwin",
            &(tile.min_x - buffer).to_string(),
            &(tile.max_y + buffer).to_string(),
            &(tile.max_x + buffer).to_string(),
            &(tile.min_y - buffer).to_string(),
        ])
        .args(["-of", "GTiff"])
        .args(["-tr", &resolution.to_string(), &resolution.to_string()]);

    if let Some(output_type) = output_type {
        translate_command.args(["-ot", output_type]);
    }

    translate_command
        .arg(&translate_input_path)
        .arg(&raster_with_buffer_path)
        .arg("-q");

    checked_output(&mut translate_command, Stage::Buffer, tile_id)?;
    ensure_file(&raster_with_buffer_path)?;

    if !neighbor_tiles.is_empty() {
        std::fs::remove_file(&vrt_with_buffer_path).context(format!(
            "could not remove temporary VRT `{}`",
            vrt_with_buffer_path.display()
        ))?;
    }
    Ok(())
}
