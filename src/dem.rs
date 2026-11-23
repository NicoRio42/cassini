use crate::{
    buffer::create_tif_with_buffer,
    constants::BUFFER,
    error::{Result, ResultContext, Stage, TileId},
    process::{checked_output, ensure_file},
    tile::Tile,
};
use log::info;
use std::{fs::create_dir_all, path::PathBuf, process::Command, time::Instant};

pub fn create_dem_with_buffer_and_slopes_tiff(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
) -> Result<()> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Generating dem with buffer",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let dem_with_buffer_path = tile.render_dir_path.join("dem_with_buffer.tif");
    create_tif_with_buffer(
        tile,
        neighbor_tiles,
        BUFFER as i64,
        "dem",
        0.5,
        Some("Float32"),
    )?;
    let tile_id = Some(TileId::from(tile));

    // Filling holes
    checked_output(
        Command::new("gdal_fillnodata")
            .arg(&dem_with_buffer_path)
            .arg(&dem_with_buffer_path),
        Stage::Dem,
        tile_id,
    )?;
    ensure_file(&dem_with_buffer_path)?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Dem with buffer generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Generating contours shapefiles",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let contours_raw_dir = tile.render_dir_path.join("contours-raw");
    create_dir_all(&contours_raw_dir).context(format!(
        "could not create contour directory `{}`",
        contours_raw_dir.display()
    ))?;
    let contours_raw_path = contours_raw_dir.join("contours-raw.shp");
    let dem_2m_with_buffer_path = tile.render_dir_path.join("dem_2m_with_buffer.tif");

    checked_output(
        Command::new("gdal_translate")
            .args(["-tr", "2", "2", "-r", "average"])
            .arg(&dem_with_buffer_path)
            .arg(&dem_2m_with_buffer_path)
            .arg("--quiet"),
        Stage::Dem,
        tile_id,
    )?;
    ensure_file(&dem_2m_with_buffer_path)?;

    checked_output(
        Command::new("gdal_contour")
            .args(["-a", "elev"])
            .arg(&dem_2m_with_buffer_path)
            .arg(&contours_raw_path)
            .args(["-i", "2.5"]),
        Stage::Contours,
        tile_id,
    )?;
    ensure_file(&contours_raw_path)?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Contours shapefiles generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Generating slopes tif image",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let slopes_path = tile.render_dir_path.join("slopes.tif");

    checked_output(
        Command::new("gdaldem")
            .arg("slope")
            .arg(&dem_with_buffer_path)
            .arg(&slopes_path),
        Stage::Dem,
        tile_id,
    )?;
    ensure_file(&slopes_path)?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Slopes tif image generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    Ok(())
}
