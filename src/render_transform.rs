//! The conversion of world coordinates, in metres, to the pixels of the
//! rendered maps.
//!
//! Every map shares one pixel grid per DPI resolution, anchored at the world
//! origin: the pixel of column `col` and row `row` covers the world square
//! from `(col, -row)` to `(col + 1, -row - 1)` times the pixel size. A tile
//! image covers the pixels whose corners are nearest to its extent, so that
//! adjacent tiles touch without gap or overlap, and the merged map places
//! each tile image at the offset between their grid positions.

use std::path::Path;

use crate::{config::Config, grid::GridExtent, tile::Tile, world_file::WorldFile};

/// The ground distance, in metres, of one inch on the map at 1:10 000.
const METRES_PER_MAP_INCH: f64 = 254.0;
const MILLIMETRES_PER_INCH: f32 = 25.4;

/// The pixel grid of the maps at a DPI resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelGrid {
    dpi_resolution: f32,
}

impl PixelGrid {
    pub fn new(dpi_resolution: f32) -> Self {
        Self { dpi_resolution }
    }

    pub fn from_config(config: &Config) -> Self {
        Self::new(config.dpi_resolution)
    }

    pub fn pixels_per_metre(&self) -> f64 {
        f64::from(self.dpi_resolution) / METRES_PER_MAP_INCH
    }

    /// The ground size of a pixel, in metres.
    pub fn pixel_size(&self) -> f64 {
        METRES_PER_MAP_INCH / f64::from(self.dpi_resolution)
    }

    /// The pixels covering the extent, to the pixel corners nearest to its
    /// edges.
    pub fn rect(&self, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> PixelRect {
        let scale = self.pixels_per_metre();
        PixelRect {
            left: (min_x * scale).round() as i64,
            top: (-max_y * scale).round() as i64,
            right: (max_x * scale).round() as i64,
            bottom: (-min_y * scale).round() as i64,
        }
    }
}

/// A rectangle of pixels, of the grid or of an image, from `left` and `top`
/// included to `right` and `bottom` excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

impl PixelRect {
    pub fn width(&self) -> i64 {
        self.right - self.left
    }

    pub fn height(&self) -> i64 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub fn intersects(&self, other: &PixelRect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

/// The conversion of world coordinates to the pixels of one image: a tile,
/// or a chunk of the merged map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderTransform {
    grid: PixelGrid,
    rect: PixelRect,
}

impl RenderTransform {
    pub fn new(grid: PixelGrid, rect: PixelRect) -> Self {
        Self { grid, rect }
    }

    pub fn for_tile(tile: &Tile, config: &Config) -> Self {
        let grid = PixelGrid::from_config(config);
        let rect = grid.rect(
            tile.min_x as f64,
            tile.min_y as f64,
            tile.max_x as f64,
            tile.max_y as f64,
        );
        Self::new(grid, rect)
    }

    pub fn grid(&self) -> PixelGrid {
        self.grid
    }

    /// The pixels of the image on the grid.
    pub fn pixel_rect(&self) -> PixelRect {
        self.rect
    }

    pub fn width(&self) -> u32 {
        self.rect.width() as u32
    }

    pub fn height(&self) -> u32 {
        self.rect.height() as u32
    }

    pub fn dpi_resolution(&self) -> f32 {
        self.grid.dpi_resolution
    }

    /// The pixels on the map of `mm` millimetres on paper, for symbol widths.
    pub fn mm_to_px(&self, mm: f32) -> f32 {
        mm * self.grid.dpi_resolution / MILLIMETRES_PER_INCH
    }

    /// The position in the image of the world point (`x`, `y`).
    pub fn world_to_px(&self, x: f64, y: f64) -> (f64, f64) {
        let scale = self.grid.pixels_per_metre();
        (
            x * scale - self.rect.left as f64,
            -y * scale - self.rect.top as f64,
        )
    }

    /// The pixel of the image containing the centre of the cell at `col` and
    /// `row` of a raster, around which a shape centred on the cell is drawn.
    /// The pixel `(x, y)` covers `x` to `x + 1`, so rounding the position
    /// instead would draw the shapes half a pixel south-east on average.
    pub fn cell_center_pixel(&self, raster: &GridExtent, col: usize, row: usize) -> (i32, i32) {
        let (x, y) = self.world_to_px(
            raster.min_x + (col as f64 + 0.5) * raster.resolution,
            raster.max_y - (row as f64 + 0.5) * raster.resolution,
        );
        (x.floor() as i32, y.floor() as i32)
    }

    /// The pixels of the image covered by the cell at `col` and `row` of a
    /// raster, to the pixel corners nearest to its edges, as the image
    /// itself. Adjacent cells share their edges, so the cells of a raster
    /// covering the image's extent cover each of its pixels once. The
    /// rectangle is empty when the cell is smaller than a pixel and misses
    /// every pixel corner.
    pub fn cell_rect(&self, raster: &GridExtent, col: usize, row: usize) -> PixelRect {
        let cell = self.grid.rect(
            raster.min_x + col as f64 * raster.resolution,
            raster.max_y - (row + 1) as f64 * raster.resolution,
            raster.min_x + (col + 1) as f64 * raster.resolution,
            raster.max_y - row as f64 * raster.resolution,
        );
        PixelRect {
            left: cell.left - self.rect.left,
            top: cell.top - self.rect.top,
            right: cell.right - self.rect.left,
            bottom: cell.bottom - self.rect.top,
        }
    }

    /// The world file georeferencing the image.
    pub fn world_file(&self) -> WorldFile {
        let pixel_size = self.grid.pixel_size();
        WorldFile {
            pixel_size,
            // World files give the centre of the north-west pixel.
            center_x: (self.rect.left as f64 + 0.5) * pixel_size,
            center_y: -(self.rect.top as f64 + 0.5) * pixel_size,
        }
    }

    pub fn write_world_file(&self, path: &Path) -> std::io::Result<()> {
        self.world_file().write(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{PixelGrid, PixelRect, RenderTransform};
    use crate::grid::GridExtent;

    const DPI_RESOLUTIONS: [f32; 4] = [72.0, 300.0, 600.0, 1200.0];

    /// The transform of the 1 km tile with its south-west corner at
    /// (`min_x`, `min_y`).
    fn tile_transform(grid: PixelGrid, min_x: i64, min_y: i64) -> RenderTransform {
        let rect = grid.rect(
            min_x as f64,
            min_y as f64,
            (min_x + 1000) as f64,
            (min_y + 1000) as f64,
        );
        RenderTransform::new(grid, rect)
    }

    #[test]
    fn adjacent_tiles_touch_without_gap_or_overlap() {
        for dpi in DPI_RESOLUTIONS {
            let grid = PixelGrid::new(dpi);
            let exact_side = 1000.0 * grid.pixels_per_metre();
            for i in 0..20 {
                // Lambert-93 coordinates of IGN tiles.
                let (min_x, min_y) = (900_000 + i * 1000, 6_400_000 + i * 1000);
                let tile = tile_transform(grid, min_x, min_y).pixel_rect();
                let east = tile_transform(grid, min_x + 1000, min_y).pixel_rect();
                let south = tile_transform(grid, min_x, min_y - 1000).pixel_rect();
                assert_eq!(tile.right, east.left, "{dpi} DPI");
                assert_eq!(tile.top, east.top, "{dpi} DPI");
                assert_eq!(tile.bottom, south.top, "{dpi} DPI");
                assert_eq!(tile.left, south.left, "{dpi} DPI");
                assert!(!tile.intersects(&east) && !tile.intersects(&south));
                for side in [tile.width(), tile.height()] {
                    assert!((side as f64 - exact_side).abs() < 1.0, "{dpi} DPI: {side}");
                }
            }
        }
    }

    #[test]
    fn places_world_points_at_their_exact_position_in_the_image() {
        let grid = PixelGrid::new(600.0);
        let transform = tile_transform(grid, 900_000, 6_400_000);
        let rect = transform.pixel_rect();
        let scale = grid.pixels_per_metre();
        let (x, y) = transform.world_to_px(900_010.75, 6_400_090.25);
        let expected_x = 900_010.75 * scale - rect.left as f64;
        let expected_y = -6_400_090.25 * scale - rect.top as f64;
        assert!((x - expected_x).abs() < 1e-6, "{x}");
        assert!((y - expected_y).abs() < 1e-6, "{y}");
        // The corners of the tile are within half a pixel of the image's.
        let (left, top) = transform.world_to_px(900_000.0, 6_401_000.0);
        let (right, bottom) = transform.world_to_px(901_000.0, 6_400_000.0);
        assert!(left.abs() <= 0.5 && top.abs() <= 0.5, "{left} {top}");
        assert!(
            (right - f64::from(transform.width())).abs() <= 0.5,
            "{right}"
        );
        assert!(
            (bottom - f64::from(transform.height())).abs() <= 0.5,
            "{bottom}"
        );
    }

    #[test]
    fn draws_raster_cells_around_the_pixel_containing_their_centre() {
        let grid = PixelGrid::new(254.0);
        // 1 pixel per metre, and a tile on whole pixels.
        let transform = tile_transform(grid, 1000, 2000);
        let raster = |resolution: f64| GridExtent {
            min_x: 800.0,
            max_y: 3200.0,
            resolution,
            width: 2000,
            height: 2000,
        };
        // At (0.25, 0.25), (0.75, 0.75) and (3.5, 1.5) pixels.
        assert_eq!(transform.cell_center_pixel(&raster(0.5), 400, 400), (0, 0));
        assert_eq!(transform.cell_center_pixel(&raster(0.5), 401, 401), (0, 0));
        assert_eq!(transform.cell_center_pixel(&raster(1.0), 203, 201), (3, 1));
        // West and north of the image.
        assert_eq!(
            transform.cell_center_pixel(&raster(0.5), 399, 398),
            (-1, -1)
        );
    }

    #[test]
    fn the_cells_covering_a_tile_cover_each_pixel_of_its_image_once() {
        for dpi in [72.0, 150.0, 200.0, 300.0, 400.0, 600.0, 720.0, 1200.0] {
            let grid = PixelGrid::new(dpi);
            for i in 0..10 {
                let (min_x, min_y) = (900_000 + i * 1000, 6_400_000 + i * 1000);
                let transform = tile_transform(grid, min_x, min_y);
                let raster = GridExtent {
                    min_x: min_x as f64,
                    max_y: (min_y + 1000) as f64,
                    resolution: 1.0,
                    width: 1000,
                    height: 1000,
                };
                let (first, last) = (
                    transform.cell_rect(&raster, 0, 0),
                    transform.cell_rect(&raster, 999, 999),
                );
                assert_eq!((first.left, first.top), (0, 0), "{dpi} DPI");
                assert_eq!(
                    (last.right, last.bottom),
                    (i64::from(transform.width()), i64::from(transform.height())),
                    "{dpi} DPI"
                );
                for col in 1..1000 {
                    let (west, east) = (
                        transform.cell_rect(&raster, col - 1, 0),
                        transform.cell_rect(&raster, col, 0),
                    );
                    assert_eq!(west.right, east.left, "{dpi} DPI");
                    let (north, south) = (
                        transform.cell_rect(&raster, 0, col - 1),
                        transform.cell_rect(&raster, 0, col),
                    );
                    assert_eq!(north.bottom, south.top, "{dpi} DPI");
                }
            }
        }
    }

    #[test]
    fn converts_millimetres_on_paper_to_pixels() {
        let transform = tile_transform(PixelGrid::new(600.0), 900_000, 6_400_000);
        assert!((transform.mm_to_px(25.4) - 600.0).abs() < 1e-3);
        assert!((transform.mm_to_px(0.35) - 0.35 * 600.0 / 25.4).abs() < 1e-5);
    }

    #[test]
    fn world_files_give_the_centre_of_the_north_west_pixel_in_full_precision() {
        for dpi in DPI_RESOLUTIONS {
            let grid = PixelGrid::new(dpi);
            let transform = tile_transform(grid, 900_000, 6_400_000);
            let rect = transform.pixel_rect();
            let world_file = transform.world_file();
            let pixel_size = grid.pixel_size();
            assert!((world_file.pixel_size * grid.pixels_per_metre() - 1.0).abs() < 1e-12);
            // The world position of pixel centres, back to the image.
            let (x, y) = transform.world_to_px(world_file.center_x, world_file.center_y);
            assert!((x - 0.5).abs() < 1e-6 && (y - 0.5).abs() < 1e-6, "{x} {y}");
            // The corner of the image is within half a pixel of the tile's.
            let left = world_file.center_x - pixel_size / 2.0;
            let top = world_file.center_y + pixel_size / 2.0;
            assert!((left - 900_000.0).abs() <= pixel_size / 2.0, "{left}");
            assert!((top - 6_401_000.0).abs() <= pixel_size / 2.0, "{top}");
            assert_eq!(
                rect,
                grid.rect(900_000.0, 6_400_000.0, 901_000.0, 6_401_000.0)
            );
        }
    }

    #[test]
    fn a_chunk_of_the_merged_map_is_georeferenced_as_its_tiles() {
        let grid = PixelGrid::new(600.0);
        let tile = tile_transform(grid, 905_000, 6_395_000);
        let chunk = RenderTransform::new(
            grid,
            PixelRect {
                left: tile.pixel_rect().left - 15_000,
                top: tile.pixel_rect().top - 15_000,
                right: tile.pixel_rect().right,
                bottom: tile.pixel_rect().bottom,
            },
        );
        let (x, y) = chunk.world_to_px(905_000.0, 6_396_000.0);
        let (tile_x, tile_y) = tile.world_to_px(905_000.0, 6_396_000.0);
        assert!((x - tile_x - 15_000.0).abs() < 1e-6 && (y - tile_y - 15_000.0).abs() < 1e-6);
        let pixel_size = grid.pixel_size();
        let (chunk_file, tile_file) = (chunk.world_file(), tile.world_file());
        assert!((tile_file.center_x - chunk_file.center_x - 15_000.0 * pixel_size).abs() < 1e-6);
        assert!((chunk_file.center_y - tile_file.center_y - 15_000.0 * pixel_size).abs() < 1e-6);
    }
}
