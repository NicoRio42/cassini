use crate::{
    error::{CassiniError, Result, ResultContext},
    helpers::remove_if_exists,
    map_features::{
        clip_line_part, clip_ring, get_polygon_areas, AreaSymbol, Extent, LineSymbol, MapFeature,
        PointSymbol,
    },
    ocad_merge::{merge_tile_objects, take_interior_objects, Bounds, TileObjects},
    ocd::{
        format_symbol_number, read_object_record, write_object_record, Georeferencing, ObjectKind,
        OcdObject, OcdPoint, OcdTemplate, FLAG_HOLE,
    },
    tile::Tile,
};
use log::{info, warn};
use shapefile::Point;
use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
    iter::once,
    path::{Path, PathBuf},
    time::Instant,
};

/// The OCAD export of a single tile, in its render directory.
pub const TILE_OCD_FILE_NAME: &str = "full-map.ocd";
/// The OCAD export of a batch, in the output directory.
pub const MERGED_OCD_FILE_NAME: &str = "merged-map.ocd";
/// The objects of a tile of a batch that merging leaves unchanged, in its
/// render directory until the merged OCAD file is written.
pub const SPILLED_OBJECTS_FILE_NAME: &str = "ocad-objects.bin";

fn get_point_symbol_number(symbol: PointSymbol) -> i32 {
    match symbol {
        PointSymbol::LargeBoulder205 => 205000,
    }
}

/// Returns the OCD symbols drawn for a line feature. A motorway is two
/// overlaid lines. Wide road tunnels are drawn as 512 bridge / tunnel, and
/// road and footpath tunnels are left out, like the PNG renderer does.
fn get_line_symbol_numbers(symbol: LineSymbol) -> &'static [i32] {
    match symbol {
        LineSymbol::Contour101 => &[101000],
        LineSymbol::IndexContour102 => &[102000],
        LineSymbol::FormLine103 => &[103000],
        LineSymbol::UncrossableBodyOfWaterBankLine301_4 => &[301004],
        LineSymbol::CrossableWatercourse304 => &[304000],
        LineSymbol::MinorSeasonalWaterChannel306 => &[306000],
        LineSymbol::DoubleTrackWideRoad502 { tunnel: false } => &[502002, 502003],
        LineSymbol::XxlWideRoad502 { tunnel: false }
        | LineSymbol::XlWideRoad502 { tunnel: false }
        | LineSymbol::WideRoad502 { tunnel: false } => &[502000],
        LineSymbol::DoubleTrackWideRoad502 { tunnel: true }
        | LineSymbol::XxlWideRoad502 { tunnel: true }
        | LineSymbol::XlWideRoad502 { tunnel: true }
        | LineSymbol::WideRoad502 { tunnel: true } => &[512000],
        LineSymbol::Road503 { tunnel: false } => &[503000],
        LineSymbol::Footpath505 { tunnel: false } => &[505000],
        LineSymbol::Road503 { tunnel: true } | LineSymbol::Footpath505 { tunnel: true } => &[],
        LineSymbol::Railway509 => &[509000],
        LineSymbol::PowerLineCablewayOrSkilift510 => &[510000],
        LineSymbol::Cliff202 => &[202000],
    }
}

/// The OCD symbols of an area feature. Clipping an area whose symbol has a
/// border would draw that border along the tile edge, so such areas are
/// exported as a borderless fill plus their border as lines, clipped like
/// any other line.
struct AreaSymbolNumbers {
    fill: i32,
    border: Option<i32>,
}

fn get_area_symbol_numbers(symbol: AreaSymbol) -> AreaSymbolNumbers {
    let (fill, border) = match symbol {
        AreaSymbol::UncrossableBodyOfWater301 => (301001, Some(301004)),
        // Coastlines come with their own 301.004 bank line features
        AreaSymbol::UncrossableBodyOfWaterArea301_1 => (301001, None),
        AreaSymbol::Marsh308 => (308000, None),
        AreaSymbol::AreaThatShallNotBeEntered520 => (520000, None),
        // Cassini draws buildings grey with a black outline
        AreaSymbol::Building521 => (521003, Some(521004)),
        AreaSymbol::RoughOpenLand403 => (403000, None),
        AreaSymbol::VegetationSlowRunning406 => (406000, None),
        AreaSymbol::VegetationWalk408 => (408000, None),
        AreaSymbol::VegetationWalkGoodVisibility409 => (409000, None),
        AreaSymbol::VegetationFight410 => (410000, None),
        AreaSymbol::GiganticBoulderOrRockPillar206 => (206000, None),
    };

    AreaSymbolNumbers { fill, border }
}

/// Every OCD symbol number the exporter can emit, with the kind of the
/// objects it draws, sorted.
fn get_exported_symbols() -> Vec<(i32, ObjectKind)> {
    let mut symbols: Vec<(i32, ObjectKind)> = PointSymbol::ALL
        .iter()
        .map(|symbol| (get_point_symbol_number(*symbol), ObjectKind::Point))
        .chain(LineSymbol::ALL.iter().flat_map(|symbol| {
            get_line_symbol_numbers(*symbol)
                .iter()
                .map(|number| (*number, ObjectKind::Line))
        }))
        .chain(AreaSymbol::ALL.iter().flat_map(|symbol| {
            let numbers = get_area_symbol_numbers(*symbol);
            once((numbers.fill, ObjectKind::Area))
                .chain(numbers.border.map(|border| (border, ObjectKind::Line)))
        }))
        .collect();

    symbols.sort_by_key(|(number, _)| *number);
    symbols.dedup();
    symbols
}

/// Fails when the template lacks a symbol the exporter can emit, or defines
/// it with another type than the objects it draws, and warns
/// when a fill symbol has a border, which would be drawn along the tile edge.
pub fn check_template(template: &OcdTemplate) -> Result<()> {
    template.check_symbols(&get_exported_symbols())?;

    for symbol in AreaSymbol::ALL {
        let fill = get_area_symbol_numbers(symbol).fill;
        if template.has_border(fill) {
            warn!(
                "OCAD template symbol {} has a border, it will be drawn along the tile edges",
                format_symbol_number(fill)
            );
        }
    }

    Ok(())
}

/// Line ends on either side of a tile edge closer than this, in metres, are
/// joined in the merged OCAD file of a batch. Both tiles compute the lines
/// crossing their common edge from the same data, so their ends coincide
/// almost always.
const SEAM_JOIN_TOLERANCE: f64 = 1.0;

/// Writes `features`, clipped to the tile extent, to `full-map.ocd` in the
/// tile's render directory.
pub fn write_ocd_file(tile: &Tile, template: &OcdTemplate, features: &[MapFeature]) -> Result<()> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Exporting map to OCAD",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let georeferencing =
        template.georeferencing((tile.min_x + tile.max_x) / 2, (tile.min_y + tile.max_y) / 2);
    let objects = get_tile_objects(tile, &georeferencing, features)?.objects;

    let ocd_path = tile.render_dir_path.join(TILE_OCD_FILE_NAME);
    template.write_file(&ocd_path, &georeferencing, objects.iter().map(Ok))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. {} objects exported to OCAD in {:.1?}",
        tile.min_x,
        tile.min_y,
        tile.max_x,
        tile.max_y,
        objects.len(),
        duration
    );

    Ok(())
}

/// Converts `features`, clipped to the tile extent, to OCD objects.
pub fn get_tile_objects(
    tile: &Tile,
    georeferencing: &Georeferencing,
    features: &[MapFeature],
) -> Result<TileObjects> {
    let extent = Extent {
        min_x: tile.min_x as f64,
        min_y: tile.min_y as f64,
        max_x: tile.max_x as f64,
        max_y: tile.max_y as f64,
    };
    let (min_x, min_y) = georeferencing.to_ocd(extent.min_x, extent.min_y)?;
    let (max_x, max_y) = georeferencing.to_ocd(extent.max_x, extent.max_y)?;

    Ok(TileObjects {
        bounds: Bounds {
            min_x,
            min_y,
            max_x,
            max_y,
        },
        objects: get_ocd_objects(features, &extent, georeferencing)?,
    })
}

/// The georeferencing of the merged OCAD file of a batch covering `extent`
/// (min x, min y, max x, max y), centred on it. Fails when the extent does
/// not fit in an OCAD file at the template's scale, before any tile is
/// rendered.
pub fn get_batch_georeferencing(
    template: &OcdTemplate,
    (min_x, min_y, max_x, max_y): (i64, i64, i64, i64),
) -> Result<Georeferencing> {
    let georeferencing = template.georeferencing((min_x + max_x) / 2, (min_y + max_y) / 2);

    for (x, y) in [(min_x, min_y), (max_x, max_y)] {
        if georeferencing.to_ocd(x as f64, y as f64).is_err() {
            return Err(CassiniError::InvalidInput {
                message: format!(
                    "the batch covers {} m x {} m, more than an OCAD file holds at 1:{}",
                    max_x - min_x,
                    max_y - min_y,
                    georeferencing.scale
                ),
            });
        }
    }

    Ok(georeferencing)
}

/// Converts `features`, clipped to the tile extent, to OCD objects for the
/// merged OCAD file of a batch, in `georeferencing`'s paper coordinates.
/// Writes those that merging leaves unchanged to the tile's render directory,
/// for `write_merged_ocd_file` to read back, and returns the others.
pub fn prepare_tile_for_merge(
    tile: &Tile,
    georeferencing: &Georeferencing,
    features: &[MapFeature],
) -> Result<TileObjects> {
    let mut tile_objects = get_tile_objects(tile, georeferencing, features)?;
    let interior = take_interior_objects(&mut tile_objects);

    let path = tile.render_dir_path.join(SPILLED_OBJECTS_FILE_NAME);
    let file = File::create(&path).context(format!("could not create `{}`", path.display()))?;
    let mut out = BufWriter::new(file);
    for object in &interior {
        write_object_record(&mut out, object)
            .context(format!("could not write `{}`", path.display()))?;
    }
    out.flush()
        .context(format!("could not write `{}`", path.display()))?;

    Ok(tile_objects)
}

/// Writes the objects of every tile of a batch, joined across the tile
/// edges, to `merged-map.ocd` in `output_dir`: the objects `tiles` were
/// prepared with by `prepare_tile_for_merge`, which are read back and removed
/// one tile at a time, and `seam_objects`, which are merged.
pub fn write_merged_ocd_file<'a>(
    output_dir: &Path,
    template: &OcdTemplate,
    georeferencing: &Georeferencing,
    tiles: impl IntoIterator<Item = &'a Tile>,
    seam_objects: Vec<TileObjects>,
) -> Result<()> {
    info!("Merging OCAD export");

    let start = Instant::now();

    let join_tolerance = (SEAM_JOIN_TOLERANCE * georeferencing.units_per_meter()).round() as i32;
    let merged = merge_tile_objects(seam_objects, join_tolerance);

    let spilled_paths: Vec<PathBuf> = tiles
        .into_iter()
        .map(|tile| tile.render_dir_path.join(SPILLED_OBJECTS_FILE_NAME))
        .collect();
    let ocd_path = output_dir.join(MERGED_OCD_FILE_NAME);
    let mut count = 0;
    template.write_file(
        &ocd_path,
        georeferencing,
        spilled_paths
            .iter()
            .flat_map(|path| read_spilled_objects(path))
            .chain(merged.into_iter().map(Ok))
            .inspect(|_| count += 1),
    )?;

    for path in &spilled_paths {
        remove_if_exists(path).context(format!("could not remove `{}`", path.display()))?;
    }

    let duration = start.elapsed();

    info!(
        "{} objects exported to `{}` in {:.1?}",
        count,
        ocd_path.display(),
        duration
    );

    Ok(())
}

/// The objects `prepare_tile_for_merge` wrote to `path`, read one at a time.
fn read_spilled_objects(path: &Path) -> impl Iterator<Item = Result<OcdObject>> + '_ {
    let context = move || format!("could not read `{}`", path.display());
    let mut input: Option<BufReader<File>> = None;
    let mut is_done = false;

    std::iter::from_fn(move || {
        if is_done {
            return None;
        }
        let reader = match &mut input {
            Some(reader) => reader,
            None => match File::open(path).context(context()) {
                Ok(file) => input.insert(BufReader::new(file)),
                Err(error) => {
                    is_done = true;
                    return Some(Err(error));
                }
            },
        };

        let record = read_object_record(reader).context(context()).transpose();
        // Stops at the end of the file or after an error
        is_done = !matches!(record, Some(Ok(_)));
        record
    })
}

fn get_ocd_objects(
    features: &[MapFeature],
    extent: &Extent,
    georeferencing: &Georeferencing,
) -> Result<Vec<OcdObject>> {
    let mut objects: Vec<OcdObject> = vec![];

    for feature in features {
        match feature {
            MapFeature::Point(symbol, point) => {
                // A point on the edge between two tiles goes to one of them
                if point.x.is_finite() && point.y.is_finite() && extent.contains_half_open(point) {
                    objects.push(OcdObject {
                        symbol: get_point_symbol_number(*symbol),
                        kind: ObjectKind::Point,
                        points: convert_points(&[*point], georeferencing)?,
                    });
                }
            }
            MapFeature::Line(symbol, line) => {
                for part in line.parts() {
                    push_line_objects(
                        &mut objects,
                        get_line_symbol_numbers(*symbol),
                        part,
                        extent,
                        georeferencing,
                    )?;
                }
            }
            MapFeature::Area(symbol, polygon) => {
                let numbers = get_area_symbol_numbers(*symbol);

                // OCAD areas have one outer ring each, so a multipolygon
                // becomes one object per outer ring with its holes following it.
                for area in get_polygon_areas(polygon) {
                    if let Some(points) =
                        get_clipped_area_points(area.outer, &area.holes, extent, georeferencing)?
                    {
                        objects.push(OcdObject {
                            symbol: numbers.fill,
                            kind: ObjectKind::Area,
                            points,
                        });
                    }

                    if let Some(border) = numbers.border {
                        for ring in once(area.outer).chain(area.holes.iter().copied()) {
                            push_line_objects(
                                &mut objects,
                                &[border],
                                ring,
                                extent,
                                georeferencing,
                            )?;
                        }
                    }
                }
            }
        }
    }

    Ok(objects)
}

/// Pushes one line object per symbol for each part of `points` inside the extent.
fn push_line_objects(
    objects: &mut Vec<OcdObject>,
    symbols: &[i32],
    points: &[Point],
    extent: &Extent,
    georeferencing: &Georeferencing,
) -> Result<()> {
    if symbols.is_empty() {
        return Ok(());
    }

    for clipped_part in clip_line_part(&finite_points(points), extent) {
        let points = convert_points(&clipped_part, georeferencing)?;
        if points.len() < 2 {
            continue;
        }

        for symbol in symbols {
            objects.push(OcdObject {
                symbol: *symbol,
                kind: ObjectKind::Line,
                points: points.clone(),
            });
        }
    }

    Ok(())
}

/// Returns the outer ring clipped to the extent followed by the clipped
/// holes, the first point of each hole flagged, or `None` when nothing of the
/// outer ring remains.
fn get_clipped_area_points(
    outer: &[Point],
    holes: &[&[Point]],
    extent: &Extent,
    georeferencing: &Georeferencing,
) -> Result<Option<Vec<OcdPoint>>> {
    let Some(clipped_outer) = clip_ring(&finite_points(outer), extent) else {
        return Ok(None);
    };
    let mut points = convert_points(&clipped_outer, georeferencing)?;
    if points.len() < 4 {
        return Ok(None);
    }

    for hole in holes {
        let Some(clipped_hole) = clip_ring(&finite_points(hole), extent) else {
            continue;
        };
        let mut hole_points = convert_points(&clipped_hole, georeferencing)?;
        if hole_points.len() >= 4 {
            hole_points[0].y_flags |= FLAG_HOLE;
            points.append(&mut hole_points);
        }
    }

    Ok(Some(points))
}

fn finite_points(points: &[Point]) -> Vec<Point> {
    points
        .iter()
        .copied()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .collect()
}

/// Converts points to paper units and drops consecutive duplicates, which
/// appear when points are closer than the OCD resolution.
fn convert_points(points: &[Point], georeferencing: &Georeferencing) -> Result<Vec<OcdPoint>> {
    let mut converted: Vec<OcdPoint> = Vec::with_capacity(points.len());

    for point in points {
        let (x, y) = georeferencing.to_ocd(point.x, point.y)?;
        let ocd_point = OcdPoint { x, y, y_flags: 0 };
        if converted.last() != Some(&ocd_point) {
            converted.push(ocd_point);
        }
    }

    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocad_merge::merge_tile_objects;
    use shapefile::{Polygon, PolygonRing, Polyline};

    const EXTENT: Extent = Extent {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 1000.0,
        max_y: 1000.0,
    };
    const GEOREFERENCING: Georeferencing = Georeferencing {
        offset_x: 500,
        offset_y: 500,
        scale: 10_000.0,
    };

    fn ring(coordinates: &[(f64, f64)]) -> Vec<Point> {
        coordinates
            .iter()
            .map(|(x, y)| Point::new(*x, *y))
            .collect()
    }

    fn square(min: f64, max: f64) -> Vec<Point> {
        ring(&[(min, min), (min, max), (max, max), (max, min), (min, min)])
    }

    fn point(x: i32, y: i32) -> OcdPoint {
        OcdPoint { x, y, y_flags: 0 }
    }

    #[test]
    fn built_in_template_defines_every_exported_symbol() {
        let template = OcdTemplate::load(None).unwrap();

        check_template(&template).unwrap();
        for symbol in AreaSymbol::ALL {
            let fill = get_area_symbol_numbers(symbol).fill;
            assert!(!template.has_border(fill), "{fill} has a border");
        }
    }

    #[test]
    fn exports_the_symbols_of_every_variant() {
        let symbols = get_exported_symbols();
        let numbers: Vec<i32> = symbols.iter().map(|(number, _)| *number).collect();

        for symbol in [
            (101000, ObjectKind::Line),
            (202000, ObjectKind::Line),
            (205000, ObjectKind::Point),
            (206000, ObjectKind::Area),
            (301001, ObjectKind::Area),
            (301004, ObjectKind::Line),
            (403000, ObjectKind::Area),
            (406000, ObjectKind::Area),
            (408000, ObjectKind::Area),
            (409000, ObjectKind::Area),
            (410000, ObjectKind::Area),
            (502002, ObjectKind::Line),
            (512000, ObjectKind::Line),
            (521003, ObjectKind::Area),
            (521004, ObjectKind::Line),
        ] {
            assert!(symbols.contains(&symbol), "{symbol:?} is not exported");
        }
        // Each number once, with one kind
        assert!(numbers.windows(2).all(|pair| pair[0] < pair[1]));
        // Lakes and buildings are exported without their bordered symbols
        assert!(!numbers.contains(&301000));
        assert!(!numbers.contains(&521002));
    }

    #[test]
    fn clips_and_converts_lines() {
        let line = Polyline::new(ring(&[(-100.0, 500.0), (500.0, 500.0), (500.0, 600.0)]));

        let objects = get_ocd_objects(
            &[MapFeature::Line(LineSymbol::Contour101, line)],
            &EXTENT,
            &GEOREFERENCING,
        )
        .unwrap();

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].symbol, 101000);
        assert_eq!(objects[0].kind, ObjectKind::Line);
        assert_eq!(
            objects[0].points,
            vec![point(-5_000, 0), point(0, 0), point(0, 1_000)]
        );
    }

    #[test]
    fn keeps_the_points_of_the_tile_edges_in_one_tile_only() {
        let boulder = |x, y| MapFeature::Point(PointSymbol::LargeBoulder205, Point::new(x, y));
        let features = [
            boulder(500.0, 600.0),
            boulder(0.0, 0.0),
            boulder(1000.0, 500.0),
            boulder(500.0, 1000.0),
            boulder(-1.0, 500.0),
        ];

        let objects = get_ocd_objects(&features, &EXTENT, &GEOREFERENCING).unwrap();

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].symbol, 205000);
        assert_eq!(objects[0].kind, ObjectKind::Point);
        assert_eq!(objects[0].points, vec![point(0, 1_000)]);
        assert_eq!(objects[1].points, vec![point(-5_000, -5_000)]);
    }

    #[test]
    fn writes_motorways_as_two_lines_and_wide_road_tunnels_as_512() {
        let line = || Polyline::new(ring(&[(100.0, 100.0), (200.0, 200.0)]));

        let objects = get_ocd_objects(
            &[
                MapFeature::Line(LineSymbol::DoubleTrackWideRoad502 { tunnel: false }, line()),
                MapFeature::Line(LineSymbol::WideRoad502 { tunnel: true }, line()),
                MapFeature::Line(LineSymbol::Footpath505 { tunnel: true }, line()),
            ],
            &EXTENT,
            &GEOREFERENCING,
        )
        .unwrap();

        let symbols: Vec<i32> = objects.iter().map(|object| object.symbol).collect();
        assert_eq!(symbols, vec![502002, 502003, 512000]);
    }

    #[test]
    fn splits_multipolygons_and_flags_holes() {
        let polygon = Polygon::with_rings(vec![
            PolygonRing::Outer(square(100.0, 400.0)),
            PolygonRing::Outer(square(600.0, 700.0)),
            // A hole of the first outer ring, after the second one
            PolygonRing::Inner(square(200.0, 300.0)),
            // Outside the tile
            PolygonRing::Outer(square(2000.0, 2100.0)),
        ]);

        let objects = get_ocd_objects(
            &[MapFeature::Area(AreaSymbol::Marsh308, polygon)],
            &EXTENT,
            &GEOREFERENCING,
        )
        .unwrap();

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].symbol, 308000);
        assert_eq!(objects[0].kind, ObjectKind::Area);
        assert_eq!(objects[0].points.len(), 10);
        let flags: Vec<i32> = objects[0]
            .points
            .iter()
            .map(|point| point.y_flags)
            .collect();
        assert_eq!(flags, vec![0, 0, 0, 0, 0, FLAG_HOLE, 0, 0, 0, 0]);
        assert_eq!(objects[1].points.len(), 5);
    }

    #[test]
    fn draws_borders_as_lines_that_do_not_follow_the_tile_edge() {
        // A lake crossing the left tile edge
        let lake = Polygon::new(PolygonRing::Outer(square(-100.0, 100.0)));

        let objects = get_ocd_objects(
            &[MapFeature::Area(
                AreaSymbol::UncrossableBodyOfWater301,
                lake,
            )],
            &EXTENT,
            &GEOREFERENCING,
        )
        .unwrap();

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].symbol, 301001);
        assert_eq!(objects[0].kind, ObjectKind::Area);
        assert_eq!(objects[1].symbol, 301004);
        assert_eq!(objects[1].kind, ObjectKind::Line);
        // The bank line follows the shore inside the tile only, not the cuts
        // along x = 0 and y = 0 (-5 000 in paper units)
        assert_eq!(
            objects[1].points,
            vec![
                point(-5_000, -4_000),
                point(-4_000, -4_000),
                point(-4_000, -5_000)
            ]
        );
    }

    #[test]
    fn merges_features_cut_by_tile_edges() {
        let tile = |min_x: i64| Tile {
            lidar_dir_path: Default::default(),
            render_dir_path: Default::default(),
            min_x,
            min_y: 0,
            max_x: min_x + 1000,
            max_y: 1000,
        };
        // Both tiles hold the whole features, as they do with OpenStreetMap
        let features = || {
            vec![
                MapFeature::Line(
                    LineSymbol::Footpath505 { tunnel: false },
                    Polyline::new(ring(&[(500.0, 500.0), (1500.0, 600.0)])),
                ),
                MapFeature::Area(
                    AreaSymbol::UncrossableBodyOfWater301,
                    Polygon::new(PolygonRing::Outer(ring(&[
                        (900.0, 400.0),
                        (900.0, 600.0),
                        (1100.0, 600.0),
                        (1100.0, 400.0),
                        (900.0, 400.0),
                    ]))),
                ),
            ]
        };

        let tiles: Vec<TileObjects> = [tile(0), tile(1000)]
            .iter()
            .map(|tile| get_tile_objects(tile, &GEOREFERENCING, &features()).unwrap())
            .collect();
        assert_eq!(tiles[0].bounds.max_x, 5_000);
        assert_eq!(tiles[1].bounds.min_x, 5_000);
        // The bank line is cut where its ring starts too
        let counts: Vec<usize> = tiles.iter().map(|tile| tile.objects.len()).collect();
        assert_eq!(counts, vec![4, 3]);

        let objects = merge_tile_objects(tiles, 10);

        let mut kinds: Vec<(i32, ObjectKind, usize)> = objects
            .iter()
            .map(|object| (object.symbol, object.kind, object.points.len()))
            .collect();
        kinds.sort_by_key(|(symbol, ..)| *symbol);
        assert_eq!(
            kinds,
            vec![
                // The lake, its bank line closed, and the footpath, without
                // the vertices where the tile edge cut their straight sides
                (301001, ObjectKind::Area, 5),
                (301004, ObjectKind::Line, 5),
                (505000, ObjectKind::Line, 2),
            ]
        );
    }

    #[test]
    fn rejects_batches_larger_than_an_ocad_file() {
        let template = OcdTemplate::load(None).unwrap();

        assert!(get_batch_georeferencing(&template, (0, 0, 100_000, 100_000)).is_ok());
        assert!(get_batch_georeferencing(&template, (0, 0, 2_000_000, 1_000)).is_err());
    }

    #[test]
    fn writes_the_merged_file_from_the_prepared_tiles() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let output_dir = std::env::temp_dir().join(format!("cassini-merged-ocd-{unique}"));
        let tiles: Vec<Tile> = [0, 1000]
            .into_iter()
            .map(|min_x| {
                let render_dir_path = output_dir.join(min_x.to_string());
                std::fs::create_dir_all(&render_dir_path).unwrap();
                Tile {
                    lidar_dir_path: Default::default(),
                    render_dir_path,
                    min_x,
                    min_y: 0,
                    max_x: min_x + 1000,
                    max_y: 1000,
                }
            })
            .collect();
        let features = vec![
            // Crossing the edge between the tiles
            MapFeature::Line(
                LineSymbol::Footpath505 { tunnel: false },
                Polyline::new(ring(&[(500.0, 500.0), (1500.0, 600.0)])),
            ),
            // Inside the first tile
            MapFeature::Line(
                LineSymbol::Footpath505 { tunnel: false },
                Polyline::new(ring(&[(100.0, 100.0), (200.0, 200.0), (300.0, 100.0)])),
            ),
        ];
        let template = OcdTemplate::load(None).unwrap();
        let georeferencing = get_batch_georeferencing(&template, (0, 0, 2000, 1000)).unwrap();

        let seam_objects: Vec<TileObjects> = tiles
            .iter()
            .map(|tile| prepare_tile_for_merge(tile, &georeferencing, &features).unwrap())
            .collect();
        let counts: Vec<usize> = seam_objects.iter().map(|tile| tile.objects.len()).collect();
        assert_eq!(counts, vec![1, 1]);

        write_merged_ocd_file(
            &output_dir,
            &template,
            &georeferencing,
            &tiles,
            seam_objects,
        )
        .unwrap();

        let bytes = std::fs::read(output_dir.join(MERGED_OCD_FILE_NAME)).unwrap();
        let spilled_files_left = tiles.iter().any(|tile| {
            tile.render_dir_path
                .join(SPILLED_OBJECTS_FILE_NAME)
                .exists()
        });
        std::fs::remove_dir_all(&output_dir).unwrap();

        let mut point_counts: Vec<usize> = crate::ocd::tests::read_objects(&bytes)
            .iter()
            .map(|written| written.object.points.len())
            .collect();
        point_counts.sort();
        assert_eq!(point_counts, vec![2, 3]);
        assert!(!spilled_files_left);
    }
}
