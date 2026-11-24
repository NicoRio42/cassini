use crate::{
    error::{Result, ResultContext},
    map_features::{
        clip_line_part, clip_ring, get_polygon_areas, AreaSymbol, Extent, LineSymbol, MapFeature,
    },
    ocad_merge::{merge_tile_objects, Bounds, TileObjects},
    ocd::{
        format_symbol_number, Georeferencing, ObjectKind, OcdObject, OcdPoint, OcdTemplate,
        FLAG_HOLE,
    },
    tile::Tile,
};
use log::{info, warn};
use shapefile::Point;
use std::{iter::once, path::Path, time::Instant};

pub const MERGED_OCD_FILE_NAME: &str = "merged-map.ocd";

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
    };

    AreaSymbolNumbers { fill, border }
}

/// Every OCD symbol number the exporter can emit, sorted.
fn get_exported_symbols() -> Vec<i32> {
    let mut numbers: Vec<i32> = LineSymbol::ALL
        .iter()
        .flat_map(|symbol| get_line_symbol_numbers(*symbol).iter().copied())
        .chain(AreaSymbol::ALL.iter().flat_map(|symbol| {
            let numbers = get_area_symbol_numbers(*symbol);
            once(numbers.fill).chain(numbers.border)
        }))
        .collect();

    numbers.sort();
    numbers.dedup();
    numbers
}

/// Fails when the template lacks a symbol the exporter can emit, and warns
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

    let bytes = template.write(&georeferencing, &objects)?;
    let ocd_path = tile.render_dir_path.join("full-map.ocd");
    std::fs::write(&ocd_path, bytes)
        .context(format!("could not write `{}`", ocd_path.display()))?;

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

/// Writes the objects of every tile of a batch, joined across the tile
/// edges, to `merged-map.ocd` in `output_dir`. The objects must be in
/// `georeferencing`'s paper coordinates.
pub fn write_merged_ocd_file(
    output_dir: &Path,
    template: &OcdTemplate,
    georeferencing: &Georeferencing,
    tiles: Vec<TileObjects>,
) -> Result<()> {
    info!("Merging OCAD export");

    let start = Instant::now();

    let join_tolerance = (SEAM_JOIN_TOLERANCE * georeferencing.units_per_meter()).round() as i32;
    let objects = merge_tile_objects(tiles, join_tolerance);

    let bytes = template.write(georeferencing, &objects)?;
    let ocd_path = output_dir.join(MERGED_OCD_FILE_NAME);
    std::fs::write(&ocd_path, bytes)
        .context(format!("could not write `{}`", ocd_path.display()))?;

    let duration = start.elapsed();

    info!(
        "{} objects exported to `{}` in {:.1?}",
        objects.len(),
        ocd_path.display(),
        duration
    );

    Ok(())
}

fn get_ocd_objects(
    features: &[MapFeature],
    extent: &Extent,
    georeferencing: &Georeferencing,
) -> Result<Vec<OcdObject>> {
    let mut objects: Vec<OcdObject> = vec![];

    for feature in features {
        match feature {
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
        let numbers = get_exported_symbols();

        for number in [
            101000, 301001, 301004, 403000, 406000, 408000, 409000, 410000, 502002, 512000, 521003,
            521004,
        ] {
            assert!(numbers.contains(&number), "{number} is not exported");
        }
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
                // The lake, its bank line closed, and the footpath
                (301001, ObjectKind::Area, 5),
                (301004, ObjectKind::Line, 7),
                (505000, ObjectKind::Line, 3),
            ]
        );
    }
}
