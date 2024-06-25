use std::path::PathBuf;

use crate::canvas::Canvas;

pub fn render_full_map_to_png(image_width: u32, image_height: u32, out_dir: &PathBuf) {
    println!("Rendering map to png");

    let mut full_map_canvas = Canvas::new(image_width as i32, image_height as i32);

    let cliffs_path = out_dir.join("cliffs.png");
    let mut cliff_canvas = Canvas::load_from(&cliffs_path.to_str().unwrap());
    let vegetation_path = out_dir.join("vegetation.png");
    let mut vegetation_canvas = Canvas::load_from(&vegetation_path.to_str().unwrap());
    let contours_path = out_dir.join("contours.png");
    let mut contours_canvas = Canvas::load_from(&contours_path.to_str().unwrap());
    let vectors_path = out_dir.join("vectors.png");
    let mut vectors_canvas = Canvas::load_from(&vectors_path.to_str().unwrap());

    full_map_canvas.overlay(&mut vegetation_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut contours_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut cliff_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut vectors_canvas, 0.0, 0.0);

    let full_map_path = out_dir.join("full-map.png");
    full_map_canvas.save_as(&full_map_path.to_str().unwrap())
}
