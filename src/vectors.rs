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
use log::info;
use shapefile::{
    dbase::{FieldValue, Record},
    read_as, Polygon, Polyline,
};
use std::{collections::HashMap, path::Path, time::Instant};

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

/// Classifies the OSM shapefiles in `shapes_path` into ISOM map features.
pub fn get_map_features_from_osm_shapes(
    tile: &Tile,
    shapes_path: &Path,
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

    let multipolygons_path = shapes_path.join("multipolygons.shp");
    let multipolygons = read_as::<_, Polygon, Record>(&multipolygons_path).context(format!(
        "could not read multipolygon shapefile `{}`",
        multipolygons_path.display()
    ))?;

    let mut islands: Vec<Vec<(f32, f32)>> = vec![];

    for (polygon, record) in multipolygons {
        let natural = match record.get("natural") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

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

        let building = match record.get("building") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

        // 521 building
        if building != "" {
            features.push(MapFeature::Area(AreaSymbol::Building521, polygon));
            continue;
        }

        if !skip_520 {
            let landuse = match record.get("landuse") {
                Some(FieldValue::Character(Some(x))) => x,
                Some(_) => "",
                None => "",
            };

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

    let lines_path = shapes_path.join("lines.shp");
    let lines = read_as::<_, Polyline, Record>(&lines_path).context(format!(
        "could not read line shapefile `{}`",
        lines_path.display()
    ))?;

    let mut coastlines: Vec<Vec<(f32, f32)>> = vec![];

    for (line, record) in lines {
        let highway = match record.get("highway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

        let other_tags = get_and_parse_other_tags(&record);
        let tunnel = is_tunnel(&record, &other_tags);

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

        let waterway = match record.get("waterway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

        let should_draw_water_course =
            waterway == "stream" || waterway == "drain" || waterway == "ditch";

        // 304 crossable watercourse
        if should_draw_water_course {
            let itermittent = match record.get("itermittent") {
                Some(FieldValue::Character(Some(x))) => x,
                Some(_) => "",
                None => "",
            };

            let seasonal = match record.get("seasonal") {
                Some(FieldValue::Character(Some(x))) => x,
                Some(_) => "",
                None => "",
            };

            if itermittent != "" || seasonal != "" {
                features.push(MapFeature::Line(
                    LineSymbol::MinorSeasonalWaterChannel306,
                    line,
                ));
            } else {
                features.push(MapFeature::Line(LineSymbol::CrossableWatercourse304, line));
            }

            continue;
        }

        let railway = match record.get("railway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

        // 509 railway
        if railway == "rail" {
            features.push(MapFeature::Line(LineSymbol::Railway509, line));
            continue;
        }

        let power = match other_tags.get("power") {
            Some(p) => p,
            None => "",
        };

        let aerialway = match record.get("aerialway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

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

        let natural = match other_tags.get("natural") {
            Some(p) => p,
            None => "",
        };

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

fn is_tunnel(record: &Record, other_tags: &HashMap<String, String>) -> bool {
    let tunnel = match record.get("tunnel") {
        Some(FieldValue::Character(Some(x))) => x.as_str(),
        Some(_) => "",
        None => "",
    };
    let tunnel = if tunnel == "" {
        match other_tags.get("tunnel") {
            Some(x) => x.as_str(),
            None => "",
        }
    } else {
        tunnel
    };

    return tunnel != "" && tunnel != "no" && tunnel != "false" && tunnel != "0";
}

fn get_and_parse_other_tags(record: &Record) -> HashMap<String, String> {
    let mut other_tags = HashMap::new();

    let raw_other_tags = match record.get("other_tags") {
        Some(FieldValue::Character(Some(x))) => x,
        Some(_) => "",
        None => "",
    };

    for row in raw_other_tags.split(",") {
        let parts: Vec<&str> = row.split("=>").collect();

        if parts.len() != 2 {
            continue;
        }

        let key = parts[0].trim_matches('"');
        let value = parts[1].trim_matches('"');

        other_tags.insert(key.to_string(), value.to_string());
    }

    return other_tags;
}
