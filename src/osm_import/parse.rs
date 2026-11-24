//! Reading an OSM XML file, as OGR's `osm_parser.cpp` reads it.

use std::collections::HashMap;

use quick_xml::{
    escape::unescape,
    events::{BytesStart, Event},
    Reader, XmlVersion,
};

use super::{LonLat, Tag};

#[derive(Debug, Default)]
pub(super) struct OsmData {
    pub(super) nodes: HashMap<i64, LonLat>,
    pub(super) ways: Vec<Way>,
    pub(super) relations: Vec<Relation>,
    /// The text of the `remark` elements, where Overpass reports a query
    /// that failed after it started writing its result.
    pub(super) remarks: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct Way {
    pub(super) id: i64,
    pub(super) node_ids: Vec<i64>,
    pub(super) tags: Vec<Tag>,
}

#[derive(Debug, Default)]
pub(super) struct Relation {
    pub(super) id: i64,
    pub(super) members: Vec<Member>,
    pub(super) tags: Vec<Tag>,
}

#[derive(Debug)]
pub(super) struct Member {
    pub(super) is_way: bool,
    pub(super) id: i64,
    pub(super) role: String,
}

/// Reads an OSM XML document. Fails unless it is a whole `osm` element, so
/// that an error page or a truncated download is not read as an empty map.
pub(super) fn parse_osm(xml: &str) -> std::result::Result<OsmData, String> {
    let mut reader = Reader::from_str(xml);
    let mut data = OsmData::default();
    // The open `node`, `way` or `relation`, whose children are read.
    let mut current: Option<Element> = None;
    // Whether the `osm` root element has been opened, then closed.
    let (mut is_open, mut is_closed) = (false, false);

    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("invalid XML at byte {}: {error}", reader.error_position()))?;
        let (element, is_empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(element) => {
                match element.name().as_ref() {
                    "osm" => is_closed = true,
                    "node" | "way" | "relation" => {
                        if let Some(element) = current.take() {
                            data.push(element);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };

        if !is_open {
            let name = element.name();
            if name.as_ref() != "osm" {
                return Err(format!(
                    "not an OSM file: its root element is `<{}>`",
                    name.as_ref()
                ));
            }
            is_open = true;
            is_closed = is_empty;
            continue;
        }

        let attributes = Attributes::of(element)?;
        match (element.name().as_ref(), &mut current) {
            ("remark", None) => {
                if !is_empty {
                    let text = reader
                        .read_text(element.name())
                        .map_err(|error| format!("invalid `remark`: {error}"))?;
                    let text = text.xml10_content();
                    let text = unescape(&text)
                        .map_err(|error| format!("invalid `remark` text: {error}"))?;
                    data.remarks.push(text.trim().to_owned());
                }
            }
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

    if !is_open {
        return Err("not an OSM file: it has no `<osm>` element".to_owned());
    }
    if !is_closed {
        return Err("truncated OSM file: it has no `</osm>`".to_owned());
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
            // The nodes table has the node id as primary key: the first node
            // wins, as the first way does.
            Element::Node(id, position) => {
                self.nodes.entry(id).or_insert(position);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_files_that_are_not_a_whole_osm_element() {
        for (xml, expected) in [
            ("", "no `<osm>` element"),
            ("error: rate limited", "no `<osm>` element"),
            (
                "<html><body>Too many requests</body></html>",
                "root element is `<html>`",
            ),
            (
                r#"<?xml version="1.0"?><osm><node id="1" lat="1" lon="1"/><way id="2"><nd ref="1"/>"#,
                "no `</osm>`",
            ),
            (r#"<osm><way id="2"></osm>"#, "invalid XML"),
        ] {
            let error = parse_osm(xml).unwrap_err();
            assert!(error.contains(expected), "{xml:?}: {error}");
        }
        assert!(parse_osm("<osm/>").is_ok());
    }

    #[test]
    fn keeps_overpass_remarks() {
        let data = parse_osm(
            r#"<osm><node id="1" lat="1" lon="1"/>
            <remark> runtime error: Query timed out in "query" at line 3 &amp; more </remark>
            </osm>"#,
        )
        .unwrap();
        assert_eq!(data.nodes.len(), 1);
        assert_eq!(
            data.remarks,
            [r#"runtime error: Query timed out in "query" at line 3 & more"#]
        );
    }

    #[test]
    fn the_first_node_with_an_id_wins() {
        let data =
            parse_osm(r#"<osm><node id="1" lat="1" lon="2"/><node id="1" lat="3" lon="4"/></osm>"#)
                .unwrap();
        assert_eq!(data.nodes[&1], LonLat::from_degrees(2.0, 1.0));
    }
}
