//! Measures what the DEM changes in the render of a batch output directory,
//! or compares two of them.
//!
//! ```text
//! cargo run --release --example measure_dem_render -- out-a [out-b]
//! ```
//!
//! For each tile (a directory with an `extent.txt`), within the tile extent:
//!
//! - the share of `DEM_BLOCK_SIZE` cells of `dem_with_buffer.tif` whose
//!   centre falls in a `dem.tif` cell with data (a cell with ground points
//!   or, since the 1 m fallback, a cell whose 1 m block has some), and the
//!   share filled by `fill_nodata`,
//! - the cliff pixels of `cliffs.png` (pixels that are not transparent),
//! - the length and number of lines of `contours-raw/contours-raw.shp` and
//!   `contours/contours.shp`, per level and in all, clipped to the tile.
//!
//! For the directory, the number of 202 (cliff) and 205 (large boulder)
//! objects of `merged-map.ocd`. With two directories, each number is printed
//! for both with the relative change, and contour levels whose length changes
//! by more than `--level-threshold` percent are listed.

use cassini::grid::Grid;
use clap::Parser;
use shapefile::{dbase::FieldValue, Shape};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
struct Args {
    a: PathBuf,
    b: Option<PathBuf>,

    /// List the contour levels whose length changes by more than this, in
    /// percent.
    #[arg(long, default_value_t = 5.0)]
    level_threshold: f64,
}

/// The lengths of the lines of a contour file, by level in centimetres.
#[derive(Default, Clone)]
struct Contours {
    levels: BTreeMap<i64, (f64, usize)>,
}

impl Contours {
    fn total(&self) -> (f64, usize) {
        self.levels
            .values()
            .fold((0.0, 0), |(length, count), &(l, c)| (length + l, count + c))
    }
}

#[derive(Default, Clone)]
struct TileMeasures {
    dem_cells: usize,
    dem_with_points: usize,
    dem_filled: usize,
    cliff_pixels: usize,
    raw_contours: Contours,
    smoothed_contours: Contours,
}

struct Measures {
    tiles: BTreeMap<String, TileMeasures>,
    ocd_cliffs: Option<(usize, usize)>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let result = (|| -> Result<(), String> {
        let a = measure(&args.a)?;
        let b = args.b.as_deref().map(measure).transpose()?;
        report(&a, b.as_ref(), args.level_threshold);
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn measure(dir: &Path) -> Result<Measures, String> {
    let mut tiles = BTreeMap::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("extent.txt").exists())
        .collect();
    entries.sort();
    for tile_dir in entries {
        let name = tile_dir.file_name().unwrap().to_string_lossy().into_owned();
        tiles.insert(name, measure_tile(&tile_dir)?);
    }
    let ocd_path = dir.join("merged-map.ocd");
    let ocd_cliffs = ocd_path
        .exists()
        .then(|| count_ocd_cliffs(&ocd_path))
        .transpose()?;
    Ok(Measures { tiles, ocd_cliffs })
}

fn measure_tile(dir: &Path) -> Result<TileMeasures, String> {
    let extent = fs::read_to_string(dir.join("extent.txt"))
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    let extent: Vec<f64> = extent
        .trim()
        .split('|')
        .map(|value| value.parse().map_err(|_| format!("bad extent {extent}")))
        .collect::<Result<_, _>>()?;
    let [min_x, min_y, max_x, max_y] = extent[..] else {
        return Err(format!("bad extent in {}", dir.display()));
    };

    let read = |name: &str| {
        Grid::<f64>::read_geotiff(&dir.join(name)).map_err(|error| format!("{name}: {error}"))
    };
    let dem = read("dem.tif")?;
    let filled = read("dem_with_buffer.tif")?;
    let mut measures = TileMeasures::default();
    for row in 0..filled.height {
        for col in 0..filled.width {
            let (x, y) = filled.cell_center(col, row);
            if !(min_x..max_x).contains(&x) || !(min_y..max_y).contains(&y) {
                continue;
            }
            measures.dem_cells += 1;
            let with_points = dem
                .cell_at(x, y)
                .is_some_and(|(col, row)| dem.has_data(dem.get(col, row)));
            if with_points {
                measures.dem_with_points += 1;
            } else if filled.has_data(filled.get(col, row)) {
                measures.dem_filled += 1;
            }
        }
    }

    let cliffs = image::open(dir.join("cliffs.png"))
        .map_err(|error| format!("cliffs.png: {error}"))?
        .into_rgba8();
    measures.cliff_pixels = cliffs.pixels().filter(|pixel| pixel[3] != 0).count();

    let clip = [min_x, min_y, max_x, max_y];
    measures.raw_contours = read_contours(&dir.join("contours-raw/contours-raw.shp"), clip)?;
    measures.smoothed_contours = read_contours(&dir.join("contours/contours.shp"), clip)?;
    Ok(measures)
}

fn read_contours(path: &Path, clip: [f64; 4]) -> Result<Contours, String> {
    let error = |error: shapefile::Error| format!("{}: {error}", path.display());
    let mut reader = shapefile::Reader::from_path(path).map_err(error)?;
    let mut contours = Contours::default();
    for shape_record in reader.iter_shapes_and_records() {
        let (shape, record) = shape_record.map_err(error)?;
        let Some(FieldValue::Numeric(Some(elevation))) = record.get("elev") else {
            return Err(format!("{}: a line has no `elev`", path.display()));
        };
        let parts = match shape {
            Shape::Polyline(line) => line.parts().to_vec(),
            Shape::NullShape => continue,
            other => {
                return Err(format!(
                    "{}: unexpected {}",
                    path.display(),
                    other.shapetype()
                ))
            }
        };
        let length: f64 = parts
            .iter()
            .flat_map(|part| part.windows(2))
            .map(|segment| {
                clipped_length(segment[0].x, segment[0].y, segment[1].x, segment[1].y, clip)
            })
            .sum();
        if length > 0.0 {
            let level = contours
                .levels
                .entry((elevation * 100.0).round() as i64)
                .or_default();
            level.0 += length;
            level.1 += 1;
        }
    }
    Ok(contours)
}

/// The length of the segment from (`x0`, `y0`) to (`x1`, `y1`) inside the
/// `[min_x, min_y, max_x, max_y]` rectangle (Liang-Barsky).
fn clipped_length(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    [min_x, min_y, max_x, max_y]: [f64; 4],
) -> f64 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-dx, x0 - min_x),
        (dx, max_x - x0),
        (-dy, y0 - min_y),
        (dy, max_y - y0),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return 0.0;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t1 <= t0 {
        return 0.0;
    }
    (t1 - t0) * dx.hypot(dy)
}

/// The OCD 12 layout, as `src/ocd.rs` writes it: the header field holding
/// the offset of the first object index block, the entries of a block, and
/// the size of an entry and the offset of its object record in it.
const HEADER_FIRST_OBJECT_BLOCK: usize = 12;
const INDEX_BLOCK_ENTRIES: usize = 256;
const OBJECT_ENTRY_SIZE: usize = 40;
const OBJECT_ENTRY_POSITION: usize = 16;

/// The number of 202 and 205 objects of an OCD 12 file.
fn count_ocd_cliffs(path: &Path) -> Result<(usize, usize), String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let u32_at = |pos: usize| -> Result<usize, String> {
        bytes
            .get(pos..pos + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
            .ok_or_else(|| format!("{}: truncated at {pos}", path.display()))
    };
    let (mut cliffs, mut boulders) = (0, 0);
    let mut block = u32_at(HEADER_FIRST_OBJECT_BLOCK)?;
    let mut blocks = 0;
    while block != 0 {
        blocks += 1;
        if blocks > bytes.len() {
            return Err(format!(
                "{}: object index blocks form a cycle",
                path.display()
            ));
        }
        for entry in 0..INDEX_BLOCK_ENTRIES {
            let pos = u32_at(block + 4 + entry * OBJECT_ENTRY_SIZE + OBJECT_ENTRY_POSITION)?;
            if pos == 0 {
                continue;
            }
            match u32_at(pos)? as i32 / 1000 {
                202 => cliffs += 1,
                205 => boulders += 1,
                _ => {}
            }
        }
        block = u32_at(block)?;
    }
    Ok((cliffs, boulders))
}

fn percent(part: usize, whole: usize) -> f64 {
    100.0 * part as f64 / whole.max(1) as f64
}

fn change(a: f64, b: f64) -> String {
    if a == 0.0 {
        return String::new();
    }
    format!("{:+.1}%", 100.0 * (b - a) / a)
}

fn report(a: &Measures, b: Option<&Measures>, level_threshold: f64) {
    let both = |tile: &str| (a.tiles.get(tile), b.and_then(|b| b.tiles.get(tile)));
    let mut names: Vec<&String> = a.tiles.keys().collect();
    if let Some(b) = b {
        names.extend(b.tiles.keys().filter(|name| !a.tiles.contains_key(*name)));
    }

    println!("DEM cells in the tile: with ground points / filled by fill_nodata");
    for name in &names {
        let (ta, tb) = both(name);
        let describe = |t: Option<&TileMeasures>| {
            t.map(|t| {
                format!(
                    "{:5.1}% / {:5.1}%",
                    percent(t.dem_with_points, t.dem_cells),
                    percent(t.dem_filled, t.dem_cells)
                )
            })
            .unwrap_or_else(|| "-".to_owned())
        };
        println!(
            "  {name}  {}  |  {}",
            describe(ta),
            if b.is_some() {
                describe(tb)
            } else {
                String::new()
            }
        );
    }

    println!("\nCliff pixels in cliffs.png");
    let mut totals = (0usize, 0usize);
    for name in &names {
        let (ta, tb) = both(name);
        let (pa, pb) = (
            ta.map_or(0, |t| t.cliff_pixels),
            tb.map_or(0, |t| t.cliff_pixels),
        );
        totals.0 += pa;
        totals.1 += pb;
        if b.is_some() {
            println!(
                "  {name}  {pa:>9}  {pb:>9}  {}",
                change(pa as f64, pb as f64)
            );
        } else {
            println!("  {name}  {pa:>9}");
        }
    }
    if b.is_some() {
        println!(
            "  {:29}  {:>9}  {:>9}  {}",
            "all",
            totals.0,
            totals.1,
            change(totals.0 as f64, totals.1 as f64)
        );
    } else {
        println!("  {:29}  {:>9}", "all", totals.0);
    }

    println!("\nOCAD objects in merged-map.ocd (202 cliffs, 205 boulders)");
    let describe_ocd = |m: &Measures| {
        m.ocd_cliffs
            .map_or("-".to_owned(), |(c, bo)| format!("{c} / {bo}"))
    };
    print!("  {}", describe_ocd(a));
    if let Some(b) = b {
        print!("  |  {}", describe_ocd(b));
        if let (Some(ca), Some(cb)) = (a.ocd_cliffs, b.ocd_cliffs) {
            print!(
                "  ({} / {})",
                change(ca.0 as f64, cb.0 as f64),
                change(ca.1 as f64, cb.1 as f64)
            );
        }
    }
    println!();

    for (title, pick) in [
        (
            "contours-raw.shp",
            (|t: &TileMeasures| &t.raw_contours) as fn(&TileMeasures) -> &Contours,
        ),
        ("contours.shp", |t: &TileMeasures| &t.smoothed_contours),
    ] {
        let merge = |m: Option<&Measures>| {
            let mut all = Contours::default();
            for t in m.into_iter().flat_map(|m| m.tiles.values()) {
                for (&level, &(length, count)) in &pick(t).levels {
                    let entry = all.levels.entry(level).or_default();
                    entry.0 += length;
                    entry.1 += count;
                }
            }
            all
        };
        let (ca, cb) = (merge(Some(a)), merge(b));
        println!("\n{title}: total length (m) and lines, clipped to the tiles");
        for name in &names {
            let (ta, tb) = both(name);
            let (la, na) = ta.map_or((0.0, 0), |t| pick(t).total());
            if b.is_some() {
                let (lb, nb) = tb.map_or((0.0, 0), |t| pick(t).total());
                println!(
                    "  {name}  {la:>10.0} {na:>6}  {lb:>10.0} {nb:>6}  {} {}",
                    change(la, lb),
                    change(na as f64, nb as f64)
                );
            } else {
                println!("  {name}  {la:>10.0} {na:>6}");
            }
        }
        let (la, na) = ca.total();
        if b.is_some() {
            let (lb, nb) = cb.total();
            println!(
                "  {:29}  {la:>10.0} {na:>6}  {lb:>10.0} {nb:>6}  {} {}",
                "all",
                change(la, lb),
                change(na as f64, nb as f64)
            );
            let mut levels: Vec<i64> = ca.levels.keys().chain(cb.levels.keys()).copied().collect();
            levels.sort();
            levels.dedup();
            let changed: Vec<String> = levels
                .iter()
                .filter_map(|level| {
                    let la = ca.levels.get(level).map_or(0.0, |l| l.0);
                    let lb = cb.levels.get(level).map_or(0.0, |l| l.0);
                    let relative = 100.0 * (lb - la) / la.max(1e-9);
                    (relative.abs() > level_threshold).then(|| {
                        format!(
                            "{:.2} m: {la:.0} -> {lb:.0} ({relative:+.1}%)",
                            *level as f64 / 100.0
                        )
                    })
                })
                .collect();
            println!(
                "  {} of {} levels change by more than {level_threshold}%",
                changed.len(),
                levels.len()
            );
            for line in changed {
                println!("    {line}");
            }
            let mut relatives: Vec<f64> = levels
                .iter()
                .filter_map(|level| {
                    let la = ca.levels.get(level).map_or(0.0, |l| l.0);
                    let lb = cb.levels.get(level).map_or(0.0, |l| l.0);
                    (la > 0.0).then(|| (100.0 * (lb - la) / la).abs())
                })
                .collect();
            relatives.sort_by(f64::total_cmp);
            if !relatives.is_empty() {
                println!(
                    "  per-level |change|: median {:.2}%, max {:.2}%",
                    relatives[relatives.len() / 2],
                    relatives[relatives.len() - 1]
                );
            }
        } else {
            println!("  {:29}  {la:>10.0} {na:>6}", "all");
        }
    }
}
