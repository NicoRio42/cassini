//! Compares the OpenStreetMap shapefiles of two directories, as written by
//! `ogr2ogr` or by Cassini: `multipolygons.shp` and `lines.shp`.
//!
//! ```text
//! cargo run --release --example compare_shapefiles -- a_dir b_dir [--tolerance 1e-4]
//! ```
//!
//! For each layer it prints the feature counts and, per main tag, the count
//! and total area or length, then matches features by OSM id and compares
//! their fields and vertices. Exits with status 1 if the feature order, a
//! field or a vertex differs (by more than `--tolerance` metres).
//!
//! The `.dbf` is read as ISO-8859-1 when its language driver says so (as
//! OGR writes it), as UTF-8 otherwise.

use clap::Parser;
use shapefile::{Point, PolygonRing, Shape};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
struct Args {
    a: PathBuf,
    b: PathBuf,

    /// Largest vertex distance, in metres, that counts as equal.
    #[arg(long, default_value_t = 1e-4)]
    tolerance: f64,
}

/// The ISO-8859-1 language driver ID (LDID/87) that OGR writes.
const LATIN_1_LANGUAGE_DRIVER: u8 = 0x57;

/// The fields whose values are grouped in the per-tag statistics.
const MULTIPOLYGON_TAG_KEYS: [&str; 3] = ["natural", "building", "landuse"];
const LINE_TAG_KEYS: [&str; 4] = ["highway", "waterway", "railway", "aerialway"];
/// The `other_tags` keys grouped in the per-tag statistics of lines.
const LINE_OTHER_TAG_KEYS: [&str; 3] = ["power", "natural", "tunnel"];

struct Feature {
    id: String,
    fields: BTreeMap<String, String>,
    /// Polyline parts, or polygon rings with whether they are outer.
    parts: Vec<(bool, Vec<Point>)>,
}

impl Feature {
    /// The area of a polygon, or the length of a polyline, in m² or m.
    fn measure(&self, is_polygon: bool) -> f64 {
        self.parts
            .iter()
            .map(|(is_outer, points)| {
                if is_polygon {
                    let area = ring_area(points);
                    if *is_outer {
                        area
                    } else {
                        -area
                    }
                } else {
                    points
                        .windows(2)
                        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y))
                        .sum()
                }
            })
            .sum()
    }
}

fn ring_area(points: &[Point]) -> f64 {
    let Some(origin) = points.first() else {
        return 0.0;
    };
    // Relative to the first vertex: Lambert-93 coordinates are large enough
    // for the products to cancel out the area of small buildings.
    let sum: f64 = points
        .windows(2)
        .map(|pair| {
            let (x0, y0) = (pair[0].x - origin.x, pair[0].y - origin.y);
            let (x1, y1) = (pair[1].x - origin.x, pair[1].y - origin.y);
            x0 * y1 - x1 * y0
        })
        .sum();
    sum.abs() / 2.0
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut identical = true;
    for (layer, is_polygon) in [("multipolygons", true), ("lines", false)] {
        match compare_layer(&args, layer, is_polygon) {
            Ok(layer_identical) => identical &= layer_identical,
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::from(2);
            }
        }
    }
    if identical {
        println!("\nidentical");
        ExitCode::SUCCESS
    } else {
        println!("\ndifferent");
        ExitCode::from(1)
    }
}

fn compare_layer(args: &Args, layer: &str, is_polygon: bool) -> Result<bool, String> {
    let a = read_layer(&args.a.join(format!("{layer}.shp")))?;
    let b = read_layer(&args.b.join(format!("{layer}.shp")))?;
    println!("\n== {layer}: {} vs {} features", a.len(), b.len());

    let unit = if is_polygon { "m²" } else { "m" };
    let stats_a = tag_statistics(&a, is_polygon);
    let stats_b = tag_statistics(&b, is_polygon);
    let mut tags: Vec<&String> = stats_a.keys().chain(stats_b.keys()).collect();
    tags.sort();
    tags.dedup();
    println!(
        "{:<34} {:>7} {:>7} {:>8}   {:>14} {:>14} {:>9}",
        "tag", "count a", "count b", "Δ count", "a", "b", "Δ"
    );
    for tag in tags {
        let (count_a, measure_a) = stats_a.get(tag).copied().unwrap_or((0, 0.0));
        let (count_b, measure_b) = stats_b.get(tag).copied().unwrap_or((0, 0.0));
        println!(
            "{tag:<34} {count_a:>7} {count_b:>7} {:>8}   {measure_a:>12.1}{unit:<2} {measure_b:>12.1}{unit:<2} {:>8.3}%",
            count_b as i64 - count_a as i64,
            relative_difference(measure_a, measure_b),
        );
    }

    let mut identical = a.len() == b.len();
    let ids_a: Vec<&str> = a.iter().map(|feature| feature.id.as_str()).collect();
    let ids_b: Vec<&str> = b.iter().map(|feature| feature.id.as_str()).collect();
    if ids_a != ids_b {
        identical = false;
        println!("feature order differs");
    }

    let by_id_b: HashMap<&str, &Feature> = b
        .iter()
        .map(|feature| (feature.id.as_str(), feature))
        .collect();
    let by_id_a: HashMap<&str, &Feature> = a
        .iter()
        .map(|feature| (feature.id.as_str(), feature))
        .collect();
    let only_a: Vec<&str> = ids_a
        .iter()
        .copied()
        .filter(|id| !by_id_b.contains_key(id))
        .collect();
    let only_b: Vec<&str> = ids_b
        .iter()
        .copied()
        .filter(|id| !by_id_a.contains_key(id))
        .collect();
    for (side, ids) in [("a", &only_a), ("b", &only_b)] {
        if !ids.is_empty() {
            identical = false;
            println!("{} features only in {side}: {}", ids.len(), preview(ids));
        }
    }

    let mut field_differences = 0;
    let mut geometry_differences = 0;
    let mut max_distance: f64 = 0.0;
    for feature_a in &a {
        let Some(feature_b) = by_id_b.get(feature_a.id.as_str()) else {
            continue;
        };
        let names: std::collections::BTreeSet<&String> = feature_a
            .fields
            .keys()
            .chain(feature_b.fields.keys())
            .collect();
        let empty = String::new();
        let differing: Vec<String> = names
            .into_iter()
            .filter(|name| {
                feature_a.fields.get(*name).unwrap_or(&empty)
                    != feature_b.fields.get(*name).unwrap_or(&empty)
            })
            .map(|name| {
                format!(
                    "{name}: {:?} vs {:?}",
                    feature_a.fields.get(name).unwrap_or(&empty),
                    feature_b.fields.get(name).unwrap_or(&empty)
                )
            })
            .collect();
        if !differing.is_empty() {
            field_differences += 1;
            if field_differences <= 10 {
                println!(
                    "feature {} fields differ: {}",
                    feature_a.id,
                    differing.join(", ")
                );
            }
        }

        match geometry_distance(feature_a, feature_b) {
            Some(distance) => {
                max_distance = max_distance.max(distance);
                if distance > args.tolerance {
                    geometry_differences += 1;
                }
            }
            None => {
                geometry_differences += 1;
                if geometry_differences <= 10 {
                    println!(
                        "feature {} geometry differs: parts {:?} vs {:?}",
                        feature_a.id,
                        part_sizes(feature_a),
                        part_sizes(feature_b)
                    );
                }
            }
        }
    }
    println!(
        "matched features: {field_differences} with different fields, {geometry_differences} with different geometries, max vertex distance {max_distance:.2e} m"
    );
    Ok(identical && field_differences == 0 && geometry_differences == 0)
}

fn preview(ids: &[&str]) -> String {
    let mut text = ids.iter().take(10).copied().collect::<Vec<_>>().join(", ");
    if ids.len() > 10 {
        text.push_str(", …");
    }
    text
}

fn part_sizes(feature: &Feature) -> Vec<(bool, usize)> {
    feature
        .parts
        .iter()
        .map(|(is_outer, points)| (*is_outer, points.len()))
        .collect()
}

/// The largest distance between matching vertices, `None` if the parts or
/// their vertex counts differ.
fn geometry_distance(a: &Feature, b: &Feature) -> Option<f64> {
    if part_sizes(a) != part_sizes(b) {
        return None;
    }
    let mut max: f64 = 0.0;
    for ((_, points_a), (_, points_b)) in a.parts.iter().zip(&b.parts) {
        for (point_a, point_b) in points_a.iter().zip(points_b) {
            max = max.max((point_a.x - point_b.x).hypot(point_a.y - point_b.y));
        }
    }
    Some(max)
}

fn relative_difference(a: f64, b: f64) -> f64 {
    if a == 0.0 && b == 0.0 {
        0.0
    } else {
        (b - a) / a.abs().max(b.abs()) * 100.0
    }
}

/// Counts and total measures per `key=value` of the main tags.
fn tag_statistics(features: &[Feature], is_polygon: bool) -> BTreeMap<String, (usize, f64)> {
    let mut statistics = BTreeMap::new();
    let mut add = |tag: String, measure: f64| {
        let entry = statistics.entry(tag).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += measure;
    };
    for feature in features {
        let measure = feature.measure(is_polygon);
        add("(all)".to_owned(), measure);
        let keys: &[&str] = if is_polygon {
            &MULTIPOLYGON_TAG_KEYS
        } else {
            &LINE_TAG_KEYS
        };
        for key in keys {
            if let Some(value) = feature.fields.get(*key).filter(|value| !value.is_empty()) {
                add(format!("{key}={value}"), measure);
            }
        }
        if !is_polygon {
            let other_tags =
                parse_other_tags(feature.fields.get("other_tags").map_or("", String::as_str));
            for key in LINE_OTHER_TAG_KEYS {
                if let Some(value) = other_tags.get(key) {
                    add(format!("other_tags {key}={value}"), measure);
                }
            }
        }
    }
    statistics
}

/// The `"key"=>"value"` pairs of an `other_tags` value, as `vectors.rs`
/// splits them.
fn parse_other_tags(raw: &str) -> HashMap<&str, &str> {
    raw.split(',')
        .filter_map(|pair| {
            let mut parts = pair.split("=>");
            match (parts.next(), parts.next(), parts.next()) {
                (Some(key), Some(value), None) => {
                    Some((key.trim_matches('"'), value.trim_matches('"')))
                }
                _ => None,
            }
        })
        .collect()
}

fn read_layer(path: &Path) -> Result<Vec<Feature>, String> {
    let shapes = shapefile::ShapeReader::from_path(path)
        .and_then(|reader| reader.read())
        .map_err(|error| format!("could not read `{}`: {error}", path.display()))?;
    let records = read_dbf(&path.with_extension("dbf"))?;
    if shapes.len() != records.len() {
        return Err(format!(
            "`{}` has {} shapes and {} records",
            path.display(),
            shapes.len(),
            records.len()
        ));
    }

    shapes
        .into_iter()
        .zip(records)
        .map(|(shape, fields)| {
            let parts = match shape {
                Shape::Polygon(polygon) => polygon
                    .into_inner()
                    .into_iter()
                    .map(|ring| match ring {
                        PolygonRing::Outer(points) => (true, points),
                        PolygonRing::Inner(points) => (false, points),
                    })
                    .collect(),
                Shape::Polyline(polyline) => polyline
                    .into_inner()
                    .into_iter()
                    .map(|part| (true, part))
                    .collect(),
                other => {
                    return Err(format!(
                        "`{}` has an unexpected {} shape",
                        path.display(),
                        other.shapetype()
                    ))
                }
            };
            let field = |name: &str| fields.get(name).filter(|value| !value.is_empty()).cloned();
            let id = match (field("osm_way_id"), field("osm_id")) {
                (Some(id), _) => format!("way {id}"),
                (None, Some(id)) => id,
                (None, None) => String::new(),
            };
            Ok(Feature { id, fields, parts })
        })
        .collect()
}

/// The records of a `.dbf`, trimmed, by field name.
fn read_dbf(path: &Path) -> Result<Vec<BTreeMap<String, String>>, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read `{}`: {error}", path.display()))?;
    let truncated = || format!("`{}` is truncated", path.display());
    let header = bytes.get(..32).ok_or_else(truncated)?;
    let record_count = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let header_length = u16::from_le_bytes(header[8..10].try_into().unwrap()) as usize;
    let record_length = u16::from_le_bytes(header[10..12].try_into().unwrap()) as usize;
    let is_latin_1 = header[29] == LATIN_1_LANGUAGE_DRIVER;

    let mut fields = vec![];
    let mut offset = 32;
    while *bytes.get(offset).ok_or_else(truncated)? != 0x0d {
        let descriptor = bytes.get(offset..offset + 32).ok_or_else(truncated)?;
        let name_end = descriptor[..11]
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(11);
        let name = String::from_utf8_lossy(&descriptor[..name_end]).into_owned();
        fields.push((name, descriptor[16] as usize));
        offset += 32;
    }

    (0..record_count)
        .map(|index| {
            let start = header_length + index * record_length;
            let record = bytes
                .get(start..start + record_length)
                .ok_or_else(truncated)?;
            // Skip the deletion flag.
            let mut position = 1;
            let mut values = BTreeMap::new();
            for (name, width) in &fields {
                let raw = record
                    .get(position..position + width)
                    .ok_or_else(truncated)?;
                position += width;
                let value = if is_latin_1 {
                    raw.iter().map(|&byte| char::from(byte)).collect()
                } else {
                    String::from_utf8_lossy(raw).into_owned()
                };
                values.insert(name.clone(), value.trim().to_owned());
            }
            Ok(values)
        })
        .collect()
}
