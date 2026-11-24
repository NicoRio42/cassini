//! Ring assembly and nesting: `OGRBuildPolygonFromEdges`
//! (`ogr/ograssemblepolygon.cpp`) and `OGRGeometryFactory::organizePolygons`
//! (`ogr/ogrgeometryfactory.cpp`). Rings are in longitude and latitude, as
//! OGR assembles them before projecting.

use super::{LonLat, PolygonRings, Ring};

/// `OGRBuildPolygonFromEdges` with no tolerance and no auto-closing: joins
/// the edges end to end into rings, which may stay open. Every edge has at
/// least two points.
pub(super) fn rings_from_edges(mut edges: Vec<Ring>) -> Vec<Ring> {
    let mut rings = vec![];
    while !edges.is_empty() {
        let mut ring = vec![];
        add_edge_to_ring(&mut ring, &edges.remove(0), false);
        while ring.first() != ring.last() {
            let end = ring[ring.len() - 1];
            // The first edge touching the end of the ring, reversed if its
            // last point does.
            let next = edges.iter().enumerate().find_map(|(index, edge)| {
                if edge[edge.len() - 1] == end {
                    Some((index, true))
                } else if edge[0] == end {
                    Some((index, false))
                } else {
                    None
                }
            });
            let Some((index, reverse)) = next else {
                break;
            };
            add_edge_to_ring(&mut ring, &edges.remove(index), reverse);
        }
        rings.push(ring);
    }
    rings
}

fn add_edge_to_ring(ring: &mut Ring, edge: &[LonLat], reverse: bool) {
    let points: Box<dyn Iterator<Item = &LonLat>> = if reverse {
        Box::new(edge.iter().rev())
    } else {
        Box::new(edge.iter())
    };
    let mut points = points.peekable();
    if ring.last().is_some() && ring.last() == points.peek().copied() {
        points.next();
    }
    ring.extend(points);
}

/// `OGRGeometryFactory::organizePolygons` with the default method: nests
/// the rings into polygons with holes. A ring inside a polygon is a hole of
/// it, a ring inside a hole is a new polygon. Polygons and holes keep the
/// order of `rings`.
pub(super) fn organize_polygons(rings: Vec<Ring>) -> Vec<PolygonRings> {
    if rings.len() <= 1 {
        return rings
            .into_iter()
            .map(|exterior| PolygonRings {
                exterior,
                interiors: vec![],
            })
            .collect();
    }

    let infos: Vec<RingInfo> = rings.iter().map(|ring| RingInfo::of(ring)).collect();
    // A degenerate ring makes OGR give up on nesting.
    let mixed_up = rings.iter().any(|ring| ring.len() < 4);
    let mut is_top_level = vec![mixed_up; rings.len()];
    let mut enclosing: Vec<Option<usize>> = vec![None; rings.len()];

    if !mixed_up {
        let mut by_area: Vec<usize> = (0..rings.len()).collect();
        by_area.sort_by(|&a, &b| infos[b].area.total_cmp(&infos[a].area));
        is_top_level[by_area[0]] = true;

        for &this in &by_area[1..] {
            // Candidate enclosing rings by increasing area.
            let mut candidates: Vec<usize> = (0..rings.len())
                .filter(|&other| {
                    other != this && infos[other].envelope.intersects(&infos[this].envelope)
                })
                .collect();
            candidates.sort_by(|&a, &b| infos[a].area.total_cmp(&infos[b].area));

            let container = candidates.into_iter().find(|&other| {
                infos[other].area >= infos[this].area
                    && infos[other].envelope.contains(&infos[this].envelope)
                    && is_ring_inside(&rings[this], &rings[other])
            });
            match container {
                Some(other) if is_top_level[other] => enclosing[this] = Some(other),
                _ => is_top_level[this] = true,
            }
        }
    }

    let mut polygons: Vec<Option<PolygonRings>> = vec![None; rings.len()];
    let mut holes: Vec<(usize, Ring)> = vec![];
    for (index, ring) in rings.into_iter().enumerate() {
        match enclosing[index] {
            Some(outer) if !is_top_level[index] => holes.push((outer, ring)),
            _ => {
                polygons[index] = Some(PolygonRings {
                    exterior: ring,
                    interiors: vec![],
                })
            }
        }
    }
    for (outer, ring) in holes {
        polygons[outer]
            .as_mut()
            .expect("holes are in top-level polygons")
            .interiors
            .push(ring);
    }
    polygons.into_iter().flatten().collect()
}

struct RingInfo {
    envelope: Envelope,
    area: f64,
}

impl RingInfo {
    fn of(ring: &[LonLat]) -> Self {
        // OGR only measures rings of at least four points.
        let area = if ring.len() >= 4 {
            linear_area(ring)
        } else {
            0.0
        };
        Self {
            envelope: Envelope::of(ring),
            area,
        }
    }
}

/// `OGRSimpleCurve::get_LinearArea`.
fn linear_area(ring: &[LonLat]) -> f64 {
    let n = ring.len();
    let mut sum = ring[0].x() * (ring[1].y() - ring[n - 1].y());
    for i in 1..n - 1 {
        sum += ring[i].x() * (ring[i + 1].y() - ring[i - 1].y());
    }
    sum += ring[n - 1].x() * (ring[0].y() - ring[n - 2].y());
    0.5 * sum.abs()
}

#[derive(Debug, Clone, Copy)]
struct Envelope {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Envelope {
    fn of(ring: &[LonLat]) -> Self {
        ring.iter().fold(
            Self {
                min_x: f64::INFINITY,
                min_y: f64::INFINITY,
                max_x: f64::NEG_INFINITY,
                max_y: f64::NEG_INFINITY,
            },
            |envelope, point| Self {
                min_x: envelope.min_x.min(point.x()),
                min_y: envelope.min_y.min(point.y()),
                max_x: envelope.max_x.max(point.x()),
                max_y: envelope.max_y.max(point.y()),
            },
        )
    }

    fn intersects(&self, other: &Self) -> bool {
        self.min_x <= other.max_x
            && other.min_x <= self.max_x
            && self.min_y <= other.max_y
            && other.min_y <= self.max_y
    }

    fn contains(&self, other: &Self) -> bool {
        self.min_x <= other.min_x
            && self.min_y <= other.min_y
            && other.max_x <= self.max_x
            && other.max_y <= self.max_y
    }
}

/// The fast inclusion test of `organizePolygons`: the first point of `this`
/// is strictly inside `other`. If it is on the boundary of `other`, the
/// first next point that is not decides, then the first segment middle.
fn is_ring_inside(this: &[LonLat], other: &[LonLat]) -> bool {
    let start = (this[0].x(), this[0].y());
    if !is_point_on_ring_boundary(start, other) {
        return is_point_in_ring(start, other);
    }

    let mut previous = start;
    let mut all_on_boundary = true;
    for point in &this[1..] {
        let point = (point.x(), point.y());
        if point == previous {
            continue;
        }
        if !is_point_on_ring_boundary(point, other) {
            if is_point_in_ring(point, other) {
                return true;
            }
            all_on_boundary = false;
            break;
        }
        previous = point;
    }
    if !all_on_boundary || this.len() <= 2 {
        return false;
    }

    let mut previous = start;
    for point in &this[1..] {
        let point = (point.x(), point.y());
        if point == previous {
            continue;
        }
        let middle = ((point.0 + previous.0) / 2.0, (point.1 + previous.1) / 2.0);
        if !is_point_on_ring_boundary(middle, other) {
            return is_point_in_ring(middle, other);
        }
        previous = point;
    }
    false
}

/// `OGRLinearRing::isPointInRing`: strict inclusion, by ray crossing.
fn is_point_in_ring((test_x, test_y): (f64, f64), ring: &[LonLat]) -> bool {
    if ring.len() < 4 {
        return false;
    }
    let mut crossings = 0;
    let mut previous = (ring[0].x() - test_x, ring[0].y() - test_y);
    for point in &ring[1..] {
        let (x1, y1) = (point.x() - test_x, point.y() - test_y);
        let (x2, y2) = previous;
        if (y1 > 0.0 && y2 <= 0.0) || (y2 > 0.0 && y1 <= 0.0) {
            let intersection = (x1 * y2 - x2 * y1) / (y2 - y1);
            if 0.0 < intersection {
                crossings += 1;
            }
        }
        previous = (x1, y1);
    }
    crossings % 2 == 1
}

/// `OGRLinearRing::isPointOnRingBoundary`: the point is exactly on a
/// segment.
fn is_point_on_ring_boundary((test_x, test_y): (f64, f64), ring: &[LonLat]) -> bool {
    if ring.len() < 4 {
        return false;
    }
    let mut previous = (test_x - ring[0].x(), test_y - ring[0].y());
    for (index, point) in ring.iter().enumerate().skip(1) {
        let (dx1, dy1) = (test_x - point.x(), test_y - point.y());
        let (dx2, dy2) = previous;
        if dx1 * dy2 - dx2 * dy1 == 0.0 && !(dx1 == dx2 && dy1 == dy2) {
            let dx_segment = point.x() - ring[index - 1].x();
            let dy_segment = point.y() - ring[index - 1].y();
            let dot_product = dx2 * dx_segment + dy2 * dy_segment;
            if dot_product >= 0.0
                && dot_product <= dx_segment * dx_segment + dy_segment * dy_segment
            {
                return true;
            }
        }
        previous = (dx1, dy1);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osm_import::test_support::ring;

    fn square(min: f64, max: f64) -> Ring {
        ring(&[(min, min), (min, max), (max, max), (max, min), (min, min)])
    }

    fn polygon(exterior: &Ring, interiors: &[&Ring]) -> PolygonRings {
        PolygonRings {
            exterior: exterior.clone(),
            interiors: interiors.iter().map(|&ring| ring.clone()).collect(),
        }
    }

    #[test]
    fn rings_inside_holes_are_new_polygons() {
        let outer = square(0.0, 10.0);
        let hole = square(2.0, 8.0);
        let island = square(4.0, 6.0);
        let polygons = organize_polygons(vec![island.clone(), outer.clone(), hole.clone()]);

        assert_eq!(polygons, [polygon(&island, &[]), polygon(&outer, &[&hole])]);
    }

    #[test]
    fn holes_may_touch_their_polygon() {
        let outer = square(0.0, 10.0);
        // A corner on a corner of the outer ring, an edge along its edge, and
        // all corners on it.
        let corner = ring(&[(0.0, 0.0), (5.0, 2.0), (2.0, 5.0), (0.0, 0.0)]);
        let edge = ring(&[(0.0, 6.0), (0.0, 9.0), (3.0, 8.0), (0.0, 6.0)]);
        let diamond = ring(&[(5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (0.0, 5.0), (5.0, 0.0)]);
        for hole in [&corner, &edge, &diamond] {
            assert!(is_ring_inside(hole, &outer));
        }
        assert_eq!(
            organize_polygons(vec![outer.clone(), corner.clone(), edge.clone()]),
            [polygon(&outer, &[&corner, &edge])]
        );

        // A ring on the boundary only, the same ring or one outside along an
        // edge, is not inside.
        assert!(!is_ring_inside(&outer, &outer));
        let outside = ring(&[(10.0, 0.0), (10.0, 10.0), (12.0, 5.0), (10.0, 0.0)]);
        assert!(!is_ring_inside(&outside, &outer));
    }

    #[test]
    fn a_degenerate_ring_stops_the_nesting() {
        let outer = square(0.0, 10.0);
        let hole = square(2.0, 8.0);
        let degenerate = ring(&[(20.0, 20.0), (21.0, 21.0), (20.0, 20.0)]);
        assert_eq!(
            organize_polygons(vec![outer.clone(), hole.clone(), degenerate.clone()]),
            [
                polygon(&outer, &[]),
                polygon(&hole, &[]),
                polygon(&degenerate, &[]),
            ]
        );
        assert!(organize_polygons(vec![]).is_empty());
    }

    #[test]
    fn edges_are_joined_end_to_end() {
        let rings = rings_from_edges(vec![
            ring(&[(0.0, 0.0), (0.0, 1.0)]),
            ring(&[(1.0, 0.0), (0.0, 0.0)]),
            ring(&[(1.0, 1.0), (1.0, 0.0)]),
            ring(&[(0.0, 1.0), (1.0, 1.0)]),
            ring(&[(5.0, 5.0), (6.0, 6.0)]),
        ]);
        assert_eq!(
            rings,
            [
                ring(&[(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)]),
                ring(&[(5.0, 5.0), (6.0, 6.0)]),
            ]
        );
    }
}
