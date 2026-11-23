use shapefile::{Point, Polygon, Polyline};

/// A classified map feature, in projected coordinates (EPSG:2154 metres).
/// The PNG renderer draws these and the OCAD exporter serializes them.
pub enum MapFeature {
    Line(LineSymbol, Polyline),
    Area(AreaSymbol, Polygon),
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaSymbol {
    UncrossableBodyOfWater301,
    UncrossableBodyOfWaterArea301_1,
    Marsh308,
    AreaThatShallNotBeEntered520,
    Building521,
}

/// An axis-aligned rectangle in projected coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Extent {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
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
    use super::{clip_line_part, clip_ring, Extent};
    use shapefile::Point;

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

    #[test]
    fn drops_ring_outside_extent() {
        let ring = points(&[(20.0, 20.0), (30.0, 20.0), (30.0, 30.0), (20.0, 20.0)]);

        assert_eq!(clip_ring(&ring, &EXTENT), None);
    }
}
