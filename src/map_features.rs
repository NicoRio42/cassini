use shapefile::{Point, Polygon, PolygonRing, Polyline};

/// A classified map feature, in projected coordinates (EPSG:2154 metres).
/// The PNG renderer draws these and the OCAD exporter serializes them.
#[derive(Debug, PartialEq)]
pub enum MapFeature {
    Point(PointSymbol, Point),
    Line(LineSymbol, Polyline),
    Area(AreaSymbol, Polygon),
}

impl MapFeature {
    /// Whether the feature's bounding box overlaps `extent`.
    pub fn overlaps(&self, extent: &Extent) -> bool {
        let bbox = match self {
            MapFeature::Point(_, point) => return extent.overlaps_bbox(point, point),
            MapFeature::Line(_, line) => line.bbox(),
            MapFeature::Area(_, polygon) => polygon.bbox(),
        };
        extent.overlaps_bbox(&bbox.min, &bbox.max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointSymbol {
    // Cliffs are only exported to OCAD, the PNG renderer draws the cliffs
    // raster instead
    LargeBoulder205,
}

impl PointSymbol {
    /// Every point symbol. Add new variants here too, the OCAD export checks
    /// its template against the symbols of this list.
    pub const ALL: [PointSymbol; 1] = [PointSymbol::LargeBoulder205];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSymbol {
    Contour101,
    IndexContour102,
    FormLine103,
    UncrossableBodyOfWaterBankLine301_4,
    CrossableWatercourse304,
    MinorSeasonalWaterChannel306,
    DoubleTrackWideRoad502 { tunnel: bool },
    XxlWideRoad502 { tunnel: bool },
    XlWideRoad502 { tunnel: bool },
    WideRoad502 { tunnel: bool },
    Road503 { tunnel: bool },
    Footpath505 { tunnel: bool },
    Railway509,
    PowerLineCablewayOrSkilift510,
    // Cliffs are only exported to OCAD
    Cliff202,
}

impl LineSymbol {
    /// Every line symbol. Add new variants here too, the OCAD export checks
    /// its template against the symbols of this list.
    pub const ALL: [LineSymbol; 21] = [
        LineSymbol::Contour101,
        LineSymbol::IndexContour102,
        LineSymbol::FormLine103,
        LineSymbol::UncrossableBodyOfWaterBankLine301_4,
        LineSymbol::CrossableWatercourse304,
        LineSymbol::MinorSeasonalWaterChannel306,
        LineSymbol::DoubleTrackWideRoad502 { tunnel: false },
        LineSymbol::DoubleTrackWideRoad502 { tunnel: true },
        LineSymbol::XxlWideRoad502 { tunnel: false },
        LineSymbol::XxlWideRoad502 { tunnel: true },
        LineSymbol::XlWideRoad502 { tunnel: false },
        LineSymbol::XlWideRoad502 { tunnel: true },
        LineSymbol::WideRoad502 { tunnel: false },
        LineSymbol::WideRoad502 { tunnel: true },
        LineSymbol::Road503 { tunnel: false },
        LineSymbol::Road503 { tunnel: true },
        LineSymbol::Footpath505 { tunnel: false },
        LineSymbol::Footpath505 { tunnel: true },
        LineSymbol::Railway509,
        LineSymbol::PowerLineCablewayOrSkilift510,
        LineSymbol::Cliff202,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaSymbol {
    UncrossableBodyOfWater301,
    UncrossableBodyOfWaterArea301_1,
    Marsh308,
    AreaThatShallNotBeEntered520,
    Building521,
    // Vegetation areas are only exported to OCAD, the PNG renderer draws
    // the vegetation rasters instead
    RoughOpenLand403,
    VegetationSlowRunning406,
    VegetationWalk408,
    VegetationWalkGoodVisibility409,
    VegetationFight410,
    // Cliffs are only exported to OCAD
    GiganticBoulderOrRockPillar206,
}

impl AreaSymbol {
    /// Every area symbol. Add new variants here too, the OCAD export checks
    /// its template against the symbols of this list.
    pub const ALL: [AreaSymbol; 11] = [
        AreaSymbol::UncrossableBodyOfWater301,
        AreaSymbol::UncrossableBodyOfWaterArea301_1,
        AreaSymbol::Marsh308,
        AreaSymbol::AreaThatShallNotBeEntered520,
        AreaSymbol::Building521,
        AreaSymbol::RoughOpenLand403,
        AreaSymbol::VegetationSlowRunning406,
        AreaSymbol::VegetationWalk408,
        AreaSymbol::VegetationWalkGoodVisibility409,
        AreaSymbol::VegetationFight410,
        AreaSymbol::GiganticBoulderOrRockPillar206,
    ];
}

/// An axis-aligned rectangle in projected coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Extent {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Extent {
    pub fn expanded(&self, margin: f64) -> Extent {
        Extent {
            min_x: self.min_x - margin,
            min_y: self.min_y - margin,
            max_x: self.max_x + margin,
            max_y: self.max_y + margin,
        }
    }

    /// Whether `point` is in the extent, its maximum edges excluded, so that
    /// a point on the edge between two tiles is in one of them only.
    pub fn contains_half_open(&self, point: &Point) -> bool {
        point.x >= self.min_x
            && point.x < self.max_x
            && point.y >= self.min_y
            && point.y < self.max_y
    }

    /// Whether the bounding box from `min` to `max` overlaps the extent.
    pub fn overlaps_bbox(&self, min: &Point, max: &Point) -> bool {
        min.x <= self.max_x && max.x >= self.min_x && min.y <= self.max_y && max.y >= self.min_y
    }
}

/// An outer ring and the holes it contains.
pub struct PolygonArea<'a> {
    pub outer: &'a [Point],
    pub holes: Vec<&'a [Point]>,
}

/// Groups the rings of a polygon into areas. Shapefiles do not guarantee that
/// holes follow their outer ring, so each hole goes to the smallest outer ring
/// that contains it, or to the outer ring before it when none does.
pub fn get_polygon_areas(polygon: &Polygon) -> Vec<PolygonArea<'_>> {
    let mut areas: Vec<PolygonArea> = vec![];
    let mut holes: Vec<(&[Point], Option<usize>)> = vec![];

    for ring in polygon.rings() {
        match ring {
            PolygonRing::Outer(points) => areas.push(PolygonArea {
                outer: points,
                holes: vec![],
            }),
            PolygonRing::Inner(points) => holes.push((points, areas.len().checked_sub(1))),
        }
    }

    for (hole, preceding_area) in holes {
        let containing_area = areas
            .iter()
            .enumerate()
            .filter(|(_, area)| hole.iter().any(|point| is_point_in_ring(point, area.outer)))
            .min_by(|(_, a), (_, b)| ring_area(a.outer).total_cmp(&ring_area(b.outer)))
            .map(|(index, _)| index);

        if let Some(index) = containing_area.or(preceding_area) {
            areas[index].holes.push(hole);
        }
    }

    areas
}

/// Even-odd ray casting test.
pub fn is_point_in_ring(point: &Point, ring: &[Point]) -> bool {
    let mut inside = false;

    for segment in ring.windows(2) {
        let (a, b) = (segment[0], segment[1]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
        {
            inside = !inside;
        }
    }

    inside
}

/// Absolute shoelace area of a closed ring.
fn ring_area(ring: &[Point]) -> f64 {
    ring.windows(2)
        .map(|segment| segment[0].x * segment[1].y - segment[1].x * segment[0].y)
        .sum::<f64>()
        .abs()
        / 2.0
}

/// Clips a polyline part to the extent. A part that leaves and re-enters the
/// extent is split into several parts.
pub fn clip_line_part(points: &[Point], extent: &Extent) -> Vec<Vec<Point>> {
    let mut parts: Vec<Vec<Point>> = vec![];
    let mut current: Vec<Point> = vec![];

    for segment in points.windows(2) {
        let Some((start, end)) = clip_segment(segment[0], segment[1], extent) else {
            continue;
        };

        if current.last() != Some(&start) {
            if current.len() >= 2 {
                parts.push(current);
            }
            current = vec![start];
        }

        current.push(end);
    }

    if current.len() >= 2 {
        parts.push(current);
    }

    parts
}

/// Liang–Barsky segment clipping.
fn clip_segment(start: Point, end: Point, extent: &Extent) -> Option<(Point, Point)> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;

    for (p, q) in [
        (-dx, start.x - extent.min_x),
        (dx, extent.max_x - start.x),
        (-dy, start.y - extent.min_y),
        (dy, extent.max_y - start.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
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

    if t0 > t1 {
        return None;
    }

    let clipped_start = if t0 == 0.0 {
        start
    } else {
        Point::new(start.x + t0 * dx, start.y + t0 * dy)
    };
    let clipped_end = if t1 == 1.0 {
        end
    } else {
        Point::new(start.x + t1 * dx, start.y + t1 * dy)
    };

    Some((clipped_start, clipped_end))
}

/// Clips a closed ring to the extent with the Sutherland–Hodgman algorithm.
/// Returns a closed ring, or `None` when less than a triangle remains.
pub fn clip_ring(ring: &[Point], extent: &Extent) -> Option<Vec<Point>> {
    let mut points: Vec<Point> = ring.to_vec();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }

    for edge in [Edge::MinX, Edge::MaxX, Edge::MinY, Edge::MaxY] {
        if points.is_empty() {
            return None;
        }

        let input = std::mem::take(&mut points);
        let mut previous = input[input.len() - 1];

        for current in input {
            let current_inside = edge.is_inside(&current, extent);
            let previous_inside = edge.is_inside(&previous, extent);

            if current_inside {
                if !previous_inside {
                    points.push(edge.intersect(&previous, &current, extent));
                }
                points.push(current);
            } else if previous_inside {
                points.push(edge.intersect(&previous, &current, extent));
            }

            previous = current;
        }
    }

    points.dedup();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }

    if points.len() < 3 {
        return None;
    }

    points.push(points[0]);
    Some(points)
}

#[derive(Clone, Copy)]
enum Edge {
    MinX,
    MaxX,
    MinY,
    MaxY,
}

impl Edge {
    fn is_inside(self, point: &Point, extent: &Extent) -> bool {
        match self {
            Edge::MinX => point.x >= extent.min_x,
            Edge::MaxX => point.x <= extent.max_x,
            Edge::MinY => point.y >= extent.min_y,
            Edge::MaxY => point.y <= extent.max_y,
        }
    }

    /// Intersection of segment `a`-`b` with the edge. Only called when the
    /// segment crosses it, so the denominator is never zero.
    fn intersect(self, a: &Point, b: &Point, extent: &Extent) -> Point {
        match self {
            Edge::MinX | Edge::MaxX => {
                let x = if matches!(self, Edge::MinX) {
                    extent.min_x
                } else {
                    extent.max_x
                };
                let t = (x - a.x) / (b.x - a.x);
                Point::new(x, a.y + t * (b.y - a.y))
            }
            Edge::MinY | Edge::MaxY => {
                let y = if matches!(self, Edge::MinY) {
                    extent.min_y
                } else {
                    extent.max_y
                };
                let t = (y - a.y) / (b.y - a.y);
                Point::new(a.x + t * (b.x - a.x), y)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clip_line_part, clip_ring, get_polygon_areas, Extent};
    use shapefile::{Point, Polygon, PolygonRing};

    const EXTENT: Extent = Extent {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 10.0,
        max_y: 10.0,
    };

    fn points(coordinates: &[(f64, f64)]) -> Vec<Point> {
        coordinates
            .iter()
            .map(|(x, y)| Point::new(*x, *y))
            .collect()
    }

    #[test]
    fn keeps_line_inside_extent() {
        let line = points(&[(1.0, 1.0), (5.0, 5.0), (9.0, 1.0)]);

        assert_eq!(clip_line_part(&line, &EXTENT), vec![line]);
    }

    #[test]
    fn splits_line_that_leaves_and_reenters_extent() {
        let line = points(&[
            (-5.0, 5.0),
            (5.0, 5.0),
            (5.0, 15.0),
            (8.0, 15.0),
            (8.0, 5.0),
        ]);

        assert_eq!(
            clip_line_part(&line, &EXTENT),
            vec![
                points(&[(0.0, 5.0), (5.0, 5.0), (5.0, 10.0)]),
                points(&[(8.0, 10.0), (8.0, 5.0)]),
            ]
        );
    }

    #[test]
    fn drops_line_outside_extent() {
        let line = points(&[(-5.0, -5.0), (-1.0, 20.0)]);

        assert!(clip_line_part(&line, &EXTENT).is_empty());
    }

    #[test]
    fn clips_ring_to_extent() {
        let ring = points(&[
            (-5.0, -5.0),
            (5.0, -5.0),
            (5.0, 5.0),
            (-5.0, 5.0),
            (-5.0, -5.0),
        ]);

        assert_eq!(
            clip_ring(&ring, &EXTENT),
            Some(points(&[
                (0.0, 0.0),
                (5.0, 0.0),
                (5.0, 5.0),
                (0.0, 5.0),
                (0.0, 0.0)
            ]))
        );
    }

    fn square(min: f64, max: f64) -> Vec<Point> {
        points(&[(min, min), (min, max), (max, max), (max, min), (min, min)])
    }

    #[test]
    fn assigns_holes_to_the_outer_ring_containing_them() {
        // The hole of the first area comes after the second outer ring
        let polygon = Polygon::with_rings(vec![
            PolygonRing::Outer(square(0.0, 10.0)),
            PolygonRing::Outer(square(20.0, 30.0)),
            PolygonRing::Inner(square(2.0, 4.0)),
            PolygonRing::Inner(square(22.0, 24.0)),
        ]);

        let areas = get_polygon_areas(&polygon);

        assert_eq!(areas.len(), 2);
        assert_eq!(areas[0].holes.len(), 1);
        assert_eq!(areas[0].holes[0][0], Point::new(2.0, 2.0));
        assert_eq!(areas[1].holes.len(), 1);
        assert_eq!(areas[1].holes[0][0], Point::new(22.0, 22.0));
    }

    #[test]
    fn assigns_holes_to_the_smallest_containing_outer_ring() {
        // An island in a lake in an island
        let polygon = Polygon::with_rings(vec![
            PolygonRing::Outer(square(0.0, 100.0)),
            PolygonRing::Inner(square(10.0, 90.0)),
            PolygonRing::Outer(square(20.0, 80.0)),
            PolygonRing::Inner(square(30.0, 70.0)),
        ]);

        let areas = get_polygon_areas(&polygon);

        assert_eq!(areas[0].holes.len(), 1);
        assert_eq!(areas[0].holes[0][0], Point::new(10.0, 10.0));
        assert_eq!(areas[1].holes.len(), 1);
        assert_eq!(areas[1].holes[0][0], Point::new(30.0, 30.0));
    }

    #[test]
    fn drops_ring_outside_extent() {
        let ring = points(&[(20.0, 20.0), (30.0, 20.0), (30.0, 30.0), (20.0, 20.0)]);

        assert_eq!(clip_ring(&ring, &EXTENT), None);
    }
}
