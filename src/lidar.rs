//! The DEM and vegetation density rasters of a LiDAR tile, computed in one
//! streaming pass over its points. Given a DEM file, such as IGN's, the
//! points are read for the vegetation only and the DEM is copied.
//!
//! The vegetation rasters reproduce what the PDAL pipeline Cassini used to
//! run produced, and the DEM is computed the same way, but on
//! `DEM_BLOCK_SIZE` cells instead of 1 m ones, the cells without ground
//! points taking the mean of their 1 m block. That
//! pipeline (`writers.gdal`, `filters.hag_dem`, `filters.voxeldownsize`,
//! `filters.assign`) was meant to classify vegetation by height above ground,
//! but PDAL ran it in stream mode, where `filters.hag_dem` opens the DEM
//! before `writers.gdal` has written it. PDAL up to 2.9 ignores that error and
//! leaves every height at 0, so no point was reclassified: the vegetation
//! rasters count the low, medium and high vegetation classes of the LiDAR HD
//! classification, after voxel thinning.

use las::{Bounds, PointData, PointDataBuilder, Reader};
use log::info;
use std::collections::HashSet;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, Write};
use std::path::Path;
use std::time::Instant;

use crate::{
    constants::{DEM_BLOCK_SIZE, VEGETATION_BLOCK_SIZE},
    error::{CassiniError, Result, ResultContext, TileId},
    external_dem::read_external_dem,
    grid::{Compression, Grid, Sample},
    helpers::remove_dir_content_except,
};

/// The ASPRS classifications of ground and of low, medium and high
/// vegetation.
const GROUND: u8 = 2;
const LOW_VEGETATION: u8 = 3;
const MEDIUM_VEGETATION: u8 = 4;
const HIGH_VEGETATION: u8 = 5;

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
/// `laz_path` to `output_dir_path`, returning the tile's extent. The DEM is
/// the one at `dem_path` if given, else it is computed from the ground
/// points.
pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: &Path,
    dem_path: Option<&Path>,
    output_dir_path: &Path,
) -> Result<TileId> {
    let start = Instant::now();

    let mut reader = open_reader(laz_path)?;
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

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Computing {}vegetation densities",
        min_x,
        min_y,
        max_x,
        max_y,
        if external_dem.is_some() {
            ""
        } else {
            "DEM and "
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

    let mut points = PointDataBuilder::new().for_header(reader.header()).build();
    let mut count = fill_points(&mut reader, &mut points, laz_path)?;
    // A file without points has no voxel to thin, so any origin will do.
    let first_point = points
        .x()
        .zip(points.y())
        .zip(points.z())
        .map(|((x, y), z)| [x, y, z])
        .next()
        .unwrap_or_default();
    let mut rasters = TileRasters::new(&tile, VoxelSet::new(first_point, &bounds), external_dem);
    while count > 0 {
        rasters.add_points(&points);
        count = fill_points(&mut reader, &mut points, laz_path)?;
    }
    let (dem, [low, medium, high]) = rasters.into_grids();

    dem.write_geotiff(&output_dir_path.join("dem.tif"), Compression::Deflate)?;
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
        .write_all(format!("{min_x}|{min_y}|{max_x}|{max_y}").as_bytes())
        .context(format!("could not write `{}`", extent_path.display()))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. DEM and vegetation densities computed in {:.1?}",
        min_x, min_y, max_x, max_y, duration
    );

    Ok(tile)
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

/// A reader of the LAS or LAZ file at `laz_path`. LAZ chunks are decompressed
/// on rayon's global thread pool, which the tiles of a batch share, so the
/// batch threads do not oversubscribe the CPU.
fn open_reader(laz_path: &Path) -> Result<Reader> {
    let file = File::open(laz_path).context(format!(
        "could not open LiDAR file `{}`",
        laz_path.display()
    ))?;
    Reader::new(BufReader::new(file)).context(format!(
        "could not read LiDAR header `{}`",
        laz_path.display()
    ))
}

/// Replaces `points` with the next points of `reader`, returning how many
/// were read, 0 at the end of the file.
fn fill_points(reader: &mut Reader, points: &mut PointData, laz_path: &Path) -> Result<u64> {
    reader
        .fill_points(POINTS_PER_BATCH, points)
        .context(format!(
            "could not read LiDAR points `{}`",
            laz_path.display()
        ))
}

/// The rasters of a tile, accumulated point by point: the DEM, and the
/// vegetation densities at `VEGETATION_BLOCK_SIZE`.
struct TileRasters {
    dem: TileDem,
    vegetation_cells: TileCells,
    /// The number of low, medium and high vegetation points of each
    /// vegetation cell.
    vegetation_counts: [Vec<u8>; 3],
    voxels: VoxelSet,
}

impl TileRasters {
    /// The empty rasters of `tile`, whose points are thinned with `voxels`,
    /// with `external_dem` as the DEM if given, else that of the ground
    /// points.
    fn new(tile: &TileId, voxels: VoxelSet, external_dem: Option<Grid<f32>>) -> Self {
        let vegetation_cells = TileCells::new(tile, VEGETATION_BLOCK_SIZE);
        let vegetation_len = vegetation_cells.width * vegetation_cells.height;
        Self {
            dem: match external_dem {
                Some(dem) => TileDem::External(dem),
                None => TileDem::Ground(GroundDem::new(tile)),
            },
            vegetation_cells,
            vegetation_counts: [(); 3].map(|_| vec![0; vegetation_len]),
            voxels,
        }
    }

    /// Adds `points`, which must come in file order. PDAL read LiDAR HD's
    /// COPC files in octree order instead, which only changes the voxel
    /// origin and the rounding of the DEM mean.
    fn add_points(&mut self, points: &PointData) {
        let columns = points
            .x()
            .zip(points.y())
            .zip(points.z())
            .zip(points.classification());
        for (((x, y), z), classification) in columns {
            self.add_point(x, y, z, classification);
        }
    }

    fn add_point(&mut self, x: f64, y: f64, z: f64, classification: u8) {
        // The DEM writer comes before the thinning, so it gets every point.
        if classification == GROUND {
            if let TileDem::Ground(ground_dem) = &mut self.dem {
                ground_dem.add_point(x, y, z);
            }
        }

        if !self.voxels.insert(x, y, z) {
            return;
        }

        let layer = match classification {
            LOW_VEGETATION => 0,
            MEDIUM_VEGETATION => 1,
            HIGH_VEGETATION => 2,
            _ => return,
        };
        if let Some(index) = self.vegetation_cells.bin_index(x, y) {
            let count = &mut self.vegetation_counts[layer][index];
            *count = count.saturating_add(1).min(NO_DATA_COUNT - 1);
        }
    }

    /// The DEM, and the low, medium and high vegetation densities.
    fn into_grids(self) -> (Grid<f32>, [Grid<u8>; 3]) {
        let dem = match self.dem {
            TileDem::External(dem) => dem,
            TileDem::Ground(ground_dem) => ground_dem.grid(),
        };
        let vegetation_cells = self.vegetation_cells;
        (
            dem,
            self.vegetation_counts
                .map(|counts| vegetation_cells.grid(counts, NO_DATA_COUNT)),
        )
    }
}

/// Where the DEM of a tile comes from.
enum TileDem {
    /// A DEM file, such as IGN's, read before the points.
    External(Grid<f32>),
    /// The ground points, at `DEM_BLOCK_SIZE`.
    Ground(GroundDem),
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

    fn add_point(&mut self, x: f64, y: f64, z: f64) {
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

/// The square cells of `resolution` metres covering exactly a tile, which
/// points are binned into.
#[derive(Clone, Copy)]
struct TileCells {
    tile: TileId,
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
            tile: *tile,
            resolution,
            width: cells(tile.min_x, tile.max_x),
            height: cells(tile.min_y, tile.max_y),
        }
    }

    /// The index of the cell the point (`x`, `y`) is binned into, as
    /// `writers.gdal` bins it in `binmode`, row-major from the north-west
    /// corner. A point on the
    /// edge between two cells goes to the east or north one, so the east and
    /// north edges of the tile are out.
    fn bin_index(&self, x: f64, y: f64) -> Option<usize> {
        let col = ((x - self.tile.min_x as f64) / self.resolution).floor();
        let row_from_south = ((y - self.tile.min_y as f64) / self.resolution).floor();
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
            min_x: self.tile.min_x as f64,
            max_y: self.tile.max_y as f64,
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

    fn rasters() -> TileRasters {
        rasters_with(None)
    }

    fn rasters_with(external_dem: Option<Grid<f32>>) -> TileRasters {
        let bounds = bounds([1000.0, 2000.0, 0.0], [1003.0, 2002.0, 50.0]);
        // The first point of `dem_is_the_mean_of_ground_points_and_vegetation_the_class_counts`.
        TileRasters::new(
            &RASTERS_TILE,
            VoxelSet::new([1000.2, 2001.5, 10.0], &bounds),
            external_dem,
        )
    }

    fn ground_dem(rasters: &TileRasters) -> &GroundDem {
        match &rasters.dem {
            TileDem::Ground(ground_dem) => ground_dem,
            TileDem::External(_) => panic!("expected the DEM of the ground points"),
        }
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
        let rasters = rasters();
        let vegetation = rasters.vegetation_cells;
        assert_eq!(vegetation.bin_index(1000.0, 2000.0), Some(3));
        assert_eq!(vegetation.bin_index(1000.5, 2001.0), Some(0));
        assert_eq!(vegetation.bin_index(1002.999, 2001.999), Some(2));
        assert_eq!(vegetation.bin_index(1003.0, 2001.0), None);
        assert_eq!(vegetation.bin_index(1001.0, 2002.0), None);
        assert_eq!(vegetation.bin_index(999.99, 2001.0), None);
        assert_eq!(vegetation.bin_index(f64::NAN, 2001.0), None);

        let dem = ground_dem(&rasters).cells;
        assert_eq!((dem.width, dem.height), (6, 4));
        assert_eq!(dem.bin_index(1000.0, 2000.0), Some(18));
        assert_eq!(dem.bin_index(1000.5, 2001.0), Some(7));
        assert_eq!(dem.bin_index(1000.49, 2001.5), Some(0));
        assert_eq!(dem.bin_index(1002.999, 2001.999), Some(5));
        assert_eq!(dem.bin_index(1003.0, 2001.0), None);
        assert_eq!(dem.bin_index(1001.0, 2002.0), None);
    }

    #[test]
    fn dem_cells_lie_inside_the_vegetation_cell_of_their_points() {
        let rasters = rasters();
        let (dem, vegetation) = (ground_dem(&rasters).cells, rasters.vegetation_cells);
        // A 1 cm grid of points over the tile, edges included, as in LiDAR HD.
        for x in 99_990..=100_310 {
            for y in 199_990..=200_210 {
                let (x, y) = (x as f64 / 100.0, y as f64 / 100.0);
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
        let mut rasters = rasters();
        for (x, y, z, classification) in POINTS {
            rasters.add_point(x, y, z, classification);
        }
        let (dem, [low, medium, high]) = rasters.into_grids();
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
    fn vegetation_is_the_same_with_an_external_dem() {
        let external_dem = Grid::new(999.75, 2002.25, 0.5, 6, 4, Some(-9999.0), 7.0f32);
        let (mut ground, mut external) = (rasters(), rasters_with(Some(external_dem.clone())));
        for (x, y, z, classification) in POINTS {
            ground.add_point(x, y, z, classification);
            external.add_point(x, y, z, classification);
        }
        let (_, vegetation) = ground.into_grids();
        // The DEM is the external one, untouched by the ground points.
        assert_eq!(external.into_grids(), (external_dem, vegetation));
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
