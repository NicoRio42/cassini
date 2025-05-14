use std::path::PathBuf;

pub fn create_world_file(
    top_left_x: f32,
    top_left_y: f32,
    resolution: f32,
    world_file_path: &PathBuf,
) -> Result<(), std::io::Error> {
    let world_file = format!(
        "{:.6}\n0.0\n0.0\n{:.6}\n{:.6}\n{:.6}",
        resolution, -resolution, top_left_x, top_left_y
    );

    std::fs::write(world_file_path, world_file)
}
