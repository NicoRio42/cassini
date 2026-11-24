//! The `lines` and `multipolygons` shapefiles of a tile's OpenStreetMap
//! file, as `ogr2ogr -t_srs EPSG:2154 -sql "SELECT * FROM <layer>"` wrote
//! them with OGR's OSM driver and its default `osmconf.ini`: the same
//! features in the same order, the same fields and the same `other_tags`.
//!
//! Ported from GDAL 3.12: the OSM driver (`ogr/ogrsf_frmts/osm`), the ring
//! assembly of `OGRBuildPolygonFromEdges` (`ogr/ograssemblepolygon.cpp`) and
//! `OGRGeometryFactory::organizePolygons` (`ogr/ogrgeometryfactory.cpp`):
//!
//! > Copyright (c) 1999, Frank Warmerdam
//! > Copyright (c) 2008-2014, Even Rouault <even dot rouault at spatialys.com>
//! >
//! > SPDX-License-Identifier: MIT
//!
//! OGR resolves the nodes of the ways it has read so far when it reaches the
//! first relation or the end of the file. Overpass lists the ways before
//! the nodes, and the download step moves the relations after them, so
//! every node is known by then: here ways are resolved against all the nodes
//! of the file.
//!
//! The `.dbf` is UTF-8, with a `.cpg` saying so, where OGR recodes to
//! ISO-8859-1. Values longer than a field are cut at 254 bytes, as the dBase
//! format requires, on a character boundary.

use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::Write,
    path::Path,
};

use log::{debug, warn};
use quick_xml::{
    events::{BytesStart, Event},
    Reader, XmlVersion,
};
use shapefile::{
    dbase::{FieldIOError, FieldName, FieldValue, FieldWriter, TableWriterBuilder, WritableRecord},
    Point, Polygon, PolygonRing, Polyline, ShapeType,
};

use crate::{
    error::{CassiniError, Result, ResultContext},
    projection::wgs84_to_lambert93,
    shapefile_output::write_shapefile,
};

/// The `[general] closed_ways_are_polygons` keys and `key=value` pairs of
/// the default `osmconf.ini`: a closed way with one of them is an area.
const CLOSED_WAYS_ARE_POLYGONS: [&str; 18] = [
    "aeroway",
    "amenity",
    "boundary",
    "building",
    "craft",
    "geological",
    "historic",
    "landuse",
    "leisure",
    "military",
    "natural",
    "office",
    "place",
    "shop",
    "sport",
    "tourism",
    "highway=platform",
    "public_transport=platform",
];

/// The `[lines] attributes` of the default `osmconf.ini`.
const LINE_ATTRIBUTES: [&str; 7] = [
    "name",
    "highway",
    "waterway",
    "aerialway",
    "barrier",
    "man_made",
    "railway",
];

/// The `[multipolygons] attributes` of the default `osmconf.ini`.
const MULTIPOLYGON_ATTRIBUTES: [&str; 22] = [
    "name",
    "type",
    "aeroway",
    "amenity",
    "admin_level",
    "barrier",
    "boundary",
    "building",
    "craft",
    "geological",
    "historic",
    "land_area",
    "landuse",
    "leisure",
    "man_made",
    "military",
    "natural",
    "office",
    "place",
    "shop",
    "sport",
    "tourism",
];

/// The `[lines] ignore` keys of the default `osmconf.ini`, left out of
/// `other_tags`. A key ending with `:` also ignores the keys it prefixes.
const LINE_IGNORED_KEYS: [&str; 10] = [
    "created_by",
    "converted_by",
    "source",
    "time",
    "ele",
    "note",
    "todo",
    "openGeoDB:",
    "fixme",
    "FIXME",
];

/// The `[multipolygons] ignore` keys of the default `osmconf.ini`.
const MULTIPOLYGON_IGNORED_KEYS: [&str; 11] = [
    "area",
    "created_by",
    "converted_by",
    "source",
    "time",
    "ele",
    "note",
    "todo",
    "openGeoDB:",
    "fixme",
    "FIXME",
];

/// The keys OGR drops from the tags it keeps for area ways
/// (`m_ignoredKeys`): an area way with no other tag is no standalone
/// polygon.
const AREA_WAY_IGNORED_KEYS: [&str; 7] = [
    "area",
    "created_by",
    "converted_by",
    "note",
    "todo",
    "fixme",
    "FIXME",
];

/// The most tags OGR keeps for an area way.
const MAX_TAGS_PER_AREA_WAY: usize = 255;

/// The width OGR gives a string field, and the most the dBase format allows.
const DEFAULT_STRING_WIDTH: usize = 80;
const MAX_STRING_WIDTH: usize = 254;

/// The width of the `z_order` integer field.
const Z_ORDER_WIDTH: u8 = 9;

type Tag = (String, String);

/// A position in 1e-7 degrees, as OGR stores node coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LonLat {
    lon: i32,
    lat: i32,
}

impl LonLat {
    fn from_degrees(lon: f64, lat: f64) -> Self {
        let fixed = |degrees: f64| (degrees * 1.0e7 + 0.5).floor() as i32;
        Self {
            lon: fixed(lon),
            lat: fixed(lat),
        }
    }

    fn x(self) -> f64 {
        f64::from(self.lon) / 1.0e7
    }

    fn y(self) -> f64 {
        f64::from(self.lat) / 1.0e7
    }

    fn to_lambert93(self) -> Point {
        let (x, y) = wgs84_to_lambert93(self.x(), self.y());
        Point::new(x, y)
    }
}

type Ring = Vec<LonLat>;

#[derive(Debug, Default)]
struct OsmData {
    nodes: HashMap<i64, LonLat>,
    ways: Vec<Way>,
    relations: Vec<Relation>,
}

#[derive(Debug, Default)]
struct Way {
    id: i64,
    node_ids: Vec<i64>,
    tags: Vec<Tag>,
}

#[derive(Debug, Default)]
struct Relation {
    id: i64,
    members: Vec<Member>,
    tags: Vec<Tag>,
}

#[derive(Debug)]
struct Member {
    is_way: bool,
    id: i64,
    role: String,
}

/// A way of the `lines` layer.
#[derive(Debug)]
struct LineFeature {
    way_id: i64,
    tags: Vec<Tag>,
    points: Vec<LonLat>,
}

/// A polygon and its holes.
#[derive(Debug, Clone, PartialEq)]
struct PolygonRings {
    exterior: Ring,
    interiors: Vec<Ring>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AreaSource {
    Relation(i64),
    Way(i64),
}

/// A feature of the `multipolygons` layer.
#[derive(Debug)]
struct AreaFeature {
    source: AreaSource,
    tags: Vec<Tag>,
    polygons: Vec<PolygonRings>,
}

#[derive(Debug, Default)]
struct OsmLayers {
    lines: Vec<LineFeature>,
    multipolygons: Vec<AreaFeature>,
}

/// A way with at least two resolved nodes, which relations can use. Only
/// area ways keep their tags.
struct IndexedWay {
    points: Vec<LonLat>,
    tags: Vec<Tag>,
}

/// Writes the `lines.shp` and `multipolygons.shp` layers of the OSM XML
/// file `osm_path` to `output_dir`, in Lambert-93.
pub fn convert_osm_file_to_shapefiles(osm_path: &Path, output_dir: &Path) -> Result<()> {
    let xml = fs::read_to_string(osm_path)
        .context(format!("could not read OSM file `{}`", osm_path.display()))?;
    let data = parse_osm(&xml).map_err(|message| CassiniError::InvalidArtifact {
        path: osm_path.to_path_buf(),
        message,
    })?;
    let layers = build_layers(&data);

    fs::create_dir_all(output_dir).context(format!(
        "could not create shapefile directory `{}`",
        output_dir.display()
    ))?;
    write_lines(&output_dir.join("lines.shp"), &layers.lines)?;
    write_multipolygons(&output_dir.join("multipolygons.shp"), &layers.multipolygons)
}

fn parse_osm(xml: &str) -> std::result::Result<OsmData, String> {
    let mut reader = Reader::from_str(xml);
    let mut data = OsmData::default();
    // The open `node`, `way` or `relation`, whose children are read.
    let mut current: Option<Element> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("invalid XML at byte {}: {error}", reader.error_position()))?;
        let (element, is_empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(element) => {
                if matches!(element.name().as_ref(), "node" | "way" | "relation") {
                    if let Some(element) = current.take() {
                        data.push(element);
                    }
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };

        let attributes = Attributes::of(element)?;
        match (element.name().as_ref(), &mut current) {
            ("node", None) => {
                let node = Element::Node(
                    attributes.id("id")?,
                    LonLat::from_degrees(attributes.number("lon")?, attributes.number("lat")?),
                );
                if is_empty {
                    data.push(node);
                } else {
                    current = Some(node);
                }
            }
            ("way", None) => {
                let way = Element::Way(Way {
                    id: attributes.id("id")?,
                    ..Way::default()
                });
                if is_empty {
                    data.push(way);
                } else {
                    current = Some(way);
                }
            }
            ("relation", None) => {
                let relation = Element::Relation(Relation {
                    id: attributes.id("id")?,
                    ..Relation::default()
                });
                if is_empty {
                    data.push(relation);
                } else {
                    current = Some(relation);
                }
            }
            ("nd", Some(Element::Way(way))) => way.node_ids.push(attributes.id("ref")?),
            ("member", Some(Element::Relation(relation))) => relation.members.push(Member {
                is_way: attributes.get("type") == Some("way"),
                id: attributes.id("ref")?,
                role: attributes.get("role").unwrap_or_default().to_owned(),
            }),
            ("tag", Some(element)) => {
                let tag = (
                    attributes.get("k").unwrap_or_default().to_owned(),
                    attributes.get("v").unwrap_or_default().to_owned(),
                );
                match element {
                    Element::Node(..) => {}
                    Element::Way(way) => way.tags.push(tag),
                    Element::Relation(relation) => relation.tags.push(tag),
                }
            }
            _ => {}
        }
    }
    Ok(data)
}

enum Element {
    Node(i64, LonLat),
    Way(Way),
    Relation(Relation),
}

impl OsmData {
    fn push(&mut self, element: Element) {
        match element {
            Element::Node(id, position) => {
                self.nodes.insert(id, position);
            }
            Element::Way(way) => self.ways.push(way),
            Element::Relation(relation) => self.relations.push(relation),
        }
    }
}

/// The unescaped attributes of an element.
struct Attributes(Vec<(String, String)>);

impl Attributes {
    fn of(element: &BytesStart) -> std::result::Result<Self, String> {
        let name = element.name().as_ref().to_owned();
        let mut attributes = Vec::new();
        for attribute in element.attributes() {
            let attribute =
                attribute.map_err(|error| format!("invalid `{name}` attribute: {error}"))?;
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| format!("invalid `{name}` attribute value: {error}"))?;
            attributes.push((attribute.key.as_ref().to_owned(), value.into_owned()));
        }
        Ok(Self(attributes))
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn id(&self, key: &str) -> std::result::Result<i64, String> {
        let value = self.get(key).ok_or(format!("missing `{key}` attribute"))?;
        value
            .parse()
            .map_err(|_| format!("invalid `{key}` attribute `{value}`"))
    }

    fn number(&self, key: &str) -> std::result::Result<f64, String> {
        let value = self.get(key).ok_or(format!("missing `{key}` attribute"))?;
        value
            .parse()
            .map_err(|_| format!("invalid `{key}` attribute `{value}`"))
    }
}

fn build_layers(data: &OsmData) -> OsmLayers {
    let mut layers = OsmLayers::default();
    let mut indexed_ways: HashMap<i64, IndexedWay> = HashMap::new();
    // Area ways with tags, which become polygons unless a relation uses
    // them as an outer ring. OGR emits them by increasing id.
    let mut standalone_area_ways = BTreeSet::new();

    // `OGROSMDataSource::NotifyWay` and `ProcessWaysBatch`.
    for way in &data.ways {
        let node_ids = &way.node_ids;
        if node_ids.len() < 2 {
            debug!("OSM way {} has {} nodes, skipped", way.id, node_ids.len());
            continue;
        }
        let is_area = node_ids[0] == node_ids[node_ids.len() - 1]
            && is_closed_way_tagged_as_polygon(&way.tags);

        let looked_up = if is_area {
            &node_ids[..node_ids.len() - 1]
        } else {
            &node_ids[..]
        };
        let mut points: Vec<LonLat> = looked_up
            .iter()
            .filter_map(|id| data.nodes.get(id).copied())
            .collect();
        if is_area && !points.is_empty() {
            points.push(points[0]);
        }
        if points.len() < 2 {
            debug!("OSM way {} has fewer than 2 known nodes, skipped", way.id);
            continue;
        }

        let tags = if is_area {
            area_way_tags(&way.tags)
        } else {
            vec![]
        };
        if is_area && !tags.is_empty() {
            standalone_area_ways.insert(way.id);
        }
        // Lines have no insignificant keys: any tag makes a line.
        if !is_area && !way.tags.is_empty() {
            layers.lines.push(LineFeature {
                way_id: way.id,
                tags: way.tags.clone(),
                points: points.clone(),
            });
        }
        // The ways table has the way id as primary key: the first way wins.
        indexed_ways
            .entry(way.id)
            .or_insert(IndexedWay { points, tags });
    }

    // `OGROSMDataSource::NotifyRelation`.
    for relation in &data.relations {
        let mut is_multipolygon = false;
        let mut has_interesting_tag = false;
        let mut type_value = "";
        for (key, value) in &relation.tags {
            if key == "type" {
                type_value = value;
                if value == "multipolygon" || value == "boundary" {
                    is_multipolygon = true;
                }
            } else if key != "created_by" {
                has_interesting_tag = true;
            }
        }
        if !is_multipolygon {
            continue;
        }

        let Some((polygons, outer_tags)) =
            build_multipolygon(relation, &indexed_ways, &mut standalone_area_ways)
        else {
            continue;
        };
        // An old-style multipolygon, with only its type, takes the tags of
        // its first outer way that has some.
        let tags = if has_interesting_tag {
            relation.tags.clone()
        } else {
            let mut tags = outer_tags;
            tags.push(("type".to_owned(), type_value.to_owned()));
            tags
        };
        layers.multipolygons.push(AreaFeature {
            source: AreaSource::Relation(relation.id),
            tags,
            polygons,
        });
    }

    // `OGROSMDataSource::ProcessPolygonsStandalone`.
    for id in standalone_area_ways {
        let way = &indexed_ways[&id];
        layers.multipolygons.push(AreaFeature {
            source: AreaSource::Way(id),
            tags: way.tags.clone(),
            polygons: vec![PolygonRings {
                exterior: way.points.clone(),
                interiors: vec![],
            }],
        });
    }

    layers
}

/// `OGROSMDataSource::IsClosedWayTaggedAsPolygon`: whether a closed way is
/// an area rather than a line.
fn is_closed_way_tagged_as_polygon(tags: &[Tag]) -> bool {
    let mut is_area = false;
    for (key, value) in tags {
        if key == "area" {
            if value == "yes" {
                return true;
            } else if value == "no" {
                return false;
            }
        }
        if is_area {
            continue;
        }
        if CLOSED_WAYS_ARE_POLYGONS.contains(&key.as_str())
            || CLOSED_WAYS_ARE_POLYGONS.contains(&format!("{key}={value}").as_str())
        {
            is_area = true;
        }
    }
    is_area
}

/// The tags OGR keeps for an area way.
fn area_way_tags(tags: &[Tag]) -> Vec<Tag> {
    tags.iter()
        .filter(|(key, _)| !AREA_WAY_IGNORED_KEYS.contains(&key.as_str()))
        .take(MAX_TAGS_PER_AREA_WAY)
        .cloned()
        .collect()
}

/// `OGROSMDataSource::BuildMultiPolygon`: the polygons of a multipolygon
/// relation, and the tags of its first outer way that has some. `None`
/// if a member way is missing or no ring closes.
///
/// Roles only choose the ways whose tags are taken and that stop being
/// standalone polygons: rings are nested by geometry.
fn build_multipolygon(
    relation: &Relation,
    indexed_ways: &HashMap<i64, IndexedWay>,
    standalone_area_ways: &mut BTreeSet<i64>,
) -> Option<(Vec<PolygonRings>, Vec<Tag>)> {
    let members: Vec<&Member> = relation
        .members
        .iter()
        .filter(|member| member.is_way && member.role != "subarea")
        .collect();
    if let Some(member) = members
        .iter()
        .find(|member| !indexed_ways.contains_key(&member.id))
    {
        warn!(
            "OSM relation {} has missing way {}, skipped",
            relation.id, member.id
        );
        return None;
    }

    let mut outer_tags = vec![];
    let mut rings = vec![];
    let mut edges = vec![];
    for member in members {
        let way = &indexed_ways[&member.id];
        let is_outer = member.role == "outer";
        if outer_tags.is_empty() && is_outer {
            outer_tags = way.tags.clone();
        }
        if way.points.first() == way.points.last() {
            rings.push(way.points.clone());
            if is_outer {
                standalone_area_ways.remove(&member.id);
            }
        } else {
            edges.push(way.points.clone());
        }
    }

    if !edges.is_empty() {
        for polygon in organize_polygons(rings_from_edges(edges)) {
            for ring in polygon
                .interiors
                .into_iter()
                .rev()
                .chain([polygon.exterior])
            {
                if ring.len() >= 4 && ring.first() == ring.last() {
                    rings.push(ring);
                }
            }
        }
    }

    if rings.is_empty() {
        warn!("OSM relation {} has no closed ring, skipped", relation.id);
        return None;
    }
    Some((organize_polygons(rings), outer_tags))
}

/// `OGRBuildPolygonFromEdges` with no tolerance and no auto-closing: joins
/// the edges end to end into rings, which may stay open.
fn rings_from_edges(edges: Vec<Ring>) -> Vec<Ring> {
    let mut edges: Vec<Ring> = edges.into_iter().filter(|edge| edge.len() >= 2).collect();
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
fn organize_polygons(rings: Vec<Ring>) -> Vec<PolygonRings> {
    if rings.len() == 1 {
        let exterior = rings.into_iter().next().expect("one ring");
        return vec![PolygonRings {
            exterior,
            interiors: vec![],
        }];
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

/// The fields of a feature: the attribute columns, then `other_tags`
/// (`OGROSMLayer::SetFieldsFromTags`).
struct FeatureFields {
    attributes: Vec<Option<String>>,
    other_tags: Option<String>,
}

impl FeatureFields {
    /// `other_fields` are the layer's other columns, whose tags are not in
    /// `other_tags` either.
    fn of(tags: &[Tag], attributes: &[&str], other_fields: &[&str], ignored_keys: &[&str]) -> Self {
        let mut values = vec![None; attributes.len()];
        let mut other_tags = String::new();
        for (key, value) in tags {
            if let Some(index) = attributes.iter().position(|attribute| attribute == key) {
                values[index] = Some(value.clone());
                continue;
            }
            if other_fields.contains(&key.as_str()) || is_ignored_key(key, ignored_keys) {
                continue;
            }
            if !other_tags.is_empty() {
                other_tags.push(',');
            }
            push_hstore_string(&mut other_tags, key);
            other_tags.push_str("=>");
            push_hstore_string(&mut other_tags, value);
        }
        Self {
            attributes: values,
            other_tags: (!other_tags.is_empty()).then_some(other_tags),
        }
    }
}

/// `OGROSMLayer::AddInOtherOrAllTags`: an ignored key, or a key whose
/// prefix up to its first `:` is ignored.
fn is_ignored_key(key: &str, ignored_keys: &[&str]) -> bool {
    ignored_keys.contains(&key)
        || key
            .find(':')
            .is_some_and(|colon| ignored_keys.contains(&&key[..=colon]))
}

/// `OGROSMEscapeStringHSTORE`.
fn push_hstore_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        if character == '"' || character == '\\' {
            output.push('\\');
        }
        output.push(character);
    }
    output.push('"');
}

/// The `z_order` of a line, from the default `osmconf.ini` expression, as
/// OGR computes it: road class, bridges up, tunnels down, railways up,
/// then ten times the layer.
fn z_order(highway: Option<&str>, railway: Option<&str>, tags: &[Tag]) -> i32 {
    let tag = |key: &str| {
        tags.iter()
            .find(|(tag_key, _)| tag_key == key)
            .map(|(_, value)| value.as_str())
    };
    let is_true = |value: Option<&str>| matches!(value, Some("yes" | "true" | "1"));

    let mut z_order: i32 = match highway {
        Some("minor" | "road" | "unclassified" | "residential") => 3,
        Some("tertiary_link" | "tertiary") => 4,
        Some("secondary_link" | "secondary") => 6,
        Some("primary_link" | "primary") => 7,
        Some("trunk_link" | "trunk") => 8,
        Some("motorway_link" | "motorway") => 9,
        _ => 0,
    };
    if is_true(tag("bridge")) {
        z_order += 10;
    }
    if is_true(tag("tunnel")) {
        z_order -= 10;
    }
    if railway.is_some() {
        z_order += 5;
    }
    if let Some(layer) = tag("layer") {
        z_order = z_order.wrapping_add(atoi(layer).wrapping_mul(10));
    }
    z_order
}

/// C's `atoi`: the leading integer of `value`, 0 if there is none.
fn atoi(value: &str) -> i32 {
    let value = value.trim_start();
    let (sign, digits) = match value.as_bytes().first() {
        Some(b'-') => (-1, &value[1..]),
        Some(b'+') => (1, &value[1..]),
        _ => (1, value),
    };
    digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i32, |number, digit| {
            number
                .wrapping_mul(10)
                .wrapping_add(i32::from(digit - b'0'))
        })
        .wrapping_mul(sign)
}

/// A column of a layer.
enum Column {
    String(&'static str),
    Integer(&'static str, u8),
}

/// A record whose values follow the columns of its layer.
struct OrderedRecord(Vec<FieldValue>);

impl WritableRecord for OrderedRecord {
    fn write_using<W: Write>(
        &self,
        field_writer: &mut FieldWriter<'_, W>,
    ) -> std::result::Result<(), FieldIOError> {
        for value in &self.0 {
            field_writer.write_next_field_value(value)?;
        }
        Ok(())
    }
}

/// The table of `columns`, whose string fields are as wide as their longest
/// value in `records`, from 80 to 254 bytes like OGR's, and `records` with
/// the longer values cut on a character boundary.
fn table_and_records(
    columns: &[Column],
    mut records: Vec<OrderedRecord>,
) -> (TableWriterBuilder, Vec<OrderedRecord>) {
    let mut table = TableWriterBuilder::new();
    for (index, column) in columns.iter().enumerate() {
        table = match *column {
            Column::String(name) => {
                let longest = records
                    .iter()
                    .filter_map(|record| match &record.0[index] {
                        FieldValue::Character(Some(value)) => Some(value.len()),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                let width = longest.clamp(DEFAULT_STRING_WIDTH, MAX_STRING_WIDTH);
                for record in &mut records {
                    if let FieldValue::Character(Some(value)) = &mut record.0[index] {
                        if value.len() > width {
                            debug!("OSM field `{name}` value cut to {width} bytes: {value}");
                            let cut = (0..=width)
                                .rev()
                                .find(|&end| value.is_char_boundary(end))
                                .unwrap_or(0);
                            value.truncate(cut);
                        }
                    }
                }
                table.add_character_field(field_name(name), width as u8)
            }
            Column::Integer(name, width) => table.add_numeric_field(field_name(name), width, 0),
        };
    }
    (table, records)
}

fn field_name(name: &str) -> FieldName {
    FieldName::try_from(name).expect("valid field name")
}

/// Writes the `.cpg` that tells readers the `.dbf` is UTF-8.
fn write_code_page(path: &Path) -> Result<()> {
    let cpg_path = path.with_extension("cpg");
    fs::write(&cpg_path, "UTF-8").context(format!("could not write `{}`", cpg_path.display()))
}

fn write_lines(path: &Path, lines: &[LineFeature]) -> Result<()> {
    let mut columns = vec![Column::String("osm_id")];
    columns.extend(LINE_ATTRIBUTES.map(Column::String));
    columns.push(Column::Integer("z_order", Z_ORDER_WIDTH));
    columns.push(Column::String("other_tags"));

    let records = lines
        .iter()
        .map(|line| {
            let fields = FeatureFields::of(
                &line.tags,
                &LINE_ATTRIBUTES,
                &["z_order"],
                &LINE_IGNORED_KEYS,
            );
            let attribute = |name: &str| {
                let index = LINE_ATTRIBUTES
                    .iter()
                    .position(|&attribute| attribute == name);
                index.and_then(|index| fields.attributes[index].as_deref())
            };
            let z_order = z_order(attribute("highway"), attribute("railway"), &line.tags);

            let mut values = vec![FieldValue::Character(Some(line.way_id.to_string()))];
            values.extend(fields.attributes.iter().cloned().map(FieldValue::Character));
            values.push(FieldValue::Numeric(Some(f64::from(z_order))));
            values.push(FieldValue::Character(fields.other_tags));
            OrderedRecord(values)
        })
        .collect();
    let (table, records) = table_and_records(&columns, records);

    let shapes = lines.iter().map(|line| {
        Polyline::new(
            line.points
                .iter()
                .map(|point| point.to_lambert93())
                .collect(),
        )
    });
    write_shapefile(path, ShapeType::Polyline, table, shapes.zip(records))?;
    write_code_page(path)
}

fn write_multipolygons(path: &Path, areas: &[AreaFeature]) -> Result<()> {
    // dBase field names have at most 10 characters: OGR cuts `admin_level`.
    let attribute_columns = MULTIPOLYGON_ATTRIBUTES.map(|attribute| match attribute {
        "admin_level" => "admin_leve",
        attribute => attribute,
    });
    let mut columns = vec![Column::String("osm_id"), Column::String("osm_way_id")];
    columns.extend(attribute_columns.map(Column::String));
    columns.push(Column::String("other_tags"));

    let records = areas
        .iter()
        .map(|area| {
            let fields = FeatureFields::of(
                &area.tags,
                &MULTIPOLYGON_ATTRIBUTES,
                &["osm_way_id"],
                &MULTIPOLYGON_IGNORED_KEYS,
            );
            let (relation_id, way_id) = match area.source {
                AreaSource::Relation(id) => (Some(id.to_string()), None),
                AreaSource::Way(id) => (None, Some(id.to_string())),
            };
            let mut values = vec![
                FieldValue::Character(relation_id),
                FieldValue::Character(way_id),
            ];
            values.extend(fields.attributes.iter().cloned().map(FieldValue::Character));
            values.push(FieldValue::Character(fields.other_tags));
            OrderedRecord(values)
        })
        .collect();
    let (table, records) = table_and_records(&columns, records);

    let shapes = areas.iter().map(|area| {
        let to_lambert93 = |ring: &Ring| ring.iter().map(|point| point.to_lambert93()).collect();
        let rings = area
            .polygons
            .iter()
            .flat_map(|polygon| {
                [PolygonRing::Outer(to_lambert93(&polygon.exterior))]
                    .into_iter()
                    .chain(
                        polygon
                            .interiors
                            .iter()
                            .map(|ring| PolygonRing::Inner(to_lambert93(ring))),
                    )
            })
            .collect();
        Polygon::with_rings(rings)
    });
    write_shapefile(path, ShapeType::Polygon, table, shapes.zip(records))?;
    write_code_page(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shapefile::dbase::Record;

    fn tags(pairs: &[(&str, &str)]) -> Vec<Tag> {
        pairs
            .iter()
            .map(|&(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    fn ring(corners: &[(f64, f64)]) -> Ring {
        corners
            .iter()
            .map(|&(lon, lat)| LonLat::from_degrees(lon, lat))
            .collect()
    }

    fn square(min: f64, max: f64) -> Ring {
        ring(&[(min, min), (min, max), (max, max), (max, min), (min, min)])
    }

    /// An OSM file with nodes 1–4 at the corners of a 0.01° square, nodes
    /// 5–8 at the corners of a smaller square inside it, the given ways and
    /// relations.
    fn osm(body: &str) -> OsmData {
        let nodes = [
            (1, 5.70, 45.10),
            (2, 5.70, 45.11),
            (3, 5.71, 45.11),
            (4, 5.71, 45.10),
            (5, 5.702, 45.102),
            (6, 5.702, 45.108),
            (7, 5.708, 45.108),
            (8, 5.708, 45.102),
        ]
        .map(|(id, lon, lat)| format!(r#"<node id="{id}" lat="{lat}" lon="{lon}"/>"#))
        .join("\n");
        parse_osm(&format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><osm version="0.6">{nodes}{body}</osm>"#
        ))
        .unwrap()
    }

    fn way(id: i64, node_ids: &[i64], tags: &[(&str, &str)]) -> String {
        let nodes: String = node_ids
            .iter()
            .map(|id| format!(r#"<nd ref="{id}"/>"#))
            .collect();
        let tags: String = tags
            .iter()
            .map(|(key, value)| format!(r#"<tag k="{key}" v="{value}"/>"#))
            .collect();
        format!(r#"<way id="{id}">{nodes}{tags}</way>"#)
    }

    #[test]
    fn closed_ways_are_areas_by_their_tags() {
        assert!(is_closed_way_tagged_as_polygon(&tags(&[(
            "building", "yes"
        )])));
        assert!(is_closed_way_tagged_as_polygon(&tags(&[(
            "highway", "platform"
        )])));
        assert!(is_closed_way_tagged_as_polygon(&tags(&[
            ("highway", "pedestrian"),
            ("area", "yes")
        ])));
        assert!(!is_closed_way_tagged_as_polygon(&tags(&[(
            "highway",
            "residential"
        )])));
        assert!(!is_closed_way_tagged_as_polygon(&tags(&[
            ("building", "yes"),
            ("area", "no")
        ])));
        assert!(!is_closed_way_tagged_as_polygon(&tags(&[])));
    }

    #[test]
    fn ways_become_lines_or_standalone_polygons() {
        let data = osm(&[
            way(10, &[1, 2, 3], &[("highway", "track")]),
            // A closed way without area tag is a line.
            way(11, &[1, 2, 3, 1], &[("highway", "residential")]),
            way(
                12,
                &[1, 2, 3, 4, 1],
                &[("building", "yes"), ("note", "dropped")],
            ),
            way(13, &[1, 2], &[]),
            // An area with ignored keys only is neither a line nor a polygon.
            way(14, &[5, 6, 7, 8, 5], &[("area", "yes"), ("note", "x")]),
            way(15, &[1, 2, 3, 1], &[("landuse", "meadow")]),
            // A single known node.
            way(16, &[1, 99], &[("highway", "path")]),
        ]
        .concat());
        let layers = build_layers(&data);

        let line_ids: Vec<i64> = layers.lines.iter().map(|line| line.way_id).collect();
        assert_eq!(line_ids, [10, 11]);
        assert_eq!(layers.lines[1].points.len(), 4);

        let area_sources: Vec<AreaSource> = layers
            .multipolygons
            .iter()
            .map(|area| area.source)
            .collect();
        assert_eq!(area_sources, [AreaSource::Way(12), AreaSource::Way(15)]);
        // The area way's ignored keys are dropped, and its ring is closed.
        assert_eq!(layers.multipolygons[0].tags, tags(&[("building", "yes")]));
        assert_eq!(layers.multipolygons[0].polygons[0].exterior.len(), 5);
    }

    #[test]
    fn relations_join_their_ways_into_rings_with_holes() {
        let data = osm(&[
            way(20, &[1, 2, 3], &[]),
            way(21, &[1, 4, 3], &[]),
            way(22, &[5, 6, 7, 8, 5], &[("building", "yes")]),
            r#"<relation id="30">
                <member type="way" ref="20" role="outer"/>
                <member type="way" ref="21" role="outer"/>
                <member type="way" ref="22" role="inner"/>
                <tag k="type" v="multipolygon"/><tag k="natural" v="water"/>
            </relation>"#
                .to_owned(),
        ]
        .concat());
        let layers = build_layers(&data);

        assert_eq!(layers.multipolygons.len(), 2);
        let lake = &layers.multipolygons[0];
        assert_eq!(lake.source, AreaSource::Relation(30));
        assert_eq!(
            lake.tags,
            tags(&[("type", "multipolygon"), ("natural", "water")])
        );
        assert_eq!(lake.polygons.len(), 1);
        // Way 20, then way 21 reversed.
        let expected_outer: Vec<LonLat> = [1, 2, 3, 4, 1].map(|id| data.nodes[&id]).to_vec();
        assert_eq!(lake.polygons[0].exterior, expected_outer);
        assert_eq!(lake.polygons[0].interiors.len(), 1);
        // An inner way stays a standalone polygon of its own.
        assert_eq!(layers.multipolygons[1].source, AreaSource::Way(22));
    }

    #[test]
    fn old_style_multipolygons_take_the_tags_of_their_outer_way() {
        let data = osm(&[
            way(
                40,
                &[1, 2, 3, 4, 1],
                &[("landuse", "residential"), ("name", "Village")],
            ),
            way(41, &[5, 6, 7, 8, 5], &[]),
            r#"<relation id="50">
                <member type="way" ref="40" role="outer"/>
                <member type="way" ref="41" role="inner"/>
                <tag k="type" v="multipolygon"/><tag k="created_by" v="JOSM"/>
            </relation>"#
                .to_owned(),
        ]
        .concat());
        let layers = build_layers(&data);

        // The outer way is no longer a standalone polygon.
        assert_eq!(layers.multipolygons.len(), 1);
        assert_eq!(
            layers.multipolygons[0].tags,
            tags(&[
                ("landuse", "residential"),
                ("name", "Village"),
                ("type", "multipolygon")
            ])
        );
        assert_eq!(layers.multipolygons[0].polygons[0].interiors.len(), 1);
    }

    #[test]
    fn relations_with_missing_ways_are_skipped() {
        let data = osm(&[
            way(60, &[1, 2, 3, 4, 1], &[("natural", "wetland")]),
            r#"<relation id="70">
                <member type="way" ref="60" role="outer"/>
                <member type="way" ref="61" role="inner"/>
                <tag k="type" v="multipolygon"/><tag k="natural" v="water"/>
            </relation>"#
                .to_owned(),
        ]
        .concat());
        let layers = build_layers(&data);

        let area_sources: Vec<AreaSource> = layers
            .multipolygons
            .iter()
            .map(|area| area.source)
            .collect();
        assert_eq!(area_sources, [AreaSource::Way(60)]);
    }

    #[test]
    fn rings_inside_holes_are_new_polygons() {
        let outer = square(0.0, 10.0);
        let hole = square(2.0, 8.0);
        let island = square(4.0, 6.0);
        let polygons = organize_polygons(vec![island.clone(), outer.clone(), hole.clone()]);

        assert_eq!(
            polygons,
            [
                PolygonRings {
                    exterior: island,
                    interiors: vec![],
                },
                PolygonRings {
                    exterior: outer,
                    interiors: vec![hole],
                },
            ]
        );
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

    #[test]
    fn other_tags_are_escaped_hstore_without_ignored_keys() {
        let fields = FeatureFields::of(
            &tags(&[
                ("highway", "path"),
                ("source", "survey"),
                ("openGeoDB:id", "1"),
                ("source:date", "2020"),
                ("note", r#"a "b" \c"#),
                ("tunnel", "yes"),
                ("z_order", "4"),
            ]),
            &LINE_ATTRIBUTES,
            &["z_order"],
            &LINE_IGNORED_KEYS,
        );
        assert_eq!(fields.attributes[1].as_deref(), Some("path"));
        assert_eq!(
            fields.other_tags.as_deref(),
            Some(r#""source:date"=>"2020","tunnel"=>"yes""#)
        );

        let fields =
            FeatureFields::of(&tags(&[("key", r#"a "b" \c"#)]), &LINE_ATTRIBUTES, &[], &[]);
        assert_eq!(
            fields.other_tags.as_deref(),
            Some(r#""key"=>"a \"b\" \\c""#)
        );
    }

    #[test]
    fn z_order_follows_the_default_osmconf() {
        assert_eq!(z_order(Some("motorway"), None, &[]), 9);
        assert_eq!(
            z_order(Some("residential"), None, &tags(&[("bridge", "yes")])),
            13
        );
        assert_eq!(
            z_order(
                Some("primary"),
                None,
                &tags(&[("tunnel", "yes"), ("layer", "-1")])
            ),
            -13
        );
        assert_eq!(z_order(None, Some("rail"), &tags(&[("layer", "2;3")])), 25);
        assert_eq!(z_order(Some("path"), None, &tags(&[("layer", "x")])), 0);
    }

    #[test]
    fn writes_the_shapefiles_that_the_vector_step_reads() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cassini-osm-import-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let osm_path = dir.join("tile.osm");
        let long_name = "é".repeat(200);
        fs::write(
            &osm_path,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><osm version="0.6">
                <way id="1"><nd ref="1"/><nd ref="2"/>
                    <tag k="highway" v="primary"/><tag k="power" v="line"/><tag k="name" v="{long_name}"/></way>
                <way id="2"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="1"/><tag k="building" v="yes"/></way>
                <node id="1" lat="45.1" lon="5.7"/><node id="2" lat="45.11" lon="5.7"/><node id="3" lat="45.11" lon="5.71"/>
                </osm>"#
            ),
        )
        .unwrap();
        let shapes_dir = dir.join("shapes");
        convert_osm_file_to_shapefiles(&osm_path, &shapes_dir).unwrap();

        let lines =
            shapefile::read_as::<_, Polyline, Record>(shapes_dir.join("lines.shp")).unwrap();
        let multipolygons =
            shapefile::read_as::<_, Polygon, Record>(shapes_dir.join("multipolygons.shp")).unwrap();
        let cpg = fs::read_to_string(shapes_dir.join("lines.cpg")).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(cpg, "UTF-8");
        assert_eq!(lines.len(), 1);
        let (line, record) = &lines[0];
        let (x, y) = wgs84_to_lambert93(5.7, 45.1);
        assert!((line.parts()[0][0].x - x).abs() < 1e-6 && (line.parts()[0][0].y - y).abs() < 1e-6);
        let text = |record: &Record, name: &str| match record.get(name) {
            Some(FieldValue::Character(value)) => value.clone(),
            other => panic!("unexpected {name}: {other:?}"),
        };
        assert_eq!(text(record, "osm_id").as_deref(), Some("1"));
        assert_eq!(text(record, "highway").as_deref(), Some("primary"));
        assert_eq!(
            text(record, "other_tags").as_deref(),
            Some(r#""power"=>"line""#)
        );
        assert_eq!(record.get("z_order"), Some(&FieldValue::Numeric(Some(7.0))));
        // 400 bytes of UTF-8, cut to 254 on a character boundary.
        assert_eq!(text(record, "name"), Some("é".repeat(127)));

        assert_eq!(multipolygons.len(), 1);
        let (polygon, record) = &multipolygons[0];
        assert_eq!(text(record, "osm_way_id").as_deref(), Some("2"));
        assert_eq!(text(record, "osm_id"), None);
        assert_eq!(text(record, "building").as_deref(), Some("yes"));
        assert!(matches!(polygon.rings()[..], [PolygonRing::Outer(_)]));
    }
}
