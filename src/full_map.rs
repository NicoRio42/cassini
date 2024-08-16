use crate::{canvas::Canvas, tile::Tile};

pub fn render_full_map_to_png(tile: &Tile, image_width: u32, image_height: u32) {
    println!("Rendering map to png");

    let mut full_map_canvas = Canvas::new(image_width as i32, image_height as i32);

    let cliffs_path = tile.dir_path.join("cliffs.png");
    let mut cliff_canvas = Canvas::load_from(&cliffs_path.to_str().unwrap());
    let vegetation_path = tile.dir_path.join("vegetation.png");
    let mut vegetation_canvas = Canvas::load_from(&vegetation_path.to_str().unwrap());
    let contours_path = tile.dir_path.join("contours.png");
    let mut contours_canvas = Canvas::load_from(&contours_path.to_str().unwrap());
    let vectors_path = tile.dir_path.join("vectors.png");
    let mut vectors_canvas = Canvas::load_from(&vectors_path.to_str().unwrap());

    full_map_canvas.overlay(&mut vegetation_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut contours_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut cliff_canvas, 0.0, 0.0);
    full_map_canvas.overlay(&mut vectors_canvas, 0.0, 0.0);

    let full_map_path = tile.dir_path.join("full-map.png");
    full_map_canvas.save_as(&full_map_path.to_str().unwrap())
}
