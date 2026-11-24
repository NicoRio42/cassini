//! Fills the nodata cells of a raster from the nearest data cells, as
//! `gdal_fillnodata` does with its default options (inverse distance
//! weighting, no smoothing).
//!
//! Ported from `GDALFillNodata` in GDAL's `alg/rasterfill.cpp` (GDAL 3.12):
//!
//! > Copyright (c) 2008, Frank Warmerdam
//! > Copyright (c) 2009-2013, Even Rouault <even dot rouault at spatialys.com>
//! > Copyright (c) 2015, Sean Gillies <sean@mapbox.com>
//! >
//! > SPDX-License-Identifier: MIT

use crate::grid::Grid;

/// The search distance of `gdal_fillnodata`, in cells.
pub const DEFAULT_MAX_SEARCH_DISTANCE: f64 = 100.0;

/// The nearest data cell of a column, above or below a cell: its row and
/// value.
type ColumnNeighbor = Option<(u32, f32)>;

/// Fills the nodata cells of `grid` that have a data cell within
/// `max_search_distance` cells.
///
/// Each nodata cell takes the inverse distance weighted mean of up to four
/// data cells, the nearest in each quadrant around it. GDAL finds them with a
/// top-down pass that records the nearest data cell above each cell of each
/// column, then a bottom-up pass that tracks the nearest one below and scans
/// the columns to the left and right. The search is not exhaustive: within a
/// column only the nearest data cell vertically is a candidate. It is
/// reproduced as is, so the result is bit-identical to GDAL's. Filled cells
/// are not used to fill others.
pub fn fill_nodata(grid: &mut Grid<f32>, max_search_distance: f64) {
    let (width, height) = (grid.width, grid.height);
    let max_steps = max_search_distance.floor() as usize;
    let is_data: Vec<bool> = grid
        .values
        .iter()
        .map(|&value| grid.has_data(value))
        .collect();

    // Top-down pass: the nearest data cell at or above each cell, within the
    // search distance.
    let mut above: Vec<ColumnNeighbor> = vec![None; width * height];
    for row in 0..height {
        for col in 0..width {
            let index = row * width + col;
            above[index] = if is_data[index] {
                Some((row as u32, grid.values[index]))
            } else if row == 0 {
                None
            } else {
                above[index - width]
                    .filter(|&(data_row, _)| row as f64 <= max_search_distance + data_row as f64)
            };
        }
    }

    // Bottom-up pass. `below` is the nearest data cell strictly below the
    // current row in each column, from the previous iteration.
    let mut below: Vec<ColumnNeighbor> = vec![None; width];
    let mut next_below: Vec<ColumnNeighbor> = vec![None; width];
    // How many columns up to each column have a candidate above or below,
    // to skip the cells with none within the search distance, which GDAL
    // scans without finding anything.
    let mut candidates_before = vec![0usize; width + 1];
    for row in (0..height).rev() {
        let row_start = row * width;
        let row_above = &above[row_start..row_start + width];
        for col in 0..width {
            let index = row_start + col;
            next_below[col] = if is_data[index] {
                Some((row as u32, grid.values[index]))
            } else {
                below[col]
                    .filter(|&(data_row, _)| (data_row - row as u32) as f64 <= max_search_distance)
            };
            candidates_before[col + 1] = candidates_before[col]
                + usize::from(row_above[col].is_some() || below[col].is_some());
        }

        for col in 0..width {
            if is_data[row_start + col] {
                continue;
            }
            let first = col.saturating_sub(max_steps);
            let last = (col + max_steps).min(width - 1);
            if candidates_before[last + 1] == candidates_before[first] {
                continue;
            }
            if let Some(value) =
                interpolate(col, row, row_above, &below, max_search_distance, max_steps)
            {
                grid.values[row_start + col] = value;
            }
        }

        std::mem::swap(&mut below, &mut next_below);
    }
}

/// The value interpolated at nodata cell (`col`, `row`) from the nearest data
/// cells above (`row_above[c]` for column `c`, including the cell's row) and
/// below, or `None` if there are none within the search distance.
fn interpolate(
    col: usize,
    row: usize,
    row_above: &[ColumnNeighbor],
    below: &[ColumnNeighbor],
    max_search_distance: f64,
    max_steps: usize,
) -> Option<f32> {
    const TOP_LEFT: usize = 0;
    const BOTTOM_LEFT: usize = 1;
    const TOP_RIGHT: usize = 2;
    const BOTTOM_RIGHT: usize = 3;

    let width = row_above.len();
    let mut distances = [max_search_distance + 1.0; 4];
    let mut values = [0.0f32; 4];
    // Keeps `neighbor` for `quadrant` if it is nearer than the current one.
    let mut check =
        |distances: &mut [f64; 4], quadrant: usize, target_col: usize, neighbor: ColumnNeighbor| {
            if let Some((target_row, value)) = neighbor {
                let dx = target_col as f64 - col as f64;
                let dy = target_row as f64 - row as f64;
                let squared_distance = dx * dx + dy * dy;
                if squared_distance < distances[quadrant] * distances[quadrant] {
                    distances[quadrant] = squared_distance.sqrt();
                    values[quadrant] = value;
                }
            }
        };

    // Steps left and right one column at a time, looking for the nearest
    // data cell of each quadrant. GDAL clamps the columns to the grid, so the
    // edge column is checked again, with no effect.
    let mut steps = max_steps;
    let mut step = 0;
    while step <= steps {
        let left = col.saturating_sub(step);
        let right = (col + step).min(width - 1);

        // The left quadrants include the cell's column, and the top ones its
        // row.
        check(&mut distances, TOP_LEFT, left, row_above[left]);
        check(&mut distances, BOTTOM_LEFT, left, below[left]);

        if step > 0 {
            check(&mut distances, TOP_RIGHT, right, row_above[right]);
            check(&mut distances, BOTTOM_RIGHT, right, below[right]);

            // Every four steps, stop at the farthest distance found so far.
            if step % 4 == 0 {
                steps = distances.iter().copied().fold(0.0, f64::max).floor() as usize;
            }
        }
        step += 1;
    }

    let mut weight_sum = 0.0f64;
    let mut value_sum = 0.0f64;
    for (&distance, &value) in distances.iter().zip(&values) {
        if distance <= max_search_distance {
            let weight = 1.0 / distance;
            weight_sum += weight;
            value_sum += value as f64 * weight;
        }
    }
    (weight_sum > 0.0).then(|| (value_sum / weight_sum) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODATA: f32 = -9999.0;

    fn grid(width: usize, height: usize, values: Vec<f32>) -> Grid<f32> {
        Grid {
            min_x: 0.0,
            max_y: 0.0,
            resolution: 1.0,
            width,
            height,
            nodata: Some(NODATA),
            values,
        }
    }

    #[test]
    fn weights_the_nearest_cell_of_each_quadrant_by_inverse_distance() {
        #[rustfmt::skip]
        let mut dem = grid(3, 3, vec![
            NODATA, 4.0, NODATA,
            NODATA, NODATA, 1.0,
            NODATA, 2.0, NODATA,
        ]);
        fill_nodata(&mut dem, 100.0);
        // The centre: 4.0 at distance 1 in the top left quadrant, which
        // includes its column, 1.0 at distance 1 in the top right one, which
        // includes its row, and 2.0 at distance 1 in the bottom left one. The
        // bottom right quadrant has no data cell.
        assert_eq!(dem.get(1, 1), (7.0f64 / 3.0) as f32);
        assert!(dem.values.iter().all(|&value| value != NODATA));
    }

    #[test]
    fn leaves_cells_beyond_the_search_distance() {
        let mut values = vec![NODATA; 10];
        values[0] = 2.0;
        let mut dem = grid(10, 1, values);
        fill_nodata(&mut dem, 4.0);
        assert_eq!(dem.values[..5], [2.0; 5]);
        assert_eq!(dem.values[5..], [NODATA; 5]);
    }

    #[test]
    fn matches_gdal_fillnodata() {
        let mut dem = ramp_with_holes();
        fill_nodata(&mut dem, 3.0);
        // From `gdal_fillnodata -md 3` (GDAL 3.12) on `ramp_with_holes`
        // written as a Float32 GeoTIFF.
        #[rustfmt::skip]
        let expected: [f32; 140] = [
            0.0, 3.5, 7.0, 5.5, 9.0, 7.5, 11.0, 14.5, 13.0, 16.5, 15.0, 18.5, 22.0, 20.5,
            24.0, 22.5, 26.0, 29.5, 15.980869, 8.949747, 12.449747, 13.878679, 23.490309, 35.651417, 39.0, 37.5, 41.0, 44.5,
            43.0, 46.5, 45.0, 38.2094, 34.386715, 36.054398, 39.554398, 41.308216, 62.62393, 64.84418, 58.0, 61.5, 60.0, 63.5,
            67.0, 65.5, 69.0, 84.577835, 97.98936, 91.143616, 89.89026, 89.65217, 83.964035, 86.4684, 82.0, 80.5, 84.0, 82.5,
            86.0, 89.5, 88.0, 103.10276, 107.62937, 109.71051, 117.449745, 115.957954, 115.27013, 113.53554, 101.0, 104.5, 103.0, 106.5,
            105.0, 108.5, 112.0, 110.5, 114.0, 112.5, 116.0, 119.5, 118.0, 121.5, 120.0, 123.5, 127.0, 125.5,
            129.0, 127.5, 131.0, 134.5, 133.0, 136.5, 128.5944, 126.48087, 119.449745, 120.87868, 121.449745, 124.949745, 126.37868, 125.5,
            148.0, 151.5, 150.0, 153.5, 157.0, 155.5, 155.90425, 138.26183, 119.65247, 120.791794, 121.65247, 125.15247, 126.291794, 125.5,
            172.0, 170.5, 174.0, 172.5, 176.0, 179.5, 187.37006, 188.47058, 118.0, 121.5, 120.0, 123.5, 127.0, 125.5,
            191.0, 171.94975, 173.37868, 183.75, 195.0, 198.5, 198.5, 198.5, 198.5, NODATA, NODATA, NODATA, NODATA, NODATA,
        ];
        assert_eq!(dem.values, expected);
    }

    /// A 14 x 10 uneven ramp with a 7 x 4 hole in the middle, a 3-cell hole
    /// on the bottom row and an 8 x 4 hole in the bottom right corner, whose
    /// corner is farther than 3 cells from any data.
    fn ramp_with_holes() -> Grid<f32> {
        let mut dem = grid(14, 10, Vec::new());
        dem.values = (0..140)
            .map(|i| i as f32 * 1.5 + ((i % 14) * 7 + (i / 14) * 3) as f32 % 5.0)
            .collect();
        for row in 1..5 {
            for col in 3..10 {
                if row > 1 || col >= 4 {
                    dem.set(col, row, NODATA);
                }
            }
        }
        for col in 1..4 {
            dem.set(col, 9, NODATA);
        }
        for row in 6..10 {
            for col in 6..14 {
                dem.set(col, row, NODATA);
            }
        }
        dem
    }
}
