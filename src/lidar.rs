//! The DEM and vegetation density rasters of a LiDAR tile, computed in one
//! streaming pass over its points.
//!
//! They reproduce what the PDAL pipeline Cassini used to run produced. That
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
    error::{Result, ResultContext, TileId},
    grid::{Compression, Grid, Sample},
    helpers::remove_dir_content_except,
    process::ensure_file,
};

/// The ASPRS classifications of ground and of low, medium and high
/// vegetation.
const GROUND: u8 = 2;
const LOW_VEGETATION: u8 = 3;
const MEDIUM_VEGETATION: u8 = 4;
const HIGH_VEGETATION: u8 = 5;

/// The value of the DEM cells without ground points.
const NO_DATA_ELEVATION: f64 = -9999.0;

/// The nodata value of the vegetation rasters. Their cells hold point counts,
/// with 0 for cells without points, so counts saturate just below it.
const NO_DATA_COUNT: u8 = 255;

/// The edge of the cubic voxels the points are thinned with, in metres.
const VOXEL_SIZE: f64 = 0.5;

/// The number of points decompressed at a time. A batch spans many LAZ
/// chunks, which are decompressed in parallel.
const POINTS_PER_BATCH: u64 = 1 << 21;

pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: &Path,
    output_dir_path: &Path,
) -> Result<()> {
    let start = Instant::now();

    let mut reader = open_reader(laz_path)?;
    let bounds = reader.header().bounds();
    let tile = TileId {
        min_x: bounds.min.x.round() as i64,
        min_y: bounds.min.y.round() as i64,
        max_x: bounds.max.x.round() as i64,
        max_y: bounds.max.y.round() as i64,
    };
    let TileId {
        min_x,
        min_y,
        max_x,
        max_y,
    } = tile;

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Computing DEM and vegetation densities",
        min_x, min_y, max_x, max_y
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

    let mut rasters = TileRasters::new(&tile, bounds);
    let mut points = PointDataBuilder::new().for_header(reader.header()).build();
    loop {
        let count = reader
            .fill_points(POINTS_PER_BATCH, &mut points)
            .context(format!(
                "could not read LiDAR points `{}`",
                laz_path.display()
            ))?;
        if count == 0 {
            break;
        }
        rasters.add_points(&points);
    }
    let (dem, [low, medium, high]) = rasters.into_grids();

    let dem_path = output_dir_path.join("dem.tif");
    let low_vegetation_path = output_dir_path.join("low_vegetation.tif");
    let medium_vegetation_path = output_dir_path.join("medium_vegetation.tif");
    let high_vegetation_path = output_dir_path.join("high_vegetation.tif");

    dem.write_geotiff(&dem_path, Compression::Deflate)?;
    low.write_geotiff(&low_vegetation_path, Compression::Deflate)?;
    medium.write_geotiff(&medium_vegetation_path, Compression::Deflate)?;
    high.write_geotiff(&high_vegetation_path, Compression::Deflate)?;

    for output in [
        &dem_path,
        &low_vegetation_path,
        &medium_vegetation_path,
        &high_vegetation_path,
    ] {
        ensure_file(output)?;
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

    Ok(())
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

/// The 1 m rasters of a tile, accumulated point by point.
struct TileRasters {
    tile: TileId,
    width: usize,
    height: usize,
    /// The running mean elevation of the ground points of each cell, and
    /// their number.
    elevations: Vec<f64>,
    ground_counts: Vec<u32>,
    /// The number of low, medium and high vegetation points of each cell.
    vegetation_counts: [Vec<u8>; 3],
    bounds: Bounds,
    /// Created from the first point.
    voxels: Option<VoxelSet>,
}

impl TileRasters {
    /// The empty rasters of `tile`, whose points are within `bounds`.
    fn new(tile: &TileId, bounds: Bounds) -> Self {
        let width = (tile.max_x - tile.min_x) as usize;
        let height = (tile.max_y - tile.min_y) as usize;
        Self {
            tile: *tile,
            width,
            height,
            elevations: vec![0.0; width * height],
            ground_counts: vec![0; width * height],
            vegetation_counts: [(); 3].map(|_| vec![0; width * height]),
            bounds,
            voxels: None,
        }
    }

    /// Adds `points`, which must come in file order, as PDAL read them.
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
            if let Some(index) = self.bin_index(x, y) {
                // `writers.gdal`'s running mean, so the rounding matches.
                self.ground_counts[index] += 1;
                self.elevations[index] +=
                    (z - self.elevations[index]) / self.ground_counts[index] as f64;
            }
        }

        let bounds = &self.bounds;
        let voxels = self
            .voxels
            .get_or_insert_with(|| VoxelSet::new([x, y, z], bounds));
        if !voxels.insert(x, y, z) {
            return;
        }

        let layer = match classification {
            LOW_VEGETATION => 0,
            MEDIUM_VEGETATION => 1,
            HIGH_VEGETATION => 2,
            _ => return,
        };
        if let Some(index) = self.bin_index(x, y) {
            let count = &mut self.vegetation_counts[layer][index];
            *count = count.saturating_add(1).min(NO_DATA_COUNT - 1);
        }
    }

    /// The index of the cell `writers.gdal` bins the point (`x`, `y`) into
    /// in `binmode`, row-major from the north-west corner. A point on the
    /// edge between two cells goes to the east or north one, so the east and
    /// north edges of the tile are out.
    fn bin_index(&self, x: f64, y: f64) -> Option<usize> {
        let col = (x - self.tile.min_x as f64).floor();
        let row_from_south = (y - self.tile.min_y as f64).floor();
        if !(0.0..self.width as f64).contains(&col)
            || !(0.0..self.height as f64).contains(&row_from_south)
        {
            return None;
        }
        Some((self.height - 1 - row_from_south as usize) * self.width + col as usize)
    }

    /// The DEM, and the low, medium and high vegetation densities.
    fn into_grids(self) -> (Grid<f64>, [Grid<u8>; 3]) {
        let elevations = self
            .elevations
            .into_iter()
            .zip(self.ground_counts)
            .map(|(mean, count)| if count == 0 { NO_DATA_ELEVATION } else { mean })
            .collect();
        (
            tile_grid(&self.tile, elevations, NO_DATA_ELEVATION),
            self.vegetation_counts
                .map(|counts| tile_grid(&self.tile, counts, NO_DATA_COUNT)),
        )
    }
}

/// The 1 m grid of `tile` with `values`, row-major from the north-west corner.
fn tile_grid<T: Sample>(tile: &TileId, values: Vec<T>, nodata: T) -> Grid<T> {
    Grid {
        min_x: tile.min_x as f64,
        max_y: tile.max_y as f64,
        resolution: 1.0,
        width: (tile.max_x - tile.min_x) as usize,
        height: (tile.max_y - tile.min_y) as usize,
        nodata: Some(nodata),
        values,
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
struct VoxelSet {
    origin: [f64; 3],
    min_col: i32,
    min_row: i32,
    width: usize,
    height: usize,
    /// The voxel `z` of the lowest bit of each column, row-major from the
    /// south-west, valid once the column has a bit set.
    bases: Vec<i32>,
    bits: Vec<u128>,
    others: HashSet<[i32; 3]>,
}

/// The number of voxels of a column bitset, 64 m.
const COLUMN_VOXELS: i32 = u128::BITS as i32;

impl VoxelSet {
    fn new(first_point: [f64; 3], bounds: &Bounds) -> Self {
        let origin = first_point.map(|coordinate| coordinate - VOXEL_SIZE / 2.0);
        let min_col = voxel_index(bounds.min.x, origin[0]);
        let min_row = voxel_index(bounds.min.y, origin[1]);
        let width = (voxel_index(bounds.max.x, origin[0]) - min_col + 1).max(0) as usize;
        let height = (voxel_index(bounds.max.y, origin[1]) - min_row + 1).max(0) as usize;
        Self {
            origin,
            min_col,
            min_row,
            width,
            height,
            bases: vec![0; width * height],
            bits: vec![0; width * height],
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
        if let Some(column) = self.column(voxel) {
            if self.bits[column] == 0 {
                self.bases[column] = voxel[2].saturating_sub(COLUMN_VOXELS / 2);
            }
            let offset = i64::from(voxel[2]) - i64::from(self.bases[column]);
            if (0..i64::from(COLUMN_VOXELS)).contains(&offset) {
                let mask = 1 << offset;
                let inserted = self.bits[column] & mask == 0;
                self.bits[column] |= mask;
                return inserted;
            }
        }
        self.others.insert(voxel)
    }

    /// The index of the column of `voxel`, if within the header bounds.
    fn column(&self, voxel: [i32; 3]) -> Option<usize> {
        let col = usize::try_from(i64::from(voxel[0]) - i64::from(self.min_col)).ok()?;
        let row = usize::try_from(i64::from(voxel[1]) - i64::from(self.min_row)).ok()?;
        (col < self.width && row < self.height).then_some(row * self.width + col)
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

    fn rasters() -> TileRasters {
        let tile = TileId {
            min_x: 1000,
            min_y: 2000,
            max_x: 1003,
            max_y: 2002,
        };
        TileRasters::new(&tile, bounds([1000.0, 2000.0, 0.0], [1003.0, 2002.0, 50.0]))
    }

    #[test]
    fn points_on_cell_edges_are_binned_east_and_north() {
        let rasters = rasters();
        assert_eq!(rasters.bin_index(1000.0, 2000.0), Some(3));
        assert_eq!(rasters.bin_index(1000.5, 2001.0), Some(0));
        assert_eq!(rasters.bin_index(1002.999, 2001.999), Some(2));
        assert_eq!(rasters.bin_index(1003.0, 2001.0), None);
        assert_eq!(rasters.bin_index(1001.0, 2002.0), None);
        assert_eq!(rasters.bin_index(999.99, 2001.0), None);
        assert_eq!(rasters.bin_index(f64::NAN, 2001.0), None);
    }

    #[test]
    fn dem_is_the_mean_of_ground_points_and_vegetation_the_class_counts() {
        let mut rasters = rasters();
        for (x, y, z, classification) in [
            (1000.2, 2001.5, 10.0, GROUND),
            (1000.8, 2001.2, 11.0, GROUND),
            (1000.5, 2001.5, 12.0, LOW_VEGETATION),
            (1000.5, 2001.5, 12.2, HIGH_VEGETATION),
            (1001.5, 2001.5, 13.0, MEDIUM_VEGETATION),
            (1001.5, 2001.5, 14.0, 6),
            (1002.5, 2000.5, 30.0, HIGH_VEGETATION),
            (1002.5, 2000.5, 31.0, HIGH_VEGETATION),
            (1002.5, 2000.5, 20.0, GROUND),
        ] {
            rasters.add_point(x, y, z, classification);
        }
        let (dem, [low, medium, high]) = rasters.into_grids();
        let n = NO_DATA_ELEVATION;
        assert_eq!(dem.values, [10.5, n, n, n, n, 20.0]);
        assert_eq!(dem.nodata, Some(n));
        assert_eq!(low.values, [1, 0, 0, 0, 0, 0]);
        assert_eq!(medium.values, [0, 1, 0, 0, 0, 0]);
        assert_eq!(high.values, [0, 0, 0, 0, 0, 2]);
        assert_eq!(high.nodata, Some(NO_DATA_COUNT));
        assert_eq!(
            (high.min_x, high.max_y, high.width, high.height),
            (1000.0, 2002.0, 3, 2)
        );
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
            [5.0, 1.0, 1.0],
            [5.1, 1.0, 1.0],
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
        assert_eq!(voxels.others.len(), 2);
    }
}
