use crate::{
    coastlines::get_polygon_with_holes_from_coastlines, config::Config, constants::INCH,
    helpers::remove_dir_content, map_renderer::MapRenderer, tile::Tile,
};
use log::{error, info};
use shapefile::{
    dbase::{FieldValue, Record},
    read_as, Polygon, Polyline,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Instant,
};

pub fn render_map_with_osm_vector_shapes(
    tile: &Tile,
    image_width: u32,
    image_height: u32,
    config: &Config,
    vegetation_path: &PathBuf,
    contours_path: &PathBuf,
    cliffs_path: &PathBuf,
    skip_520: bool,
) {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Transforming osm file to shapefiles",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let scale_factor = config.dpi_resolution / INCH;
    let shapes_outlput_path = tile.render_dir_path.join("shapes");

    if shapes_outlput_path.exists() {
        remove_dir_content(&shapes_outlput_path).unwrap();
    }

    let osm_path = Path::new("osm").join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));

    let ogr2ogr_output = Command::new("ogr2ogr")
        .args([
            "--config",
            "OSM_USE_CUSTOM_INDEXING",
            "NO",
            "-f",
            "ESRI Shapefile",
            &shapes_outlput_path.to_str().unwrap(),
            &osm_path.to_str().unwrap(),
            "-t_srs",
            "EPSG:2154",
            "-nlt",
            "MULTIPOLYGON",
            "-sql",
            "SELECT * FROM multipolygons",
        ])
        .arg("--quiet")
        .output()
        .expect("failed to execute ogr2ogr command");

    if !ExitStatus::success(&ogr2ogr_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Ogr2ogr command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(ogr2ogr_output.stderr).unwrap()
        );
    }

    let ogr2ogr_output = Command::new("ogr2ogr")
        .args([
            "--config",
            "OSM_USE_CUSTOM_INDEXING",
            "NO",
            "-f",
            "ESRI Shapefile",
            &shapes_outlput_path.to_str().unwrap(),
            &osm_path.to_str().unwrap(),
            "-t_srs",
            "EPSG:2154",
            "-nlt",
            "LINESTRING",
            "-sql",
            "SELECT * FROM lines",
        ])
        .arg("--quiet")
        .output()
        .expect("failed to execute ogr2ogr command");

    if !ExitStatus::success(&ogr2ogr_output.status) {
        error!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Ogr2ogr command failed {:?}",
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            String::from_utf8(ogr2ogr_output.stderr).unwrap()
        );
    }

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Osm files transformed to shapefiles in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering vectors",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let multipolygons_path = shapes_outlput_path.join("multipolygons.shp");
    let multipolygons =
        read_as::<_, Polygon, Record>(&multipolygons_path).expect("Could not open multipolygons shapefile");

    let mut map_renderer = MapRenderer::new(
        tile.min_x,
        tile.min_y,
        image_width,
        image_height,
        scale_factor,
        config.dpi_resolution,
        vegetation_path,
        contours_path,
        cliffs_path,
    );

    let mut islands: Vec<Vec<(f32, f32)>> = vec![];

    for (polygon, record) in multipolygons {
        let natural = match record.get("natural") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

        // 308 marsh
        if natural == "wetland" {
            map_renderer = map_renderer.marsh_308(&polygon);
            continue;
        }

        // 301 uncrossable body of water
        if natural == "water" {
            map_renderer = map_renderer.uncrossable_body_of_water_301(&polygon);
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
            map_renderer = map_renderer.building_521(&polygon);
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
                map_renderer = map_renderer.area_that_shall_not_be_entered_520(&polygon);
                continue;
            }
        }
    }

    let lines_path = shapes_outlput_path.join("lines.shp");
    let lines = read_as::<_, Polyline, Record>(lines_path).expect("Could not open lines shapefile");

    let mut coastlines: Vec<Vec<(f32, f32)>> = vec![];

    for (line, record) in lines {
        let highway = match record.get("highway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

        // 502 wide road
        if highway == "motorway" || highway == "motorway_link" {
            map_renderer = map_renderer.double_track_wide_road_502(&line);
            continue;
        }

        if highway == "trunk" || highway == "trunk_link" || highway == "primary" || highway == "primary_link"
        {
            map_renderer = map_renderer.xxl_wide_road_502(&line);
            continue;
        }

        if highway == "secondary"
            || highway == "secondary_link"
            || highway == "tertiary"
            || highway == "tertiary_link"
        {
            map_renderer = map_renderer.xl_wide_road_502(&line);
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
            map_renderer = map_renderer.wide_road_502(&line);
            continue;
        }

        // 503 road
        if highway == "track" || highway == "cycleway" {
            map_renderer = map_renderer.road_503(&line);
            continue;
        }

        // 505 footpath
        if highway == "footway"
            || highway == "bridleway"
            || highway == "steps"
            || highway == "path"
            || highway == "footpath"
        {
            map_renderer = map_renderer.footpath_505(&line);
            continue;
        }

        let waterway = match record.get("waterway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => "",
        };

        let should_draw_water_course = waterway == "stream" || waterway == "drain" || waterway == "ditch";

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
                map_renderer = map_renderer.minor_seasonal_water_channel_306(&line);
            } else {
                map_renderer = map_renderer.crossable_watercourse_304(&line);
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
            map_renderer = map_renderer.railway_509(&line);
            continue;
        }

        let other_tags = get_and_parse_other_tags(&record);

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
            map_renderer = map_renderer.power_line_cableway_or_skilift_510(&line);
            continue;
        }

        // 511 major power line
        if power == "line" {
            map_renderer = map_renderer.power_line_cableway_or_skilift_510(&line);
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

    if coastlines.len() != 0 || islands.len() != 0 {
        let (coastlines_polygons, coastlines_edges) = get_polygon_with_holes_from_coastlines(
            coastlines, islands, tile.min_x, tile.min_y, tile.max_x, tile.max_y,
        );

        for coastline_polygon in coastlines_polygons {
            map_renderer = map_renderer.uncrossable_body_of_water_area_301_1(&coastline_polygon);
        }

        for coastlines_edge in coastlines_edges {
            map_renderer = map_renderer.uncrossable_body_of_water_bank_line_301_4(&coastlines_edge);
        }
    }

    map_renderer.save_as(tile.render_dir_path.join("full-map.png"));

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vectors rendered in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
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
