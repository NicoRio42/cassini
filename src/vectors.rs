use crate::{
    canvas::Canvas,
    config::Config,
    constants::{
        BUILDING_OUTLINE_WIDTH, CROSSABLE_WATERCOURSE_WIDTH, FOOTPATH_DASH_INTERVAL_LENGTH,
        FOOTPATH_DASH_LENGTH, FOOTPATH_WIDTH, INCH, INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH,
        MARSH_LINE_SPACING, MARSH_LINE_WIDTH, ROAD_WIDTH, VECTOR_BLACK, VECTOR_BLUE,
        VECTOR_BUILDING_GRAY,
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
    io::{stdout, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Instant,
};

pub fn render_osm_vector_shapes(tile: &Tile, image_width: u32, image_height: u32, config: &Config) {
    print!("Transforming osm file to shapefiles");
    let _ = stdout().flush();
    let start = Instant::now();

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

    if !ExitStatus::success(&ogr2ogr_output.status) {
        println!("{}", String::from_utf8(ogr2ogr_output.stderr).unwrap());
    }

    let duration = start.elapsed();
    println!(" -> Done in {:.1?}", duration);

    print!("Rendering vectors");
    let _ = stdout().flush();
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
            map_renderer = map_renderer.draw_striped_multipolygon(polygon, VECTOR_BLUE);
            continue;
        }

        // 301 uncrossable body of water
        if natural == "water" {
            map_renderer = map_renderer.draw_multipolygon_with_border(
                polygon,
                VECTOR_BLUE,
                VECTOR_BLACK,
                INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH,
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
            map_renderer = map_renderer.draw_multipolygon_with_border(
                polygon,
                VECTOR_BUILDING_GRAY,
                VECTOR_BLACK,
                BUILDING_OUTLINE_WIDTH,
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
        if highway == "path" || highway == "unclassified" {
            map_renderer = map_renderer.draw_dashed_line(
                line,
                VECTOR_BLACK,
                FOOTPATH_WIDTH,
                FOOTPATH_DASH_LENGTH,
                FOOTPATH_DASH_INTERVAL_LENGTH,
            );

            continue;
        }

        // 503 road
        if highway == "track" {
            map_renderer = map_renderer.draw_line(line, VECTOR_BLACK, ROAD_WIDTH);
            continue;
        }

        let waterway = match record.get("waterway") {
            Some(FieldValue::Character(Some(x))) => x,
            Some(_) => "",
            None => panic!("Field 'waterway' is not within polygon-dataset"),
        };

        // 304 crossable watercourse
        if waterway == "stream" {
            map_renderer = map_renderer.draw_line(line, VECTOR_BLUE, CROSSABLE_WATERCOURSE_WIDTH);
            continue;
        }
    }

    map_renderer.save_as(tile.dir_path.join("vectors.png"));

    let duration = start.elapsed();
    println!(" -> Done in {:.1?}", duration);
}

struct MapRenderer {
    img: Canvas,
    striped_img: Canvas,
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
            img: Canvas::new(image_width as i32, image_height as i32),
            striped_img: Canvas::new(image_width as i32, image_height as i32),
            min_x,
            min_y,
            image_width,
            image_height,
            scale_factor,
            dpi_resolution,
        };
    }

    #[inline]
    fn draw_multipolygon_with_border(
        mut self,
        polygon: GenericPolygon<Point>,
        fill_color: (u8, u8, u8),
        stroke_color: (u8, u8, u8),
        stroke_width: f32,
    ) -> MapRenderer {
        for ring in polygon.rings().iter() {
            let mut points: Vec<(f32, f32)> = vec![];

            for point in ring.points().iter() {
                points.push((
                    (point.x as i64 - self.min_x) as f32 * self.scale_factor,
                    (self.image_height as f32
                        - ((point.y as i64 - self.min_y) as f32 * self.scale_factor)),
                ))
            }

            self.img.set_color(fill_color);
            self.img.draw_filled_polygon(&points);
            self.img
                .set_line_width(stroke_width * self.dpi_resolution * 10.0 / INCH);
            self.img.set_color(stroke_color);
            self.img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn draw_striped_multipolygon(
        mut self,
        polygon: GenericPolygon<Point>,
        fill_color: (u8, u8, u8),
    ) -> MapRenderer {
        for ring in polygon.rings().iter() {
            let mut points: Vec<(f32, f32)> = vec![];

            for point in ring.points().iter() {
                points.push((
                    (point.x as i64 - self.min_x) as f32 * self.scale_factor,
                    (self.image_height as f32
                        - ((point.y as i64 - self.min_y) as f32 * self.scale_factor)),
                ))
            }

            self.striped_img.set_color(fill_color);
            self.striped_img.draw_filled_polygon(&points);
        }

        return self;
    }

    #[inline]
    fn draw_line(
        mut self,
        line: GenericPolyline<Point>,
        stroke_color: (u8, u8, u8),
        stroke_width: f32,
    ) -> MapRenderer {
        for part in line.parts() {
            let mut points: Vec<(f32, f32)> = vec![];

            for point in part {
                points.push((
                    (point.x as i64 - self.min_x) as f32 * self.scale_factor,
                    (self.image_height as f32
                        - ((point.y as i64 - self.min_y) as f32 * self.scale_factor)),
                ))
            }

            self.img.set_color(stroke_color);
            self.img
                .set_line_width(stroke_width * self.dpi_resolution * 10.0 / INCH);
            self.img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn draw_dashed_line(
        mut self,
        line: GenericPolyline<Point>,
        stroke_color: (u8, u8, u8),
        stroke_width: f32,
        interval_on: f32,
        interval_off: f32,
    ) -> MapRenderer {
        for part in line.parts() {
            let mut points: Vec<(f32, f32)> = vec![];

            for point in part {
                points.push((
                    (point.x as i64 - self.min_x) as f32 * self.scale_factor,
                    (self.image_height as f32
                        - ((point.y as i64 - self.min_y) as f32 * self.scale_factor)),
                ))
            }

            self.img.set_color(stroke_color);
            self.img
                .set_line_width(stroke_width * self.dpi_resolution * 10.0 / INCH);
            self.img.set_dash(
                interval_on * self.dpi_resolution * 10.0 / INCH,
                interval_off * self.dpi_resolution * 10.0 / INCH,
            );
            self.img.draw_polyline(&points);
            self.img.unset_dash();
        }

        return self;
    }

    #[inline]
    fn save_as(mut self, path: PathBuf) {
        let pixel_marsh_interval =
            (MARSH_LINE_WIDTH + MARSH_LINE_SPACING) * self.dpi_resolution * 10.0 / INCH;

        let number_of_stripes = self.image_height / pixel_marsh_interval as u32;
        self.striped_img.set_transparent_color();

        for i in 0..number_of_stripes {
            let min_y = i as f32 * pixel_marsh_interval;

            let max_y = i as f32 * pixel_marsh_interval
                + MARSH_LINE_SPACING * self.dpi_resolution * 10.0 / INCH;

            self.striped_img.draw_filled_polygon(&vec![
                (0., min_y),
                (self.image_width as f32, min_y),
                (self.image_width as f32, max_y),
                (0. as f32, max_y),
                (0. as f32, min_y),
            ])
        }

        self.img.overlay(&mut self.striped_img, 0., 0.);
        self.img.save_as(path.to_str().unwrap());
    }
}
