use std::path::PathBuf;

pub struct Tile {
    pub dir_path: PathBuf,
    pub min_x: i64,
    pub min_y: i64,
    pub max_x: i64,
    pub max_y: i64,
}

pub struct NeighborTiles {
    pub top: Option<Tile>,
    pub top_right: Option<Tile>,
    pub right: Option<Tile>,
    pub bottom_right: Option<Tile>,
    pub bottom: Option<Tile>,
    pub bottom_left: Option<Tile>,
    pub left: Option<Tile>,
    pub top_left: Option<Tile>,
}
