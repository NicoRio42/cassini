use std::path::PathBuf;

use crate::{
    canvas::Canvas,
    constants::{
        BUILDING_OUTLINE_WIDTH, CROSSABLE_WATERCOURSE_WIDTH, DOUBLE_TRACK_WIDE_ROAD_INNER_WIDTH,
        DOUBLE_TRACK_WIDE_ROAD_OUTER_WIDTH, FOOTPATH_DASH_INTERVAL_LENGTH, FOOTPATH_DASH_LENGTH,
        FOOTPATH_WIDTH, INCH, INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH, MARSH_LINE_SPACING, MARSH_LINE_WIDTH,
        MINOR_WATERCOURSE_DASH_INTERVAL_LENGTH, MINOR_WATERCOURSE_DASH_LENGTH, MINOR_WATERCOURSE_WIDTH,
        POWERLINE_WIDTH, RAILWAY_DASH_INTERVAL_LENGTH, RAILWAY_DASH_LENGTH, RAILWAY_INNER_WIDTH,
        RAILWAY_OUTER_WIDTH, ROAD_WIDTH, VECTOR_BLACK, VECTOR_BLUE, VECTOR_BUILDING_GRAY, VECTOR_OLIVE_GREEN,
        VECTOR_PAVED_AREA_BROWN, VECTOR_WHITE, WIDE_ROAD_INNER_WIDTH, WIDE_ROAD_OUTER_WIDTH,
        XL_WIDE_ROAD_INNER_WIDTH, XL_WIDE_ROAD_OUTER_WIDTH, XXL_WIDE_ROAD_INNER_WIDTH,
        XXL_WIDE_ROAD_OUTER_WIDTH, _MAJOR_POWERLINE_INNER_WIDTH, _MAJOR_POWERLINE_OUTER_WIDTH,
    },
};
use shapefile::{
    record::{polygon::GenericPolygon, polyline::GenericPolyline},
    Point, PolygonRing,
};

pub struct MapRenderer {
    vegetation_img: Canvas,
    olive_green_img: Canvas,
    light_brown_img: Canvas,
    blue_img: Canvas,
    striped_blue_img: Canvas,
    black_road_outlines_img: Canvas,
    light_brown_road_infill_img: Canvas,
    gray_img: Canvas,
    contours_img: Canvas,
    blue_lines_and_points_img: Canvas,
    cliffs_img: Canvas,
    black_img: Canvas,
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
        vegetation_path: &PathBuf,
        contours_path: &PathBuf,
        cliffs_path: &PathBuf,
    ) -> MapRenderer {
        return MapRenderer {
            vegetation_img: Canvas::load_from(vegetation_path.to_str().unwrap()),
            olive_green_img: Canvas::new(image_width as i32, image_height as i32),
            light_brown_img: Canvas::new(image_width as i32, image_height as i32),
            blue_img: Canvas::new(image_width as i32, image_height as i32),
            striped_blue_img: Canvas::new(image_width as i32, image_height as i32),
            black_road_outlines_img: Canvas::new(image_width as i32, image_height as i32),
            light_brown_road_infill_img: Canvas::new(image_width as i32, image_height as i32),
            gray_img: Canvas::new(image_width as i32, image_height as i32),
            contours_img: Canvas::load_from(contours_path.to_str().unwrap()),
            blue_lines_and_points_img: Canvas::new(image_width as i32, image_height as i32),
            cliffs_img: Canvas::load_from(cliffs_path.to_str().unwrap()),
            black_img: Canvas::new(image_width as i32, image_height as i32),
            min_x,
            min_y,
            image_width,
            image_height,
            scale_factor,
            dpi_resolution,
        };
    }

    #[inline]
    pub fn uncrossable_body_of_water_301(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        let (outer_geometry, holes) = self.get_outer_geometry_and_holes_from_polygon(polygon);

        self = self.uncrossable_body_of_water_area_301_1(&polygon);

        self.black_img.set_color(VECTOR_BLACK);
        self.black_img
            .set_line_width(INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH * self.dpi_resolution * 10.0 / INCH);
        self.black_img.draw_polyline(&outer_geometry);

        for hole in holes {
            self.black_img.draw_polyline(&hole);
        }

        return self;
    }

    #[inline]
    pub fn uncrossable_body_of_water_area_301_1(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        let (outer_geometry, holes) = self.get_outer_geometry_and_holes_from_polygon(polygon);

        self.blue_img.set_color(VECTOR_BLUE);
        self.blue_img
            .draw_filled_polygon_with_holes(&outer_geometry, &holes);

        self.contours_img.set_transparent_color();
        self.contours_img
            .draw_filled_polygon_with_holes(&outer_geometry, &holes);

        self.cliffs_img.set_transparent_color();
        self.cliffs_img
            .draw_filled_polygon_with_holes(&outer_geometry, &holes);

        return self;
    }

    #[inline]
    pub fn uncrossable_body_of_water_bank_line_301_4(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    pub fn crossable_watercourse_304(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.blue_lines_and_points_img.set_color(VECTOR_BLUE);
            self.blue_lines_and_points_img
                .set_line_width(CROSSABLE_WATERCOURSE_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.blue_lines_and_points_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    pub fn minor_seasonal_water_channel_306(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.blue_lines_and_points_img.set_color(VECTOR_BLUE);
            self.blue_lines_and_points_img
                .set_line_width(MINOR_WATERCOURSE_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.blue_lines_and_points_img.set_dash(
                MINOR_WATERCOURSE_DASH_LENGTH * self.dpi_resolution * 10.0 / INCH,
                MINOR_WATERCOURSE_DASH_INTERVAL_LENGTH * self.dpi_resolution * 10.0 / INCH,
            );
            self.blue_lines_and_points_img.draw_polyline(&points);
            self.blue_lines_and_points_img.unset_dash();
        }

        return self;
    }

    #[inline]
    pub fn marsh_308(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        let (outer_geometry, holes) = self.get_outer_geometry_and_holes_from_polygon(polygon);
        self.striped_blue_img.set_color(VECTOR_BLUE);
        self.striped_blue_img
            .draw_filled_polygon_with_holes(&outer_geometry, &holes);

        return self;
    }

    #[inline]
    fn wide_road(mut self, line: &GenericPolyline<Point>, inner_width: f32, outer_width: f32) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);
            self.black_road_outlines_img.set_color(VECTOR_BLACK);
            self.black_road_outlines_img
                .set_line_width(outer_width * self.dpi_resolution * 10.0 / INCH);
            self.black_road_outlines_img.draw_polyline(&points);

            self.light_brown_road_infill_img
                .set_color(VECTOR_PAVED_AREA_BROWN);
            self.light_brown_road_infill_img
                .set_line_width(inner_width * self.dpi_resolution * 10.0 / INCH);
            self.light_brown_road_infill_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    pub fn double_track_wide_road_502(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);
            self.black_road_outlines_img.set_color(VECTOR_BLACK);
            self.black_road_outlines_img
                .set_line_width(DOUBLE_TRACK_WIDE_ROAD_OUTER_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_road_outlines_img.draw_polyline(&points);

            self.light_brown_road_infill_img
                .set_color(VECTOR_PAVED_AREA_BROWN);
            self.light_brown_road_infill_img
                .set_line_width(DOUBLE_TRACK_WIDE_ROAD_INNER_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.light_brown_road_infill_img.draw_polyline(&points);

            // TODO
            // self.light_brown_img.set_transparent_color();
            // self.light_brown_img.set_line_width(
            //     DOUBLE_TRACK_WIDE_ROAD_CENTRAL_WIDTH * self.dpi_resolution * 10.0 / INCH,
            // );
            // self.light_brown_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    pub fn wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        return self.wide_road(line, WIDE_ROAD_INNER_WIDTH, WIDE_ROAD_OUTER_WIDTH);
    }

    #[inline]
    pub fn xl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        return self.wide_road(line, XL_WIDE_ROAD_INNER_WIDTH, XL_WIDE_ROAD_OUTER_WIDTH);
    }

    #[inline]
    pub fn xxl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        return self.wide_road(line, XXL_WIDE_ROAD_INNER_WIDTH, XXL_WIDE_ROAD_OUTER_WIDTH);
    }

    #[inline]
    pub fn road_503(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_road_outlines_img.set_color(VECTOR_BLACK);
            self.black_road_outlines_img
                .set_line_width(ROAD_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_road_outlines_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    pub fn footpath_505(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
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
    pub fn railway_509(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(RAILWAY_OUTER_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);

            self.black_img.set_color(VECTOR_WHITE);
            self.black_img
                .set_line_width(RAILWAY_INNER_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.set_dash(
                RAILWAY_DASH_LENGTH * self.dpi_resolution * 10.0 / INCH,
                RAILWAY_DASH_INTERVAL_LENGTH * self.dpi_resolution * 10.0 / INCH,
            );
            self.black_img.draw_polyline(&points);
            self.black_img.unset_dash();
        }

        return self;
    }

    #[inline]
    pub fn power_line_cableway_or_skilift_510(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(POWERLINE_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    fn _major_power_line_511(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);
            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(_MAJOR_POWERLINE_OUTER_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);

            self.black_img.set_transparent_color();
            self.black_img
                .set_line_width(_MAJOR_POWERLINE_INNER_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
        }

        return self;
    }

    #[inline]
    pub fn area_that_shall_not_be_entered_520(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        let (outer_geometry, holes) = self.get_outer_geometry_and_holes_from_polygon(polygon);

        self.olive_green_img.set_color(VECTOR_OLIVE_GREEN);
        self.olive_green_img
            .draw_filled_polygon_with_holes(&outer_geometry, &holes);

        return self;
    }

    #[inline]
    pub fn building_521(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        let (outer_geometry, holes) = self.get_outer_geometry_and_holes_from_polygon(polygon);

        self.gray_img.set_color(VECTOR_BUILDING_GRAY);
        self.gray_img
            .draw_filled_polygon_with_holes(&outer_geometry, &holes);

        self.black_img.set_color(VECTOR_BLACK);
        self.black_img
            .set_line_width(BUILDING_OUTLINE_WIDTH * self.dpi_resolution * 10.0 / INCH);
        self.black_img.draw_polyline(&outer_geometry);

        for hole in holes {
            self.black_img.draw_polyline(&hole);
        }

        return self;
    }

    #[inline]
    fn get_points_from_line_part(&self, line_part: &Vec<Point>) -> Vec<(f32, f32)> {
        let mut points: Vec<(f32, f32)> = vec![];

        for point in line_part {
            points.push((
                (point.x as i64 - self.min_x) as f32 * self.scale_factor,
                (self.image_height as f32 - ((point.y as i64 - self.min_y) as f32 * self.scale_factor)),
            ))
        }

        return points;
    }

    #[inline]
    fn get_outer_geometry_and_holes_from_polygon(
        &self,
        polygon: &GenericPolygon<Point>,
    ) -> (Vec<(f32, f32)>, Vec<Vec<(f32, f32)>>) {
        let mut outer_geometry: Vec<(f32, f32)> = vec![];
        let mut holes: Vec<Vec<(f32, f32)>> = vec![];

        for ring in polygon.rings().iter() {
            match ring {
                PolygonRing::Outer(outer_ring) => {
                    outer_geometry = self.get_points_from_line_part(outer_ring);
                }
                PolygonRing::Inner(inner_ring) => {
                    let points = self.get_points_from_line_part(inner_ring);
                    holes.push(points);
                }
            }
        }

        return (outer_geometry, holes);
    }

    #[inline]
    pub fn save_as(mut self, path: PathBuf) {
        let pixel_marsh_interval =
            (MARSH_LINE_WIDTH + MARSH_LINE_SPACING) * self.dpi_resolution * 10.0 / INCH;

        let number_of_stripes = self.image_height / pixel_marsh_interval as u32;
        self.striped_blue_img.set_transparent_color();

        for i in 0..number_of_stripes {
            let min_y = i as f32 * pixel_marsh_interval;

            let max_y =
                i as f32 * pixel_marsh_interval + MARSH_LINE_SPACING * self.dpi_resolution * 10.0 / INCH;

            self.striped_blue_img.draw_filled_polygon(&vec![
                (0., min_y),
                (self.image_width as f32, min_y),
                (self.image_width as f32, max_y),
                (0. as f32, max_y),
                (0. as f32, min_y),
            ])
        }

        self.vegetation_img.overlay(&mut self.olive_green_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.light_brown_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.blue_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.striped_blue_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.black_road_outlines_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.light_brown_road_infill_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.gray_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.contours_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.blue_lines_and_points_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.cliffs_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.black_img, 0., 0.);

        self.vegetation_img.save_as(path.to_str().unwrap());
    }
}
