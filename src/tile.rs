use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct TileWithNeighbors {
    pub laz_path: PathBuf,
    pub tile: Tile,
    pub neighbors: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Tile {
    pub lidar_dir_path: PathBuf,
    pub render_dir_path: PathBuf,
    pub min_x: i64,
    pub min_y: i64,
    pub max_x: i64,
    pub max_y: i64,
}

pub fn get_extent_from_lidar_dir_path(lidar_dir_path: &PathBuf) -> (i64, i64, i64, i64) {
    let extent_file_path = lidar_dir_path.join("extent.txt");
    let mut file = File::open(extent_file_path).expect("Could not read the extent.txt file");

    let mut extent_content = String::new();
    file.read_to_string(&mut extent_content)
        .expect("Could not read the extent.txt file");

    let parts: Vec<i64> = extent_content
        .trim()
        .split('|')
        .map(|s| s.parse::<i64>())
        .collect::<Result<Vec<_>, _>>()
        .expect("The extent.txt file is corrupted");

    if parts.len() != 4 {
        panic!("The extent.txt file is corrupted")
    }

    return (parts[0], parts[1], parts[2], parts[3]);
}
