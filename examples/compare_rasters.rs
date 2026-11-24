//! Compares two single-band GeoTIFF rasters cell by cell.
//!
//! ```text
//! cargo run --release --example compare_rasters -- a.tif b.tif \
//!     [--crop min_x,min_y,max_x,max_y] [--diff-png diff.png]
//! ```
//!
//! The rasters must have the same resolution. The comparison covers the cells
//! of `a` whose centres fall in both extents and in the crop window. Exits with
//! status 1 if any compared cell differs.

#[path = "../src/error.rs"]
#[allow(dead_code)]
mod error;
#[path = "../src/grid.rs"]
#[allow(dead_code)]
mod grid;

use clap::Parser;
use grid::Grid;
use image::{Rgb, RgbImage};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
struct Args {
    a: PathBuf,
    b: PathBuf,

    /// Only compare the cells whose centres fall in this window.
    #[arg(long, value_name = "MIN_X,MIN_Y,MAX_X,MAX_Y", value_parser = parse_crop)]
    crop: Option<[f64; 4]>,

    /// Write an image of the differences, one pixel per compared cell.
    #[arg(long, value_name = "PATH")]
    diff_png: Option<PathBuf>,
}

fn parse_crop(value: &str) -> Result<[f64; 4], String> {
    let numbers = value
        .split(',')
        .map(|number| {
            number
                .trim()
                .parse::<f64>()
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    numbers
        .try_into()
        .map_err(|_| "expected four comma-separated numbers".to_owned())
}

/// How a cell compares between the two rasters.
#[derive(Clone, Copy)]
enum Cell {
    BothNodata,
    OnlyANodata,
    OnlyBNodata,
    Valid(f64),
}

fn describe(name: &str, grid: &Grid<f64>, path: &std::path::Path) {
    println!(
        "{name}: {}\n   {} x {} cells of {} m, extent [{}, {}, {}, {}], nodata {}",
        path.display(),
        grid.width,
        grid.height,
        grid.resolution,
        grid.min_x,
        grid.min_y(),
        grid.max_x(),
        grid.max_y,
        grid.nodata
            .map_or("none".to_owned(), |value| value.to_string()),
    );
}

fn main() -> ExitCode {
    let args = Args::parse();
    match compare(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

/// Prints the comparison and returns whether the compared cells are equal.
fn compare(args: &Args) -> Result<bool, String> {
    let a = Grid::<f64>::read_geotiff(&args.a).map_err(|error| error.to_string())?;
    let b = Grid::<f64>::read_geotiff(&args.b).map_err(|error| error.to_string())?;
    describe("a", &a, &args.a);
    describe("b", &b, &args.b);

    if a.resolution != b.resolution {
        return Err(format!(
            "the resolutions differ: {} and {}",
            a.resolution, b.resolution
        ));
    }
    let same_grid = (a.min_x, a.max_y, a.width, a.height) == (b.min_x, b.max_y, b.width, b.height);
    println!(
        "extent and size: {}",
        if same_grid { "identical" } else { "DIFFERENT" }
    );
    let offset = (
        (a.min_x - b.min_x) / a.resolution,
        (a.max_y - b.max_y) / a.resolution,
    );
    if offset.0.fract() != 0.0 || offset.1.fract() != 0.0 {
        println!(
            "warning: the cells are not aligned, offset by ({}, {}) cells",
            offset.0, offset.1
        );
    }

    let [crop_min_x, crop_min_y, crop_max_x, crop_max_y] = args.crop.unwrap_or([
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
    ]);
    let cols: Vec<usize> = (0..a.width)
        .filter(|&col| {
            let (x, _) = a.cell_center(col, 0);
            (crop_min_x..crop_max_x).contains(&x) && (b.min_x..b.max_x()).contains(&x)
        })
        .collect();
    let rows: Vec<usize> = (0..a.height)
        .filter(|&row| {
            let (_, y) = a.cell_center(0, row);
            (crop_min_y..crop_max_y).contains(&y) && (b.min_y()..b.max_y).contains(&y)
        })
        .collect();
    if cols.is_empty() || rows.is_empty() {
        return Err("the compared window is empty".to_owned());
    }
    let (first_x, first_y) = a.cell_center(cols[0], rows[0]);
    let (last_x, last_y) = a.cell_center(*cols.last().unwrap(), *rows.last().unwrap());
    let half = a.resolution / 2.0;
    println!(
        "compared window: {} x {} cells, extent [{}, {}, {}, {}]",
        cols.len(),
        rows.len(),
        first_x - half,
        last_y - half,
        last_x + half,
        first_y + half,
    );

    let mut cells = Vec::with_capacity(cols.len() * rows.len());
    let (mut both_nodata, mut only_a_nodata, mut only_b_nodata) = (0usize, 0usize, 0usize);
    let mut differences = Vec::new();
    let mut max_difference: Option<(f64, f64, f64, f64, f64)> = None;
    for &row in &rows {
        for &col in &cols {
            let (x, y) = a.cell_center(col, row);
            let (b_col, b_row) = b.cell_at(x, y).expect("the window is inside b");
            let (value_a, value_b) = (a.get(col, row), b.get(b_col, b_row));
            let cell = match (a.has_data(value_a), b.has_data(value_b)) {
                (false, false) => {
                    both_nodata += 1;
                    Cell::BothNodata
                }
                (false, true) => {
                    only_a_nodata += 1;
                    Cell::OnlyANodata
                }
                (true, false) => {
                    only_b_nodata += 1;
                    Cell::OnlyBNodata
                }
                (true, true) => {
                    let difference = (value_a - value_b).abs();
                    if max_difference.map_or(true, |max| difference > max.0) {
                        max_difference = Some((difference, x, y, value_a, value_b));
                    }
                    differences.push(difference);
                    Cell::Valid(difference)
                }
            };
            cells.push(cell);
        }
    }

    println!("nodata on both: {both_nodata}");
    println!("nodata only in a: {only_a_nodata}");
    println!("nodata only in b: {only_b_nodata}");
    let valid = differences.len();
    let identical = differences
        .iter()
        .filter(|&&difference| difference == 0.0)
        .count();
    println!(
        "valid on both: {valid}, identical: {identical} ({:.4}%)",
        percent(identical, valid)
    );
    let mut p99 = 0.0;
    if let Some((max, x, y, value_a, value_b)) = max_difference {
        let mean = differences.iter().sum::<f64>() / valid as f64;
        let rank = ((valid as f64 * 0.99).ceil() as usize).clamp(1, valid) - 1;
        p99 = *differences.select_nth_unstable_by(rank, f64::total_cmp).1;
        println!("abs difference: max {max:e}, mean {mean:e}, p99 {p99:e}");
        if max > 0.0 {
            println!("max at ({x}, {y}): a = {value_a}, b = {value_b}");
        }
    }

    if let Some(path) = &args.diff_png {
        let scale = if p99 > 0.0 {
            p99
        } else {
            max_difference.map_or(0.0, |max| max.0)
        };
        write_diff_png(path, cols.len(), rows.len(), &cells, scale)?;
        println!(
            "diff image: {} (grey: equal, white to red: |a - b| up to {scale:e}, \
             black: nodata on both, blue: nodata only in a, orange: nodata only in b)",
            path.display()
        );
    }

    Ok(identical == valid && only_a_nodata == 0 && only_b_nodata == 0)
}

fn percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        100.0
    } else {
        100.0 * count as f64 / total as f64
    }
}

fn write_diff_png(
    path: &std::path::Path,
    width: usize,
    height: usize,
    cells: &[Cell],
    scale: f64,
) -> Result<(), String> {
    let mut image = RgbImage::new(width as u32, height as u32);
    for (pixel, cell) in image.pixels_mut().zip(cells) {
        *pixel = match *cell {
            Cell::BothNodata => Rgb([0, 0, 0]),
            Cell::OnlyANodata => Rgb([40, 90, 255]),
            Cell::OnlyBNodata => Rgb([255, 150, 0]),
            Cell::Valid(difference) if difference == 0.0 => Rgb([200, 200, 200]),
            Cell::Valid(difference) => {
                // Any difference is visible, the largest ones in full red.
                let t = (0.3 + 0.7 * difference / scale).min(1.0);
                let other = (255.0 * (1.0 - t)) as u8;
                Rgb([255, other, other])
            }
        };
    }
    image
        .save(path)
        .map_err(|error| format!("could not write `{}`: {error}", path.display()))
}
