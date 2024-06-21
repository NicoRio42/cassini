mod canvas;
mod cliffs;
mod constants;
mod contours;
mod full_map;
mod vegetation;

use cliffs::render_cliffs;
use contours::render_contours_to_png;
use full_map::render_full_map_to_png;
use vegetation::render_vegetation;

fn main() {
    render_vegetation();
    render_cliffs();
    render_contours_to_png();
    render_full_map_to_png();
}
