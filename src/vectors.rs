use crate::{
    canvas::Canvas,
    config::Config,
    constants::{
        BUILDING_OUTLINE_WIDTH, CROSSABLE_WATERCOURSE_WIDTH, FOOTPATH_DASH_INTERVAL_LENGTH,
        FOOTPATH_DASH_LENGTH, FOOTPATH_WIDTH, INCH, INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH,
        VECTOR_BLACK, VECTOR_BLUE, VECTOR_BUILDING_GRAY,
    },
    tile::Tile,
};
use shapefile::{
    dbase::{FieldValue, Record},
    read_as,
    record::{polygon::GenericPolygon, polyline::GenericPolyline},
    Point, Polygon, Polyline,
};
use std::{
    path::Path,
    process::{Command, ExitStatus},
};

pub fn render_osm_vector_shapes(tile: &Tile, image_width: u32, image_height: u32, config: &Config) {
    println!("Transforming osm file to shapefiles");

    let scale_factor = config.dpi_resolution / INCH;
    let shapes_outlput_path = tile.dir_path.join("shapes");
    let osm_path = Path::new("in").join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));

    let ogr2ogr_output = Command::new("ogr2ogr")
        .args([
            "--config",
            "OSM_USE_CUSTOM_INDEXING",
            "NO",
            "-skipfailures",
            "-f",
            "ESRI Shapefile",
            &shapes_outlput_path.to_str().unwrap(),
            &osm_path.to_str().unwrap(),
            "-t_srs",
            "EPSG:2154",
        ])
        .arg("--quiet")
        .output()
        .expect("failed to execute ogr2ogr command");

    if ExitStatus::success(&ogr2ogr_output.status) {
        println!("{}", String::from_utf8(ogr2ogr_output.stdout).unwrap());
    } else {
        println!("{}", String::from_utf8(ogr2ogr_output.stderr).unwrap());
    }

    println!("Rendering vectors");

    let mut vectors_layer_img = Canvas::new(image_width as i32, image_height as i32);

    let multipolygons_path = shapes_outlput_path.join("multipolygons.shp");
    let multipolygons = read_as::<_, Polygon, Record>(multipolygons_path)
        .expect("Could not open multipolygons shapefile");

    for (polygon, record) in multipolygons {
        let natural = match record.get("natural") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'natural' is not within polygon-dataset"),
        };

        // 301 uncrossable body of water
        if natural == "water" {
            vectors_layer_img = draw_multipolygon(
                polygon,
                vectors_layer_img,
                VECTOR_BLUE,
                VECTOR_BLACK,
                INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH * config.dpi_resolution * 10.0 / INCH,
                tile.min_x,
                tile.min_y,
                image_height,
                scale_factor,
            );

            continue;
        }

        let building = match record.get("building") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'natural' is not within polygon-dataset"),
        };

        // 521 building
        if building == "yes" {
            vectors_layer_img = draw_multipolygon(
                polygon,
                vectors_layer_img,
                VECTOR_BUILDING_GRAY,
                VECTOR_BLACK,
                BUILDING_OUTLINE_WIDTH * config.dpi_resolution * 10.0 / INCH,
                tile.min_x,
                tile.min_y,
                image_height,
                scale_factor,
            );

            continue;
        }
    }

    let lines_path = shapes_outlput_path.join("lines.shp");
    let lines = read_as::<_, Polyline, Record>(lines_path).expect("Could not open lines shapefile");

    for (line, record) in lines {
        let highway = match record.get("highway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'highway' is not within polygon-dataset"),
        };

        // 505 footpath
        if highway == "path" {
            vectors_layer_img = draw_dashed_line(
                line,
                vectors_layer_img,
                VECTOR_BLACK,
                FOOTPATH_WIDTH * config.dpi_resolution * 10.0 / INCH,
                FOOTPATH_DASH_LENGTH * config.dpi_resolution * 10.0 / INCH,
                FOOTPATH_DASH_INTERVAL_LENGTH * config.dpi_resolution * 10.0 / INCH,
                tile.min_x,
                tile.min_y,
                image_height,
                scale_factor,
            );

            continue;
        }

        let waterway = match record.get("waterway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'waterway' is not within polygon-dataset"),
        };

        // 304 crossable watercourse
        if waterway == "stream" {
            vectors_layer_img = draw_line(
                line,
                vectors_layer_img,
                VECTOR_BLUE,
                CROSSABLE_WATERCOURSE_WIDTH * config.dpi_resolution * 10.0 / INCH,
                tile.min_x,
                tile.min_y,
                image_height,
                scale_factor,
            );

            continue;
        }
    }

    let vectors_output_path = tile.dir_path.join("vectors.png");
    vectors_layer_img.save_as(&vectors_output_path.to_str().unwrap());
}

fn draw_multipolygon(
    polygon: GenericPolygon<Point>,
    mut img: Canvas,
    fill_color: (u8, u8, u8),
    stroke_color: (u8, u8, u8),
    stroke_width: f32,
    min_x: i64,
    min_y: i64,
    image_height: u32,
    scale_factor: f32,
) -> Canvas {
    for ring in polygon.rings().iter() {
        let mut points: Vec<(f32, f32)> = vec![];

        for point in ring.points().iter() {
            points.push((
                (point.x as i64 - min_x) as f32 * scale_factor,
                (image_height as f32 - ((point.y as i64 - min_y) as f32 * scale_factor)),
            ))
        }

        img.set_color(fill_color);
        img.draw_filled_polygon(&points);
        img.set_line_width(stroke_width);
        img.set_color(stroke_color);
        img.draw_polyline(&points);
    }

    return img;
}

fn draw_line(
    line: GenericPolyline<Point>,
    mut img: Canvas,
    stroke_color: (u8, u8, u8),
    stroke_width: f32,
    min_x: i64,
    min_y: i64,
    image_height: u32,
    scale_factor: f32,
) -> Canvas {
    for part in line.parts() {
        let mut points: Vec<(f32, f32)> = vec![];

        for point in part {
            points.push((
                (point.x as i64 - min_x) as f32 * scale_factor,
                (image_height as f32 - ((point.y as i64 - min_y) as f32 * scale_factor)),
            ))
        }

        img.set_color(stroke_color);
        img.set_line_width(stroke_width);

        // img.set_dash(
        //     FOOTPATH_DASH_LENGTH * config.dpi_resolution * 10.0 / INCH,
        //     FOOTPATH_DASH_INTERVAL_LENGTH * config.dpi_resolution * 10.0 / INCH,
        // );

        img.draw_polyline(&points);
    }

    return img;
}

fn draw_dashed_line(
    line: GenericPolyline<Point>,
    mut img: Canvas,
    stroke_color: (u8, u8, u8),
    stroke_width: f32,
    interval_on: f32,
    interval_off: f32,
    min_x: i64,
    min_y: i64,
    image_height: u32,
    scale_factor: f32,
) -> Canvas {
    for part in line.parts() {
        let mut points: Vec<(f32, f32)> = vec![];

        for point in part {
            points.push((
                (point.x as i64 - min_x) as f32 * scale_factor,
                (image_height as f32 - ((point.y as i64 - min_y) as f32 * scale_factor)),
            ))
        }

        img.set_color(stroke_color);
        img.set_line_width(stroke_width);
        img.set_dash(interval_on, interval_off);
        img.draw_polyline(&points);
        img.unset_dash();
    }

    return img;
}
