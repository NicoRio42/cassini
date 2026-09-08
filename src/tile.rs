use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use crate::error::{CassiniError, Result, ResultContext, TileId};

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

impl From<&Tile> for TileId {
    fn from(tile: &Tile) -> Self {
        Self {
            min_x: tile.min_x,
            min_y: tile.min_y,
            max_x: tile.max_x,
            max_y: tile.max_y,
        }
    }
}

pub fn get_extent_from_lidar_dir_path(
    lidar_dir_path: &std::path::Path,
) -> Result<(i64, i64, i64, i64)> {
    let extent_file_path = lidar_dir_path.join("extent.txt");
    let mut file = File::open(&extent_file_path)
        .context(format!("could not open `{}`", extent_file_path.display()))?;

    let mut extent_content = String::new();
    file.read_to_string(&mut extent_content)
        .context(format!("could not read `{}`", extent_file_path.display()))?;

    let parts: Vec<i64> = extent_content
        .trim()
        .split('|')
        .map(|s| s.parse::<i64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| CassiniError::InvalidArtifact {
            path: extent_file_path.clone(),
            message: format!("extent contains a non-integer value: {source}"),
        })?;

    if parts.len() != 4 {
        return Err(CassiniError::InvalidArtifact {
            path: extent_file_path,
            message: format!("expected four coordinates, found {}", parts.len()),
        });
    }

    if parts[0] >= parts[2] || parts[1] >= parts[3] {
        return Err(CassiniError::InvalidArtifact {
            path: lidar_dir_path.join("extent.txt"),
            message: "extent minimums must be smaller than maximums".to_owned(),
        });
    }

    Ok((parts[0], parts[1], parts[2], parts[3]))
}
