use crate::{
    buffer::create_tif_with_buffer,
    constants::{BUFFER, DEM_BLOCK_SIZE},
    error::{Result, ResultContext, Stage, TileId},
    grid::{Compression, Grid},
    process::{checked_output, ensure_file},
    tile::Tile,
};
use log::info;
use std::{fs::create_dir_all, path::PathBuf, process::Command, time::Instant};

/// The cell size of the DEM contours are extracted from, in metres.
const CONTOUR_DEM_RESOLUTION: f32 = 2.0;

/// The nodata value of `slopes.tif`, as `gdaldem slope` writes it.
const SLOPE_NODATA: f32 = -9999.0;

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
    create_tif_with_buffer::<f32>(
        tile,
        neighbor_tiles,
        BUFFER as i64,
        "dem",
        DEM_BLOCK_SIZE as f64,
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

    let dem_with_buffer = Grid::<f32>::read_geotiff(&dem_with_buffer_path)?;
    downsample_average(
        &dem_with_buffer,
        (CONTOUR_DEM_RESOLUTION / DEM_BLOCK_SIZE) as usize,
    )
    .write_geotiff(&dem_2m_with_buffer_path, Compression::None)?;

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

    slopes_in_degrees(&dem_with_buffer).write_geotiff(&slopes_path, Compression::None)?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Slopes tif image generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    Ok(())
}

/// `grid` with cells `factor` times larger, each the mean of the data cells of
/// its `factor` x `factor` block, or nodata if the block has none, as
/// `gdal_translate -tr … -r average` computes it. The grid size must be a
/// multiple of `factor`.
fn downsample_average(grid: &Grid<f32>, factor: usize) -> Grid<f32> {
    debug_assert!(grid.width.is_multiple_of(factor) && grid.height.is_multiple_of(factor));
    let nodata = grid.nodata.unwrap_or(f32::NAN);
    let mut downsampled = Grid::new(
        grid.min_x,
        grid.max_y,
        grid.resolution * factor as f64,
        grid.width / factor,
        grid.height / factor,
        grid.nodata,
        nodata,
    );
    for row in 0..downsampled.height {
        for col in 0..downsampled.width {
            let mut sum = 0.0f64;
            let mut count = 0usize;
            for source_row in row * factor..(row + 1) * factor {
                for source_col in col * factor..(col + 1) * factor {
                    let value = grid.get(source_col, source_row);
                    if grid.has_data(value) {
                        sum += value as f64;
                        count += 1;
                    }
                }
            }
            if count > 0 {
                downsampled.set(col, row, (sum / count as f64) as f32);
            }
        }
    }
    downsampled
}

/// The slope of `dem` in degrees, with Horn's method, as `gdaldem slope`
/// computes it: the edge cells and the cells next to a nodata cell are
/// nodata.
fn slopes_in_degrees(dem: &Grid<f32>) -> Grid<f32> {
    let mut slopes = Grid::new(
        dem.min_x,
        dem.max_y,
        dem.resolution,
        dem.width,
        dem.height,
        Some(SLOPE_NODATA),
        SLOPE_NODATA,
    );
    let ew_scale = 1.0 / dem.resolution;
    let ns_scale = -1.0 / dem.resolution;
    for row in 1..dem.height.saturating_sub(1) {
        for col in 1..dem.width.saturating_sub(1) {
            // The 3 x 3 window, row-major from the north-west.
            let mut window = [0.0f32; 9];
            let mut has_nodata = false;
            for (index, value) in window.iter_mut().enumerate() {
                *value = dem.get(col + index % 3 - 1, row + index / 3 - 1);
                has_nodata |= !dem.has_data(*value);
            }
            if has_nodata {
                continue;
            }
            let [z0, z1, z2, z3, _, z5, z6, z7, z8] = window;
            // Summed as `f32`, as GDAL does for a `Float32` DEM.
            let dx = ((z0 + z3 + z3 + z6) - (z2 + z5 + z5 + z8)) as f64 * ew_scale;
            let dy = ((z6 + z7 + z7 + z8) - (z0 + z1 + z1 + z2)) as f64 * ns_scale;
            let slope = ((dx * dx + dy * dy).sqrt() / 8.0).atan().to_degrees();
            slopes.set(col, row, slope as f32);
        }
    }
    slopes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(width: usize, height: usize, values: Vec<f32>) -> Grid<f32> {
        Grid {
            min_x: 1000.0,
            max_y: 2000.0,
            resolution: 0.5,
            width,
            height,
            nodata: Some(-9999.0),
            values,
        }
    }

    #[test]
    fn averages_blocks_ignoring_nodata() {
        #[rustfmt::skip]
        let dem = grid(4, 2, vec![
            1.0, 2.0, -9999.0, -9999.0,
            3.0, -9999.0, -9999.0, -9999.0,
        ]);
        let downsampled = downsample_average(&dem, 2);
        assert_eq!((downsampled.width, downsampled.height), (2, 1));
        assert_eq!(downsampled.resolution, 1.0);
        assert_eq!((downsampled.min_x, downsampled.max_y), (1000.0, 2000.0));
        assert_eq!(downsampled.values, [2.0, -9999.0]);
    }

    #[test]
    fn computes_slopes_with_horn_method() {
        // A plane rising 1 m every 0.5 m to the east: 63.43°.
        let mut dem = grid(4, 3, (0..12).map(|i| (i % 4) as f32 * 1.0).collect());
        let slopes = slopes_in_degrees(&dem);
        let expected = 2.0f64.atan().to_degrees() as f32;
        #[rustfmt::skip]
        assert_eq!(slopes.values, [
            -9999.0, -9999.0, -9999.0, -9999.0,
            -9999.0, expected, expected, -9999.0,
            -9999.0, -9999.0, -9999.0, -9999.0,
        ]);

        dem.set(3, 0, -9999.0);
        assert_eq!(slopes_in_degrees(&dem).get(2, 1), SLOPE_NODATA);
        assert_eq!(slopes_in_degrees(&dem).get(1, 1), expected);
    }
}
