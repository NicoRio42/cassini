//! The `lines.shp` and `multipolygons.shp` files `ogr2ogr` wrote: OGR's
//! columns, `other_tags` and `z_order` (`OGROSMLayer`), in a UTF-8 `.dbf`.

use std::{fs, io::Write, path::Path};

use log::debug;
use shapefile::dbase::{
    FieldIOError, FieldName, FieldValue, FieldWriter, TableWriterBuilder, WritableRecord,
};

use super::{AreaFeature, AreaSource, LineFeature, Tag};
use crate::{
    error::{Result, ResultContext},
    shapefile_output::write_shapefile,
};

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

/// The width OGR gives a string field, and the most the dBase format allows.
const DEFAULT_STRING_WIDTH: usize = 80;
const MAX_STRING_WIDTH: usize = 254;

/// The width of the `z_order` integer field.
const Z_ORDER_WIDTH: u8 = 9;

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

pub(super) fn write_lines(path: &Path, lines: &[LineFeature]) -> Result<()> {
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

    let shapes = lines.iter().map(LineFeature::to_polyline);
    write_shapefile(path, table, shapes.zip(records))?;
    write_code_page(path)
}

pub(super) fn write_multipolygons(path: &Path, areas: &[AreaFeature]) -> Result<()> {
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

    let shapes = areas.iter().map(AreaFeature::to_polygon);
    write_shapefile(path, table, shapes.zip(records))?;
    write_code_page(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        osm_import::{convert_osm_file_to_shapefiles, test_support::tags},
        projection::wgs84_to_lambert93,
    };
    use shapefile::{dbase::Record, Polygon, PolygonRing, Polyline};

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
    fn writes_the_shapefiles_that_ogr2ogr_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let osm_path = dir.path().join("tile.osm");
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
        let shapes_dir = dir.path().join("shapes");
        convert_osm_file_to_shapefiles(&osm_path, &shapes_dir).unwrap();

        let lines =
            shapefile::read_as::<_, Polyline, Record>(shapes_dir.join("lines.shp")).unwrap();
        let multipolygons =
            shapefile::read_as::<_, Polygon, Record>(shapes_dir.join("multipolygons.shp")).unwrap();
        let cpg = fs::read_to_string(shapes_dir.join("lines.cpg")).unwrap();

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
