//! Merges the OCD objects of the tiles of a batch into seamless objects.
//!
//! Each tile's objects are clipped to the tile, in the paper coordinates the
//! batch shares. A line that ends on an edge shared with another tile is
//! joined to the line of the same symbol that ends at the same place on the
//! other side. Areas that share a stretch of such an edge with an area of the
//! same symbol on the other side are unioned with it. The union covers every
//! area of the group, so areas of the same symbol that overlap one another
//! within a tile are unioned too when one of them reaches a seam.

use crate::ocd::{ObjectKind, OcdObject, OcdPoint, FLAG_HOLE};
use i_overlay::{
    core::{
        fill_rule::FillRule,
        overlay::{Overlay, ShapeType},
        overlay_rule::OverlayRule,
    },
    i_float::int::point::IntPoint,
};
use std::collections::{HashMap, HashSet};

/// A tile's objects, clipped to its bounds, in paper units.
pub struct TileObjects {
    pub bounds: Bounds,
    pub objects: Vec<OcdObject>,
}

/// An axis-aligned rectangle in paper units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// Points clipped to a tile edge are computed in projected coordinates, and
/// may round to the paper unit next to the edge.
const EDGE_TOLERANCE: i32 = 1;

/// A joined line end closer than this, in paper units, to the segment between
/// its neighbours is a vertex the tile edge cut a straight segment at, and is
/// dropped. The cut point and both neighbours are rounded to paper units.
const STRAIGHT_JOIN_TOLERANCE: f64 = 2.0;

/// The line a tile edge lies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum EdgeLine {
    /// x = the coordinate
    Vertical(i32),
    /// y = the coordinate
    Horizontal(i32),
}

impl EdgeLine {
    fn contains(self, point: &OcdPoint) -> bool {
        match self {
            EdgeLine::Vertical(x) => (point.x - x).abs() <= EDGE_TOLERANCE,
            EdgeLine::Horizontal(y) => (point.y - y).abs() <= EDGE_TOLERANCE,
        }
    }

    /// The position of `point` along the line.
    fn position(self, point: &OcdPoint) -> i32 {
        match self {
            EdgeLine::Vertical(_) => point.y,
            EdgeLine::Horizontal(_) => point.x,
        }
    }
}

impl Bounds {
    fn edge_lines(&self) -> [EdgeLine; 4] {
        [
            EdgeLine::Vertical(self.min_x),
            EdgeLine::Vertical(self.max_x),
            EdgeLine::Horizontal(self.min_y),
            EdgeLine::Horizontal(self.max_y),
        ]
    }
}

/// The ends of a line object, start first.
fn line_ends(object: &OcdObject) -> impl Iterator<Item = (bool, OcdPoint)> + '_ {
    let first = object.points.first().map(|point| (true, *point));
    let last = object.points.last().map(|point| (false, *point));
    first.into_iter().chain(last)
}

/// The stretches of `edge_line` that the rings of an area object run along,
/// as the positions of their ends along it.
fn edge_contacts(object: &OcdObject, edge_line: EdgeLine) -> impl Iterator<Item = (i32, i32)> + '_ {
    get_rings(&object.points)
        .into_iter()
        .flat_map(|ring| ring.windows(2))
        .filter(move |segment| edge_line.contains(&segment[0]) && edge_line.contains(&segment[1]))
        .map(move |segment| {
            let (a, b) = (
                edge_line.position(&segment[0]),
                edge_line.position(&segment[1]),
            );
            (a.min(b), a.max(b))
        })
}

/// Whether merging may change `object`: a line ending on an edge of
/// `bounds`, or an area running along one.
fn reaches_edge(object: &OcdObject, bounds: &Bounds) -> bool {
    bounds
        .edge_lines()
        .into_iter()
        .any(|edge_line| match object.kind {
            ObjectKind::Line => line_ends(object).any(|(_, point)| edge_line.contains(&point)),
            ObjectKind::Area => edge_contacts(object, edge_line).next().is_some(),
        })
}

/// Removes from `tile` and returns its objects that do not reach its edges,
/// which `merge_tile_objects` would leave unchanged.
pub fn take_interior_objects(tile: &mut TileObjects) -> Vec<OcdObject> {
    let (edge, interior) = std::mem::take(&mut tile.objects)
        .into_iter()
        .partition(|object| reaches_edge(object, &tile.bounds));
    tile.objects = edge;
    interior
}

/// The stretch of edge two tiles share.
struct Seam {
    line: EdgeLine,
    start: i32,
    end: i32,
    tiles: [usize; 2],
}

impl Seam {
    fn contains(&self, position: i32) -> bool {
        self.start - EDGE_TOLERANCE <= position && position <= self.end + EDGE_TOLERANCE
    }
}

fn get_seams(bounds: &[Bounds]) -> Vec<Seam> {
    let mut seams = vec![];

    for (first_index, a) in bounds.iter().enumerate() {
        for (second_index, b) in bounds.iter().enumerate() {
            // `a` west or south of `b`
            let (line, start, end) = if a.max_x == b.min_x {
                (
                    EdgeLine::Vertical(a.max_x),
                    a.min_y.max(b.min_y),
                    a.max_y.min(b.max_y),
                )
            } else if a.max_y == b.min_y {
                (
                    EdgeLine::Horizontal(a.max_y),
                    a.min_x.max(b.min_x),
                    a.max_x.min(b.max_x),
                )
            } else {
                continue;
            };

            if start < end {
                seams.push(Seam {
                    line,
                    start,
                    end,
                    tiles: [first_index, second_index],
                });
            }
        }
    }

    seams
}

/// An object and the index of its tile.
struct TileObject {
    tile: usize,
    object: OcdObject,
}

/// The tile edges, by tile index, that are part of a seam, the only ones
/// objects are merged across.
struct SeamEdges<'a> {
    bounds: &'a [Bounds],
    seams: &'a [Seam],
    edges: HashSet<(usize, EdgeLine)>,
}

impl<'a> SeamEdges<'a> {
    fn new(bounds: &'a [Bounds], seams: &'a [Seam]) -> Self {
        Self {
            bounds,
            seams,
            edges: seams
                .iter()
                .flat_map(|seam| seam.tiles.map(|tile| (tile, seam.line)))
                .collect(),
        }
    }

    /// The edge lines of `tile` that are part of a seam.
    fn of_tile(&self, tile: usize) -> impl Iterator<Item = EdgeLine> + '_ {
        self.bounds[tile]
            .edge_lines()
            .into_iter()
            .filter(move |edge_line| self.edges.contains(&(tile, *edge_line)))
    }
}

/// Joins lines and unions areas across the seams of `tiles`. Line ends on
/// either side of a seam are joined when they are at most `join_tolerance`
/// paper units apart.
pub fn merge_tile_objects(tiles: Vec<TileObjects>, join_tolerance: i32) -> Vec<OcdObject> {
    let bounds: Vec<Bounds> = tiles.iter().map(|tile| tile.bounds).collect();
    let seams = get_seams(&bounds);
    let seam_edges = SeamEdges::new(&bounds, &seams);

    let mut lines: Vec<TileObject> = vec![];
    let mut areas: Vec<TileObject> = vec![];
    for (tile_index, tile) in tiles.into_iter().enumerate() {
        for object in tile.objects {
            let tile_object = TileObject {
                tile: tile_index,
                object,
            };
            match tile_object.object.kind {
                ObjectKind::Line => lines.push(tile_object),
                ObjectKind::Area => areas.push(tile_object),
            }
        }
    }

    let mut joined_lines: Vec<OcdObject> = vec![];
    let mut is_joined = vec![false; lines.len()];
    for chain in get_line_chains(&lines, &seam_edges, join_tolerance) {
        if chain.lines.len() < 2 && !chain.closed {
            continue;
        }
        for (line, _) in &chain.lines {
            is_joined[*line] = true;
        }
        joined_lines.push(join_lines(&lines, &chain));
    }

    let mut unioned_areas: Vec<OcdObject> = vec![];
    let mut is_unioned = vec![false; areas.len()];
    for group in get_area_groups(&areas, &seam_edges) {
        if group.len() < 2 {
            continue;
        }
        let group_objects: Vec<&OcdObject> = group
            .iter()
            .map(|area| {
                is_unioned[*area] = true;
                &areas[*area].object
            })
            .collect();
        unioned_areas.extend(union_areas(&group_objects));
    }

    let unchanged = |objects: Vec<TileObject>, is_changed: Vec<bool>| {
        objects
            .into_iter()
            .zip(is_changed)
            .filter(|(_, is_changed)| !is_changed)
            .map(|(tile_object, _)| tile_object.object)
    };
    unchanged(lines, is_joined)
        .chain(joined_lines)
        .chain(unchanged(areas, is_unioned))
        .chain(unioned_areas)
        .collect()
}

/// An end of a line, by index in the list of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LineEnd {
    line: usize,
    is_start: bool,
}

/// Lines to join, each one reversed or not.
struct LineChain {
    lines: Vec<(usize, bool)>,
    closed: bool,
}

/// Pairs the line ends on either side of each seam, closest pairs first, and
/// returns the chains of lines the pairs form, single lines included.
fn get_line_chains(
    lines: &[TileObject],
    seam_edges: &SeamEdges,
    join_tolerance: i32,
) -> Vec<LineChain> {
    // The line ends on each seam edge of each tile
    let mut ends_on_edges: HashMap<(usize, EdgeLine), Vec<(LineEnd, OcdPoint)>> = HashMap::new();
    for (line, tile_object) in lines.iter().enumerate() {
        for (is_start, point) in line_ends(&tile_object.object) {
            for edge_line in seam_edges.of_tile(tile_object.tile) {
                if edge_line.contains(&point) {
                    ends_on_edges
                        .entry((tile_object.tile, edge_line))
                        .or_default()
                        .push((LineEnd { line, is_start }, point));
                }
            }
        }
    }

    let mut candidates: Vec<(i64, LineEnd, LineEnd)> = vec![];
    let max_squared_distance = (join_tolerance as i64).pow(2);
    for seam in seam_edges.seams {
        let [Some(first_ends), Some(second_ends)] =
            seam.tiles.map(|tile| ends_on_edges.get(&(tile, seam.line)))
        else {
            continue;
        };

        // The ends of the second side by symbol and position along the seam,
        // to find those within the tolerance of each end of the first side
        let symbol = |end: &LineEnd| lines[end.line].object.symbol;
        let mut second_ends: Vec<(i32, i32, LineEnd, OcdPoint)> = second_ends
            .iter()
            .map(|(end, point)| (symbol(end), seam.line.position(point), *end, *point))
            .filter(|(_, position, ..)| seam.contains(*position))
            .collect();
        second_ends.sort_unstable_by_key(|(symbol, position, end, _)| (*symbol, *position, *end));

        for (first_end, first_point) in first_ends {
            let position = seam.line.position(first_point);
            if !seam.contains(position) {
                continue;
            }
            let symbol = symbol(first_end);

            let nearby = second_ends.partition_point(|(other_symbol, other_position, ..)| {
                (*other_symbol, *other_position) < (symbol, position - join_tolerance)
            });
            for (_, _, second_end, second_point) in
                second_ends[nearby..]
                    .iter()
                    .take_while(|(other_symbol, other_position, ..)| {
                        (*other_symbol, *other_position) <= (symbol, position + join_tolerance)
                    })
            {
                let squared_distance = (first_point.x as i64 - second_point.x as i64).pow(2)
                    + (first_point.y as i64 - second_point.y as i64).pow(2);
                if squared_distance <= max_squared_distance {
                    candidates.push((squared_distance, *first_end, *second_end));
                }
            }
        }
    }

    // `partners[line][0]` is joined to the start of the line, `[1]` to its end
    let mut partners: Vec<[Option<LineEnd>; 2]> = vec![[None, None]; lines.len()];
    let slot = |end: LineEnd| if end.is_start { 0 } else { 1 };
    candidates.sort_by_key(|(squared_distance, ..)| *squared_distance);
    for (_, first, second) in candidates {
        if partners[first.line][slot(first)].is_none()
            && partners[second.line][slot(second)].is_none()
        {
            partners[first.line][slot(first)] = Some(second);
            partners[second.line][slot(second)] = Some(first);
        }
    }

    let mut visited = vec![false; lines.len()];
    let mut chains = vec![];
    for line in 0..lines.len() {
        if visited[line] {
            continue;
        }

        // Walks back to the first line of the chain, unless it is a loop
        let mut head = (line, false);
        loop {
            let (current, reversed) = head;
            let Some(previous) = partners[current][slot(LineEnd {
                line: current,
                is_start: !reversed,
            })] else {
                break;
            };
            if previous.line == line {
                break;
            }
            // The previous line ends where the current one starts
            head = (previous.line, previous.is_start);
        }

        let mut chain = LineChain {
            lines: vec![],
            closed: false,
        };
        let mut current = head;
        loop {
            visited[current.0] = true;
            chain.lines.push(current);
            let (line, reversed) = current;
            let Some(next) = partners[line][slot(LineEnd {
                line,
                is_start: reversed,
            })] else {
                break;
            };
            if next.line == head.0 {
                chain.closed = true;
                break;
            }
            current = (next.line, !next.is_start);
        }

        chains.push(chain);
    }

    chains
}

/// Concatenates the lines of `chain`. The ends joined are within the join
/// tolerance, they are moved half way, and dropped where the line runs
/// straight through them.
fn join_lines(lines: &[TileObject], chain: &LineChain) -> OcdObject {
    let mut points: Vec<OcdPoint> = vec![];

    for (line, reversed) in &chain.lines {
        let mut line_points = lines[*line].object.points.clone();
        if *reversed {
            line_points.reverse();
        }

        if points.is_empty() {
            points = line_points;
            continue;
        }

        let joined = midpoint(points.last().unwrap(), &line_points[0]);
        points.pop();
        if !is_on_segment(&joined, points.last().unwrap(), &line_points[1]) {
            points.push(joined);
        }
        points.extend_from_slice(&line_points[1..]);
    }

    if chain.closed {
        let joined = midpoint(&points[0], &points[points.len() - 1]);
        points[0] = joined;
        *points.last_mut().unwrap() = joined;

        // The closing point moves to the next vertex when it is dropped
        let len = points.len();
        if len > 4 && is_on_segment(&joined, &points[len - 2], &points[1]) {
            points.remove(0);
            *points.last_mut().unwrap() = points[0];
        }
    }
    points.dedup();

    OcdObject {
        symbol: lines[chain.lines[0].0].object.symbol,
        kind: ObjectKind::Line,
        points,
    }
}

fn midpoint(a: &OcdPoint, b: &OcdPoint) -> OcdPoint {
    OcdPoint {
        x: a.x + (b.x - a.x) / 2,
        y: a.y + (b.y - a.y) / 2,
        y_flags: a.y_flags,
    }
}

/// Whether `point` is within `STRAIGHT_JOIN_TOLERANCE` of the segment from
/// `a` to `b`.
fn is_on_segment(point: &OcdPoint, a: &OcdPoint, b: &OcdPoint) -> bool {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let (px, py) = ((point.x - a.x) as f64, (point.y - a.y) as f64);
    let squared_length = dx * dx + dy * dy;
    let t = if squared_length == 0.0 {
        0.0
    } else {
        ((px * dx + py * dy) / squared_length).clamp(0.0, 1.0)
    };
    (px - t * dx).powi(2) + (py - t * dy).powi(2) <= STRAIGHT_JOIN_TOLERANCE.powi(2)
}

/// A stretch of a seam edge that an area, by index in the list of areas,
/// runs along.
struct EdgeContact {
    area: usize,
    start: i32,
    end: i32,
}

/// Groups the areas, by index in the list of areas, that share a stretch of
/// a seam with an area of the same symbol on the other side, transitively.
fn get_area_groups(areas: &[TileObject], seam_edges: &SeamEdges) -> Vec<Vec<usize>> {
    let mut contacts: HashMap<(usize, EdgeLine), Vec<EdgeContact>> = HashMap::new();
    for (area, tile_object) in areas.iter().enumerate() {
        for edge_line in seam_edges.of_tile(tile_object.tile) {
            for (start, end) in edge_contacts(&tile_object.object, edge_line) {
                contacts
                    .entry((tile_object.tile, edge_line))
                    .or_default()
                    .push(EdgeContact { area, start, end });
            }
        }
    }

    let mut groups = UnionFind::new(areas.len());
    for seam in seam_edges.seams {
        let [Some(first_contacts), Some(second_contacts)] =
            seam.tiles.map(|tile| contacts.get(&(tile, seam.line)))
        else {
            continue;
        };

        // Sweeps the contacts of both sides, within the seam, by symbol and
        // along the seam. A contact overlaps the contacts of the other side
        // that started before it and end after its start.
        let mut sweep: Vec<(i32, i32, i32, usize, usize)> = [first_contacts, second_contacts]
            .into_iter()
            .enumerate()
            .flat_map(|(side, side_contacts)| {
                side_contacts.iter().filter_map(move |contact| {
                    let start = contact.start.max(seam.start);
                    let end = contact.end.min(seam.end);
                    (start < end).then(|| {
                        let symbol = areas[contact.area].object.symbol;
                        (symbol, start, end, side, contact.area)
                    })
                })
            })
            .collect();
        sweep.sort_unstable();

        let mut symbol = None;
        let mut active: [Vec<(i32, usize)>; 2] = [vec![], vec![]];
        for (contact_symbol, start, end, side, area) in sweep {
            if symbol != Some(contact_symbol) {
                symbol = Some(contact_symbol);
                active = [vec![], vec![]];
            }
            for side_active in &mut active {
                side_active.retain(|(active_end, _)| *active_end > start);
            }
            for (_, other) in &active[1 - side] {
                groups.union(area, *other);
            }
            active[side].push((end, area));
        }
    }

    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for area in 0..areas.len() {
        members.entry(groups.find(area)).or_default().push(area);
    }
    let mut groups: Vec<Vec<usize>> = members.into_values().collect();
    // Deterministic output
    groups.sort_by_key(|group| group[0]);
    groups
}

/// The rings of an area object: the outer ring, then its holes.
fn get_rings(points: &[OcdPoint]) -> Vec<&[OcdPoint]> {
    let mut rings = vec![];
    let mut start = 0;
    for index in 1..points.len() {
        if points[index].y_flags & FLAG_HOLE != 0 {
            rings.push(&points[start..index]);
            start = index;
        }
    }
    if start < points.len() {
        rings.push(&points[start..]);
    }
    rings
}

/// Unions areas of the same symbol. Returns one object per resulting outer
/// ring, with its holes.
fn union_areas(areas: &[&OcdObject]) -> Vec<OcdObject> {
    let mut overlay = Overlay::new(areas.iter().map(|area| area.points.len()).sum());

    for area in areas {
        for (index, ring) in get_rings(&area.points).into_iter().enumerate() {
            let mut contour: Vec<IntPoint> = ring
                .iter()
                .map(|point| IntPoint::new(point.x, point.y))
                .collect();
            if contour.len() > 1 && contour.first() == contour.last() {
                contour.pop();
            }

            // With the non-zero rule, outer rings counter-clockwise and holes
            // clockwise make overlapping areas union and holes cut them
            let is_outer = index == 0;
            if (signed_area(&contour) > 0) != is_outer {
                contour.reverse();
            }
            overlay.add_contour(&contour, ShapeType::Subject);
        }
    }

    overlay
        .overlay(OverlayRule::Subject, FillRule::NonZero)
        .into_iter()
        .map(|shape| {
            let mut points: Vec<OcdPoint> = vec![];
            for (index, contour) in shape.iter().enumerate() {
                let ring_start = points.len();
                points.extend(contour.iter().chain(contour.first()).map(|point| OcdPoint {
                    x: point.x,
                    y: point.y,
                    y_flags: 0,
                }));
                if index > 0 {
                    points[ring_start].y_flags = FLAG_HOLE;
                }
            }

            OcdObject {
                symbol: areas[0].symbol,
                kind: ObjectKind::Area,
                points,
            }
        })
        .collect()
}

/// Twice the signed area of a ring, positive when counter-clockwise.
fn signed_area(ring: &[IntPoint]) -> i64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a.x as i64 * b.y as i64 - b.x as i64 * a.y as i64)
        .sum()
}

struct UnionFind {
    parents: Vec<usize>,
}

impl UnionFind {
    fn new(len: usize) -> Self {
        Self {
            parents: (0..len).collect(),
        }
    }

    fn find(&mut self, mut item: usize) -> usize {
        while self.parents[item] != item {
            self.parents[item] = self.parents[self.parents[item]];
            item = self.parents[item];
        }
        item
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.parents[a.max(b)] = a.min(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two 100 x 100 tiles, west and east of x = 0.
    const WEST: Bounds = Bounds {
        min_x: -100,
        min_y: 0,
        max_x: 0,
        max_y: 100,
    };
    const EAST: Bounds = Bounds {
        min_x: 0,
        min_y: 0,
        max_x: 100,
        max_y: 100,
    };

    fn points(coordinates: &[(i32, i32)]) -> Vec<OcdPoint> {
        coordinates
            .iter()
            .map(|(x, y)| OcdPoint {
                x: *x,
                y: *y,
                y_flags: 0,
            })
            .collect()
    }

    fn line(symbol: i32, coordinates: &[(i32, i32)]) -> OcdObject {
        OcdObject {
            symbol,
            kind: ObjectKind::Line,
            points: points(coordinates),
        }
    }

    fn area(symbol: i32, rings: &[&[(i32, i32)]]) -> OcdObject {
        let mut area_points = vec![];
        for (index, ring) in rings.iter().enumerate() {
            let mut ring_points = points(ring);
            ring_points.push(ring_points[0]);
            if index > 0 {
                ring_points[0].y_flags = FLAG_HOLE;
            }
            area_points.append(&mut ring_points);
        }

        OcdObject {
            symbol,
            kind: ObjectKind::Area,
            points: area_points,
        }
    }

    fn merge(west: Vec<OcdObject>, east: Vec<OcdObject>) -> Vec<OcdObject> {
        merge_tile_objects(
            vec![
                TileObjects {
                    bounds: WEST,
                    objects: west,
                },
                TileObjects {
                    bounds: EAST,
                    objects: east,
                },
            ],
            2,
        )
    }

    fn ring_area(points: &[OcdPoint]) -> i64 {
        let ring: Vec<IntPoint> = points
            .iter()
            .map(|point| IntPoint::new(point.x, point.y))
            .collect();
        signed_area(&ring).abs() / 2
    }

    #[test]
    fn finds_the_seams_of_tiles_sharing_an_edge() {
        let seams = get_seams(&[
            WEST,
            EAST,
            // North of `EAST`, diagonal to `WEST`
            Bounds {
                min_x: 0,
                min_y: 100,
                max_x: 100,
                max_y: 200,
            },
        ]);

        let found: Vec<(EdgeLine, i32, i32, [usize; 2])> = seams
            .iter()
            .map(|seam| (seam.line, seam.start, seam.end, seam.tiles))
            .collect();
        assert_eq!(
            found,
            vec![
                (EdgeLine::Vertical(0), 0, 100, [0, 1]),
                (EdgeLine::Horizontal(100), 0, 100, [1, 2]),
            ]
        );
    }

    #[test]
    fn joins_lines_across_seams_whatever_their_direction() {
        let objects = merge(
            vec![line(101000, &[(-50, 10), (0, 20)])],
            // Drawn east to west, ending one unit off the west line's end
            vec![line(101000, &[(50, 50), (0, 21)])],
        );

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].points, points(&[(-50, 10), (0, 20), (50, 50)]));
    }

    #[test]
    fn drops_the_vertices_where_seams_cut_straight_lines() {
        let objects = merge(
            vec![line(101000, &[(-50, 10), (0, 20)])],
            vec![line(101000, &[(0, 20), (50, 31)])],
        );

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].points, points(&[(-50, 10), (50, 31)]));
    }

    #[test]
    fn joins_lines_crossing_a_seam_twice_into_a_loop() {
        let objects = merge(
            vec![line(101000, &[(0, 10), (-50, 10), (-50, 90), (0, 90)])],
            vec![line(101000, &[(0, 10), (50, 10), (50, 90), (0, 90)])],
        );

        assert_eq!(objects.len(), 1);
        // The rectangle's corners, closed, without the points on the seam
        let points = &objects[0].points;
        assert_eq!(points.len(), 5);
        assert_eq!(points.first(), points.last());
        assert!(points.iter().all(|point| point.x != 0));
    }

    #[test]
    fn joins_the_closest_ends_of_the_same_symbol_only() {
        let objects = merge(
            vec![
                line(101000, &[(-50, 10), (0, 10)]),
                line(102000, &[(-50, 50), (0, 50)]),
                line(505000, &[(-50, 90), (0, 90)]),
            ],
            vec![
                // Farther than the tolerance
                line(101000, &[(0, 13), (50, 13)]),
                // Another symbol
                line(103000, &[(0, 50), (50, 50)]),
                // The closest of two candidates is joined
                line(505000, &[(0, 91), (50, 91)]),
                line(505000, &[(0, 90), (50, 80)]),
            ],
        );

        assert_eq!(objects.len(), 6);
        let joined = objects.last().unwrap();
        assert_eq!(joined.points, points(&[(-50, 90), (0, 90), (50, 80)]));
    }

    #[test]
    fn leaves_line_ends_outside_seams_alone() {
        // Both end on the north edge of their tile, near the shared corner,
        // closer than the join tolerance
        let objects = merge_tile_objects(
            vec![
                TileObjects {
                    bounds: WEST,
                    objects: vec![line(101000, &[(-3, 50), (-3, 100)])],
                },
                TileObjects {
                    bounds: EAST,
                    objects: vec![line(101000, &[(3, 50), (3, 100)])],
                },
            ],
            10,
        );

        assert_eq!(objects.len(), 2);
    }

    #[test]
    fn unions_areas_sharing_a_stretch_of_seam() {
        let objects = merge(
            vec![
                // Holding a hole that crosses the seam
                area(
                    406000,
                    &[
                        &[(-50, 10), (0, 10), (0, 90), (-50, 90)],
                        &[(-10, 40), (0, 40), (0, 60), (-10, 60)],
                    ],
                ),
                // Another symbol
                area(408000, &[&[(-50, 95), (0, 95), (0, 99), (-50, 99)]]),
            ],
            vec![
                area(
                    406000,
                    &[
                        &[(0, 10), (50, 10), (50, 90), (0, 90)],
                        &[(0, 40), (10, 40), (10, 60), (0, 60)],
                    ],
                ),
                area(410000, &[&[(0, 95), (50, 95), (50, 99), (0, 99)]]),
            ],
        );

        assert_eq!(objects.len(), 3);
        let merged = objects.last().unwrap();
        assert_eq!(merged.symbol, 406000);
        let rings = get_rings(&merged.points);
        assert_eq!(rings.len(), 2);
        assert_eq!(ring_area(rings[0]), 100 * 80);
        assert_eq!(ring_area(rings[1]), 20 * 20);
        assert_eq!(rings[1][0].y_flags, FLAG_HOLE);
        assert!(merged.points.iter().all(|point| point.x != 0));
    }

    #[test]
    fn unions_areas_along_a_seam_with_several_areas_across() {
        let objects = merge(
            vec![area(406000, &[&[(-50, 10), (0, 10), (0, 90), (-50, 90)]])],
            vec![
                area(406000, &[&[(0, 20), (50, 20), (50, 30), (0, 30)]]),
                area(406000, &[&[(0, 50), (50, 50), (50, 60), (0, 60)]]),
                area(406000, &[&[(0, 80), (50, 80), (50, 85), (0, 85)]]),
            ],
        );

        assert_eq!(objects.len(), 1);
        assert_eq!(ring_area(&objects[0].points), 50 * 80 + 50 * 25);
    }

    #[test]
    fn takes_the_objects_away_from_the_tile_edges() {
        let mut tile = TileObjects {
            bounds: EAST,
            objects: vec![
                line(101000, &[(0, 10), (50, 10)]),
                line(101000, &[(10, 10), (50, 10)]),
                area(406000, &[&[(0, 20), (50, 20), (50, 30), (0, 30)]]),
                // Touching the edge at a vertex only
                area(406000, &[&[(0, 50), (50, 40), (50, 60)]]),
            ],
        };

        let interior = take_interior_objects(&mut tile);

        assert_eq!(tile.objects.len(), 2);
        assert_eq!(interior.len(), 2);
        assert_eq!(interior[0].kind, ObjectKind::Line);
        assert_eq!(interior[1].kind, ObjectKind::Area);
    }

    #[test]
    fn does_not_union_areas_touching_at_a_point() {
        let objects = merge(
            vec![area(406000, &[&[(-50, 10), (0, 10), (-50, 50)]])],
            vec![area(406000, &[&[(0, 10), (50, 10), (50, 50)]])],
        );

        assert_eq!(objects, {
            let mut expected = vec![area(406000, &[&[(-50, 10), (0, 10), (-50, 50)]])];
            expected.push(area(406000, &[&[(0, 10), (50, 10), (50, 50)]]));
            expected
        });
    }

    #[test]
    fn unions_overlapping_areas_of_a_group() {
        let objects = merge(
            vec![
                area(521003, &[&[(-50, 10), (0, 10), (0, 50), (-50, 50)]]),
                area(521003, &[&[(-30, 10), (0, 10), (0, 30), (-30, 30)]]),
            ],
            vec![area(521003, &[&[(0, 10), (50, 10), (50, 50), (0, 50)]])],
        );

        assert_eq!(objects.len(), 1);
        assert_eq!(get_rings(&objects[0].points).len(), 1);
        assert_eq!(ring_area(&objects[0].points), 100 * 40);
    }
}
