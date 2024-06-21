use crate::{
    canvas::Canvas,
    constants::{BROWN, DEM_RESOLUTION, MIN_X, MIN_Y},
};
use shapefile::dbase;

pub fn render_contours_to_png() {
    let contours = shapefile::read_as::<_, shapefile::Polyline, shapefile::dbase::Record>(
        "./out/contours.shp",
    )
    .expect("Could not open contours shapefile");

    let mut contours_layer_img = Canvas::new(
        (1001 * DEM_RESOLUTION) as i32,
        (1001 * DEM_RESOLUTION) as i32,
    );

    for (polyline, record) in contours {
        let elevation = match record.get("elev") {
            Some(dbase::FieldValue::Numeric(Some(x))) => x,
            Some(_) => panic!("Expected 'elev' to be a numeric in polygon-dataset"),
            None => panic!("Field 'elev' is not within polygon-dataset"),
        };

        let is_normal_contour = *elevation as i32 % 5 == 0;

        for part in polyline.parts() {
            if part.len() < 2 {
                continue;
            }

            let mut points: Vec<(f32, f32)> = vec![];

            for point in part {
                points.push((
                    (point.x as i32 - MIN_X) as f32,
                    (1001 - (point.y as i32 - MIN_Y)) as f32,
                ))
            }

            contours_layer_img.set_stroke_cap_round();
            contours_layer_img.set_color(BROWN);

            if is_normal_contour {
                contours_layer_img.set_line_width(2.0);
            } else {
                contours_layer_img.set_line_width(1.3);
                contours_layer_img.set_dash(20.0, 13.0)
            }

            contours_layer_img.draw_polyline(&points);
            contours_layer_img.unset_dash();
        }
    }

    contours_layer_img.save_as("./out/contours.png");
}
