use crate::{
    error::{Result, ResultContext},
    map_features::{clip_line_part, clip_ring, AreaSymbol, Extent, LineSymbol, MapFeature},
    ocd::{Georeferencing, ObjectKind, OcdObject, OcdPoint, OcdTemplate, FLAG_HOLE},
    tile::Tile,
};
use log::info;
use shapefile::{Point, PolygonRing};
use std::time::Instant;

/// Every OCD symbol number the exporter can emit. The template must define them all.
pub const EXPORTED_SYMBOLS: [i32; 18] = [
    101000, 102000, 103000, 301000, 301001, 301004, 304000, 306000, 308000, 502000, 502002, 502003,
    503000, 505000, 509000, 510000, 520000, 521002,
];

/// Returns the OCD symbols drawn for a line feature. A motorway is two
/// overlaid lines. Tunnels are left out, like the PNG renderer does for
/// roads and footpaths.
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
        LineSymbol::Road503 { tunnel: false } => &[503000],
        LineSymbol::Footpath505 { tunnel: false } => &[505000],
        LineSymbol::DoubleTrackWideRoad502 { tunnel: true }
        | LineSymbol::XxlWideRoad502 { tunnel: true }
        | LineSymbol::XlWideRoad502 { tunnel: true }
        | LineSymbol::WideRoad502 { tunnel: true }
        | LineSymbol::Road503 { tunnel: true }
        | LineSymbol::Footpath505 { tunnel: true } => &[],
        LineSymbol::Railway509 => &[509000],
        LineSymbol::PowerLineCablewayOrSkilift510 => &[510000],
    }
}

fn get_area_symbol_numbers(symbol: AreaSymbol) -> &'static [i32] {
    match symbol {
        // The template's 301.000 area has the 301.004 bank line as its border
        AreaSymbol::UncrossableBodyOfWater301 => &[301000],
        AreaSymbol::UncrossableBodyOfWaterArea301_1 => &[301001],
        AreaSymbol::Marsh308 => &[308000],
        AreaSymbol::AreaThatShallNotBeEntered520 => &[520000],
        // Cassini draws buildings grey with a black outline
        AreaSymbol::Building521 => &[521002],
    }
}

/// Writes `features`, clipped to the tile extent, to `full-map.ocd` in the
/// tile's render directory.
pub fn write_ocd_file(tile: &Tile, template: &OcdTemplate, features: &[MapFeature]) -> Result<()> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Exporting map to OCAD",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let extent = Extent {
        min_x: tile.min_x as f64,
        min_y: tile.min_y as f64,
        max_x: tile.max_x as f64,
        max_y: tile.max_y as f64,
    };
    let georeferencing =
        template.georeferencing((tile.min_x + tile.max_x) / 2, (tile.min_y + tile.max_y) / 2);
    let objects = get_ocd_objects(features, &extent, &georeferencing)?;

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

fn get_ocd_objects(
    features: &[MapFeature],
    extent: &Extent,
    georeferencing: &Georeferencing,
) -> Result<Vec<OcdObject>> {
    let mut objects: Vec<OcdObject> = vec![];

    for feature in features {
        match feature {
            MapFeature::Line(symbol, line) => {
                let symbols = get_line_symbol_numbers(*symbol);
                if symbols.is_empty() {
                    continue;
                }

                for part in line.parts() {
                    for clipped_part in clip_line_part(&finite_points(part), extent) {
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
                }
            }
            MapFeature::Area(symbol, polygon) => {
                let symbols = get_area_symbol_numbers(*symbol);

                // OCAD areas have one outer ring each, so a multipolygon
                // becomes one object per outer ring with the holes following it.
                let mut current: Option<Vec<OcdPoint>> = None;
                let mut areas: Vec<Vec<OcdPoint>> = vec![];

                for ring in polygon.rings() {
                    let clipped_ring = clip_ring(&finite_points(ring.points()), extent);

                    match ring {
                        PolygonRing::Outer(_) => {
                            areas.extend(current.take());
                            if let Some(clipped_ring) = clipped_ring {
                                let points = convert_points(&clipped_ring, georeferencing)?;
                                if points.len() >= 4 {
                                    current = Some(points);
                                }
                            }
                        }
                        PolygonRing::Inner(_) => {
                            let (Some(area), Some(clipped_ring)) = (current.as_mut(), clipped_ring)
                            else {
                                continue;
                            };
                            let mut points = convert_points(&clipped_ring, georeferencing)?;
                            if points.len() >= 4 {
                                points[0].y_flags |= FLAG_HOLE;
                                area.append(&mut points);
                            }
                        }
                    }
                }
                areas.extend(current.take());

                for area in areas {
                    for symbol in symbols {
                        objects.push(OcdObject {
                            symbol: *symbol,
                            kind: ObjectKind::Area,
                            points: area.clone(),
                        });
                    }
                }
            }
        }
    }

    Ok(objects)
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
    use shapefile::{Polygon, Polyline};

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

    #[test]
    fn every_mapped_symbol_is_exported_and_in_the_built_in_template() {
        let line_symbols = [
            LineSymbol::Contour101,
            LineSymbol::IndexContour102,
            LineSymbol::FormLine103,
            LineSymbol::UncrossableBodyOfWaterBankLine301_4,
            LineSymbol::CrossableWatercourse304,
            LineSymbol::MinorSeasonalWaterChannel306,
            LineSymbol::DoubleTrackWideRoad502 { tunnel: false },
            LineSymbol::XxlWideRoad502 { tunnel: false },
            LineSymbol::XlWideRoad502 { tunnel: false },
            LineSymbol::WideRoad502 { tunnel: false },
            LineSymbol::Road503 { tunnel: false },
            LineSymbol::Footpath505 { tunnel: false },
            LineSymbol::Railway509,
            LineSymbol::PowerLineCablewayOrSkilift510,
        ];
        let area_symbols = [
            AreaSymbol::UncrossableBodyOfWater301,
            AreaSymbol::UncrossableBodyOfWaterArea301_1,
            AreaSymbol::Marsh308,
            AreaSymbol::AreaThatShallNotBeEntered520,
            AreaSymbol::Building521,
        ];

        let mapped: Vec<i32> = line_symbols
            .iter()
            .flat_map(|symbol| get_line_symbol_numbers(*symbol).iter().copied())
            .chain(
                area_symbols
                    .iter()
                    .flat_map(|symbol| get_area_symbol_numbers(*symbol).iter().copied()),
            )
            .collect();

        for number in &mapped {
            assert!(
                EXPORTED_SYMBOLS.contains(number),
                "{number} is not exported"
            );
        }
        for number in &EXPORTED_SYMBOLS {
            assert!(mapped.contains(number), "{number} is never mapped");
        }

        OcdTemplate::load(None)
            .unwrap()
            .check_symbols(&EXPORTED_SYMBOLS)
            .unwrap();
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
            vec![
                OcdPoint {
                    x: -5_000,
                    y: 0,
                    y_flags: 0
                },
                OcdPoint {
                    x: 0,
                    y: 0,
                    y_flags: 0
                },
                OcdPoint {
                    x: 0,
                    y: 1_000,
                    y_flags: 0
                },
            ]
        );
    }

    #[test]
    fn writes_motorways_as_two_lines_and_skips_tunnels() {
        let line = || Polyline::new(ring(&[(100.0, 100.0), (200.0, 200.0)]));

        let objects = get_ocd_objects(
            &[
                MapFeature::Line(LineSymbol::DoubleTrackWideRoad502 { tunnel: false }, line()),
                MapFeature::Line(LineSymbol::WideRoad502 { tunnel: true }, line()),
            ],
            &EXTENT,
            &GEOREFERENCING,
        )
        .unwrap();

        let symbols: Vec<i32> = objects.iter().map(|object| object.symbol).collect();
        assert_eq!(symbols, vec![502002, 502003]);
    }

    #[test]
    fn splits_multipolygons_and_flags_holes() {
        let polygon = Polygon::with_rings(vec![
            PolygonRing::Outer(ring(&[
                (100.0, 100.0),
                (100.0, 400.0),
                (400.0, 400.0),
                (400.0, 100.0),
                (100.0, 100.0),
            ])),
            PolygonRing::Inner(ring(&[
                (200.0, 200.0),
                (300.0, 200.0),
                (300.0, 300.0),
                (200.0, 300.0),
                (200.0, 200.0),
            ])),
            PolygonRing::Outer(ring(&[
                (600.0, 600.0),
                (600.0, 700.0),
                (700.0, 700.0),
                (700.0, 600.0),
                (600.0, 600.0),
            ])),
            // Outside the tile
            PolygonRing::Outer(ring(&[
                (2000.0, 2000.0),
                (2000.0, 2100.0),
                (2100.0, 2100.0),
                (2100.0, 2000.0),
                (2000.0, 2000.0),
            ])),
        ]);

        let objects = get_ocd_objects(
            &[MapFeature::Area(AreaSymbol::Building521, polygon)],
            &EXTENT,
            &GEOREFERENCING,
        )
        .unwrap();

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].symbol, 521002);
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
}
