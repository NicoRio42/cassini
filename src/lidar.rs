//! The DEM and vegetation density rasters of a LiDAR tile, computed in
//! streaming passes over its points. Given a DEM file, such as IGN's, the DEM
//! is copied instead of computed from the ground points.
//!
//! The DEM is computed as the PDAL pipeline Cassini used to run computed it,
//! but on `DEM_BLOCK_SIZE` cells instead of 1 m ones, the cells without
//! ground points taking the mean of their 1 m block.
//!
//! The vegetation rasters count the points of each 1 m cell, after voxel
//! thinning, in three layers:
//!
//! - [`VegetationLayers::Height`]: by height above the filled DEM of the
//!   buffered tile, `dem_with_buffer.tif`. This is what that pipeline
//!   (`writers.gdal`, `filters.hag_dem`, `filters.voxeldownsize`,
//!   `filters.assign`) was meant to do. It needs the filled DEM, so a second
//!   pass over the points once the DEM of the tile and those of its
//!   neighbours are written.
//! - [`VegetationLayers::LidarClass`]: by the low, medium and high vegetation
//!   classes of the LiDAR HD classification, in the same pass as the DEM.
//!   This is what the PDAL pipeline did: it ran in stream mode, where
//!   `filters.hag_dem` opens the DEM before `writers.gdal` has written it.
//!   PDAL up to 2.9 ignores that error and leaves every height at 0, so no
//!   point was reclassified.

use las::{Bounds, PointData};
use log::info;
use std::collections::HashSet;
use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use crate::{
    buffer::paste_with_buffer,
    constants::{DEM_BLOCK_SIZE, VEGETATION_BLOCK_SIZE},
    dem::fill_dem_with_buffer,
    error::{CassiniError, Result, ResultContext, TileId},
    external_dem::read_external_dem,
    grid::{Compression, Grid, Sample},
    helpers::remove_dir_content_except,
    point_reader::PointReader,
};

/// How the points are sorted into the low, medium and high vegetation
/// layers.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq)]
pub enum VegetationLayers {
    /// By height above the ground.
    Height,
    /// By the LiDAR classification.
    LidarClass,
}

/// The ASPRS classifications of unclassified points, of ground and of low,
/// medium and high vegetation.
const UNCLASSIFIED: u8 = 1;
const GROUND: u8 = 2;
const LOW_VEGETATION: u8 = 3;
const MEDIUM_VEGETATION: u8 = 4;
const HIGH_VEGETATION: u8 = 5;

/// The heights above the ground bounding the low, medium and high vegetation
/// layers, in metres: a layer holds the points higher than its lower bound
/// and up to its upper one.
const VEGETATION_LAYER_HEIGHTS: [f64; 4] = [0.0, 1.0, 4.0, 30.0];

/// The value of the DEM cells whose fallback block has no ground points.
const NO_DATA_ELEVATION: f32 = -9999.0;

/// How many DEM cells fit along a vegetation cell, so that a vegetation cell
/// is exactly a block of DEM cells.
const DEM_CELLS_PER_VEGETATION_CELL: usize = (VEGETATION_BLOCK_SIZE / DEM_BLOCK_SIZE) as usize;
const _: () = assert!(
    DEM_CELLS_PER_VEGETATION_CELL as f64 * DEM_BLOCK_SIZE == VEGETATION_BLOCK_SIZE,
    "the vegetation resolution must be a multiple of the DEM resolution"
);

/// The side of the blocks of DEM cells whose ground points fill the DEM cells
/// without any, in metres: the cell size of the DEM before it was computed at
/// `DEM_BLOCK_SIZE`. A whole metre, so that the blocks cover exactly the
/// tiles, whose sides are whole metres.
const FALLBACK_BLOCK_SIZE: f64 = 1.0;

/// How many DEM cells fit along a fallback block.
const DEM_CELLS_PER_FALLBACK_BLOCK: usize = (FALLBACK_BLOCK_SIZE / DEM_BLOCK_SIZE) as usize;
const _: () = assert!(
    DEM_CELLS_PER_FALLBACK_BLOCK as f64 * DEM_BLOCK_SIZE == FALLBACK_BLOCK_SIZE,
    "the fallback block size must be a multiple of the DEM resolution"
);

/// The nodata value of the vegetation rasters. Their cells hold point counts,
/// with 0 for cells without points, so counts saturate just below it.
const NO_DATA_COUNT: u8 = 255;

/// The edge of the cubic voxels the points are thinned with, in metres.
const VOXEL_SIZE: f64 = 0.5;

/// The largest tile side accepted, in metres. LiDAR HD tiles are 1 km, and
/// other national grids a few kilometres. A larger extent most likely comes
/// from outlier points in the header bounds, and its rasters would not fit
/// in memory: they are allocated over the header bounds, and the DEM alone
/// takes 64 MB per km² while it is computed (four 0.5 m cells per m², each
/// with an `f64` mean, a `u32` count and then an `f32` elevation), 1.6 GB
/// for a 5 km tile.
const MAX_TILE_SIDE: i64 = 5_000;

/// The number of points decompressed at a time. A batch spans many LAZ
/// chunks, which are decompressed in parallel.
const POINTS_PER_BATCH: u64 = 1 << 21;

/// Writes the DEM and vegetation density rasters of the LiDAR file at
/// `laz_path` to `output_dir_path`, for a tile processed alone, returning
/// the tile's extent. The DEM is the one at `dem_path` if given, else it is
/// computed from the ground points.
///
/// In `Height` mode, the heights are taken above the tile's DEM pasted onto
/// its buffered grid without neighbours and filled, which is not written. In
/// a batch, the neighbours' DEMs fill the buffer instead, so the heights near
/// the tile edges can differ.
pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: &Path,
    dem_path: Option<&Path>,
    output_dir_path: &Path,
    layers: VegetationLayers,
) -> Result<TileId> {
    let (tile, dem) =
        generate_dem_tiff_image_from_laz_file(laz_path, dem_path, output_dir_path, layers)?;
    if layers == VegetationLayers::Height {
        let nodata = dem.nodata.unwrap_or(NO_DATA_ELEVATION);
        let mut dem_with_buffer = paste_with_buffer(&tile, &dem, nodata, DEM_BLOCK_SIZE);
        drop(dem);
        fill_dem_with_buffer(&mut dem_with_buffer, &tile);
        generate_vegetation_density_tiff_images_from_laz_file(
            laz_path,
            &tile,
            &dem_with_buffer,
            output_dir_path,
        )?;
    }
    Ok(tile)
}

/// Writes the DEM of the LiDAR file at `laz_path` to `output_dir_path` as
/// `dem.tif`, returning the tile's extent and its DEM. The DEM is the one at
/// `dem_path` if given, else it is computed from the ground points.
///
/// In `LidarClass` mode, the same pass over the points writes the vegetation
/// density rasters, and the LiDAR output is complete. In `Height` mode,
/// [`generate_vegetation_density_tiff_images_from_laz_file`] writes them
/// once the filled DEM of the buffered tile is known. With a DEM file, that
/// is the only pass over the points.
pub fn generate_dem_tiff_image_from_laz_file(
    laz_path: &Path,
    dem_path: Option<&Path>,
    output_dir_path: &Path,
    layers: VegetationLayers,
) -> Result<(TileId, Grid<f32>)> {
    let start = Instant::now();

    let mut reader = PointReader::open(laz_path)?;
    let bounds = reader.header().bounds();
    let tile = tile_id(
        laz_path,
        [bounds.min.x, bounds.min.y],
        [bounds.max.x, bounds.max.y],
    )?;
    let TileId {
        min_x,
        min_y,
        max_x,
        max_y,
    } = tile;

    // Read before the output directory is cleaned, which may hold it.
    let external_dem = dem_path
        .map(|dem_path| read_external_dem(dem_path, &tile))
        .transpose()?;
    let with_vegetation = layers == VegetationLayers::LidarClass;

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. {}",
        min_x,
        min_y,
        max_x,
        max_y,
        match (external_dem.is_some(), with_vegetation) {
            (true, true) => "Computing vegetation densities by LiDAR class",
            (true, false) => "Copying the DEM",
            (false, true) => "Computing DEM and vegetation densities by LiDAR class",
            (false, false) => "Computing DEM",
        }
    );

    // Cleaning up output directory to fix https://github.com/NicoRio42/cassini/issues/7
    // The OSM file only depends on the tile extent and is slow to download, so it is kept.
    if output_dir_path.exists() {
        let osm_file_name = format!("{:0>7}_{:0>7}.osm", min_x, max_y);
        remove_dir_content_except(output_dir_path, &[&osm_file_name]).context(format!(
            "could not clean LiDAR output directory `{}`",
            output_dir_path.display()
        ))?;
    } else {
        create_dir_all(output_dir_path).context(format!(
            "could not create LiDAR output directory `{}`",
            output_dir_path.display()
        ))?;
    }

    let mut ground_dem = external_dem.is_none().then(|| GroundDem::new(&tile));
    let mut vegetation = None;
    if ground_dem.is_some() || with_vegetation {
        read_points(&mut reader, |first_point, points| {
            if vegetation.is_none() && with_vegetation {
                vegetation = Some(VegetationDensities::new(
                    &tile,
                    VoxelSet::new(first_point, &bounds),
                    None,
                ));
            }
            for (((x, y), z), classification) in point_columns(points) {
                if let Some(ground_dem) = &mut ground_dem {
                    // The DEM writer comes before the thinning, so it gets
                    // every point.
                    ground_dem.add_point(x, y, z, classification);
                }
                if let Some(vegetation) = &mut vegetation {
                    vegetation.add_point(x, y, z, classification);
                }
            }
        })?;
    }

    let dem = match (external_dem, ground_dem) {
        (Some(dem), _) => dem,
        (None, Some(ground_dem)) => ground_dem.grid(),
        (None, None) => unreachable!("a tile without a DEM file has a ground DEM"),
    };
    dem.write_geotiff(&output_dir_path.join("dem.tif"), Compression::Deflate)?;
    if with_vegetation {
        // A file without points has no voxel to thin, so any origin will do.
        let vegetation = vegetation.unwrap_or_else(|| {
            VegetationDensities::new(&tile, VoxelSet::new([0.0; 3], &bounds), None)
        });
        write_vegetation(vegetation, &tile, output_dir_path)?;
    }

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. {} in {:.1?}",
        min_x,
        min_y,
        max_x,
        max_y,
        if with_vegetation {
            "DEM and vegetation densities computed"
        } else {
            "DEM computed"
        },
        start.elapsed()
    );

    Ok((tile, dem))
}

/// Writes the vegetation density rasters of the LiDAR file at `laz_path` to
/// `output_dir_path`, by height above `dem_with_buffer`, the filled DEM of
/// the buffered tile `tile`. This completes the LiDAR output that
/// [`generate_dem_tiff_image_from_laz_file`] started in `Height` mode.
pub fn generate_vegetation_density_tiff_images_from_laz_file(
    laz_path: &Path,
    tile: &TileId,
    dem_with_buffer: &Grid<f32>,
    output_dir_path: &Path,
) -> Result<()> {
    let start = Instant::now();
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Computing vegetation densities by height",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let mut reader = PointReader::open(laz_path)?;
    let bounds = reader.header().bounds();
    let ground = GroundElevations::new(dem_with_buffer);
    let mut vegetation = None;
    read_points(&mut reader, |first_point, points| {
        let vegetation = vegetation.get_or_insert_with(|| {
            VegetationDensities::new(tile, VoxelSet::new(first_point, &bounds), Some(&ground))
        });
        for (((x, y), z), classification) in point_columns(points) {
            vegetation.add_point(x, y, z, classification);
        }
    })?;
    let vegetation = vegetation.unwrap_or_else(|| {
        VegetationDensities::new(tile, VoxelSet::new([0.0; 3], &bounds), Some(&ground))
    });
    write_vegetation(vegetation, tile, output_dir_path)?;

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vegetation densities computed in {:.1?}",
        tile.min_x,
        tile.min_y,
        tile.max_x,
        tile.max_y,
        start.elapsed()
    );
    Ok(())
}

/// Reads the points of `reader` in file order, giving each batch of points
/// to `add_points` with the first point of the file, the origin of the voxel
/// grid.
///
/// PDAL read LiDAR HD's COPC files in octree order instead, which only
/// changes the voxel origin and the rounding of the DEM mean.
fn read_points(
    reader: &mut PointReader,
    mut add_points: impl FnMut([f64; 3], &PointData),
) -> Result<()> {
    let mut points = reader.point_data();
    let mut first_point = None;
    while reader.fill_points(POINTS_PER_BATCH, &mut points)? > 0 {
        let first_point = *first_point.get_or_insert_with(|| {
            points
                .x()
                .zip(points.y())
                .zip(points.z())
                .map(|((x, y), z)| [x, y, z])
                .next()
                .unwrap_or_default()
        });
        add_points(first_point, &points);
    }
    Ok(())
}

/// The X, Y, Z and classification of `points`.
fn point_columns(points: &PointData) -> impl Iterator<Item = (((f64, f64), f64), u8)> + '_ {
    points
        .x()
        .zip(points.y())
        .zip(points.z())
        .zip(points.classification())
}

/// Writes the low, medium and high vegetation density rasters of `tile`,
/// then `extent.txt`, which completes the LiDAR output.
fn write_vegetation(
    vegetation: VegetationDensities,
    tile: &TileId,
    output_dir_path: &Path,
) -> Result<()> {
    let grids = vegetation.into_grids();
    let cells = grids[0].values.len().max(1) as f64;
    let [low, medium, high] = grids.each_ref().map(|grid| {
        grid.values
            .iter()
            .map(|&count| f64::from(count))
            .sum::<f64>()
            / cells
    });
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Mean vegetation points per cell: {:.2} low, {:.2} medium, {:.2} high",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, low, medium, high
    );

    let [low, medium, high] = grids;
    for (grid, name) in [
        (low, "low_vegetation.tif"),
        (medium, "medium_vegetation.tif"),
        (high, "high_vegetation.tif"),
    ] {
        grid.write_geotiff(&output_dir_path.join(name), Compression::Deflate)?;
    }

    // Downstream tools use extent.txt as the completion marker, so write it last.
    let extent_path = output_dir_path.join("extent.txt");
    let mut extent_file = File::create(&extent_path)
        .context(format!("could not create `{}`", extent_path.display()))?;
    extent_file
        .write_all(
            format!(
                "{}|{}|{}|{}",
                tile.min_x, tile.min_y, tile.max_x, tile.max_y
            )
            .as_bytes(),
        )
        .context(format!("could not write `{}`", extent_path.display()))
}

/// The extent of the LiDAR file at `laz_path`: its header bounds, rounded to
/// the metre.
pub fn read_tile_id(laz_path: &Path) -> Result<TileId> {
    let mut file = File::open(laz_path).context(format!(
        "could not open LiDAR file `{}`",
        laz_path.display()
    ))?;
    let header = las::raw::Header::read_from(&mut file).context(format!(
        "could not read LiDAR header `{}`",
        laz_path.display()
    ))?;
    tile_id(
        laz_path,
        [header.min_x, header.min_y],
        [header.max_x, header.max_y],
    )
}

/// The tile of the LiDAR file at `laz_path`, whose header bounds are `min`
/// and `max`, rejecting an empty extent or one too large to allocate the
/// rasters of.
fn tile_id(laz_path: &Path, min: [f64; 2], max: [f64; 2]) -> Result<TileId> {
    if !min.iter().chain(&max).all(|bound| bound.is_finite()) {
        return Err(CassiniError::InvalidInput {
            message: format!(
                "LiDAR file `{}` has non-finite header bounds",
                laz_path.display()
            ),
        });
    }
    let tile = TileId {
        min_x: min[0].round() as i64,
        min_y: min[1].round() as i64,
        max_x: max[0].round() as i64,
        max_y: max[1].round() as i64,
    };
    let width = tile.max_x - tile.min_x;
    let height = tile.max_y - tile.min_y;
    if width <= 0 || height <= 0 {
        return Err(CassiniError::InvalidInput {
            message: format!(
                "LiDAR file `{}` has an invalid extent {tile}",
                laz_path.display()
            ),
        });
    }
    if width > MAX_TILE_SIDE || height > MAX_TILE_SIDE {
        return Err(CassiniError::InvalidInput {
            message: format!(
                "LiDAR file `{}` covers {width} m x {height} m, more than the {MAX_TILE_SIDE} m \
                 per side Cassini accepts; its header bounds may include outlier points",
                laz_path.display()
            ),
        });
    }
    Ok(tile)
}

/// The vegetation densities of a tile, accumulated point by point: the
/// number of low, medium and high vegetation points of each
/// `VEGETATION_BLOCK_SIZE` cell, after voxel thinning.
struct VegetationDensities<'a> {
    cells: TileCells,
    counts: [Vec<u8>; 3],
    voxels: VoxelSet,
    /// The ground the heights are taken above in `Height` mode, `None` in
    /// `LidarClass` mode.
    ground: Option<&'a GroundElevations<'a>>,
}

impl<'a> VegetationDensities<'a> {
    /// The empty densities of `tile`, whose points are thinned with
    /// `voxels`, by height above `ground` if given, else by class.
    fn new(tile: &TileId, voxels: VoxelSet, ground: Option<&'a GroundElevations<'a>>) -> Self {
        let cells = TileCells::new(tile, VEGETATION_BLOCK_SIZE);
        let len = cells.width * cells.height;
        Self {
            cells,
            counts: [(); 3].map(|_| vec![0; len]),
            voxels,
            ground,
        }
    }

    /// Adds a point, in file order.
    fn add_point(&mut self, x: f64, y: f64, z: f64, classification: u8) {
        // Every point is thinned, whatever its class.
        if !self.voxels.insert(x, y, z) {
            return;
        }
        let Some(index) = self.cells.bin_index(x, y) else {
            return;
        };
        let layer = match self.ground {
            None => match classification {
                LOW_VEGETATION => 0,
                MEDIUM_VEGETATION => 1,
                HIGH_VEGETATION => 2,
                _ => return,
            },
            Some(ground) => {
                if !matches!(
                    classification,
                    UNCLASSIFIED | LOW_VEGETATION | MEDIUM_VEGETATION | HIGH_VEGETATION
                ) {
                    return;
                }
                match ground.height(x, y, z).and_then(height_layer) {
                    Some(layer) => layer,
                    None => return,
                }
            }
        };
        let count = &mut self.counts[layer][index];
        *count = count.saturating_add(1).min(NO_DATA_COUNT - 1);
    }

    /// The low, medium and high vegetation densities.
    fn into_grids(self) -> [Grid<u8>; 3] {
        let cells = self.cells;
        self.counts.map(|counts| cells.grid(counts, NO_DATA_COUNT))
    }
}

/// The vegetation layer of a point `height` metres above the ground, `None`
/// for a point on or below the ground, or above the highest layer.
fn height_layer(height: f64) -> Option<usize> {
    VEGETATION_LAYER_HEIGHTS
        .windows(2)
        .position(|bounds| bounds[0] < height && height <= bounds[1])
}

/// The ground elevations of a filled DEM, such as the filled DEM of the
/// buffered tile, to take heights above.
struct GroundElevations<'a> {
    dem: &'a Grid<f32>,
    cells: TileCells,
}

impl<'a> GroundElevations<'a> {
    fn new(dem: &'a Grid<f32>) -> Self {
        Self {
            dem,
            cells: TileCells {
                min_x: dem.min_x,
                min_y: dem.min_y(),
                resolution: dem.resolution,
                width: dem.width,
                height: dem.height,
            },
        }
    }

    /// The height of the point (`x`, `y`, `z`) above the DEM cell containing
    /// it, binned as the DEM's points are: a point on the edge between two
    /// cells is above the east or north one. `Grid::cell_at` would take the
    /// south one, and LiDAR HD's coordinates are on a 1 cm grid, so about 1%
    /// of the points are on an edge between two rows. `None` outside the DEM
    /// or above a cell without data.
    fn height(&self, x: f64, y: f64, z: f64) -> Option<f64> {
        let elevation = self.dem.values[self.cells.bin_index(x, y)?];
        self.dem
            .has_data(elevation)
            .then(|| z - f64::from(elevation))
    }
}

/// The DEM of a tile's ground points, accumulated point by point.
struct GroundDem {
    cells: TileCells,
    /// The running mean elevation of the ground points of each DEM cell, and
    /// their number.
    elevations: Vec<f64>,
    ground_counts: Vec<u32>,
}

impl GroundDem {
    fn new(tile: &TileId) -> Self {
        let cells = TileCells::new(tile, DEM_BLOCK_SIZE);
        let len = cells.width * cells.height;
        Self {
            cells,
            elevations: vec![0.0; len],
            ground_counts: vec![0; len],
        }
    }

    fn add_point(&mut self, x: f64, y: f64, z: f64, classification: u8) {
        if classification != GROUND {
            return;
        }
        if let Some(index) = self.cells.bin_index(x, y) {
            // `writers.gdal`'s running mean.
            self.ground_counts[index] += 1;
            self.elevations[index] +=
                (z - self.elevations[index]) / self.ground_counts[index] as f64;
        }
    }
    /// The mean elevation of the ground points of each DEM cell. A cell
    /// without any takes the mean of the ground points of its
    /// `FALLBACK_BLOCK_SIZE` block, the value the 1 m DEM gave it, and only
    /// the blocks without any stay nodata.
    ///
    /// At 0.5 m, a quarter to two thirds of the cells of a LiDAR HD tile have
    /// no ground point. Left to `fill_nodata`, their inverse distance
    /// weighted mean smoothed the slopes and lost up to 45% of the cliffs of
    /// the 1 m DEM.
    fn grid(&self) -> Grid<f32> {
        let cells = self.cells;
        let side = DEM_CELLS_PER_FALLBACK_BLOCK;
        let mut values = vec![NO_DATA_ELEVATION; cells.width * cells.height];
        for block_row in (0..cells.height).step_by(side) {
            for block_col in (0..cells.width).step_by(side) {
                let block = (block_row..block_row + side).flat_map(|row| {
                    (block_col..block_col + side).map(move |col| row * cells.width + col)
                });
                let (sum, count) = block.clone().fold((0.0, 0), |(sum, count), index| {
                    let cell_count = self.ground_counts[index];
                    (
                        sum + self.elevations[index] * cell_count as f64,
                        count + u64::from(cell_count),
                    )
                });
                for index in block {
                    if self.ground_counts[index] > 0 {
                        values[index] = self.elevations[index] as f32;
                    } else if count > 0 {
                        values[index] = (sum / count as f64) as f32;
                    }
                }
            }
        }
        cells.grid(values, NO_DATA_ELEVATION)
    }
}

/// Square cells of `resolution` metres, which points are binned into, such
/// as the cells covering exactly a tile.
#[derive(Clone, Copy)]
struct TileCells {
    min_x: f64,
    min_y: f64,
    resolution: f64,
    width: usize,
    height: usize,
}

impl TileCells {
    /// The cells of `tile`, whose sides must be multiples of `resolution`
    /// metres.
    fn new(tile: &TileId, resolution: f64) -> Self {
        let cells = |min: i64, max: i64| {
            let side = (max - min) as f64;
            let cells = (side / resolution).round();
            debug_assert_eq!(
                cells * resolution,
                side,
                "{side} m is not a multiple of {resolution} m"
            );
            cells as usize
        };
        Self {
            min_x: tile.min_x as f64,
            min_y: tile.min_y as f64,
            resolution,
            width: cells(tile.min_x, tile.max_x),
            height: cells(tile.min_y, tile.max_y),
        }
    }

    /// The index of the cell the point (`x`, `y`) is binned into, as
    /// `writers.gdal` bins it in `binmode`, row-major from the north-west
    /// corner. A point on the edge between two cells goes to the east or
    /// north one, so the east and north edges of the cells are out.
    fn bin_index(&self, x: f64, y: f64) -> Option<usize> {
        let col = ((x - self.min_x) / self.resolution).floor();
        let row_from_south = ((y - self.min_y) / self.resolution).floor();
        if !(0.0..self.width as f64).contains(&col)
            || !(0.0..self.height as f64).contains(&row_from_south)
        {
            return None;
        }
        Some((self.height - 1 - row_from_south as usize) * self.width + col as usize)
    }

    /// The grid of these cells with `values`, row-major from the north-west
    /// corner.
    fn grid<T: Sample>(&self, values: Vec<T>, nodata: T) -> Grid<T> {
        Grid {
            min_x: self.min_x,
            max_y: self.min_y + self.height as f64 * self.resolution,
            resolution: self.resolution,
            width: self.width,
            height: self.height,
            nodata: Some(nodata),
            values,
        }
    }
}

/// The voxels of `filters.voxeldownsize` holding a point, to keep only the
/// first point of each voxel as `mode: first` does. The voxel grid origin is
/// half a voxel below the first point read, as in PDAL.
///
/// Each voxel column within the header bounds has a bitset of the
/// `COLUMN_VOXELS` voxels centred on the voxel of its first point, and the
/// voxels out of it go to a hash set. That is far smaller than a hash set of
/// the tens of millions of voxels of a tile.
///
/// The column bitsets are allocated by blocks of `BLOCK_SIDE` x `BLOCK_SIDE`
/// columns, when a point first falls in the block, so their memory follows
/// the area the points cover (about 80 MB per km²) rather than the header
/// bounds.
struct VoxelSet {
    origin: [f64; 3],
    min_col: i32,
    min_row: i32,
    /// The number of blocks across and down the header bounds.
    blocks_width: usize,
    blocks_height: usize,
    /// Row-major from the south-west.
    blocks: Vec<Option<ColumnBlock>>,
    others: HashSet<[i32; 3]>,
}

/// The number of voxels of a column bitset, 64 m.
const COLUMN_VOXELS: i32 = u128::BITS as i32;

/// The number of columns along the side of a block, 32 m.
const BLOCK_SIDE: usize = 64;

/// The bitsets of a block of columns, row-major from the south-west.
struct ColumnBlock {
    /// The voxel `z` of the lowest bit of each column, valid once the
    /// column has a bit set.
    bases: Box<[i32]>,
    bits: Box<[u128]>,
}

impl ColumnBlock {
    fn new() -> Self {
        Self {
            bases: vec![0; BLOCK_SIDE * BLOCK_SIDE].into_boxed_slice(),
            bits: vec![0; BLOCK_SIDE * BLOCK_SIDE].into_boxed_slice(),
        }
    }
}

impl VoxelSet {
    fn new(first_point: [f64; 3], bounds: &Bounds) -> Self {
        let origin = first_point.map(|coordinate| coordinate - VOXEL_SIZE / 2.0);
        let min_col = voxel_index(bounds.min.x, origin[0]);
        let min_row = voxel_index(bounds.min.y, origin[1]);
        let width = (voxel_index(bounds.max.x, origin[0]) - min_col + 1).max(0) as usize;
        let height = (voxel_index(bounds.max.y, origin[1]) - min_row + 1).max(0) as usize;
        let blocks_width = width.div_ceil(BLOCK_SIDE);
        let blocks_height = height.div_ceil(BLOCK_SIDE);
        Self {
            origin,
            min_col,
            min_row,
            blocks_width,
            blocks_height,
            blocks: (0..blocks_width * blocks_height).map(|_| None).collect(),
            others: HashSet::new(),
        }
    }

    /// Adds the voxel of the point (`x`, `y`, `z`), returning whether it was
    /// empty.
    fn insert(&mut self, x: f64, y: f64, z: f64) -> bool {
        let voxel = [
            voxel_index(x, self.origin[0]),
            voxel_index(y, self.origin[1]),
            voxel_index(z, self.origin[2]),
        ];
        if let Some((block, column)) = self.column(voxel) {
            let block = self.blocks[block].get_or_insert_with(ColumnBlock::new);
            let bits = &mut block.bits[column];
            if *bits == 0 {
                block.bases[column] = voxel[2].saturating_sub(COLUMN_VOXELS / 2);
            }
            let offset = i64::from(voxel[2]) - i64::from(block.bases[column]);
            if (0..i64::from(COLUMN_VOXELS)).contains(&offset) {
                let mask = 1 << offset;
                let inserted = *bits & mask == 0;
                *bits |= mask;
                return inserted;
            }
        }
        self.others.insert(voxel)
    }

    /// The index of the block of `voxel` and of its column in the block, if
    /// within the blocks covering the header bounds.
    fn column(&self, voxel: [i32; 3]) -> Option<(usize, usize)> {
        let col = usize::try_from(i64::from(voxel[0]) - i64::from(self.min_col)).ok()?;
        let row = usize::try_from(i64::from(voxel[1]) - i64::from(self.min_row)).ok()?;
        let (block_col, block_row) = (col / BLOCK_SIDE, row / BLOCK_SIDE);
        (block_col < self.blocks_width && block_row < self.blocks_height).then_some((
            block_row * self.blocks_width + block_col,
            (row % BLOCK_SIDE) * BLOCK_SIDE + col % BLOCK_SIDE,
        ))
    }
}

/// The voxel index of `coordinate` along an axis, as PDAL computes it.
fn voxel_index(coordinate: f64, origin: f64) -> i32 {
    ((coordinate - origin) / VOXEL_SIZE).floor() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::BUFFER;
    use las::Vector;

    fn bounds(min: [f64; 3], max: [f64; 3]) -> Bounds {
        let [x, y, z] = min;
        let min = Vector { x, y, z };
        let [x, y, z] = max;
        let max = Vector { x, y, z };
        Bounds { min, max }
    }

    const RASTERS_TILE: TileId = TileId {
        min_x: 1000,
        min_y: 2000,
        max_x: 1003,
        max_y: 2002,
    };

    /// The voxels of the points of `RASTERS_TILE`, whose first point is
    /// `first_point`.
    fn voxels(first_point: [f64; 3]) -> VoxelSet {
        VoxelSet::new(
            first_point,
            &bounds([1000.0, 2000.0, 0.0], [1003.0, 2002.0, 50.0]),
        )
    }

    #[test]
    fn tile_id_rounds_the_header_bounds_and_rejects_bad_extents() {
        let path = Path::new("tile.laz");
        assert_eq!(
            tile_id(path, [906999.6, 6454000.01], [908000.4, 6454999.5]).unwrap(),
            TileId {
                min_x: 907000,
                min_y: 6454000,
                max_x: 908000,
                max_y: 6455000,
            }
        );
        // An empty file's header, inverted bounds, NaN and an outlier point.
        for (min, max) in [
            ([0.0, 0.0], [0.0, 0.0]),
            ([1000.0, 2000.0], [900.0, 3000.0]),
            ([f64::NAN, 2000.0], [1000.0, 3000.0]),
            ([1000.0, 2000.0], [6001.0, 3000.0]),
        ] {
            assert!(tile_id(path, min, max).is_err(), "{min:?} {max:?}");
        }
        assert!(tile_id(path, [0.0, 0.0], [5_000.0, 5_000.0]).is_ok());
    }

    #[test]
    fn points_on_cell_edges_are_binned_east_and_north() {
        let vegetation = TileCells::new(&RASTERS_TILE, VEGETATION_BLOCK_SIZE);
        assert_eq!(vegetation.bin_index(1000.0, 2000.0), Some(3));
        assert_eq!(vegetation.bin_index(1000.5, 2001.0), Some(0));
        assert_eq!(vegetation.bin_index(1002.999, 2001.999), Some(2));
        assert_eq!(vegetation.bin_index(1003.0, 2001.0), None);
        assert_eq!(vegetation.bin_index(1001.0, 2002.0), None);
        assert_eq!(vegetation.bin_index(999.99, 2001.0), None);
        assert_eq!(vegetation.bin_index(f64::NAN, 2001.0), None);

        let dem = TileCells::new(&RASTERS_TILE, DEM_BLOCK_SIZE);
        assert_eq!((dem.width, dem.height), (6, 4));
        assert_eq!(dem.bin_index(1000.0, 2000.0), Some(18));
        assert_eq!(dem.bin_index(1000.5, 2001.0), Some(7));
        assert_eq!(dem.bin_index(1000.49, 2001.5), Some(0));
        assert_eq!(dem.bin_index(1002.999, 2001.999), Some(5));
        assert_eq!(dem.bin_index(1003.0, 2001.0), None);
        assert_eq!(dem.bin_index(1001.0, 2002.0), None);
    }

    /// A 1 cm grid of points over `RASTERS_TILE`, edges included, as in
    /// LiDAR HD.
    fn centimetre_points() -> impl Iterator<Item = (f64, f64)> {
        (99_990..=100_310)
            .flat_map(|x| (199_990..=200_210).map(move |y| (x as f64 / 100.0, y as f64 / 100.0)))
    }

    #[test]
    fn dem_cells_lie_inside_the_vegetation_cell_of_their_points() {
        let dem = TileCells::new(&RASTERS_TILE, DEM_BLOCK_SIZE);
        let vegetation = TileCells::new(&RASTERS_TILE, VEGETATION_BLOCK_SIZE);
        for (x, y) in centimetre_points() {
            let dem_index = dem.bin_index(x, y);
            let vegetation_index = vegetation.bin_index(x, y);
            assert_eq!(dem_index.is_some(), vegetation_index.is_some(), "{x} {y}");
            if let (Some(dem_index), Some(vegetation_index)) = (dem_index, vegetation_index) {
                let (dem_col, dem_row) = (dem_index % dem.width, dem_index / dem.width);
                assert_eq!(
                    (
                        dem_col / DEM_CELLS_PER_VEGETATION_CELL,
                        dem_row / DEM_CELLS_PER_VEGETATION_CELL
                    ),
                    (
                        vegetation_index % vegetation.width,
                        vegetation_index / vegetation.width
                    ),
                    "{x} {y}"
                );
            }
        }
    }

    #[test]
    fn heights_are_above_the_buffered_dem_cell_the_tile_dem_bins_the_point_into() {
        // The buffered grid of `RASTERS_TILE`, each cell holding its index.
        let tile_dem = Grid::new(1000.0, 2002.0, DEM_BLOCK_SIZE, 6, 4, Some(-9999.0), 0.0f32);
        let mut dem_with_buffer = paste_with_buffer(&RASTERS_TILE, &tile_dem, -9999.0, 0.5);
        for (index, value) in dem_with_buffer.values.iter_mut().enumerate() {
            *value = index as f32;
        }
        let ground = GroundElevations::new(&dem_with_buffer);
        let tile_cells = TileCells::new(&RASTERS_TILE, DEM_BLOCK_SIZE);
        let buffer_cells = (BUFFER as f64 / DEM_BLOCK_SIZE) as usize;
        for (x, y) in centimetre_points() {
            // Every point of the tile, and those around it, is above a cell.
            let height = ground.height(x, y, 0.0).unwrap();
            if let Some(index) = tile_cells.bin_index(x, y) {
                let (col, row) = (index % tile_cells.width, index / tile_cells.width);
                let buffered_index =
                    (row + buffer_cells) * dem_with_buffer.width + col + buffer_cells;
                assert_eq!(height, -(buffered_index as f64), "{x} {y}");
            }
        }
    }

    #[test]
    fn heights_are_above_the_cell_east_and_north_of_an_edge() {
        #[rustfmt::skip]
        let dem = Grid {
            min_x: 0.0,
            max_y: 2.0,
            resolution: 0.5,
            width: 4,
            height: 4,
            nodata: Some(-9999.0),
            values: vec![
                0.0, 1.0, 2.0, 3.0,
                4.0, 5.0, 6.0, 7.0,
                8.0, 9.0, -9999.0, 11.0,
                12.0, 13.0, 14.0, 15.0,
            ],
        };
        let ground = GroundElevations::new(&dem);
        assert_eq!(ground.height(0.5, 1.0, 100.0), Some(95.0));
        assert_eq!(ground.height(0.0, 0.0, 100.0), Some(88.0));
        assert_eq!(ground.height(1.99, 1.99, 100.0), Some(97.0));
        // Above a cell without data, and outside the DEM.
        assert_eq!(ground.height(1.2, 0.7, 100.0), None);
        assert_eq!(ground.height(2.0, 1.0, 100.0), None);
        assert_eq!(ground.height(1.0, 2.0, 100.0), None);
    }

    #[test]
    fn height_layers_hold_the_points_above_their_lower_bound_and_up_to_their_upper_one() {
        for (height, layer) in [
            (-1.0, None),
            (0.0, None),
            (0.01, Some(0)),
            (1.0, Some(0)),
            (1.01, Some(1)),
            (4.0, Some(1)),
            (4.01, Some(2)),
            (30.0, Some(2)),
            (30.01, None),
            (f64::NAN, None),
        ] {
            assert_eq!(height_layer(height), layer, "{height}");
        }
    }

    /// Ground, vegetation and building points, some in the same voxel.
    const POINTS: [(f64, f64, f64, u8); 10] = [
        (1000.2, 2001.5, 10.0, GROUND),
        (1000.4, 2001.9, 11.0, GROUND),
        (1000.8, 2001.2, 15.0, GROUND),
        (1000.5, 2001.5, 12.0, LOW_VEGETATION),
        (1000.5, 2001.5, 12.2, HIGH_VEGETATION),
        (1001.5, 2001.5, 13.0, MEDIUM_VEGETATION),
        (1001.5, 2001.5, 14.0, 6),
        (1002.5, 2000.5, 30.0, HIGH_VEGETATION),
        (1002.5, 2000.5, 31.0, HIGH_VEGETATION),
        (1002.5, 2000.5, 20.0, GROUND),
    ];

    #[test]
    fn dem_is_the_mean_of_ground_points_and_vegetation_the_class_counts() {
        let mut ground_dem = GroundDem::new(&RASTERS_TILE);
        let mut vegetation =
            VegetationDensities::new(&RASTERS_TILE, voxels([1000.2, 2001.5, 10.0]), None);
        for (x, y, z, classification) in POINTS {
            ground_dem.add_point(x, y, z, classification);
            vegetation.add_point(x, y, z, classification);
        }
        let dem = ground_dem.grid();
        let [low, medium, high] = vegetation.into_grids();
        let n = NO_DATA_ELEVATION;
        // The cells without ground points take the mean of the ground points
        // of their 1 m block, (10 + 11 + 15) / 3 in the north-west one.
        #[rustfmt::skip]
        assert_eq!(dem.values, [
            10.5, 12.0, n, n, n, n,
            12.0, 15.0, n, n, n, n,
            n, n, n, n, 20.0, 20.0,
            n, n, n, n, 20.0, 20.0,
        ]);
        assert_eq!(dem.nodata, Some(n));
        assert_eq!(
            (dem.min_x, dem.max_y, dem.resolution, dem.width, dem.height),
            (1000.0, 2002.0, DEM_BLOCK_SIZE, 6, 4)
        );
        assert_eq!(low.values, [1, 0, 0, 0, 0, 0]);
        assert_eq!(medium.values, [0, 1, 0, 0, 0, 0]);
        assert_eq!(high.values, [0, 0, 0, 0, 0, 2]);
        assert_eq!(high.nodata, Some(NO_DATA_COUNT));
        assert_eq!(
            (
                high.min_x,
                high.max_y,
                high.resolution,
                high.width,
                high.height
            ),
            (1000.0, 2002.0, 1.0, 3, 2)
        );
    }

    #[test]
    fn vegetation_by_height_counts_the_vegetation_classes_above_the_ground() {
        // Flat ground at 10 m but for a hole, on a grid larger than the tile.
        let mut dem = Grid::new(999.0, 2003.0, 0.5, 10, 10, Some(-9999.0), 10.0f32);
        dem.set(5, 4, -9999.0); // (1001.5, 2000.5)-(1002, 2001)
        let ground = GroundElevations::new(&dem);
        let points = [
            // Low, whatever their class.
            (1000.2, 2001.5, 10.5, UNCLASSIFIED),
            (1000.2, 2001.5, 10.9, HIGH_VEGETATION),
            // In the voxel of the previous point.
            (1000.3, 2001.6, 11.0, LOW_VEGETATION),
            // Medium.
            (1001.5, 2001.5, 13.0, LOW_VEGETATION),
            (1001.5, 2001.5, 14.0, MEDIUM_VEGETATION),
            // Never vegetation: ground, building, water and an unassigned
            // point.
            (1001.5, 2001.5, 12.0, GROUND),
            (1001.5, 2001.5, 15.0, 6),
            (1001.5, 2001.5, 16.0, 9),
            (1001.5, 2001.5, 17.0, 0),
            // High, then on or below the ground, and too high.
            (1002.5, 2000.5, 40.0, MEDIUM_VEGETATION),
            (1002.5, 2000.5, 10.0, HIGH_VEGETATION),
            (1002.5, 2000.5, 9.0, HIGH_VEGETATION),
            (1002.5, 2000.5, 45.0, HIGH_VEGETATION),
            // Above the hole.
            (1001.7, 2000.7, 15.0, HIGH_VEGETATION),
            // Outside the tile.
            (1003.0, 2000.5, 15.0, HIGH_VEGETATION),
        ];
        let mut vegetation =
            VegetationDensities::new(&RASTERS_TILE, voxels([1000.2, 2001.5, 10.5]), Some(&ground));
        for (x, y, z, classification) in points {
            vegetation.add_point(x, y, z, classification);
        }
        let [low, medium, high] = vegetation.into_grids();
        assert_eq!(low.values, [2, 0, 0, 0, 0, 0]);
        assert_eq!(medium.values, [0, 2, 0, 0, 0, 0]);
        assert_eq!(high.values, [0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn voxel_set_keeps_the_first_point_of_each_voxel() {
        let points = [
            [1.0, 1.0, 1.0],
            [1.2, 0.8, 1.24],
            [1.25, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 40.0],
            [1.0, 1.0, 40.2],
            [100.0, 1.0, 1.0],
            [100.1, 1.0, 1.0],
            [1.0, 1.0, 0.76],
        ];
        let mut voxels = VoxelSet::new(points[0], &bounds([0.0; 3], [2.0; 3]));
        let inserted: Vec<bool> = points
            .iter()
            .map(|&[x, y, z]| voxels.insert(x, y, z))
            .collect();
        assert_eq!(
            inserted,
            [true, false, true, true, true, false, true, false, false]
        );
        // The voxel 39 m above its column's first, and the one out of the
        // blocks covering the bounds.
        assert_eq!(voxels.others.len(), 2);
        assert_eq!(voxels.blocks.iter().flatten().count(), 1);
    }

    #[test]
    fn voxel_set_allocates_only_the_blocks_holding_points() {
        let mut voxels = VoxelSet::new([0.25; 3], &bounds([0.0; 3], [1000.0; 3]));
        assert_eq!((voxels.blocks_width, voxels.blocks_height), (32, 32));
        for [x, y] in [[0.0, 0.0], [31.9, 31.9], [32.0, 0.0], [999.9, 999.9]] {
            assert!(voxels.insert(x, y, 0.0));
        }
        assert_eq!(voxels.blocks.iter().flatten().count(), 3);
        assert!(voxels.others.is_empty());
    }
}
