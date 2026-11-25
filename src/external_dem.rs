//! DEMs given to Cassini instead of the one it computes from the ground
//! points, such as IGN's MNT LiDAR HD.
//!
//! An external DEM is written to the LiDAR output directory as `dem.tif` on
//! its own grid. The mosaic step samples it with nearest neighbour onto the
//! `DEM_BLOCK_SIZE` grid of the buffered tile, with those of the
//! neighbouring tiles. It is not resampled onto the tile's own grid: IGN's
//! cells are offset by half a cell from Cassini's, so each IGN tile leaves
//! strips of its Cassini tile to the neighbouring IGN tiles, which
//! resampling each DEM onto its own tile would lose.

use log::{info, warn};
use std::path::Path;

use crate::{
    error::{CassiniError, Result, TileId},
    grid::Grid,
};

/// The nodata value given to a DEM without one.
const NO_DATA_ELEVATION: f32 = -9999.0;

/// Reads the DEM at `path` for `tile`, as `f32`.
///
/// Fails if the DEM does not overlap the tile, and warns if it leaves more
/// than one of its cells' width of the tile uncovered: those cells come from
/// the neighbouring tiles' DEMs in a batch, or are interpolated.
pub fn read_external_dem(path: &Path, tile: &TileId) -> Result<Grid<f32>> {
    let mut dem = Grid::<f32>::read_geotiff(path)?;
    if !(dem.resolution.is_finite() && dem.resolution > 0.0) {
        return Err(CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message: format!("expected a positive cell size, found {}", dem.resolution),
        });
    }
    if dem.nodata.is_none() {
        dem.nodata = Some(NO_DATA_ELEVATION);
    }

    let uncovered = Uncovered::new(&dem, tile);
    if !uncovered.overlaps() {
        return Err(CassiniError::InvalidInput {
            message: format!(
                "DEM `{}` covers ({}, {})-({}, {}), outside tile {tile}",
                path.display(),
                dem.min_x,
                dem.min_y(),
                dem.max_x(),
                dem.max_y
            ),
        });
    }
    let sides = uncovered.sides_wider_than(dem.resolution);
    if !sides.is_empty() {
        warn!(
            "Tile {tile}. DEM `{}` leaves part of the tile uncovered: {}. Those cells come from \
             the neighbouring tiles' DEMs in a batch, or are interpolated",
            path.display(),
            sides.join(", ")
        );
    }

    info!(
        "Tile {tile}. Using DEM `{}`: {} x {} cells of {} m",
        path.display(),
        dem.width,
        dem.height,
        dem.resolution
    );
    Ok(dem)
}

/// How far a tile extends past a DEM on each side, in metres, negative where
/// the DEM extends past the tile.
struct Uncovered {
    west: f64,
    east: f64,
    south: f64,
    north: f64,
    width: f64,
    height: f64,
}

impl Uncovered {
    fn new(dem: &Grid<f32>, tile: &TileId) -> Self {
        Self {
            west: dem.min_x - tile.min_x as f64,
            east: tile.max_x as f64 - dem.max_x(),
            south: dem.min_y() - tile.min_y as f64,
            north: tile.max_y as f64 - dem.max_y,
            width: (tile.max_x - tile.min_x) as f64,
            height: (tile.max_y - tile.min_y) as f64,
        }
    }

    /// Whether the DEM and the tile share some area.
    fn overlaps(&self) -> bool {
        self.west.max(0.0) + self.east.max(0.0) < self.width
            && self.south.max(0.0) + self.north.max(0.0) < self.height
    }

    /// The sides where the tile extends more than `width` metres past the
    /// DEM, described.
    fn sides_wider_than(&self, width: f64) -> Vec<String> {
        [
            ("west", self.west),
            ("east", self.east),
            ("south", self.south),
            ("north", self.north),
        ]
        .into_iter()
        .filter(|&(_, uncovered)| uncovered > width)
        .map(|(side, uncovered)| format!("{uncovered} m on the {side}"))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Compression;

    const TILE: TileId = TileId {
        min_x: 907_000,
        min_y: 6_454_000,
        max_x: 908_000,
        max_y: 6_455_000,
    };

    /// A temporary DEM file, removed on drop.
    struct TempDem(std::path::PathBuf);

    impl TempDem {
        /// A `width` x `height` DEM of 0.5 m cells with its north-west corner
        /// at (`min_x`, `max_y`).
        fn new(min_x: f64, max_y: f64, width: usize, height: usize, nodata: Option<f32>) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("cassini-external-dem-{unique}.tif"));
            Grid::new(min_x, max_y, 0.5, width, height, nodata, 1000.0f32)
                .write_geotiff(&path, Compression::None)
                .unwrap();
            Self(path)
        }
    }

    impl Drop for TempDem {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn reads_a_dem_offset_by_half_a_cell() {
        // An IGN tile: 0.25 m west and north of the Cassini tile.
        let file = TempDem::new(906_999.75, 6_455_000.25, 2000, 2000, Some(-9999.0));
        let dem = read_external_dem(&file.0, &TILE).unwrap();
        assert_eq!(
            (dem.min_x, dem.max_y, dem.width),
            (906_999.75, 6_455_000.25, 2000)
        );
        let uncovered = Uncovered::new(&dem, &TILE);
        assert_eq!(
            [
                uncovered.west,
                uncovered.east,
                uncovered.south,
                uncovered.north
            ],
            [-0.25, 0.25, 0.25, -0.25]
        );
        assert!(uncovered.sides_wider_than(dem.resolution).is_empty());
    }

    #[test]
    fn gives_a_nodata_value_to_dems_without_one() {
        let file = TempDem::new(907_000.0, 6_455_000.0, 4, 4, None);
        let dem = read_external_dem(&file.0, &TILE).unwrap();
        assert_eq!(dem.nodata, Some(NO_DATA_ELEVATION));
    }

    #[test]
    fn reports_the_sides_a_dem_leaves_uncovered() {
        // The north-west quarter of the tile, plus one cell.
        let file = TempDem::new(906_999.5, 6_455_000.0, 1001, 1000, Some(-9999.0));
        let dem = read_external_dem(&file.0, &TILE).unwrap();
        assert_eq!(
            Uncovered::new(&dem, &TILE).sides_wider_than(dem.resolution),
            ["500 m on the east", "500 m on the south"]
        );
    }

    #[test]
    fn rejects_dems_outside_the_tile() {
        // The west neighbour's DEM, touching the tile, and one far away.
        for (min_x, max_y) in [(906_000.0, 6_455_000.0), (100_000.0, 6_455_000.0)] {
            let file = TempDem::new(min_x, max_y, 2000, 2000, Some(-9999.0));
            let error = read_external_dem(&file.0, &TILE).unwrap_err();
            assert!(error.to_string().contains("outside tile"), "{error}");
        }
    }
}
