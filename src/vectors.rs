use crate::{
    canvas::Canvas,
    config::Config,
    constants::{
        BUILDING_OUTLINE_WIDTH, CROSSABLE_WATERCOURSE_WIDTH, FOOTPATH_DASH_INTERVAL_LENGTH,
        FOOTPATH_DASH_LENGTH, FOOTPATH_WIDTH, INCH, INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH,
        MARSH_LINE_SPACING, MARSH_LINE_WIDTH, ROAD_WIDTH, VECTOR_BLACK, VECTOR_BLUE,
        VECTOR_BUILDING_GRAY, VECTOR_OLIVE_GREEN, VECTOR_PAVED_AREA_BROWN, WIDE_ROAD_INNER_WIDTH,
        WIDE_ROAD_OUTER_WIDTH, XL_WIDE_ROAD_INNER_WIDTH, XL_WIDE_ROAD_OUTER_WIDTH,
        XXL_WIDE_ROAD_INNER_WIDTH, XXL_WIDE_ROAD_OUTER_WIDTH,
    },
    tile::Tile,
};
use log::{error, info};
use shapefile::{
    dbase::{FieldValue, Record},
    read_as,
    record::{polygon::GenericPolygon, polyline::GenericPolyline},
    Point, Polygon, PolygonRing, Polyline,
};
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Instant,
};

pub fn render_osm_vector_shapes(tile: &Tile, image_width: u32, image_height: u32, config: &Config) {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Transforming osm file to shapefiles",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let scale_factor = config.dpi_resolution / INCH;
    let shapes_outlput_path = tile.render_dir_path.join("shapes");
    let osm_path = Path::new("osm").join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));

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
    let multipolygons = read_as::<_, Polygon, Record>(multipolygons_path)
        .expect("Could not open multipolygons shapefile");

    let mut map_renderer = MapRenderer::new(
        tile.min_x,
        tile.min_y,
        image_width,
        image_height,
        scale_factor,
        config.dpi_resolution,
    );

    for (polygon, record) in multipolygons {
        let natural = match record.get("natural") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'natural' is not within polygon-dataset"),
        };

        // 308 marsh
        if natural == "wetland" {
            map_renderer = map_renderer.marsh_308(polygon);
            continue;
        }

        // 301 uncrossable body of water
        if natural == "water" {
            map_renderer = map_renderer.uncrossable_body_of_water_301(polygon);
            continue;
        }

        let building = match record.get("building") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'building' is not within polygon-dataset"),
        };

        // 521 building
        if building != "" {
            map_renderer = map_renderer.building_521(polygon);
            continue;
        }

        let landuse = match record.get("landuse") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'landuse' is not within polygon-dataset"),
        };

        // 520 area that shall not be entered
        if landuse == "residential" {
            map_renderer = map_renderer.area_that_shall_not_be_entered_520(polygon);
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

        // 502 wide road
        if highway == "motorway"
            || highway == "trunk"
            || highway == "motorway_link"
            || highway == "trunk_link"
        {
            map_renderer = map_renderer.xxl_wide_road_502(&line);
            continue;
        }

        if highway == "primary"
            || highway == "secondary"
            || highway == "primary_link"
            || highway == "secondary_link"
        {
            map_renderer = map_renderer.xl_wide_road_502(&line);
            continue;
        }

        if highway == "tertiary"
            || highway == "residential"
            || highway == "unclassified"
            || highway == "living_street"
            || highway == "service"
            || highway == "pedestrian"
            || highway == "bus_guideway"
            || highway == "escape"
            || highway == "road"
            || highway == "busway"
            || highway == "tertiary_link"
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
            map_renderer = map_renderer.footpath_505(line);
            continue;
        }

        let waterway = match record.get("waterway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'waterway' is not within polygon-dataset"),
        };

        // 304 crossable watercourse
        if waterway == "stream" {
            map_renderer = map_renderer.crossable_watercourse_304(&line);
            continue;
        }
    }

    map_renderer.save_as(tile.render_dir_path.join("vectors.png"));

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vectors rendered in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}

struct MapRenderer {
    blue_img: Canvas,
    black_img: Canvas,
    olive_green_img: Canvas,
    light_brown_img: Canvas,
    striped_blue_img: Canvas,
    min_x: i64,
    min_y: i64,
    image_width: u32,
    image_height: u32,
    scale_factor: f32,
    dpi_resolution: f32,
}

impl MapRenderer {
    pub fn new(
        min_x: i64,
        min_y: i64,
        image_width: u32,
        image_height: u32,
        scale_factor: f32,
        dpi_resolution: f32,
    ) -> MapRenderer {
        return MapRenderer {
            blue_img: Canvas::new(image_width as i32, image_height as i32),
            black_img: Canvas::new(image_width as i32, image_height as i32),
            olive_green_img: Canvas::new(image_width as i32, image_height as i32),
            light_brown_img: Canvas::new(image_width as i32, image_height as i32),
            striped_blue_img: Canvas::new(image_width as i32, image_height as i32),
            min_x,
            min_y,
            image_width,
            image_height,
            scale_factor,
            dpi_resolution,
        };
    }

    #[inline]
    fn uncrossable_body_of_water_301(mut self, polygon: GenericPolygon<Point>) -> MapRenderer {
        for ring in polygon.rings().iter() {
            let points = self.get_points_from_polygon_ring(ring);
            self.blue_img.set_color(VECTOR_BLUE);
            self.blue_img.draw_filled_polygon(&points);

            self.black_img.set_line_width(
                INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH * self.dpi_resolution * 10.0 / INCH,
            );

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn building_521(mut self, polygon: GenericPolygon<Point>) -> MapRenderer {
        for ring in polygon.rings().iter() {
            let points = self.get_points_from_polygon_ring(ring);
            self.black_img.set_color(VECTOR_BUILDING_GRAY);
            self.black_img.draw_filled_polygon(&points);

            self.black_img
                .set_line_width(BUILDING_OUTLINE_WIDTH * self.dpi_resolution * 10.0 / INCH);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn area_that_shall_not_be_entered_520(mut self, polygon: GenericPolygon<Point>) -> MapRenderer {
        for ring in polygon.rings().iter() {
            let points = self.get_points_from_polygon_ring(ring);
            self.olive_green_img.set_color(VECTOR_OLIVE_GREEN);
            self.olive_green_img.draw_filled_polygon(&points);
        }

        return self;
    }

    #[inline]
    fn marsh_308(mut self, polygon: GenericPolygon<Point>) -> MapRenderer {
        for ring in polygon.rings().iter() {
            let points = self.get_points_from_polygon_ring(ring);
            self.striped_blue_img.set_color(VECTOR_BLUE);
            self.striped_blue_img.draw_filled_polygon(&points);
        }

        return self;
    }

    #[inline]
    fn wide_road(
        mut self,
        line: &GenericPolyline<Point>,
        inner_width: f32,
        outer_width: f32,
    ) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);
            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(outer_width * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
            self.light_brown_img.set_color(VECTOR_PAVED_AREA_BROWN);
            self.light_brown_img
                .set_line_width(inner_width * self.dpi_resolution * 10.0 / INCH);
            self.light_brown_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        return self.wide_road(line, WIDE_ROAD_INNER_WIDTH, WIDE_ROAD_OUTER_WIDTH);
    }

    #[inline]
    fn xl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        return self.wide_road(line, XL_WIDE_ROAD_INNER_WIDTH, XL_WIDE_ROAD_OUTER_WIDTH);
    }

    #[inline]
    fn xxl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        return self.wide_road(line, XXL_WIDE_ROAD_INNER_WIDTH, XXL_WIDE_ROAD_OUTER_WIDTH);
    }

    #[inline]
    fn road_503(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(ROAD_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn crossable_watercourse_304(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLUE);
            self.black_img
                .set_line_width(CROSSABLE_WATERCOURSE_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn footpath_505(mut self, line: GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(FOOTPATH_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.set_dash(
                FOOTPATH_DASH_LENGTH * self.dpi_resolution * 10.0 / INCH,
                FOOTPATH_DASH_INTERVAL_LENGTH * self.dpi_resolution * 10.0 / INCH,
            );
            self.black_img.draw_polyline(&points);
            self.black_img.unset_dash();
        }

        return self;
    }

    #[inline]
    fn get_points_from_line_part(&self, line_part: &Vec<Point>) -> Vec<(f32, f32)> {
        let mut points: Vec<(f32, f32)> = vec![];

        for point in line_part {
            points.push((
                (point.x as i64 - self.min_x) as f32 * self.scale_factor,
                (self.image_height as f32
                    - ((point.y as i64 - self.min_y) as f32 * self.scale_factor)),
            ))
        }

        return points;
    }

    #[inline]
    fn get_points_from_polygon_ring(&self, ring: &PolygonRing<Point>) -> Vec<(f32, f32)> {
        let mut points: Vec<(f32, f32)> = vec![];

        for point in ring.points().iter() {
            points.push((
                (point.x as i64 - self.min_x) as f32 * self.scale_factor,
                (self.image_height as f32
                    - ((point.y as i64 - self.min_y) as f32 * self.scale_factor)),
            ))
        }

        return points;
    }

    #[inline]
    fn save_as(mut self, path: PathBuf) {
        let pixel_marsh_interval =
            (MARSH_LINE_WIDTH + MARSH_LINE_SPACING) * self.dpi_resolution * 10.0 / INCH;

        let number_of_stripes = self.image_height / pixel_marsh_interval as u32;
        self.striped_blue_img.set_transparent_color();

        for i in 0..number_of_stripes {
            let min_y = i as f32 * pixel_marsh_interval;

            let max_y = i as f32 * pixel_marsh_interval
                + MARSH_LINE_SPACING * self.dpi_resolution * 10.0 / INCH;

            self.striped_blue_img.draw_filled_polygon(&vec![
                (0., min_y),
                (self.image_width as f32, min_y),
                (self.image_width as f32, max_y),
                (0. as f32, max_y),
                (0. as f32, min_y),
            ])
        }

        self.olive_green_img.overlay(&mut self.blue_img, 0., 0.);
        self.olive_green_img
            .overlay(&mut self.striped_blue_img, 0., 0.);
        self.olive_green_img.overlay(&mut self.black_img, 0., 0.);
        self.olive_green_img
            .overlay(&mut self.light_brown_img, 0., 0.);
        self.olive_green_img.save_as(path.to_str().unwrap());
    }
}
