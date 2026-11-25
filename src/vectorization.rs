//! Shared steps of the vectorization of rasters for the OCAD export: tracing
//! the areas where a field is positive, simplifying lines and rings alike in
//! neighbouring tiles, and clipping the areas to the tile.

use crate::{
    error::{Result, ResultContext},
    map_features::{AreaSymbol, Extent, MapFeature},
};
use contour::ContourBuilder;
use i_overlay::{
    core::{fill_rule::FillRule, overlay_rule::OverlayRule},
    float::single::SingleFloatOverlay,
};
use shapefile::{Point, Polygon, PolygonRing};

/// Tracing rounds the field values closer to zero than this to it, so that
/// the iso-line agrees with strict thresholds. Generalisation gives it to the
/// cells it moves into (positive) or out of (negative) an area.
pub const NUDGE: f32 = 1e-3;
/// Polygons and holes left smaller than this, in m², by simplification and
/// boolean operations are slivers.
pub const SLIVER_AREA: f64 = 1.0;

/// A shape is an outer ring followed by its holes, without closing points.
pub type Shapes = Vec<Vec<Vec<[f64; 2]>>>;

/// Collects area features clipped to the tile.
pub struct FeatureSink {
    features: Vec<MapFeature>,
    /// The vectorized window. The shapes reaching its border may continue
    /// beyond it.
    window: Extent,
    /// The tile the shapes are clipped to.
    tile: Extent,
}

impl FeatureSink {
    pub fn new(window: Extent, tile: Extent) -> Self {
        Self {
            features: vec![],
            window,
            tile,
        }
    }

    /// Pushes one area feature per shape clipped to the tile, dropping the
    /// shapes smaller than `min_area` m² away from the window border, and the
    /// slivers. The shapes are measured whole, before they are clipped, so
    /// that neighbouring tiles keep or drop the shapes crossing their common
    /// edge alike.
    pub fn push(&mut self, symbol: AreaSymbol, shapes: Shapes, min_area: f64) {
        let to_points =
            |ring: Vec<[f64; 2]>| ring.into_iter().map(|[x, y]| Point::new(x, y)).collect();
        let tile_rectangle = rectangle(&self.tile);

        for shape in shapes {
            // The outer ring comes first
            let mut rings = shape.into_iter();
            let Some(outer) = rings.next() else {
                continue;
            };
            let area = ring_area(&outer);
            if area < SLIVER_AREA || (area < min_area && !self.reaches_window_border(&outer)) {
                continue;
            }
            let shape: Vec<Vec<[f64; 2]>> = std::iter::once(outer)
                .chain(rings.filter(|hole| ring_area(hole) >= SLIVER_AREA))
                .collect();

            for clipped in shape.overlay(&tile_rectangle, OverlayRule::Intersect, FillRule::EvenOdd)
            {
                let mut rings = clipped.into_iter();
                let Some(outer) = rings.next() else {
                    continue;
                };
                let polygon = Polygon::with_rings(
                    std::iter::once(PolygonRing::Outer(to_points(outer)))
                        .chain(rings.map(|hole| PolygonRing::Inner(to_points(hole))))
                        .collect(),
                );
                self.features.push(MapFeature::Area(symbol, polygon));
            }
        }
    }

    pub fn into_features(self) -> Vec<MapFeature> {
        self.features
    }

    fn reaches_window_border(&self, ring: &[[f64; 2]]) -> bool {
        // The iso-lines close along the grid border, within a cell of it
        const TOLERANCE: f64 = 1.0;
        ring.iter().any(|[x, y]| {
            x - self.window.min_x < TOLERANCE
                || self.window.max_x - x < TOLERANCE
                || y - self.window.min_y < TOLERANCE
                || self.window.max_y - y < TOLERANCE
        })
    }
}

/// Absolute shoelace area of a ring without closing point.
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let doubled: f64 = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum();
    doubled.abs() / 2.0
}

pub struct Tracer {
    builder: ContourBuilder,
    /// The lines simplification keeps the crossings of.
    pinned_lines: PinnedLines,
    /// The vectorized window, holding every traced ring.
    window: Extent,
    simplify_tolerance: f64,
}

impl Tracer {
    /// A tracer of the fields `builder` places over `window`, simplifying the
    /// iso-lines with `simplify_tolerance` metres (0 to keep every vertex)
    /// alike on either side of the edges of `tile`.
    pub fn new(
        builder: ContourBuilder,
        window: Extent,
        tile: &Extent,
        simplify_tolerance: f64,
    ) -> Self {
        Self {
            builder,
            pinned_lines: PinnedLines::around_tile_edges(tile, &window, simplify_tolerance),
            window,
            simplify_tolerance,
        }
    }

    /// Returns the area where `field` is positive over the window. `what`
    /// names the traced areas in errors.
    ///
    /// Vertices are added to the iso-lines where they cross the pinned lines,
    /// and simplification keeps them. A neighbouring tile pins the same lines
    /// along the common edge, so that the areas clipped to either tile cross
    /// that edge at the same points, and batch mode can join them.
    pub fn trace(&self, field: &[f32], what: &str) -> Result<Shapes> {
        let values: Vec<f64> = field
            .iter()
            .map(|value| {
                if *value > 0.0 {
                    value.max(NUDGE) as f64
                } else {
                    value.min(-NUDGE) as f64
                }
            })
            .collect();

        let lines = self
            .builder
            .lines(&values, &[0.0])
            .context(format!("could not trace {what}"))?;

        let rings: Vec<Vec<[f64; 2]>> = lines
            .iter()
            .flat_map(|line| line.geometry().iter())
            .map(|line_string| {
                let mut ring: Vec<[f64; 2]> = line_string
                    .coords()
                    .map(|coord| [coord.x, coord.y])
                    .collect();
                if ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                if self.simplify_tolerance <= 0.0 {
                    return ring;
                }
                simplify_ring(
                    &self.pinned_lines.insert_ring_crossings(&ring),
                    self.simplify_tolerance,
                    |point| self.pinned_lines.contains(point),
                )
            })
            .filter(|ring| ring.len() >= 3)
            .collect();

        // Iso-lines of a threshold never cross, so the even-odd rule turns
        // nested rings into holes and islands in holes. Each ring is
        // simplified on its own, so neighbouring rings may cross by up to the
        // tolerance. The pieces this leaves after the boolean operations are
        // dropped as slivers.
        Ok(rings.overlay(
            &rectangle(&self.window),
            OverlayRule::Intersect,
            FillRule::EvenOdd,
        ))
    }
}

/// The corners of `extent`.
fn rectangle(extent: &Extent) -> Vec<[f64; 2]> {
    vec![
        [extent.min_x, extent.min_y],
        [extent.max_x, extent.min_y],
        [extent.max_x, extent.max_y],
        [extent.min_x, extent.max_y],
    ]
}

/// Axis-aligned lines, as their axis (0 for x = the coordinate, 1 for
/// y = the coordinate) and coordinate.
pub struct PinnedLines(Vec<(usize, f64)>);

impl PinnedLines {
    /// The lines of the edges of `tile`, and lines on either side of each one,
    /// `simplify_tolerance` away from it, within half of the margin between
    /// `tile` and the vectorized `window`. A neighbouring tile has the same
    /// lines along their common edge. Simplification keeps the lines within
    /// the cells these lines form, so both tiles simplify the lines near that
    /// edge alike, whatever they do beyond.
    pub fn around_tile_edges(tile: &Extent, window: &Extent, simplify_tolerance: f64) -> Self {
        let margin = (tile.min_x - window.min_x)
            .min(tile.min_y - window.min_y)
            .min(window.max_x - tile.max_x)
            .min(window.max_y - tile.max_y);
        let band = simplify_tolerance.min(margin / 2.0);

        let mut lines = vec![];
        for (axis, edge) in [
            (0, tile.min_x),
            (0, tile.max_x),
            (1, tile.min_y),
            (1, tile.max_y),
        ] {
            lines.extend([(axis, edge - band), (axis, edge), (axis, edge + band)]);
        }
        Self(lines)
    }

    pub fn contains(&self, point: [f64; 2]) -> bool {
        // The boolean operations snap the coordinates to an integer grid
        const TOLERANCE: f64 = 1e-3;
        self.0
            .iter()
            .any(|(axis, line)| (point[*axis] - line).abs() < TOLERANCE)
    }

    /// Adds a vertex to `ring`, without closing point, wherever a segment
    /// crosses one of the lines.
    pub fn insert_ring_crossings(&self, ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut with_crossings = Vec::with_capacity(ring.len());
        for (index, start) in ring.iter().enumerate() {
            with_crossings.push(*start);
            with_crossings.extend(self.crossings(*start, ring[(index + 1) % ring.len()]));
        }
        with_crossings
    }

    /// Adds a vertex to the open `line` wherever a segment crosses one of the
    /// lines.
    pub fn insert_line_crossings(&self, line: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut with_crossings = Vec::with_capacity(line.len());
        for pair in line.windows(2) {
            with_crossings.push(pair[0]);
            with_crossings.extend(self.crossings(pair[0], pair[1]));
        }
        with_crossings.extend(line.last());
        with_crossings
    }

    /// The points where the segment from `start` to `end` crosses the lines,
    /// from `start`.
    fn crossings(&self, start: [f64; 2], end: [f64; 2]) -> Vec<[f64; 2]> {
        // From the lower endpoint, so that the neighbouring tile computes the
        // same point whichever way its ring or line runs
        let (a, b) = if start < end {
            (start, end)
        } else {
            (end, start)
        };
        let mut crossings: Vec<(f64, [f64; 2])> = vec![];
        for (axis, line) in &self.0 {
            if (a[*axis] - line) * (b[*axis] - line) < 0.0 {
                let t = (line - a[*axis]) / (b[*axis] - a[*axis]);
                let mut point = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                point[*axis] = *line;
                // By distance from `start`
                crossings.push((squared_distance(start, point), point));
            }
        }

        crossings.sort_by(|first, second| first.0.total_cmp(&second.0));
        crossings.into_iter().map(|(_, point)| point).collect()
    }
}

/// Douglas–Peucker simplification of a ring without closing point, keeping
/// the vertices for which `is_pinned` holds, and three vertices at least, so
/// that a ring thinner than the tolerance is not lost.
pub fn simplify_ring(
    ring: &[[f64; 2]],
    tolerance: f64,
    is_pinned: impl Fn([f64; 2]) -> bool,
) -> Vec<[f64; 2]> {
    if tolerance <= 0.0 || ring.len() < 4 {
        return ring.to_vec();
    }

    // Douglas–Peucker keeps the ends of a chain, so the ring is split in
    // chains at its pinned vertices. It starts at the first one, and is also
    // split at the vertex farthest from it when there are fewer than two.
    // Without any, it starts at its lowest vertex, so that a neighbouring tile
    // tracing the same ring from another vertex simplifies it alike.
    let first = (0..ring.len())
        .find(|index| is_pinned(ring[*index]))
        .unwrap_or_else(|| {
            (0..ring.len())
                .min_by(|a, b| {
                    let ([ax, ay], [bx, by]) = (ring[*a], ring[*b]);
                    ax.total_cmp(&bx).then(ay.total_cmp(&by))
                })
                .unwrap()
        });
    let closed: Vec<[f64; 2]> = ring[first..]
        .iter()
        .chain(&ring[..=first])
        .copied()
        .collect();
    let len = ring.len();

    let mut anchors: Vec<usize> = (0..len).filter(|index| is_pinned(closed[*index])).collect();
    if anchors.len() < 2 {
        let farthest = (1..len)
            .max_by(|a, b| {
                squared_distance(closed[0], closed[*a])
                    .total_cmp(&squared_distance(closed[0], closed[*b]))
            })
            .unwrap();
        anchors = vec![0, farthest];
    }
    anchors.push(len);

    let mut kept = douglas_peucker(&closed, &anchors, tolerance);

    // A ring thinner than the tolerance keeps the vertex farthest from the
    // chord between the two others
    if let [a, b] = (0..len).filter(|index| kept[*index]).collect::<Vec<_>>()[..] {
        let farthest_from_chord = (0..len).filter(|index| !kept[*index]).max_by(|x, y| {
            squared_segment_distance(closed[*x], closed[a], closed[b])
                .total_cmp(&squared_segment_distance(closed[*y], closed[a], closed[b]))
        });
        if let Some(index) = farthest_from_chord {
            kept[index] = true;
        }
    }

    // The closing point duplicates the first one
    closed[..len]
        .iter()
        .zip(&kept)
        .filter(|(_, kept)| **kept)
        .map(|(point, _)| *point)
        .collect()
}

/// Douglas–Peucker simplification of an open line, keeping its ends and the
/// vertices for which `is_pinned` holds.
pub fn simplify_line(
    line: &[[f64; 2]],
    tolerance: f64,
    is_pinned: impl Fn([f64; 2]) -> bool,
) -> Vec<[f64; 2]> {
    if tolerance <= 0.0 || line.len() < 3 {
        return line.to_vec();
    }

    let last = line.len() - 1;
    let anchors: Vec<usize> = (0..=last)
        .filter(|index| *index == 0 || *index == last || is_pinned(line[*index]))
        .collect();

    line.iter()
        .zip(douglas_peucker(line, &anchors, tolerance))
        .filter(|(_, kept)| *kept)
        .map(|(point, _)| *point)
        .collect()
}

/// Whether Douglas–Peucker keeps each of `points`, simplifying the chains
/// between consecutive `anchors`, which are kept, with `tolerance`.
fn douglas_peucker(points: &[[f64; 2]], anchors: &[usize], tolerance: f64) -> Vec<bool> {
    let mut kept = vec![false; points.len()];
    for anchor in anchors {
        kept[*anchor] = true;
    }

    let squared_tolerance = tolerance * tolerance;
    let mut chains: Vec<(usize, usize)> =
        anchors.windows(2).map(|pair| (pair[0], pair[1])).collect();
    while let Some((first, last)) = chains.pop() {
        let farthest_from_chord = (first + 1..last)
            .map(|index| {
                (
                    index,
                    squared_segment_distance(points[index], points[first], points[last]),
                )
            })
            .max_by(|(_, a), (_, b)| a.total_cmp(b));

        if let Some((index, squared_distance)) = farthest_from_chord {
            if squared_distance > squared_tolerance {
                kept[index] = true;
                chains.push((first, index));
                chains.push((index, last));
            }
        }
    }

    kept
}

fn squared_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn squared_segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let squared_length = dx * dx + dy * dy;
    if squared_length == 0.0 {
        return squared_distance(point, start);
    }

    let t = (((point[0] - start[0]) * dx + (point[1] - start[1]) * dy) / squared_length)
        .clamp(0.0, 1.0);
    squared_distance(point, [start[0] + t * dx, start[1] + t * dy])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Extent {
        Extent {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    const SIZE: usize = 20;

    /// A field over a `SIZE` x `SIZE` grid, positive in the cells where
    /// `inside` holds.
    fn field(inside: impl Fn(usize, usize) -> bool) -> Vec<f32> {
        (0..SIZE * SIZE)
            .map(|cell| {
                if inside(cell % SIZE, cell / SIZE) {
                    1.0
                } else {
                    -1.0
                }
            })
            .collect()
    }

    fn square(x: usize, y: usize, size: usize) -> impl Fn(usize, usize) -> bool {
        move |cell_x, cell_y| (x..x + size).contains(&cell_x) && (y..y + size).contains(&cell_y)
    }

    #[test]
    fn simplifies_rings_within_tolerance() {
        let mut ring: Vec<[f64; 2]> = (0..=10).map(|x| [x as f64, 0.0]).collect();
        ring.push([10.0, 10.0]);
        ring.push([5.0, 10.2]);
        ring.push([0.0, 10.0]);

        let simplified = simplify_ring(&ring, 0.5, |_| false);

        assert_eq!(
            simplified,
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]
        );
        assert_eq!(simplify_ring(&ring, 0.0, |_| false), ring);
        assert_eq!(simplify_ring(&ring, 0.1, |_| false).len(), 5);
    }

    #[test]
    fn keeps_pinned_vertices_when_simplifying() {
        let mut ring: Vec<[f64; 2]> = (0..=10).map(|x| [x as f64, 0.0]).collect();
        ring.push([10.0, 10.0]);
        ring.push([0.0, 10.0]);

        let simplified = simplify_ring(&ring, 0.5, |point| point == [3.0, 0.0]);
        assert_eq!(
            simplified,
            vec![
                [3.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0]
            ]
        );

        let simplified = simplify_ring(&ring, 0.5, |point| point[1] == 0.0);
        assert_eq!(simplified.len(), ring.len());
    }

    #[test]
    fn keeps_a_triangle_of_rings_thinner_than_the_tolerance() {
        // 10 m long, 1 m wide
        let ring = [[0.0, 0.0], [5.0, -0.5], [10.0, 0.0], [5.0, 0.4]];

        let simplified = simplify_ring(&ring, 2.0, |_| false);

        assert_eq!(simplified, vec![[0.0, 0.0], [5.0, -0.5], [10.0, 0.0]]);
    }

    #[test]
    fn simplifies_lines_keeping_their_ends_and_pinned_vertices() {
        let line = [[0.0, 0.0], [1.0, 0.1], [2.0, -0.1], [3.0, 2.0], [4.0, 0.0]];

        assert_eq!(
            simplify_line(&line, 0.5, |_| false),
            vec![[0.0, 0.0], [2.0, -0.1], [3.0, 2.0], [4.0, 0.0]]
        );
        assert_eq!(
            simplify_line(&line, 0.5, |point| point == [1.0, 0.1]),
            vec![[0.0, 0.0], [1.0, 0.1], [2.0, -0.1], [3.0, 2.0], [4.0, 0.0]]
        );
    }

    #[test]
    fn inserts_the_crossings_of_the_pinned_lines_in_lines() {
        let pinned_lines = PinnedLines::around_tile_edges(
            &extent(0.0, 0.0, 10.0, 10.0),
            &extent(-10.0, -10.0, 20.0, 20.0),
            1.0,
        );
        let line = [[5.0, 5.0], [12.0, 5.0], [12.0, 6.0]];

        assert_eq!(
            pinned_lines.insert_line_crossings(&line),
            vec![
                [5.0, 5.0],
                [9.0, 5.0],
                [10.0, 5.0],
                [11.0, 5.0],
                [12.0, 5.0],
                [12.0, 6.0]
            ]
        );
    }

    #[test]
    fn keeps_the_points_where_iso_lines_cross_the_tile_edge() {
        // A wavy band crossing the tile edge at x = 10, traced in two tiles
        let values: Vec<f32> = (0..SIZE * SIZE)
            .map(|cell| {
                let (x, y) = ((cell % SIZE) as f32, (cell / SIZE) as f32);
                3.0 - (y - 10.0 - 2.0 * (x * 0.7).sin()).abs()
            })
            .collect();
        let tracer = |min_x: f64, max_x: f64| Tracer {
            builder: ContourBuilder::new(SIZE, SIZE, true),
            pinned_lines: PinnedLines::around_tile_edges(
                &extent(min_x, 0.0, max_x, 20.0),
                &extent(min_x - 2.0, -2.0, max_x + 2.0, 22.0),
                1.0,
            ),
            window: extent(0.0, 0.0, 20.0, 20.0),
            simplify_tolerance: 1.0,
        };

        let edge_points = |shapes: Shapes| {
            let mut points: Vec<[i64; 2]> = shapes
                .iter()
                .flatten()
                .flatten()
                .filter(|point| (point[0] - 10.0).abs() < 1e-3)
                .map(|point| {
                    [
                        (point[0] * 1e4).round() as i64,
                        (point[1] * 1e4).round() as i64,
                    ]
                })
                .collect();
            points.sort();
            points
        };

        let west = edge_points(tracer(0.0, 10.0).trace(&values, "test areas").unwrap());
        let east = edge_points(tracer(10.0, 19.0).trace(&values, "test areas").unwrap());
        assert_eq!(west.len(), 2);
        assert_eq!(west, east);
    }

    #[test]
    fn traces_nested_areas_with_holes() {
        // A ring of 10 x 10 cells around a 4 x 4 hole, holding a 2 x 2 island
        let values = field(|x, y| {
            (square(2, 2, 10)(x, y) && !square(5, 5, 4)(x, y)) || square(6, 6, 2)(x, y)
        });
        let tracer = Tracer {
            builder: ContourBuilder::new(SIZE, SIZE, true),
            pinned_lines: PinnedLines(vec![]),
            window: extent(0.0, 0.0, 20.0, 20.0),
            simplify_tolerance: 0.0,
        };

        let shapes = tracer.trace(&values, "test areas").unwrap();

        assert_eq!(shapes.len(), 2);
        let areas: Vec<Vec<f64>> = shapes
            .iter()
            .map(|shape| shape.iter().map(|ring| ring_area(ring)).collect())
            .collect();
        // The outer ring passes half way between the cell centers, its
        // corners cut, and so on
        assert!(areas.iter().any(|shape| shape.len() == 2
            && (shape[0] - 99.5).abs() < 1e-6
            && (shape[1] - 15.5).abs() < 1e-6));
        assert!(areas
            .iter()
            .any(|shape| shape.len() == 1 && (shape[0] - 3.5).abs() < 1e-6));

        // Clipped to the left half of the ring
        let mut sink = FeatureSink::new(extent(0.0, 0.0, 20.0, 20.0), extent(0.0, 0.0, 7.0, 20.0));
        sink.push(AreaSymbol::VegetationSlowRunning406, shapes, SLIVER_AREA);
        let features = sink.into_features();
        assert_eq!(features.len(), 2);
        assert!(features.iter().all(|feature| match feature {
            MapFeature::Area(_, polygon) => polygon.bbox().max.x <= 7.0,
            MapFeature::Point(..) | MapFeature::Line(..) => false,
        }));
    }

    fn square_shape(min: f64, max: f64) -> Vec<Vec<[f64; 2]>> {
        vec![vec![[min, min], [max, min], [max, max], [min, max]]]
    }

    #[test]
    fn measures_shapes_before_clipping_them_to_the_tile() {
        let mut sink = FeatureSink::new(
            extent(-50.0, -50.0, 100.0, 100.0),
            extent(0.0, 0.0, 50.0, 50.0),
        );

        sink.push(
            AreaSymbol::RoughOpenLand403,
            vec![
                // Large, but mostly beyond the tile edge
                square_shape(40.0, 60.0),
                // Small, crossing the tile edge
                square_shape(45.0, 52.0),
                // A sliver within the tile of a large shape
                square_shape(49.9, 70.0),
            ],
            100.0,
        );

        let areas: Vec<f64> = sink
            .into_features()
            .iter()
            .map(|feature| match feature {
                MapFeature::Area(_, polygon) => {
                    let bbox = polygon.bbox();
                    (bbox.max.x - bbox.min.x) * (bbox.max.y - bbox.min.y)
                }
                MapFeature::Point(..) | MapFeature::Line(..) => unreachable!(),
            })
            .collect();
        assert_eq!(areas.len(), 2);
        assert!((areas[0] - 100.0).abs() < 1e-6);
        assert!((areas[1] - 0.01).abs() < 1e-6);
    }

    #[test]
    fn drops_small_shapes_away_from_the_window_border() {
        let mut sink = FeatureSink::new(
            extent(0.0, 0.0, 100.0, 100.0),
            extent(0.0, 0.0, 100.0, 100.0),
        );

        sink.push(
            AreaSymbol::RoughOpenLand403,
            vec![
                // Too small
                square_shape(10.0, 15.0),
                // Reaching the window border
                square_shape(0.0, 5.0),
                square_shape(50.0, 70.0),
                // A sliver along the window border
                square_shape(0.0, 0.5),
            ],
            100.0,
        );

        let widths: Vec<f64> = sink
            .into_features()
            .iter()
            .map(|feature| match feature {
                MapFeature::Area(_, polygon) => polygon.bbox().max.x - polygon.bbox().min.x,
                MapFeature::Point(..) | MapFeature::Line(..) => unreachable!(),
            })
            .collect();
        assert_eq!(widths, vec![5.0, 20.0]);
    }
}
