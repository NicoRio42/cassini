use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct TileWithNeighbors {
    pub tile: Tile,
    pub neighbors: NeighborTiles,
}

#[derive(Debug, Clone)]
pub struct Tile {
    pub dir_path: PathBuf,
    pub laz_path: PathBuf,
    pub min_x: i64,
    pub min_y: i64,
    pub max_x: i64,
    pub max_y: i64,
}

#[derive(Debug, Clone)]
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
