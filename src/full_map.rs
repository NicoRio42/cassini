use crate::canvas::Canvas;

pub fn render_full_map_to_png(image_width: u32, image_height: u32) {
    println!("Rendering map to png");

    let mut full_map_canvas = Canvas::new(image_width as i32, image_height as i32);

    let mut cliff_canvas = Canvas::load_from("./out/cliffs.png");
    let mut vegetation_canvas = Canvas::load_from("./out/vegetation.png");
    let mut contours_canvas = Canvas::load_from("./out/contours.png");

    full_map_canvas.overlay(&mut vegetation_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut contours_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut cliff_canvas, 0.0, 0.0);

    full_map_canvas.save_as("./out/full-map.png")
}
