use crate::{
    buffer::mosaic_with_buffer,
    constants::DEM_BLOCK_SIZE,
    contour_shapefile::write_contour_shapefile,
    error::{Result, ResultContext},
    fill_nodata::{fill_nodata, DEFAULT_MAX_SEARCH_DISTANCE},
    grid::{Compression, Grid},
    marching_squares::{contour_lines, ContourLine},
    tile::Tile,
};
use log::info;
use std::{fs::create_dir_all, path::PathBuf, time::Instant};

/// The elevation interval between contours, in metres.
const CONTOUR_INTERVAL: f64 = 2.5;

/// The cell size of the DEM contours are extracted from, in metres.
const CONTOUR_DEM_RESOLUTION: f64 = 2.0;

/// How many DEM cells wide a cell of the contour DEM is.
const CONTOUR_DEM_FACTOR: usize = (CONTOUR_DEM_RESOLUTION / DEM_BLOCK_SIZE) as usize;
const _: () = assert!(
    CONTOUR_DEM_FACTOR as f64 * DEM_BLOCK_SIZE == CONTOUR_DEM_RESOLUTION,
    "the contour DEM resolution must be a multiple of the DEM resolution"
);

/// The nodata value of `slopes.tif`, as `gdaldem slope` writes it.
const SLOPE_NODATA: f32 = -9999.0;

/// Writes the filled DEM of the buffered tile (`dem_with_buffer.tif`), its
/// 2 m version (`dem_2m_with_buffer.tif`), the contours extracted from it
/// (`contours-raw/contours-raw.shp`) and the slopes (`slopes.tif`), and
/// returns the contours.
pub fn create_dem_contours_and_slopes(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
) -> Result<Vec<ContourLine>> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Generating dem with buffer",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let dem_with_buffer_path = tile.render_dir_path.join("dem_with_buffer.tif");
    let mut dem_with_buffer =
        mosaic_with_buffer::<f32>(tile, neighbor_tiles, "dem", DEM_BLOCK_SIZE)?;

    // Filling holes
    fill_nodata(&mut dem_with_buffer, DEFAULT_MAX_SEARCH_DISTANCE);
    dem_with_buffer.write_geotiff(&dem_with_buffer_path, Compression::None)?;

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

    let dem_2m_with_buffer = downsample_average(&dem_with_buffer, CONTOUR_DEM_FACTOR);
    dem_2m_with_buffer.write_geotiff(&dem_2m_with_buffer_path, Compression::None)?;

    let contours = contour_lines(&dem_2m_with_buffer, CONTOUR_INTERVAL);
    write_contour_shapefile(&contours_raw_path, &contours)?;

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

    Ok(contours)
}

/// `grid` with cells `factor` times larger, each the mean of the data cells of
/// its `factor` x `factor` block, or nodata if the block has none, as
/// `gdal_translate -tr … -r average` computes it.
///
/// As with GDAL, a size that is not a multiple of `factor` is rounded to the
/// nearest number of cells: a remainder of less than half a block is dropped,
/// and a last block partly outside the grid repeats its last row or column
/// in place of the missing cells.
fn downsample_average(grid: &Grid<f32>, factor: usize) -> Grid<f32> {
    let downsampled_size = |size: usize| (size as f64 / factor as f64).round() as usize;
    let nodata = grid.nodata.unwrap_or(f32::NAN);
    let mut downsampled = Grid::new(
        grid.min_x,
        grid.max_y,
        grid.resolution * factor as f64,
        downsampled_size(grid.width),
        downsampled_size(grid.height),
        grid.nodata,
        nodata,
    );
    for row in 0..downsampled.height {
        for col in 0..downsampled.width {
            let mut sum = 0.0f64;
            let mut count = 0usize;
            for source_row in row * factor..(row + 1) * factor {
                for source_col in col * factor..(col + 1) * factor {
                    let value = grid.get(
                        source_col.min(grid.width - 1),
                        source_row.min(grid.height - 1),
                    );
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

    /// A `width` x `height` DEM whose cell `i` is `1.25 * i`, or nodata every
    /// 7 cells.
    fn ramp(width: usize, height: usize) -> Grid<f32> {
        let values = (0..width * height)
            .map(|i| if i % 7 == 3 { -9999.0 } else { i as f32 * 1.25 })
            .collect();
        grid(width, height, values)
    }

    #[test]
    fn averages_partial_blocks_like_gdal() {
        // Expected values from `gdal_translate -tr 2 2 -r average` (GDAL 3.12)
        // on `ramp` written as a Float32 GeoTIFF of 0.5 m cells.
        let check = |width, height, size, expected: &[f32]| {
            let downsampled = downsample_average(&ramp(width, height), 4);
            assert_eq!((downsampled.width, downsampled.height), size);
            assert_eq!(downsampled.values, expected, "{width} x {height}");
        };
        // 1.25 blocks: the remainder is dropped.
        check(5, 5, (1, 1), &[10.961538]);
        // 2.25 x 1.5 blocks.
        check(9, 6, (2, 2), &[20.267857, 22.232143, 55.729168, 59.23077]);
        // 2.5 x 1.75 blocks: the last block of each row and column is partly
        // outside.
        #[rustfmt::skip]
        check(10, 7, (3, 2), &[21.153847, 25.625, 28.5, 67.666664, 72.21154, 77.21154]);
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
        let slopes = slopes_in_degrees(&dem);
        assert_eq!(slopes.get(2, 1), SLOPE_NODATA);
        assert_eq!(slopes.get(1, 1), expected);
    }
}
