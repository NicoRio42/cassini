//! Crops the same area out of the tile images of two batch output
//! directories, side by side with their differences.
//!
//! ```text
//! cargo run --release --example crop_side_by_side -- out-a out-b crop.png \
//!     --crop min_x,min_y,max_x,max_y [--images contours.png,cliffs.png,full-map.png]
//! ```
//!
//! The crop is in Lambert-93 metres and may span several tiles: each tile
//! directory (one with an `extent.txt`) places its images over its extent.
//! The output has one row per image name, and three columns: `a`, `b`, and
//! the pixels that differ in red over a faded copy of `a`. Transparent images
//! are drawn over white. The share of differing pixels of each row is
//! printed.

use clap::Parser;
use image::{Rgba, RgbaImage};
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
struct Args {
    a: PathBuf,
    b: PathBuf,
    output: PathBuf,

    /// The area to crop, in Lambert-93 metres.
    #[arg(long, value_name = "MIN_X,MIN_Y,MAX_X,MAX_Y", value_parser = parse_numbers)]
    crop: [f64; 4],

    /// The tile images to crop, one row each.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "contours.png,cliffs.png,full-map.png"
    )]
    images: Vec<String>,
}

fn parse_numbers(value: &str) -> Result<[f64; 4], String> {
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

/// The width of the grey lines between the panels, in pixels.
const GAP: u32 = 6;
const GAP_COLOR: Rgba<u8> = Rgba([128, 128, 128, 255]);
const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);

struct TileImage {
    extent: [f64; 4],
    image: RgbaImage,
}

impl TileImage {
    /// The pixel at (`x`, `y`), over white, if inside this tile.
    fn sample(&self, x: f64, y: f64) -> Option<Rgba<u8>> {
        let [min_x, min_y, max_x, max_y] = self.extent;
        if !(min_x..max_x).contains(&x) || !(min_y..max_y).contains(&y) {
            return None;
        }
        let col = ((x - min_x) / (max_x - min_x) * self.image.width() as f64) as u32;
        let row = ((max_y - y) / (max_y - min_y) * self.image.height() as f64) as u32;
        let pixel = self.image.get_pixel(
            col.min(self.image.width() - 1),
            row.min(self.image.height() - 1),
        );
        Some(over_white(*pixel))
    }
}

fn over_white(Rgba([r, g, b, a]): Rgba<u8>) -> Rgba<u8> {
    let blend =
        |channel: u8| ((channel as u32 * a as u32 + 255 * (255 - a as u32) + 127) / 255) as u8;
    Rgba([blend(r), blend(g), blend(b), 255])
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let [min_x, min_y, max_x, max_y] = args.crop;
    let mut rows = vec![];
    for name in &args.images {
        let a = read_tiles(&args.a, name)?;
        let b = read_tiles(&args.b, name)?;
        let first = a
            .first()
            .ok_or_else(|| format!("no `{name}` in {}", args.a.display()))?;
        // The pixel size of the first tile's image, in metres.
        let pixel_size = (first.extent[2] - first.extent[0]) / first.image.width() as f64;
        let width = ((max_x - min_x) / pixel_size).round() as u32;
        let height = ((max_y - min_y) / pixel_size).round() as u32;
        let crop = |tiles: &[TileImage]| {
            RgbaImage::from_fn(width, height, |col, row| {
                let x = min_x + (col as f64 + 0.5) * pixel_size;
                let y = max_y - (row as f64 + 0.5) * pixel_size;
                tiles
                    .iter()
                    .find_map(|tile| tile.sample(x, y))
                    .unwrap_or(WHITE)
            })
        };
        let (crop_a, crop_b) = (crop(&a), crop(&b));
        let mut differing = 0;
        let diff = RgbaImage::from_fn(width, height, |col, row| {
            let (pa, pb) = (crop_a.get_pixel(col, row), crop_b.get_pixel(col, row));
            if pa != pb {
                differing += 1;
                RED
            } else {
                let fade = |channel: u8| 255 - (255 - channel) / 4;
                Rgba([fade(pa[0]), fade(pa[1]), fade(pa[2]), 255])
            }
        });
        println!(
            "{name}: {:.2}% of pixels differ",
            100.0 * differing as f64 / (width as f64 * height as f64).max(1.0)
        );
        rows.push([crop_a, crop_b, diff]);
    }

    let panel_width = rows.iter().map(|row| row[0].width()).max().unwrap_or(0);
    let panel_height = rows.iter().map(|row| row[0].height()).max().unwrap_or(0);
    let mut output = RgbaImage::from_pixel(
        3 * panel_width + 2 * GAP,
        (rows.len() as u32 * (panel_height + GAP)).saturating_sub(GAP),
        GAP_COLOR,
    );
    for (row_index, row) in rows.iter().enumerate() {
        for (col_index, panel) in row.iter().enumerate() {
            image::imageops::replace(
                &mut output,
                panel,
                (col_index as u32 * (panel_width + GAP)) as i64,
                (row_index as u32 * (panel_height + GAP)) as i64,
            );
        }
    }
    output
        .save(&args.output)
        .map_err(|error| format!("{}: {error}", args.output.display()))
}

/// The `name` images of the tile directories of `dir`, with their extents.
fn read_tiles(dir: &Path, name: &str) -> Result<Vec<TileImage>, String> {
    let mut tile_dirs: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("extent.txt").exists() && path.join(name).exists())
        .collect();
    tile_dirs.sort();
    tile_dirs
        .iter()
        .map(|tile_dir| {
            let extent = fs::read_to_string(tile_dir.join("extent.txt"))
                .map_err(|error| format!("{}: {error}", tile_dir.display()))?;
            let extent = parse_numbers(&extent.trim().replace('|', ","))
                .map_err(|error| format!("{}: {error}", tile_dir.display()))?;
            let path = tile_dir.join(name);
            let image = image::open(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?
                .into_rgba8();
            Ok(TileImage { extent, image })
        })
        .collect()
}
