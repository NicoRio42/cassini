use crate::{canvas::Canvas, constants::DEM_RESOLUTION};

pub fn render_full_map_to_png() {
    let mut full_map_canvas = Canvas::new(
        (1001 * DEM_RESOLUTION) as i32,
        (1001 * DEM_RESOLUTION) as i32,
    );

    let mut cliff_canvas = Canvas::load_from("../out/cliffs.png");
    let mut vegetation_canvas = Canvas::load_from("../out/vegetation.png");
    let mut contours_canvas = Canvas::load_from("../out/contours.png");

    full_map_canvas.overlay(&mut vegetation_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut contours_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut cliff_canvas, 0.0, 0.0);

    full_map_canvas.save_as("../out/full-map.png")
}
