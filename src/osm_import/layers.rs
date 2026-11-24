//! The features of the `lines` and `multipolygons` layers, as
//! `OGROSMDataSource` builds them from the ways and relations.

use std::collections::{BTreeSet, HashMap};

use log::{debug, warn};

use super::{
    parse::{OsmData, Relation},
    rings::{organize_polygons, rings_from_edges},
    AreaFeature, AreaSource, LineFeature, LonLat, OsmLayers, PolygonRings, Tag,
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

/// A way with at least two resolved nodes, which relations can use. Only
/// area ways keep their tags.
struct IndexedWay {
    points: Vec<LonLat>,
    tags: Vec<Tag>,
}

pub(super) fn build_layers(data: &OsmData) -> OsmLayers {
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
        // Overpass lists the member ways of the relations a second time,
        // without tags, after the nodes.
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
    let members: Vec<_> = relation
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

    // OGR assembles the open ways into polygons, then takes them apart into
    // rings again, detaching each polygon's rings from the last: its holes
    // in reverse order, then its exterior. Only closed rings are kept, and
    // they are nested again with the closed ways.
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

    if rings.is_empty() {
        warn!("OSM relation {} has no closed ring, skipped", relation.id);
        return None;
    }
    Some((organize_polygons(rings), outer_tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osm_import::{parse::parse_osm, test_support::tags};

    /// An OSM file with `before`, nodes 1–4 at the corners of a 0.01° square,
    /// nodes 5–8 at the corners of a smaller square inside it, nodes 9–12 at
    /// the corners of a square beside it, then `after`.
    fn osm_around(before: &str, after: &str) -> OsmData {
        let nodes = [
            (1, 5.70, 45.10),
            (2, 5.70, 45.11),
            (3, 5.71, 45.11),
            (4, 5.71, 45.10),
            (5, 5.702, 45.102),
            (6, 5.702, 45.108),
            (7, 5.708, 45.108),
            (8, 5.708, 45.102),
            (9, 5.72, 45.10),
            (10, 5.72, 45.11),
            (11, 5.73, 45.11),
            (12, 5.73, 45.10),
        ]
        .map(|(id, lon, lat)| format!(r#"<node id="{id}" lat="{lat}" lon="{lon}"/>"#))
        .join("\n");
        parse_osm(&format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><osm version="0.6">{before}{nodes}{after}</osm>"#
        ))
        .unwrap()
    }

    fn osm(body: &str) -> OsmData {
        osm_around("", body)
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

    fn sources(layers: &OsmLayers) -> Vec<AreaSource> {
        layers
            .multipolygons
            .iter()
            .map(|area| area.source)
            .collect()
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

        assert_eq!(sources(&layers), [AreaSource::Way(12), AreaSource::Way(15)]);
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
    fn open_ways_make_several_polygons() {
        // Roles do not matter: the small square is a hole of the large one,
        // the square beside it a polygon of its own.
        let data = osm(&[
            way(101, &[1, 2, 3], &[]),
            way(102, &[3, 4, 1], &[]),
            way(103, &[9, 10, 11], &[]),
            way(104, &[11, 12, 9], &[]),
            way(105, &[5, 6, 7], &[]),
            way(106, &[7, 8, 5], &[]),
            r#"<relation id="100">
                <member type="way" ref="101" role="outer"/>
                <member type="way" ref="102" role="outer"/>
                <member type="way" ref="103" role="outer"/>
                <member type="way" ref="104" role="outer"/>
                <member type="way" ref="105" role="outer"/>
                <member type="way" ref="106" role="outer"/>
                <tag k="type" v="multipolygon"/><tag k="natural" v="water"/>
            </relation>"#
                .to_owned(),
        ]
        .concat());
        let layers = build_layers(&data);

        assert_eq!(sources(&layers), [AreaSource::Relation(100)]);
        let ring = |ids: [i64; 5]| ids.map(|id| data.nodes[&id]).to_vec();
        assert_eq!(
            layers.multipolygons[0].polygons,
            [
                PolygonRings {
                    exterior: ring([1, 2, 3, 4, 1]),
                    interiors: vec![ring([5, 6, 7, 8, 5])],
                },
                PolygonRings {
                    exterior: ring([9, 10, 11, 12, 9]),
                    interiors: vec![],
                },
            ]
        );
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
    fn the_untagged_copies_of_member_ways_are_ignored() {
        // As Overpass writes `out body; >; out skel qt;`: the ways and the
        // relations, then the nodes and the member ways again, untagged.
        let data = osm_around(
            &[
                way(80, &[1, 2, 3, 4, 1], &[("building", "yes")]),
                way(81, &[5, 6], &[("highway", "path")]),
                r#"<relation id="90">
                    <member type="way" ref="80" role="outer"/>
                    <member type="way" ref="81" role="outer"/>
                    <tag k="type" v="multipolygon"/>
                </relation>"#
                    .to_owned(),
            ]
            .concat(),
            &[way(80, &[1, 2, 3, 4, 1], &[]), way(81, &[5, 6], &[])].concat(),
        );
        let layers = build_layers(&data);

        let line_ids: Vec<i64> = layers.lines.iter().map(|line| line.way_id).collect();
        assert_eq!(line_ids, [81]);
        assert_eq!(sources(&layers), [AreaSource::Relation(90)]);
        assert_eq!(
            layers.multipolygons[0].tags,
            tags(&[("building", "yes"), ("type", "multipolygon")])
        );
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

        assert_eq!(sources(&layers), [AreaSource::Way(60)]);
    }
}
