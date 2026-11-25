use crate::{
    coastlines::get_polygon_with_holes_from_coastlines,
    config::Config,
    constants::{BUFFER, COASTLINE_EDGE_BUFFER, INCH},
    error::{Result, ResultContext},
    helpers::does_polyline_intersect_tile,
    map_features::{AreaSymbol, Extent, LineSymbol, MapFeature, PointSymbol},
    map_renderer::MapRenderer,
    tile::Tile,
};
use cassini::osm_import::{read_osm_file, Tag};
use log::info;
use shapefile::{
    dbase::{FieldValue, Record},
    read_as, Polygon, Polyline,
};
use std::{collections::HashMap, path::Path, time::Instant};

// To be replaced by a request struct (A8 in the 2026-11-25 audit).
#[allow(clippy::too_many_arguments)]
pub fn render_map_with_osm_vector_shapes(
    tile: &Tile,
    image_width: u32,
    image_height: u32,
    config: &Config,
    vegetation_path: &Path,
    undergrowth_path: &Path,
    contours_path: &Path,
    cliffs_path: &Path,
    features: &[MapFeature],
) -> Result<()> {
    let start = Instant::now();
    let scale_factor = config.dpi_resolution / INCH;

    let mut map_renderer = MapRenderer::new(
        tile.min_x,
        tile.min_y,
        image_width,
        image_height,
        scale_factor,
        config.dpi_resolution,
        vegetation_path,
        undergrowth_path,
        contours_path,
        cliffs_path,
    )?;

    for feature in features {
        map_renderer = match feature {
            // Cliffs are rasterized into cliffs.png by the cliff renderer
            MapFeature::Point(PointSymbol::LargeBoulder205, _) => map_renderer,
            MapFeature::Line(symbol, line) => match symbol {
                // Contours are rasterized into contours.png by the contour renderer
                LineSymbol::Contour101 | LineSymbol::IndexContour102 | LineSymbol::FormLine103 => {
                    map_renderer
                }
                LineSymbol::UncrossableBodyOfWaterBankLine301_4 => {
                    map_renderer.uncrossable_body_of_water_bank_line_301_4(line)
                }
                LineSymbol::CrossableWatercourse304 => map_renderer.crossable_watercourse_304(line),
                LineSymbol::MinorSeasonalWaterChannel306 => {
                    map_renderer.minor_seasonal_water_channel_306(line)
                }
                LineSymbol::DoubleTrackWideRoad502 { tunnel: false } => {
                    map_renderer.double_track_wide_road_502(line)
                }
                LineSymbol::DoubleTrackWideRoad502 { tunnel: true } => {
                    map_renderer.tunnel_double_track_wide_road_502(line)
                }
                LineSymbol::XxlWideRoad502 { tunnel: false } => {
                    map_renderer.xxl_wide_road_502(line)
                }
                LineSymbol::XxlWideRoad502 { tunnel: true } => {
                    map_renderer.tunnel_xxl_wide_road_502(line)
                }
                LineSymbol::XlWideRoad502 { tunnel: false } => map_renderer.xl_wide_road_502(line),
                LineSymbol::XlWideRoad502 { tunnel: true } => {
                    map_renderer.tunnel_xl_wide_road_502(line)
                }
                LineSymbol::WideRoad502 { tunnel: false } => map_renderer.wide_road_502(line),
                LineSymbol::WideRoad502 { tunnel: true } => map_renderer.tunnel_wide_road_502(line),
                LineSymbol::Road503 { tunnel: false } => map_renderer.road_503(line),
                LineSymbol::Road503 { tunnel: true } => map_renderer.tunnel_road_503(line),
                LineSymbol::Footpath505 { tunnel: false } => map_renderer.footpath_505(line),
                LineSymbol::Footpath505 { tunnel: true } => map_renderer.tunnel_footpath_505(line),
                LineSymbol::Railway509 => map_renderer.railway_509(line),
                LineSymbol::PowerLineCablewayOrSkilift510 => {
                    map_renderer.power_line_cableway_or_skilift_510(line)
                }
                LineSymbol::Cliff202 => map_renderer,
            },
            MapFeature::Area(symbol, polygon) => match symbol {
                AreaSymbol::UncrossableBodyOfWater301 => {
                    map_renderer.uncrossable_body_of_water_301(polygon)
                }
                AreaSymbol::UncrossableBodyOfWaterArea301_1 => {
                    map_renderer.uncrossable_body_of_water_area_301_1(polygon)
                }
                AreaSymbol::Marsh308 => map_renderer.marsh_308(polygon),
                AreaSymbol::AreaThatShallNotBeEntered520 => {
                    map_renderer.area_that_shall_not_be_entered_520(polygon)
                }
                AreaSymbol::Building521 => map_renderer.building_521(polygon),
                // Vegetation is rasterized into vegetation.png by the vegetation renderer
                AreaSymbol::RoughOpenLand403
                | AreaSymbol::VegetationSlowRunning406
                | AreaSymbol::VegetationWalk408
                | AreaSymbol::VegetationWalkGoodVisibility409
                | AreaSymbol::VegetationFight410 => map_renderer,
                AreaSymbol::GiganticBoulderOrRockPillar206 => map_renderer,
            },
        };
    }

    map_renderer.save_as(tile.render_dir_path.join("full-map.png"))?;
    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vectors rendered in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    Ok(())
}

/// The OSM tags of a shape, as the classification reads them.
struct ShapeTags {
    tags: HashMap<String, String>,
    /// Whether a lines shapefile given with `--shapefiles` has an
    /// `intermittent` or `seasonal` column set. The default `osmconf.ini`
    /// has neither, so the `ogr2ogr` shapefiles never drew symbol 306, and
    /// neither do OSM files.
    has_seasonal_column: bool,
}

impl ShapeTags {
    fn from_osm(tags: &[Tag]) -> Self {
        Self {
            tags: tags.iter().cloned().collect(),
            has_seasonal_column: false,
        }
    }

    /// The tags of a record of an `ogr2ogr` shapefile: its text columns,
    /// and the `other_tags` the columns do not hold.
    fn from_record(record: Record) -> Self {
        let mut tags = HashMap::new();
        let mut columns = vec![];
        for (name, value) in record {
            match value {
                FieldValue::Character(Some(value)) if !value.is_empty() => {
                    if name == "other_tags" {
                        tags.extend(parse_other_tags(&value));
                    } else {
                        columns.push((name, value));
                    }
                }
                _ => {}
            }
        }
        let has_seasonal_column = columns
            .iter()
            .any(|(name, _)| name == "intermittent" || name == "seasonal");
        tags.extend(columns);
        Self {
            tags,
            has_seasonal_column,
        }
    }

    /// The value of `key`, empty if there is none.
    fn get(&self, key: &str) -> &str {
        self.tags.get(key).map_or("", String::as_str)
    }
}

/// Classifies the features of the OSM file `osm_path` into ISOM map
/// features.
pub fn get_map_features_from_osm_file(
    tile: &Tile,
    osm_path: &Path,
    skip_520: bool,
) -> Result<Vec<MapFeature>> {
    let layers = read_osm_file(osm_path)?;
    let areas = layers
        .multipolygons
        .iter()
        .map(|area| (area.to_polygon(), ShapeTags::from_osm(&area.tags)));
    let lines = layers
        .lines
        .iter()
        .map(|line| (line.to_polyline(), ShapeTags::from_osm(&line.tags)));
    classify_osm_shapes(tile, areas, lines, skip_520)
}

/// Classifies the `ogr2ogr` shapefiles in `shapes_path`, given with
/// `--shapefiles`, into ISOM map features.
pub fn get_map_features_from_osm_shapefiles(
    tile: &Tile,
    shapes_path: &Path,
    skip_520: bool,
) -> Result<Vec<MapFeature>> {
    let multipolygons_path = shapes_path.join("multipolygons.shp");
    let multipolygons = read_as::<_, Polygon, Record>(&multipolygons_path).context(format!(
        "could not read multipolygon shapefile `{}`",
        multipolygons_path.display()
    ))?;
    let lines_path = shapes_path.join("lines.shp");
    let lines = read_as::<_, Polyline, Record>(&lines_path).context(format!(
        "could not read line shapefile `{}`",
        lines_path.display()
    ))?;

    let areas = multipolygons
        .into_iter()
        .map(|(polygon, record)| (polygon, ShapeTags::from_record(record)));
    let lines = lines
        .into_iter()
        .map(|(line, record)| (line, ShapeTags::from_record(record)));
    classify_osm_shapes(tile, areas, lines, skip_520)
}

fn classify_osm_shapes(
    tile: &Tile,
    areas: impl IntoIterator<Item = (Polygon, ShapeTags)>,
    lines: impl IntoIterator<Item = (Polyline, ShapeTags)>,
    skip_520: bool,
) -> Result<Vec<MapFeature>> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Classifying vectors",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let mut features: Vec<MapFeature> = vec![];
    // Shapefiles may cover much more than the tile, for instance when given
    // with `--shapefiles`
    let extent = Extent {
        min_x: tile.min_x as f64,
        min_y: tile.min_y as f64,
        max_x: tile.max_x as f64,
        max_y: tile.max_y as f64,
    }
    .expanded(BUFFER as f64);

    let mut islands: Vec<Vec<(f32, f32)>> = vec![];

    for (polygon, tags) in areas {
        let natural = tags.get("natural");

        // 308 marsh
        if natural == "wetland" {
            features.push(MapFeature::Area(AreaSymbol::Marsh308, polygon));
            continue;
        }

        // 301 uncrossable body of water
        if natural == "water" {
            features.push(MapFeature::Area(
                AreaSymbol::UncrossableBodyOfWater301,
                polygon,
            ));
            continue;
        }

        if natural == "coastline" {
            let mut points: Vec<(f32, f32)> = vec![];

            for point in polygon.rings()[0].points() {
                points.push((point.x as f32, point.y as f32))
            }

            islands.push(points);
            continue;
        }

        let building = tags.get("building");

        // 521 building
        if !building.is_empty() {
            features.push(MapFeature::Area(AreaSymbol::Building521, polygon));
            continue;
        }

        if !skip_520 {
            let landuse = tags.get("landuse");

            // 520 area that shall not be entered
            if landuse == "residential" || landuse == "railway" || landuse == "industrial" {
                features.push(MapFeature::Area(
                    AreaSymbol::AreaThatShallNotBeEntered520,
                    polygon,
                ));
                continue;
            }
        }
    }

    features.retain(|feature| feature.overlaps(&extent));

    let mut coastlines: Vec<Vec<(f32, f32)>> = vec![];

    for (line, tags) in lines {
        let highway = tags.get("highway");

        let tunnel = is_tunnel(&tags);

        // 502 wide road
        if highway == "motorway" || highway == "motorway_link" {
            features.push(MapFeature::Line(
                LineSymbol::DoubleTrackWideRoad502 { tunnel },
                line,
            ));
            continue;
        }

        if highway == "trunk"
            || highway == "trunk_link"
            || highway == "primary"
            || highway == "primary_link"
        {
            features.push(MapFeature::Line(
                LineSymbol::XxlWideRoad502 { tunnel },
                line,
            ));
            continue;
        }

        if highway == "secondary"
            || highway == "secondary_link"
            || highway == "tertiary"
            || highway == "tertiary_link"
        {
            features.push(MapFeature::Line(LineSymbol::XlWideRoad502 { tunnel }, line));
            continue;
        }

        if highway == "residential"
            || highway == "unclassified"
            || highway == "living_street"
            || highway == "service"
            || highway == "pedestrian"
            || highway == "bus_guideway"
            || highway == "escape"
            || highway == "road"
            || highway == "busway"
        {
            features.push(MapFeature::Line(LineSymbol::WideRoad502 { tunnel }, line));
            continue;
        }

        // 503 road
        if highway == "track" || highway == "cycleway" {
            features.push(MapFeature::Line(LineSymbol::Road503 { tunnel }, line));
            continue;
        }

        // 505 footpath
        if highway == "footway"
            || highway == "bridleway"
            || highway == "steps"
            || highway == "path"
            || highway == "footpath"
        {
            features.push(MapFeature::Line(LineSymbol::Footpath505 { tunnel }, line));
            continue;
        }

        let waterway = tags.get("waterway");

        let should_draw_water_course =
            waterway == "stream" || waterway == "drain" || waterway == "ditch";

        // 304 crossable watercourse
        if should_draw_water_course {
            if tags.has_seasonal_column {
                features.push(MapFeature::Line(
                    LineSymbol::MinorSeasonalWaterChannel306,
                    line,
                ));
            } else {
                features.push(MapFeature::Line(LineSymbol::CrossableWatercourse304, line));
            }

            continue;
        }

        let railway = tags.get("railway");

        // 509 railway
        if railway == "rail" {
            features.push(MapFeature::Line(LineSymbol::Railway509, line));
            continue;
        }

        let power = tags.get("power");

        let aerialway = tags.get("aerialway");

        // 510 power line, cableway or skilift
        if power == "minor_line"
            || aerialway == "cable_car"
            || aerialway == "gondola"
            || aerialway == "mixed_lift"
            || aerialway == "chair_lift"
            || aerialway == "drag_lift"
            || aerialway == "t-bar"
            || aerialway == "j-bar"
            || aerialway == "platter"
        {
            features.push(MapFeature::Line(
                LineSymbol::PowerLineCablewayOrSkilift510,
                line,
            ));
            continue;
        }

        // 511 major power line
        if power == "line" {
            features.push(MapFeature::Line(
                LineSymbol::PowerLineCablewayOrSkilift510,
                line,
            ));
            continue;
        }

        let natural = tags.get("natural");

        if natural == "coastline" {
            let mut points: Vec<(f32, f32)> = vec![];

            for point in line.parts()[0].clone() {
                points.push((point.x as f32, point.y as f32));
            }

            coastlines.push(points);
            continue;
        }
    }

    features.retain(|feature| feature.overlaps(&extent));

    let are_some_islands_inside_tile = islands.iter().any(|island| {
        does_polyline_intersect_tile(
            island,
            tile.min_x as f32 - COASTLINE_EDGE_BUFFER,
            tile.min_y as f32 - COASTLINE_EDGE_BUFFER,
            tile.max_x as f32 + COASTLINE_EDGE_BUFFER,
            tile.max_y as f32 + COASTLINE_EDGE_BUFFER,
        )
    });

    let are_some_coastlines_inside_tile = coastlines.iter().any(|coastline| {
        does_polyline_intersect_tile(
            coastline,
            tile.min_x as f32 - COASTLINE_EDGE_BUFFER,
            tile.min_y as f32 - COASTLINE_EDGE_BUFFER,
            tile.max_x as f32 + COASTLINE_EDGE_BUFFER,
            tile.max_y as f32 + COASTLINE_EDGE_BUFFER,
        )
    });

    if are_some_islands_inside_tile || are_some_coastlines_inside_tile {
        let (coastlines_polygons, coastlines_edges) = get_polygon_with_holes_from_coastlines(
            coastlines, islands, tile.min_x, tile.min_y, tile.max_x, tile.max_y,
        )?;

        for coastline_polygon in coastlines_polygons {
            features.push(MapFeature::Area(
                AreaSymbol::UncrossableBodyOfWaterArea301_1,
                coastline_polygon,
            ));
        }

        for coastlines_edge in coastlines_edges {
            features.push(MapFeature::Line(
                LineSymbol::UncrossableBodyOfWaterBankLine301_4,
                coastlines_edge,
            ));
        }
    }

    Ok(features)
}

fn is_tunnel(tags: &ShapeTags) -> bool {
    let tunnel = tags.get("tunnel");
    !tunnel.is_empty() && tunnel != "no" && tunnel != "false" && tunnel != "0"
}

/// The `"key"=>"value"` pairs of an `other_tags` value.
fn parse_other_tags(raw_other_tags: &str) -> HashMap<String, String> {
    let mut other_tags = HashMap::new();

    for row in raw_other_tags.split(",") {
        let parts: Vec<&str> = row.split("=>").collect();

        if parts.len() != 2 {
            continue;
        }

        let key = parts[0].trim_matches('"');
        let value = parts[1].trim_matches('"');

        other_tags.insert(key.to_string(), value.to_string());
    }

    other_tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::wgs84_to_lambert93;
    use cassini::osm_import::convert_osm_file_to_shapefiles;
    use std::{collections::HashMap, fs};

    /// A square of nodes 1–4, a smaller one of nodes 5–8 inside it, and ways
    /// of each symbol along them.
    const OSM: &str = r#"<?xml version="1.0" encoding="UTF-8"?><osm version="0.6">
        <way id="1"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/><tag k="building" v="yes"/></way>
        <way id="2"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="natural" v="wetland"/></way>
        <way id="3"><nd ref="1"/><nd ref="3"/><tag k="highway" v="motorway"/><tag k="tunnel" v="yes"/></way>
        <way id="4"><nd ref="2"/><nd ref="4"/><tag k="waterway" v="stream"/><tag k="seasonal" v="yes"/></way>
        <way id="5"><nd ref="1"/><nd ref="2"/><tag k="railway" v="rail"/></way>
        <way id="6"><nd ref="3"/><nd ref="4"/><tag k="power" v="line"/></way>
        <way id="7"><nd ref="1"/><nd ref="2"/><nd ref="3"/></way>
        <way id="8"><nd ref="3"/><nd ref="4"/><nd ref="1"/></way>
        <relation id="9"><member type="way" ref="7" role="outer"/><member type="way" ref="8" role="outer"/>
            <tag k="type" v="multipolygon"/><tag k="natural" v="water"/></relation>
        <node id="1" lat="45.10" lon="5.70"/><node id="2" lat="45.11" lon="5.70"/>
        <node id="3" lat="45.11" lon="5.71"/><node id="4" lat="45.10" lon="5.71"/>
        <node id="5" lat="45.102" lon="5.702"/><node id="6" lat="45.108" lon="5.702"/>
        <node id="7" lat="45.108" lon="5.708"/><node id="8" lat="45.102" lon="5.708"/>
        </osm>"#;

    #[test]
    fn osm_files_and_their_shapefiles_give_the_same_features() {
        let dir = tempfile::tempdir().unwrap();
        let osm_path = dir.path().join("tile.osm");
        fs::write(&osm_path, OSM).unwrap();
        let shapes_path = dir.path().join("shapes");
        convert_osm_file_to_shapefiles(&osm_path, &shapes_path).unwrap();

        let (min_x, min_y) = wgs84_to_lambert93(5.70, 45.10);
        let (max_x, max_y) = wgs84_to_lambert93(5.71, 45.11);
        let tile = Tile {
            lidar_dir_path: dir.path().to_path_buf(),
            render_dir_path: dir.path().to_path_buf(),
            min_x: min_x as i64,
            min_y: min_y as i64,
            max_x: max_x as i64 + 1,
            max_y: max_y as i64 + 1,
        };
        let from_osm = get_map_features_from_osm_file(&tile, &osm_path, false).unwrap();
        let from_shapefiles =
            get_map_features_from_osm_shapefiles(&tile, &shapes_path, false).unwrap();

        assert_eq!(from_osm, from_shapefiles);
        let symbols: Vec<String> = from_osm
            .iter()
            .map(|feature| match feature {
                MapFeature::Point(symbol, _) => format!("{symbol:?}"),
                MapFeature::Line(symbol, _) => format!("{symbol:?}"),
                MapFeature::Area(symbol, _) => format!("{symbol:?}"),
            })
            .collect();
        // The stream is no 306: the `seasonal` tag is no column.
        assert_eq!(
            symbols,
            [
                "UncrossableBodyOfWater301",
                "Building521",
                "Marsh308",
                "DoubleTrackWideRoad502 { tunnel: true }",
                "CrossableWatercourse304",
                "Railway509",
                "PowerLineCablewayOrSkilift510",
            ]
        );
    }

    #[test]
    fn shapefile_records_read_their_columns_then_other_tags() {
        let text = |value: &str| FieldValue::Character(Some(value.to_owned()));
        let record = Record::from(HashMap::from([
            ("highway".to_owned(), text("track")),
            ("waterway".to_owned(), FieldValue::Character(None)),
            ("intermittent".to_owned(), text("yes")),
            (
                "other_tags".to_owned(),
                text(r#""tunnel"=>"yes","highway"=>"path""#),
            ),
        ]));
        let tags = ShapeTags::from_record(record);

        assert_eq!(tags.get("highway"), "track");
        assert_eq!(tags.get("tunnel"), "yes");
        assert_eq!(tags.get("waterway"), "");
        assert!(tags.has_seasonal_column);
        assert!(is_tunnel(&tags));
    }
}
