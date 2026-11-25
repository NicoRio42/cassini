use std::path::{Path, PathBuf};

use crate::{
    canvas::Canvas,
    constants::{
        _MAJOR_POWERLINE_INNER_WIDTH, _MAJOR_POWERLINE_OUTER_WIDTH, BUILDING_OUTLINE_WIDTH,
        CROSSABLE_WATERCOURSE_WIDTH, DOUBLE_TRACK_WIDE_ROAD_INNER_WIDTH,
        DOUBLE_TRACK_WIDE_ROAD_OUTER_WIDTH, FOOTPATH_DASH_INTERVAL_LENGTH, FOOTPATH_DASH_LENGTH,
        FOOTPATH_WIDTH, INCH, INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH, MARSH_LINE_SPACING,
        MARSH_LINE_WIDTH, MINOR_WATERCOURSE_DASH_INTERVAL_LENGTH, MINOR_WATERCOURSE_DASH_LENGTH,
        MINOR_WATERCOURSE_WIDTH, POWERLINE_WIDTH, RAILWAY_DASH_INTERVAL_LENGTH,
        RAILWAY_DASH_LENGTH, RAILWAY_INNER_WIDTH, RAILWAY_OUTER_WIDTH, ROAD_WIDTH,
        UNDERGROWTH_LINE_SPACING, UNDERGROWTH_LINE_WIDTH, VECTOR_BLACK, VECTOR_BLUE,
        VECTOR_BUILDING_GRAY, VECTOR_OLIVE_GREEN, VECTOR_PAVED_AREA_BROWN, VECTOR_TUNNEL_ALPHA,
        VECTOR_WHITE, WIDE_ROAD_INNER_WIDTH, WIDE_ROAD_OUTER_WIDTH, XL_WIDE_ROAD_INNER_WIDTH,
        XL_WIDE_ROAD_OUTER_WIDTH, XXL_WIDE_ROAD_INNER_WIDTH, XXL_WIDE_ROAD_OUTER_WIDTH,
    },
    error::Result,
    map_features::get_polygon_areas,
};
use shapefile::{
    record::{polygon::GenericPolygon, polyline::GenericPolyline},
    Point,
};

enum StripeDirection {
    Horizontal,
    Vertical,
}

pub struct MapRenderer {
    vegetation_img: Canvas,
    undergrowth_img: Option<Canvas>,
    olive_green_img: Canvas,
    light_brown_img: Canvas,
    blue_img: Canvas,
    striped_blue_img: Canvas,
    black_road_outlines_img: Canvas,
    light_brown_road_infill_img: Canvas,
    black_tunnel_road_outlines_img: Canvas,
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

/// The outer ring and the holes of a polygon area, in pixels.
type PixelArea = (Vec<(f32, f32)>, Vec<Vec<(f32, f32)>>);

impl MapRenderer {
    // To be replaced by a request struct (A8 in the 2026-11-25 audit).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        min_x: i64,
        min_y: i64,
        image_width: u32,
        image_height: u32,
        scale_factor: f32,
        dpi_resolution: f32,
        vegetation_path: &Path,
        undergrowth_path: &Path,
        contours_path: &Path,
        cliffs_path: &Path,
    ) -> Result<MapRenderer> {
        let undergrowth_img = if undergrowth_path.is_file() {
            Some(Canvas::load_from(undergrowth_path)?)
        } else {
            None
        };

        Ok(MapRenderer {
            vegetation_img: Canvas::load_from(vegetation_path)?,
            undergrowth_img,
            olive_green_img: Canvas::new(image_width as i32, image_height as i32)?,
            light_brown_img: Canvas::new(image_width as i32, image_height as i32)?,
            blue_img: Canvas::new(image_width as i32, image_height as i32)?,
            striped_blue_img: Canvas::new(image_width as i32, image_height as i32)?,
            black_road_outlines_img: Canvas::new(image_width as i32, image_height as i32)?,
            light_brown_road_infill_img: Canvas::new(image_width as i32, image_height as i32)?,
            black_tunnel_road_outlines_img: Canvas::new(image_width as i32, image_height as i32)?,
            gray_img: Canvas::new(image_width as i32, image_height as i32)?,
            contours_img: Canvas::load_from(contours_path)?,
            blue_lines_and_points_img: Canvas::new(image_width as i32, image_height as i32)?,
            cliffs_img: Canvas::load_from(cliffs_path)?,
            black_img: Canvas::new(image_width as i32, image_height as i32)?,
            min_x,
            min_y,
            image_width,
            image_height,
            scale_factor,
            dpi_resolution,
        })
    }

    #[inline]
    pub fn uncrossable_body_of_water_301(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        self = self.uncrossable_body_of_water_area_301_1(polygon);

        for (outer_geometry, holes) in self.get_outer_geometries_and_holes_from_polygon(polygon) {
            self.black_img.set_color(VECTOR_BLACK);
            self.black_img.set_line_width(
                INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH * self.dpi_resolution * 10.0 / INCH,
            );
            self.black_img.draw_polyline(&outer_geometry);

            for hole in holes {
                self.black_img.draw_polyline(&hole);
            }
        }

        self
    }

    #[inline]
    pub fn uncrossable_body_of_water_area_301_1(
        mut self,
        polygon: &GenericPolygon<Point>,
    ) -> MapRenderer {
        for (outer_geometry, holes) in self.get_outer_geometries_and_holes_from_polygon(polygon) {
            self.blue_img.set_color(VECTOR_BLUE);
            self.blue_img
                .draw_filled_polygon_with_holes(&outer_geometry, &holes);

            self.contours_img.set_transparent_color();
            self.contours_img
                .draw_filled_polygon_with_holes(&outer_geometry, &holes);

            self.cliffs_img.set_transparent_color();
            self.cliffs_img
                .draw_filled_polygon_with_holes(&outer_geometry, &holes);
        }

        self
    }

    #[inline]
    pub fn uncrossable_body_of_water_bank_line_301_4(
        mut self,
        line: &GenericPolyline<Point>,
    ) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img.set_line_width(
                INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH * self.dpi_resolution * 10.0 / INCH,
            );
            self.black_img.draw_polyline(&points);
        }

        self
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

        self
    }

    #[inline]
    pub fn minor_seasonal_water_channel_306(
        mut self,
        line: &GenericPolyline<Point>,
    ) -> MapRenderer {
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

        self
    }

    #[inline]
    pub fn marsh_308(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        for (outer_geometry, holes) in self.get_outer_geometries_and_holes_from_polygon(polygon) {
            self.striped_blue_img.set_color(VECTOR_BLUE);
            self.striped_blue_img
                .draw_filled_polygon_with_holes(&outer_geometry, &holes);
        }

        self
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

        self
    }

    #[inline]
    fn tunnel_wide_road(
        mut self,
        line: &GenericPolyline<Point>,
        inner_width: f32,
        outer_width: f32,
    ) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_tunnel_road_outlines_img
                .set_color_with_alpha(VECTOR_BLACK, VECTOR_TUNNEL_ALPHA);
            self.black_tunnel_road_outlines_img
                .set_line_width(outer_width * self.dpi_resolution * 10.0 / INCH);
            self.black_tunnel_road_outlines_img.draw_polyline(&points);

            self.black_tunnel_road_outlines_img.set_transparent_color();
            self.black_tunnel_road_outlines_img
                .set_line_width(inner_width * self.dpi_resolution * 10.0 / INCH);
            self.black_tunnel_road_outlines_img.draw_polyline(&points);
        }

        self
    }

    #[inline]
    pub fn double_track_wide_road_502(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);
            self.black_road_outlines_img.set_color(VECTOR_BLACK);
            self.black_road_outlines_img.set_line_width(
                DOUBLE_TRACK_WIDE_ROAD_OUTER_WIDTH * self.dpi_resolution * 10.0 / INCH,
            );
            self.black_road_outlines_img.draw_polyline(&points);

            self.light_brown_road_infill_img
                .set_color(VECTOR_PAVED_AREA_BROWN);
            self.light_brown_road_infill_img.set_line_width(
                DOUBLE_TRACK_WIDE_ROAD_INNER_WIDTH * self.dpi_resolution * 10.0 / INCH,
            );
            self.light_brown_road_infill_img.draw_polyline(&points);

            // TODO
            // self.light_brown_img.set_transparent_color();
            // self.light_brown_img.set_line_width(
            //     DOUBLE_TRACK_WIDE_ROAD_CENTRAL_WIDTH * self.dpi_resolution * 10.0 / INCH,
            // );
            // self.light_brown_img.draw_polyline(&points);
        }

        self
    }

    #[inline]
    pub fn tunnel_double_track_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        self.tunnel_wide_road(
            line,
            DOUBLE_TRACK_WIDE_ROAD_INNER_WIDTH,
            DOUBLE_TRACK_WIDE_ROAD_OUTER_WIDTH,
        )
    }

    #[inline]
    pub fn wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        self.wide_road(line, WIDE_ROAD_INNER_WIDTH, WIDE_ROAD_OUTER_WIDTH)
    }

    #[inline]
    pub fn tunnel_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        self.tunnel_wide_road(line, WIDE_ROAD_INNER_WIDTH, WIDE_ROAD_OUTER_WIDTH)
    }

    #[inline]
    pub fn xl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        self.wide_road(line, XL_WIDE_ROAD_INNER_WIDTH, XL_WIDE_ROAD_OUTER_WIDTH)
    }

    #[inline]
    pub fn tunnel_xl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        self.tunnel_wide_road(line, XL_WIDE_ROAD_INNER_WIDTH, XL_WIDE_ROAD_OUTER_WIDTH)
    }

    #[inline]
    pub fn xxl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        self.wide_road(line, XXL_WIDE_ROAD_INNER_WIDTH, XXL_WIDE_ROAD_OUTER_WIDTH)
    }

    #[inline]
    pub fn tunnel_xxl_wide_road_502(self, line: &GenericPolyline<Point>) -> MapRenderer {
        self.tunnel_wide_road(line, XXL_WIDE_ROAD_INNER_WIDTH, XXL_WIDE_ROAD_OUTER_WIDTH)
    }

    #[inline]
    pub fn road_503(mut self, line: &GenericPolyline<Point>) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(ROAD_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
        }

        self
    }

    #[inline]
    pub fn tunnel_road_503(self, _line: &GenericPolyline<Point>) -> MapRenderer {
        self
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

        self
    }

    #[inline]
    pub fn tunnel_footpath_505(self, _line: &GenericPolyline<Point>) -> MapRenderer {
        self
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

        self
    }

    #[inline]
    pub fn power_line_cableway_or_skilift_510(
        mut self,
        line: &GenericPolyline<Point>,
    ) -> MapRenderer {
        for part in line.parts() {
            let points = self.get_points_from_line_part(part);

            self.black_img.set_color(VECTOR_BLACK);
            self.black_img
                .set_line_width(POWERLINE_WIDTH * self.dpi_resolution * 10.0 / INCH);
            self.black_img.draw_polyline(&points);
        }

        self
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

        self
    }

    #[inline]
    pub fn area_that_shall_not_be_entered_520(
        mut self,
        polygon: &GenericPolygon<Point>,
    ) -> MapRenderer {
        for (outer_geometry, holes) in self.get_outer_geometries_and_holes_from_polygon(polygon) {
            self.olive_green_img.set_color(VECTOR_OLIVE_GREEN);
            self.olive_green_img
                .draw_filled_polygon_with_holes(&outer_geometry, &holes);
        }

        self
    }

    #[inline]
    pub fn building_521(mut self, polygon: &GenericPolygon<Point>) -> MapRenderer {
        for (outer_geometry, holes) in self.get_outer_geometries_and_holes_from_polygon(polygon) {
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
        }

        self
    }

    #[inline]
    fn get_points_from_line_part(&self, line_part: &[Point]) -> Vec<(f32, f32)> {
        line_part
            .iter()
            .map(|point| {
                world_to_pixel(
                    point,
                    self.min_x,
                    self.min_y,
                    self.image_height,
                    self.scale_factor,
                )
            })
            .collect()
    }

    #[inline]
    fn get_outer_geometries_and_holes_from_polygon(
        &self,
        polygon: &GenericPolygon<Point>,
    ) -> Vec<PixelArea> {
        get_polygon_areas(polygon)
            .into_iter()
            .map(|area| {
                (
                    self.get_points_from_line_part(area.outer),
                    area.holes
                        .into_iter()
                        .map(|hole| self.get_points_from_line_part(hole))
                        .collect(),
                )
            })
            .collect()
    }

    #[inline]
    fn draw_stripes(
        image: &mut Canvas,
        image_width: u32,
        image_height: u32,
        dpi_resolution: f32,
        line_width: f32,
        spacing: f32,
        direction: StripeDirection,
    ) {
        let pixel_marsh_interval = (line_width + spacing) * dpi_resolution * 10.0 / INCH;
        image.set_transparent_color();

        match direction {
            StripeDirection::Horizontal => {
                let number_of_stripes = image_height / pixel_marsh_interval as u32;

                for i in 0..number_of_stripes {
                    let min_y = i as f32 * pixel_marsh_interval;
                    let max_y = min_y + spacing * dpi_resolution * 10.0 / INCH;

                    image.draw_filled_polygon(&[
                        (0., min_y),
                        (image_width as f32, min_y),
                        (image_width as f32, max_y),
                        (0., max_y),
                        (0., min_y),
                    ]);
                }
            }
            StripeDirection::Vertical => {
                let number_of_stripes = image_width / pixel_marsh_interval as u32;

                for i in 0..number_of_stripes {
                    let min_x = i as f32 * pixel_marsh_interval;
                    let max_x = min_x + spacing * dpi_resolution * 10.0 / INCH;

                    image.draw_filled_polygon(&[
                        (min_x, 0.),
                        (max_x, 0.),
                        (max_x, image_height as f32),
                        (min_x, image_height as f32),
                        (min_x, 0.),
                    ]);
                }
            }
        }
    }

    #[inline]
    pub fn save_as(mut self, path: PathBuf) -> Result<()> {
        Self::draw_stripes(
            &mut self.striped_blue_img,
            self.image_width,
            self.image_height,
            self.dpi_resolution,
            MARSH_LINE_WIDTH,
            MARSH_LINE_SPACING,
            StripeDirection::Horizontal,
        );

        if let Some(ref mut undergrowth_img) = self.undergrowth_img {
            Self::draw_stripes(
                undergrowth_img,
                self.image_width,
                self.image_height,
                self.dpi_resolution,
                UNDERGROWTH_LINE_WIDTH,
                UNDERGROWTH_LINE_SPACING,
                StripeDirection::Vertical,
            );

            self.vegetation_img.overlay(undergrowth_img, 0., 0.);
        }

        self.vegetation_img
            .overlay(&mut self.olive_green_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.light_brown_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.blue_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.striped_blue_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.black_road_outlines_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.light_brown_road_infill_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.black_tunnel_road_outlines_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.gray_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.contours_img, 0., 0.);
        self.vegetation_img
            .overlay(&mut self.blue_lines_and_points_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.cliffs_img, 0., 0.);
        self.vegetation_img.overlay(&mut self.black_img, 0., 0.);

        self.vegetation_img.save_as(&path)
    }
}

/// The position in the tile image, `image_height` pixels high, of `point`, in
/// world coordinates, for a tile whose south-west corner is at (`min_x`,
/// `min_y`) and `scale_factor` pixels per meter.
fn world_to_pixel(
    point: &Point,
    min_x: i64,
    min_y: i64,
    image_height: u32,
    scale_factor: f32,
) -> (f32, f32) {
    let scale_factor = f64::from(scale_factor);
    (
        ((point.x - min_x as f64) * scale_factor) as f32,
        (f64::from(image_height) - (point.y - min_y as f64) * scale_factor) as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::world_to_pixel;
    use shapefile::Point;

    #[test]
    fn places_points_at_their_exact_position_in_the_tile_image() {
        let scale_factor = 600.0 / 254.0;
        let (x, y) = world_to_pixel(
            &Point::new(900_010.75, 6_400_090.25),
            900_000,
            6_400_000,
            2362,
            scale_factor,
        );
        let scale_factor = f64::from(scale_factor);
        assert!((f64::from(x) - 10.75 * scale_factor).abs() < 1e-3, "{x}");
        assert!(
            (f64::from(y) - (2362.0 - 90.25 * scale_factor)).abs() < 1e-3,
            "{y}"
        );
    }
}
