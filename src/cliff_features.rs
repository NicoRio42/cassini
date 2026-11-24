//! Vectorizes the cliffs into ISOM features for the OCAD export.
//!
//! The cells steeper than `cliff_threshold_1`, which `cliffs.png` draws as
//! dots, are grouped into 8-connected components. Each component becomes:
//!
//! - a 205 large boulder when it fits in the symbol and stands above most
//!   of the ground around it,
//! - else 206 gigantic boulder or rock pillar areas, for its parts wider
//!   than a 202 line that are larger than the 205 symbol. They are drawn as
//!   large as the dots of `cliffs.png` would draw them,
//! - and 202 cliff lines along the middle of the rest of its steep band,
//!   from the edge of the areas, when they are long enough. A small
//!   component that does not stand above the ground around it, like a short
//!   cliff, gives a short line. The flat top of a boulder too large for the
//!   205 symbol is not steep, so the line closes around it.
//!
//! Neighbouring tiles find the same components around their common edge,
//! from the same cells, so that they classify and vectorize them alike.

use crate::{
    config::Config,
    constants::{BUFFER, CLIFF_THICKNESS_2, DEM_BLOCK_SIZE},
    error::{CassiniError, Result, ResultContext},
    map_features::{AreaSymbol, Extent, LineSymbol, MapFeature, PointSymbol},
    tile::Tile,
    vegetation_areas::{simplify_ring, squared_segment_distance, FeatureSink, PinnedLines, Tracer},
};
use contour::ContourBuilder;
use image::{GrayImage, Luma};
use imageproc::{
    distance_transform::euclidean_squared_distance_transform,
    geometry::convex_hull,
    point::Point as ImagePoint,
    region_labelling::{connected_components, Connectivity},
};
use log::info;
use shapefile::{Point, Polyline};
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    path::Path,
    time::Instant,
};
use tiff::decoder::{Decoder, DecodingResult};

const CELL_SIZE: f64 = DEM_BLOCK_SIZE as f64;
/// Metres vectorized around the tile, so that the components crossing the
/// tile edge are vectorized whole.
const MARGIN: usize = 100;
const _: () = assert!(MARGIN < BUFFER);
/// Components of fewer cells are noise.
const MIN_COMPONENT_CELLS: usize = 4;
/// Diameter of the 205 large boulder symbol on the ground, 0.6 mm at
/// 1:15 000. A component that fits in it is drawn as the symbol.
const LARGE_BOULDER_SIZE: f64 = 9.0;
/// Width of the 202 cliff line on the ground, 0.25 mm at 1:15 000. A steep
/// band wider than the line is drawn as a 206 area.
const CLIFF_LINE_WIDTH: f64 = 3.75;
/// A boulder stands this many metres above the ground around it at least...
const BOULDER_MIN_HEIGHT: f32 = 1.0;
/// ... on this fraction of the ground around it at least.
const FREE_STANDING_FRACTION: f64 = 0.75;
/// The ground around a component lies between these distances from it, in
/// metres.
const SURROUNDINGS_MIN_DISTANCE: f64 = 1.0;
const SURROUNDINGS_MAX_DISTANCE: f64 = 2.0;
/// The cells of a component's patch around it, which holds its surroundings.
const PATCH_PADDING: usize = (SURROUNDINGS_MAX_DISTANCE / CELL_SIZE) as usize + 1;
/// Holes in a component smaller than this, in m², are flat spots in a steep
/// band rather than the top of a rock, and are filled before tracing its
/// cliff lines.
const MIN_HOLE_AREA: f64 = 10.0;
/// The cliff lines of a component shorter than this in total, in metres,
/// are dropped.
const MIN_CLIFF_LENGTH: f64 = 3.0;
/// The branches of a cliff line shorter than this, in metres, ending free
/// are noise of the steep band's outline.
const SPUR_LENGTH: f64 = 2.0;
/// The chains of a cliff line's skeleton shorter than this, in metres, are
/// dropped. They join the cells of a junction.
const MIN_CHAIN_LENGTH: f64 = 1.0;
/// The chains meeting at a junction are joined two by two when they make at
/// least this angle, in degrees...
const JOIN_MIN_ANGLE: f64 = 120.0;
/// ... measured between their directions over this many cells from it.
const JOIN_DIRECTION_CELLS: usize = 4;
/// Cliff lines are smoothed with a moving average over this many cells on
/// either side, which removes the staircase of the cells.
const SMOOTHING_RADIUS: usize = 2;
/// The PNG renderer draws each steep cell as a dot of `CLIFF_THICKNESS_2`
/// x 10 m across on the ground. The 206 areas are found where these dots
/// merge, which gives them the same outline: a minimum width, and no notches
/// narrower than the dots.
const AREA_GROWTH: f64 = CLIFF_THICKNESS_2 as f64 * 10.0 / 2.0;
/// The 206 areas are simplified with this many times the tolerance of the
/// lines.
const AREA_SIMPLIFY_FACTOR: f64 = 2.0;
/// 206 areas and holes smaller than this, in m², are slivers.
const SLIVER_AREA: f64 = 1.0;
/// The DEM value of the cells without data, as PDAL writes it.
const DEM_NO_DATA: f32 = -9999.0;

/// Returns the cliffs of the tile: 205 boulders and 202 cliff lines around
/// it, which the OCAD export clips to it, and 206 areas clipped to it. Their
/// lines and boundaries are simplified with `simplify_tolerance` metres (0
/// to keep every vertex).
pub fn get_cliff_features(
    tile: &Tile,
    config: &Config,
    simplify_tolerance: f64,
) -> Result<Vec<MapFeature>> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vectorizing cliffs",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let tile_extent = Extent {
        min_x: tile.min_x as f64,
        min_y: tile.min_y as f64,
        max_x: tile.max_x as f64,
        max_y: tile.max_y as f64,
    };
    let grid = Grid::around(&tile_extent);
    let slopes = read_window(&tile.render_dir_path.join("slopes.tif"), &tile_extent, grid)?;
    let dem = read_window(
        &tile.render_dir_path.join("dem_with_buffer.tif"),
        &tile_extent,
        grid,
    )?;

    let features = vectorize_cliffs(
        grid,
        &tile_extent,
        &slopes,
        &dem,
        config.cliff_threshold_1,
        simplify_tolerance,
    )?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. {} cliffs vectorized in {:.1?}",
        tile.min_x,
        tile.min_y,
        tile.max_x,
        tile.max_y,
        features.len(),
        duration
    );

    Ok(features)
}

/// Reads the window of `grid` out of a 0.5 m raster covering the tile and
/// its buffer.
fn read_window(path: &Path, tile: &Extent, grid: Grid) -> Result<Vec<f32>> {
    let file = File::open(path).context(format!("could not open `{}`", path.display()))?;
    let mut decoder = Decoder::new(file)
        .context(format!("could not create decoder for `{}`", path.display()))?
        .with_limits(tiff::decoder::Limits::unlimited());

    let (width, height) = decoder.dimensions().context(format!(
        "could not read dimensions from `{}`",
        path.display()
    ))?;
    let buffer = 2.0 * BUFFER as f64;
    let expected_width = ((tile.max_x - tile.min_x + buffer) / CELL_SIZE) as usize;
    let expected_height = ((tile.max_y - tile.min_y + buffer) / CELL_SIZE) as usize;
    if (width as usize, height as usize) != (expected_width, expected_height) {
        return Err(CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message: format!(
                "expected {expected_width} x {expected_height} cells, found {width} x {height}"
            ),
        });
    }

    let DecodingResult::F32(values) = decoder
        .read_image()
        .context(format!("could not decode `{}`", path.display()))?
    else {
        return Err(CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message: "expected a 32-bit floating-point TIFF band".to_owned(),
        });
    };

    let offset = ((BUFFER - MARGIN) as f64 / CELL_SIZE) as usize;
    Ok((0..grid.height)
        .flat_map(|y| {
            let start = (y + offset) * expected_width + offset;
            values[start..start + grid.width].iter().copied()
        })
        .collect())
}

/// The vectorized window: the tile and a margin of `MARGIN` metres, as
/// 0.5 m cells row-major from the north-west corner.
#[derive(Debug, Clone, Copy)]
struct Grid {
    width: usize,
    height: usize,
    extent: Extent,
}

impl Grid {
    fn around(tile: &Extent) -> Self {
        let extent = tile.expanded(MARGIN as f64);
        Self {
            width: ((extent.max_x - extent.min_x) / CELL_SIZE) as usize,
            height: ((extent.max_y - extent.min_y) / CELL_SIZE) as usize,
            extent,
        }
    }

    fn len(self) -> usize {
        self.width * self.height
    }

    /// The projected coordinates of the centre of cell (`x`, `y`).
    fn center(self, x: usize, y: usize) -> [f64; 2] {
        [
            self.extent.min_x + (x as f64 + 0.5) * CELL_SIZE,
            self.extent.max_y - (y as f64 + 0.5) * CELL_SIZE,
        ]
    }

    /// The cells of each 8-connected component of `mask`, in row-major order.
    fn components(self, mask: &[bool]) -> Vec<Vec<usize>> {
        let labels = connected_components(
            &mask_image(self.width, self.height, mask.iter().copied()),
            Connectivity::Eight,
            Luma([0u8]),
        );

        let mut components: Vec<Vec<usize>> = vec![];
        for (cell, label) in labels.into_raw().into_iter().enumerate() {
            if label == 0 {
                continue;
            }
            let index = label as usize - 1;
            if index >= components.len() {
                components.resize_with(index + 1, Vec::new);
            }
            components[index].push(cell);
        }

        components
    }
}

fn mask_image(width: usize, height: usize, mask: impl Iterator<Item = bool>) -> GrayImage {
    GrayImage::from_raw(
        width as u32,
        height as u32,
        mask.map(|cell| if cell { 255 } else { 0 }).collect(),
    )
    .unwrap()
}

/// Squared distance, in cells, from each cell to the nearest cell of `mask`.
fn squared_distances(width: usize, height: usize, mask: impl Iterator<Item = bool>) -> Vec<f64> {
    euclidean_squared_distance_transform(&mask_image(width, height, mask)).into_raw()
}

fn vectorize_cliffs(
    grid: Grid,
    tile: &Extent,
    slopes: &[f32],
    dem: &[f32],
    threshold: f32,
    simplify_tolerance: f64,
) -> Result<Vec<MapFeature>> {
    let mut steep: Vec<bool> = slopes.iter().map(|slope| *slope > threshold).collect();
    let mut features: Vec<MapFeature> = vec![];

    // 205 boulders, and noise. The small components away from the tile are
    // left in, they give no area, and no line crossing the tile.
    for cells in grid.components(&steep) {
        let boulder = if cells.len() < MIN_COMPONENT_CELLS {
            None
        } else {
            let component = Component::new(grid, &cells);
            if !component.overlaps(tile) || component.diameter() > LARGE_BOULDER_SIZE {
                continue;
            }
            let Some(center) = component.free_standing_center(dem) else {
                continue;
            };
            Some(center)
        };

        for cell in &cells {
            steep[*cell] = false;
        }
        if let Some([x, y]) = boulder {
            features.push(MapFeature::Point(
                PointSymbol::LargeBoulder205,
                Point::new(x, y),
            ));
        }
    }

    // 206 areas: where the dots of `cliffs.png` merge into black wider than
    // a 202 line drawn over dots, the union of the discs of that width that
    // fit in it, when it is larger than the 205 symbol
    let squared_distances_to_steep =
        squared_distances(grid.width, grid.height, steep.iter().copied());
    let squared_growth = (AREA_GROWTH / CELL_SIZE).powi(2);
    let squared_distances_to_dotless = squared_distances(
        grid.width,
        grid.height,
        squared_distances_to_steep
            .iter()
            .map(|squared_distance| *squared_distance > squared_growth),
    );
    let squared_radius = ((CLIFF_LINE_WIDTH / 2.0 + AREA_GROWTH) / CELL_SIZE).powi(2);
    let is_center: Vec<bool> = squared_distances_to_dotless
        .iter()
        .map(|squared_distance| *squared_distance > squared_radius)
        .collect();
    let squared_distances_to_centers =
        squared_distances(grid.width, grid.height, is_center.iter().copied());
    let wide: Vec<bool> = squared_distances_to_centers
        .iter()
        .map(|squared_distance| *squared_distance <= squared_radius)
        .collect();

    let mut area_centers = vec![false; grid.len()];
    let mut has_areas = false;
    for cells in grid.components(&wide) {
        if Component::new(grid, &cells).diameter() > LARGE_BOULDER_SIZE {
            for cell in cells {
                area_centers[cell] = is_center[cell];
            }
            has_areas = true;
        }
    }
    // Positive in the areas: within the radius of their discs' centres
    let area_field: Vec<f32> = squared_distances(grid.width, grid.height, area_centers.into_iter())
        .into_iter()
        .map(|squared_distance| {
            ((squared_radius.sqrt() - squared_distance.sqrt()) * CELL_SIZE) as f32
        })
        .collect();

    // 202 lines: the other steep cells, outside the areas, and a little
    // inside so that the lines leaving an area touch it
    let thin: Vec<bool> = (0..grid.len())
        .map(|cell| steep[cell] && area_field[cell] < CELL_SIZE as f32)
        .collect();

    for cells in grid.components(&thin) {
        if cells.len() < MIN_COMPONENT_CELLS {
            continue;
        }
        let component = Component::new(grid, &cells);
        if !component.overlaps(tile) {
            continue;
        }

        features.extend(
            component
                .cliff_lines(simplify_tolerance)
                .into_iter()
                .map(|line| {
                    MapFeature::Line(
                        LineSymbol::Cliff202,
                        Polyline::new(line.into_iter().map(|[x, y]| Point::new(x, y)).collect()),
                    )
                }),
        );
    }

    if has_areas {
        let area_tolerance = AREA_SIMPLIFY_FACTOR * simplify_tolerance;
        let tracer = Tracer {
            // Values at the cell centres
            builder: ContourBuilder::new(grid.width, grid.height, true)
                .x_origin(grid.extent.min_x + CELL_SIZE / 2.0)
                .y_origin(grid.extent.max_y - CELL_SIZE / 2.0)
                .x_step(CELL_SIZE)
                .y_step(-CELL_SIZE),
            pinned_lines: PinnedLines::around_tile_edges(
                tile,
                area_tolerance.min(MARGIN as f64 / 2.0),
            ),
            window: grid.extent,
            simplify_tolerance: area_tolerance,
        };
        let mut sink = FeatureSink {
            features,
            window: grid.extent,
            tile: *tile,
        };
        sink.push(
            AreaSymbol::GiganticBoulderOrRockPillar206,
            tracer.trace(&area_field)?,
            SLIVER_AREA,
        );
        features = sink.features;
    }

    Ok(features)
}

/// A component of steep cells, in a patch of the grid around it.
struct Component {
    grid: Grid,
    /// The north-west cell of the patch in the grid.
    x0: usize,
    y0: usize,
    width: usize,
    height: usize,
    /// Whether each cell of the patch, row-major, is in the component.
    cells: Vec<bool>,
}

impl Component {
    fn new(grid: Grid, cells: &[usize]) -> Self {
        let xs = || cells.iter().map(|cell| cell % grid.width);
        let ys = || cells.iter().map(|cell| cell / grid.width);
        let x0 = xs().min().unwrap().saturating_sub(PATCH_PADDING);
        let y0 = ys().min().unwrap().saturating_sub(PATCH_PADDING);
        let width = (xs().max().unwrap() + PATCH_PADDING + 1).min(grid.width) - x0;
        let height = (ys().max().unwrap() + PATCH_PADDING + 1).min(grid.height) - y0;

        let mut patch = vec![false; width * height];
        for cell in cells {
            let (x, y) = (cell % grid.width - x0, cell / grid.width - y0);
            patch[y * width + x] = true;
        }

        Self {
            grid,
            x0,
            y0,
            width,
            height,
            cells: patch,
        }
    }

    /// The projected coordinates of the centre of patch cell `index`.
    fn center(&self, index: usize) -> [f64; 2] {
        self.grid
            .center(self.x0 + index % self.width, self.y0 + index / self.width)
    }

    fn grid_index(&self, index: usize) -> usize {
        (self.y0 + index / self.width) * self.grid.width + self.x0 + index % self.width
    }

    fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.cells.len()).filter(|index| self.cells[*index])
    }

    fn overlaps(&self, extent: &Extent) -> bool {
        let [min_x, max_y] = self.grid.center(self.x0, self.y0);
        let [max_x, min_y] = self
            .grid
            .center(self.x0 + self.width - 1, self.y0 + self.height - 1);
        min_x <= extent.max_x
            && max_x >= extent.min_x
            && min_y <= extent.max_y
            && max_y >= extent.min_y
    }

    /// The largest distance across the component, in metres.
    fn diameter(&self) -> f64 {
        let points: Vec<ImagePoint<i32>> = self
            .indices()
            .map(|index| ImagePoint::new((index % self.width) as i32, (index / self.width) as i32))
            .collect();
        let hull = convex_hull(points);

        let mut max_squared_distance = 0;
        for (index, a) in hull.iter().enumerate() {
            for b in &hull[index + 1..] {
                max_squared_distance =
                    max_squared_distance.max((a.x - b.x).pow(2) + (a.y - b.y).pow(2));
            }
        }

        // From the outer edges of the cells
        ((max_squared_distance as f64).sqrt() + 1.0) * CELL_SIZE
    }

    /// The centre of the component when its top stands above the ground
    /// around it.
    fn free_standing_center(&self, dem: &[f32]) -> Option<[f64; 2]> {
        let footprint = self.footprint();
        let footprint_cells: Vec<usize> = (0..footprint.len())
            .filter(|index| footprint[*index])
            .collect();
        let top = footprint_cells
            .iter()
            .map(|index| dem[self.grid_index(*index)])
            .fold(f32::MIN, f32::max);

        let squared_distances = squared_distances(self.width, self.height, footprint.into_iter());
        let (min, max) = (
            (SURROUNDINGS_MIN_DISTANCE / CELL_SIZE).powi(2),
            (SURROUNDINGS_MAX_DISTANCE / CELL_SIZE).powi(2),
        );
        let surroundings: Vec<f32> = (0..squared_distances.len())
            .filter(|index| squared_distances[*index] > min && squared_distances[*index] <= max)
            .map(|index| dem[self.grid_index(index)])
            .filter(|height| *height > DEM_NO_DATA)
            .collect();
        if surroundings.is_empty() {
            return None;
        }

        let lower = surroundings
            .iter()
            .filter(|height| **height < top - BOULDER_MIN_HEIGHT)
            .count();
        if (lower as f64) < FREE_STANDING_FRACTION * surroundings.len() as f64 {
            return None;
        }

        let count = footprint_cells.len() as f64;
        let [sum_x, sum_y] = footprint_cells
            .iter()
            .map(|index| self.center(*index))
            .fold([0.0, 0.0], |[sum_x, sum_y], [x, y]| [sum_x + x, sum_y + y]);
        Some([sum_x / count, sum_y / count])
    }

    /// The component with its holes filled: the patch cells that cannot be
    /// reached from the patch border without crossing the component.
    fn footprint(&self) -> Vec<bool> {
        let mut outside = vec![false; self.cells.len()];
        let mut stack: Vec<usize> = (0..self.cells.len())
            .filter(|index| {
                let (x, y) = (index % self.width, index / self.width);
                x == 0 || y == 0 || x == self.width - 1 || y == self.height - 1
            })
            .filter(|index| !self.cells[*index])
            .collect();
        for index in &stack {
            outside[*index] = true;
        }

        while let Some(index) = stack.pop() {
            let (x, y) = (index % self.width, index / self.width);
            for neighbor in [
                (x > 0).then(|| index - 1),
                (x + 1 < self.width).then(|| index + 1),
                (y > 0).then(|| index - self.width),
                (y + 1 < self.height).then(|| index + self.width),
            ]
            .into_iter()
            .flatten()
            {
                if !self.cells[neighbor] && !outside[neighbor] {
                    outside[neighbor] = true;
                    stack.push(neighbor);
                }
            }
        }

        outside.into_iter().map(|outside| !outside).collect()
    }

    /// The component with its holes smaller than `MIN_HOLE_AREA` filled,
    /// which would give small loops.
    fn without_small_holes(&self) -> Vec<bool> {
        let footprint = self.footprint();
        let mut filled = self.cells.clone();
        let mut visited = vec![false; self.cells.len()];

        for start in 0..self.cells.len() {
            if !footprint[start] || self.cells[start] || visited[start] {
                continue;
            }
            visited[start] = true;
            let mut hole = vec![];
            let mut stack = vec![start];
            while let Some(index) = stack.pop() {
                hole.push(index);
                let (x, y) = (index % self.width, index / self.width);
                for neighbor in [
                    (x > 0).then(|| index - 1),
                    (x + 1 < self.width).then(|| index + 1),
                    (y > 0).then(|| index - self.width),
                    (y + 1 < self.height).then(|| index + self.width),
                ]
                .into_iter()
                .flatten()
                {
                    if !self.cells[neighbor] && !visited[neighbor] {
                        visited[neighbor] = true;
                        stack.push(neighbor);
                    }
                }
            }

            if (hole.len() as f64) * CELL_SIZE * CELL_SIZE < MIN_HOLE_AREA {
                for index in hole {
                    filled[index] = true;
                }
            }
        }

        filled
    }

    /// The lines along the middle of the steep band, smoothed and simplified
    /// with `simplify_tolerance` metres, or none when they are too short.
    fn cliff_lines(&self, simplify_tolerance: f64) -> Vec<Vec<[f64; 2]>> {
        let mut skeleton = Skeleton::thin(self.width, self.height, &self.without_small_holes());
        let chains = skeleton.prune_spurs();
        let chains = skeleton.join_at_junctions(chains);

        let total_length: f64 = chains.iter().map(|chain| skeleton.length(chain)).sum();
        if total_length < MIN_CLIFF_LENGTH {
            return vec![];
        }

        chains
            .iter()
            .map(|chain| {
                let points: Vec<[f64; 2]> = chain
                    .cells
                    .iter()
                    .map(|index| self.center(*index))
                    .collect();
                let smoothed = smooth(&points, chain.closed);
                if chain.closed {
                    let mut ring = simplify_ring(&smoothed, simplify_tolerance, |_| false);
                    ring.push(ring[0]);
                    ring
                } else {
                    simplify_line(&smoothed, simplify_tolerance)
                }
            })
            .collect()
    }
}

/// A one cell wide skeleton of a component, in its patch.
struct Skeleton {
    width: usize,
    height: usize,
    cells: Vec<bool>,
}

/// A chain of skeleton cells between two cells that are not on a line (ends
/// and junctions), or a closed loop.
#[derive(Clone)]
struct Chain {
    cells: Vec<usize>,
    closed: bool,
}

impl Skeleton {
    /// Thins `cells` with the Zhang–Suen algorithm, which keeps its
    /// topology: a band becomes a line, and a ring a loop.
    fn thin(width: usize, height: usize, cells: &[bool]) -> Self {
        let mut skeleton = Self {
            width,
            height,
            cells: cells.to_vec(),
        };

        loop {
            let mut changed = false;
            for step in 0..2 {
                let removed: Vec<usize> = (0..skeleton.cells.len())
                    .filter(|index| skeleton.cells[*index] && skeleton.is_removable(*index, step))
                    .collect();
                changed |= !removed.is_empty();
                for index in removed {
                    skeleton.cells[index] = false;
                }
            }
            if !changed {
                return skeleton;
            }
        }
    }

    fn get(&self, x: isize, y: isize) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < self.width
            && (y as usize) < self.height
            && self.cells[y as usize * self.width + x as usize]
    }

    fn is_removable(&self, index: usize, step: usize) -> bool {
        let (x, y) = ((index % self.width) as isize, (index / self.width) as isize);
        // Clockwise from north
        let neighbors = [
            self.get(x, y - 1),
            self.get(x + 1, y - 1),
            self.get(x + 1, y),
            self.get(x + 1, y + 1),
            self.get(x, y + 1),
            self.get(x - 1, y + 1),
            self.get(x - 1, y),
            self.get(x - 1, y - 1),
        ];
        let [north, _, east, _, south, _, west, _] = neighbors;

        let count = neighbors.iter().filter(|neighbor| **neighbor).count();
        let transitions = (0..8)
            .filter(|index| !neighbors[*index] && neighbors[(index + 1) % 8])
            .count();
        let condition = if step == 0 {
            !(north && east && south) && !(east && south && west)
        } else {
            !(north && east && west) && !(north && south && west)
        };

        (2..=6).contains(&count) && transitions == 1 && condition
    }

    /// The skeleton cells linked to `index`: the cells beside it, and those
    /// diagonal to it that are not linked through a cell beside both, so that
    /// a staircase is a line.
    fn links(&self, index: usize) -> Vec<usize> {
        let (x, y) = ((index % self.width) as isize, (index / self.width) as isize);
        let to_index = |x: isize, y: isize| y as usize * self.width + x as usize;

        let mut links = vec![];
        for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            if self.get(x + dx, y + dy) {
                links.push(to_index(x + dx, y + dy));
            }
        }
        for (dx, dy) in [(1, -1), (1, 1), (-1, 1), (-1, -1)] {
            if self.get(x + dx, y + dy) && !self.get(x + dx, y) && !self.get(x, y + dy) {
                links.push(to_index(x + dx, y + dy));
            }
        }
        links
    }

    /// Splits the skeleton into chains.
    fn chains(&self) -> Vec<Chain> {
        let degrees: Vec<usize> = (0..self.cells.len())
            .map(|index| {
                if self.cells[index] {
                    self.links(index).len()
                } else {
                    0
                }
            })
            .collect();
        let mut visited: HashSet<(usize, usize)> = HashSet::new();
        let mut visit = |a: usize, b: usize| visited.insert((a.min(b), a.max(b)));
        let mut on_chain = vec![false; self.cells.len()];
        let mut chains = vec![];

        // From the ends and junctions
        for start in 0..self.cells.len() {
            if degrees[start] == 0 || degrees[start] == 2 {
                continue;
            }
            for next in self.links(start) {
                if !visit(start, next) {
                    continue;
                }
                let mut cells = vec![start, next];
                let (mut previous, mut current) = (start, next);
                while degrees[current] == 2 {
                    let Some(following) = self
                        .links(current)
                        .into_iter()
                        .find(|link| *link != previous)
                    else {
                        break;
                    };
                    if !visit(current, following) {
                        break;
                    }
                    cells.push(following);
                    (previous, current) = (current, following);
                }
                for cell in &cells {
                    on_chain[*cell] = true;
                }
                chains.push(Chain {
                    cells,
                    closed: false,
                });
            }
        }

        // The loops left, without end or junction
        for start in 0..self.cells.len() {
            if degrees[start] != 2 || on_chain[start] {
                continue;
            }
            let mut cells = vec![start];
            on_chain[start] = true;
            let mut current = start;
            while let Some(following) = self
                .links(current)
                .into_iter()
                .find(|link| !on_chain[*link])
            {
                cells.push(following);
                on_chain[following] = true;
                current = following;
            }
            chains.push(Chain {
                closed: cells.len() >= 3,
                cells,
            });
        }

        chains
    }

    /// Removes the short branches ending free off the skeleton, and returns
    /// its chains, without the short ones between two junctions.
    fn prune_spurs(&mut self) -> Vec<Chain> {
        loop {
            let chains = self.chains();
            let spurs: Vec<&[usize]> = chains
                .iter()
                .filter(|chain| !chain.closed && self.length(chain) < SPUR_LENGTH)
                .filter_map(|chain| {
                    let degree = |index: usize| self.links(index).len();
                    let last = chain.cells.len() - 1;
                    // Without the junction
                    match (degree(chain.cells[0]), degree(chain.cells[last])) {
                        (1, degree) if degree >= 3 => Some(&chain.cells[..last]),
                        (degree, 1) if degree >= 3 => Some(&chain.cells[1..]),
                        _ => None,
                    }
                })
                .collect();

            if spurs.is_empty() {
                return chains
                    .into_iter()
                    .filter(|chain| self.length(chain) >= MIN_CHAIN_LENGTH)
                    .collect();
            }

            let removed: Vec<usize> = spurs.concat();
            for cell in removed {
                self.cells[cell] = false;
            }
        }
    }

    /// Joins the chains meeting at each junction two by two, the straightest
    /// continuations first, so that a cliff line does not stop at each
    /// branch.
    fn join_at_junctions(&self, chains: Vec<Chain>) -> Vec<Chain> {
        // The ends of chain `index` are `2 * index` for its first cell and
        // `2 * index + 1` for its last one
        let junctions = self.junctions();
        let mut ends_by_junction: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, chain) in chains.iter().enumerate() {
            if chain.closed {
                continue;
            }
            let ends = [chain.cells[0], chain.cells[chain.cells.len() - 1]];
            for (end, cell) in [2 * index, 2 * index + 1].into_iter().zip(ends) {
                if let Some(junction) = junctions[cell] {
                    ends_by_junction.entry(junction).or_default().push(end);
                }
            }
        }

        let direction = |end: usize| {
            let cells = &chains[end / 2].cells;
            let steps = JOIN_DIRECTION_CELLS.min(cells.len() - 1);
            let (from, to) = if end.is_multiple_of(2) {
                (cells[0], cells[steps])
            } else {
                (cells[cells.len() - 1], cells[cells.len() - 1 - steps])
            };
            let dx = (to % self.width) as f64 - (from % self.width) as f64;
            let dy = (to / self.width) as f64 - (from / self.width) as f64;
            let length = dx.hypot(dy);
            [dx / length, dy / length]
        };
        let max_cosine = JOIN_MIN_ANGLE.to_radians().cos();

        let mut partners: Vec<Option<usize>> = vec![None; 2 * chains.len()];
        for mut ends in ends_by_junction.into_values() {
            loop {
                let straightest = ends
                    .iter()
                    .enumerate()
                    .flat_map(|(index, a)| ends[index + 1..].iter().map(move |b| (*a, *b)))
                    .map(|(a, b)| {
                        let ([ax, ay], [bx, by]) = (direction(a), direction(b));
                        (a, b, ax * bx + ay * by)
                    })
                    .filter(|(_, _, cosine)| *cosine <= max_cosine)
                    .min_by(|first, second| first.2.total_cmp(&second.2));
                let Some((a, b, _)) = straightest else {
                    break;
                };
                partners[a] = Some(b);
                partners[b] = Some(a);
                ends.retain(|end| *end != a && *end != b);
            }
        }

        let mut used = vec![false; chains.len()];
        let mut joined = vec![];
        for index in 0..chains.len() {
            if used[index] {
                continue;
            }
            if chains[index].closed {
                used[index] = true;
                joined.push(chains[index].clone());
                continue;
            }

            // Back to the free end of the joined chains, or around the loop
            // they close
            let mut start = 2 * index;
            let mut closed = false;
            while let Some(partner) = partners[start] {
                if partner / 2 == index {
                    closed = true;
                    start = 2 * index;
                    break;
                }
                start = partner ^ 1;
            }

            let mut cells: Vec<usize> = vec![];
            let mut end = start;
            while !used[end / 2] {
                used[end / 2] = true;
                let chain_cells = &chains[end / 2].cells;
                let ordered: Box<dyn Iterator<Item = &usize>> = if end.is_multiple_of(2) {
                    Box::new(chain_cells.iter())
                } else {
                    Box::new(chain_cells.iter().rev())
                };
                for cell in ordered {
                    if cells.last() != Some(cell) {
                        cells.push(*cell);
                    }
                }
                match partners[end ^ 1] {
                    Some(next) => end = next,
                    None => break,
                }
            }
            if closed && cells.len() > 1 && cells.first() == cells.last() {
                cells.pop();
            }
            joined.push(Chain { cells, closed });
        }

        joined
    }

    /// The junction each cell is part of, as the first cell of the junction:
    /// the cells with three links or more, grouped with those next to them.
    fn junctions(&self) -> Vec<Option<usize>> {
        let is_junction: Vec<bool> = (0..self.cells.len())
            .map(|index| self.cells[index] && self.links(index).len() >= 3)
            .collect();
        let mut junctions: Vec<Option<usize>> = vec![None; self.cells.len()];

        for first in 0..self.cells.len() {
            if !is_junction[first] || junctions[first].is_some() {
                continue;
            }
            junctions[first] = Some(first);
            let mut stack = vec![first];
            while let Some(index) = stack.pop() {
                let (x, y) = ((index % self.width) as isize, (index / self.width) as isize);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (nx, ny) = (x + dx, y + dy);
                        if !self.get(nx, ny) {
                            continue;
                        }
                        let neighbor = ny as usize * self.width + nx as usize;
                        if is_junction[neighbor] && junctions[neighbor].is_none() {
                            junctions[neighbor] = Some(first);
                            stack.push(neighbor);
                        }
                    }
                }
            }
        }

        junctions
    }

    /// The length of `chain` in metres.
    fn length(&self, chain: &Chain) -> f64 {
        let step = |a: usize, b: usize| {
            let (dx, dy) = (
                (a % self.width).abs_diff(b % self.width),
                (a / self.width).abs_diff(b / self.width),
            );
            ((dx * dx + dy * dy) as f64).sqrt()
        };
        let mut length: f64 = chain
            .cells
            .windows(2)
            .map(|pair| step(pair[0], pair[1]))
            .sum();
        if chain.closed {
            length += step(chain.cells[0], chain.cells[chain.cells.len() - 1]);
        }
        length * CELL_SIZE
    }
}

/// Moving average over `SMOOTHING_RADIUS` points on either side. The ends of
/// an open line stay where they are, so that lines meeting at a junction
/// still meet.
fn smooth(points: &[[f64; 2]], closed: bool) -> Vec<[f64; 2]> {
    let len = points.len();
    (0..len)
        .map(|index| {
            let radius = if closed {
                if len <= 2 * SMOOTHING_RADIUS {
                    return points[index];
                }
                SMOOTHING_RADIUS
            } else {
                SMOOTHING_RADIUS.min(index).min(len - 1 - index)
            };
            let window =
                (index + len - radius..=index + len + radius).map(|index| points[index % len]);
            let [sum_x, sum_y] =
                window.fold([0.0, 0.0], |[sum_x, sum_y], [x, y]| [sum_x + x, sum_y + y]);
            let count = (2 * radius + 1) as f64;
            [sum_x / count, sum_y / count]
        })
        .collect()
}

/// Douglas–Peucker simplification of an open line, keeping its ends.
fn simplify_line(line: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    if tolerance <= 0.0 || line.len() < 3 {
        return line.to_vec();
    }

    let mut kept = vec![false; line.len()];
    kept[0] = true;
    kept[line.len() - 1] = true;

    let squared_tolerance = tolerance * tolerance;
    let mut chains = vec![(0, line.len() - 1)];
    while let Some((first, last)) = chains.pop() {
        let farthest_from_chord = (first + 1..last)
            .map(|index| {
                (
                    index,
                    squared_segment_distance(line[index], line[first], line[last]),
                )
            })
            .max_by(|(_, a), (_, b)| a.total_cmp(b));

        if let Some((index, squared_distance)) = farthest_from_chord {
            if squared_distance > squared_tolerance {
                kept[index] = true;
                chains.push((first, index));
                chains.push((index, last));
            }
        }
    }

    line.iter()
        .zip(kept)
        .filter(|(_, kept)| *kept)
        .map(|(point, _)| *point)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_features::clip_line_part;

    fn extent(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Extent {
        Extent {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    const TILE: Extent = Extent {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 40.0,
        max_y: 40.0,
    };

    /// The DEM of `height` over the grid, and its slopes in degrees, as
    /// `gdaldem slope` computes them (Horn), without the border cells.
    fn rasters(grid: Grid, height: impl Fn(f64, f64) -> f64) -> (Vec<f32>, Vec<f32>) {
        let mut dem = vec![0.0; grid.len()];
        for y in 0..grid.height {
            for x in 0..grid.width {
                let [px, py] = grid.center(x, y);
                dem[y * grid.width + x] = height(px, py);
            }
        }

        let mut slopes = vec![DEM_NO_DATA; grid.len()];
        for y in 1..grid.height - 1 {
            for x in 1..grid.width - 1 {
                let z = |dx: isize, dy: isize| {
                    dem[(y as isize + dy) as usize * grid.width + (x as isize + dx) as usize]
                };
                let dz_dx = ((z(1, -1) + 2.0 * z(1, 0) + z(1, 1))
                    - (z(-1, -1) + 2.0 * z(-1, 0) + z(-1, 1)))
                    / (8.0 * CELL_SIZE);
                let dz_dy = ((z(-1, 1) + 2.0 * z(0, 1) + z(1, 1))
                    - (z(-1, -1) + 2.0 * z(0, -1) + z(1, -1)))
                    / (8.0 * CELL_SIZE);
                slopes[y * grid.width + x] = (dz_dx.hypot(dz_dy)).atan().to_degrees() as f32;
            }
        }

        (dem.into_iter().map(|z| z as f32).collect(), slopes)
    }

    fn vectorize(tile: &Extent, height: impl Fn(f64, f64) -> f64) -> Vec<MapFeature> {
        let grid = Grid::around(tile);
        let (dem, slopes) = rasters(grid, height);
        vectorize_cliffs(grid, tile, &slopes, &dem, 60.0, 0.5).unwrap()
    }

    fn lines(features: &[MapFeature]) -> Vec<Vec<Point>> {
        features
            .iter()
            .filter_map(|feature| match feature {
                MapFeature::Line(LineSymbol::Cliff202, line) => Some(line.parts()[0].clone()),
                _ => None,
            })
            .collect()
    }

    fn length(line: &[Point]) -> f64 {
        line.windows(2)
            .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y))
            .sum()
    }

    /// A block of `height` metres over the square of `size` metres centred
    /// on (`x`, `y`).
    fn block(x: f64, y: f64, size: f64, height: f64) -> impl Fn(f64, f64) -> f64 {
        move |px, py| {
            if (px - x).abs() < size / 2.0 && (py - y).abs() < size / 2.0 {
                height
            } else {
                0.0
            }
        }
    }

    #[test]
    fn draws_a_small_free_standing_rock_as_a_large_boulder() {
        let features = vectorize(&TILE, block(20.0, 20.0, 3.0, 2.5));

        assert_eq!(features.len(), 1);
        let MapFeature::Point(PointSymbol::LargeBoulder205, point) = &features[0] else {
            panic!("expected a 205 large boulder");
        };
        assert!((point.x - 20.0).abs() < 0.5 && (point.y - 20.0).abs() < 0.5);
    }

    #[test]
    fn draws_a_short_step_in_a_slope_as_a_short_line() {
        // A 2 m step, 5 m long, in a gentle slope rising to the north. Its
        // ends fade out on 3.5 m, less steeply than the threshold.
        let features = vectorize(&TILE, |x, y| {
            let step = (1.0 - ((x - 20.0).abs() - 2.5).max(0.0) / 3.5).max(0.0);
            0.1 * y + if y > 20.0 { 2.0 * step } else { 0.0 }
        });

        let lines = lines(&features);
        assert_eq!(features.len(), 1);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!((3.0..10.0).contains(&length(line)), "{}", length(line));
        assert!(line.iter().all(|point| (point.y - 20.0).abs() < 1.0));
    }

    #[test]
    fn draws_a_long_step_as_a_line_across_the_tile() {
        let features = vectorize(&TILE, |_, y| if y > 20.0 { 3.0 } else { 0.0 });

        let lines = lines(&features);
        assert_eq!(features.len(), lines.len());
        assert!(lines
            .iter()
            .flatten()
            .all(|point| (point.y - 20.0).abs() < 1.0));
        let clipped: Vec<Vec<Point>> = lines
            .iter()
            .flat_map(|line| clip_line_part(line, &TILE))
            .collect();
        assert_eq!(clipped.len(), 1);
        assert!((length(&clipped[0]) - 40.0).abs() < 0.01);
    }

    #[test]
    fn draws_a_rock_larger_than_the_boulder_symbol_as_a_closed_line() {
        let features = vectorize(&TILE, block(20.0, 20.0, 14.0, 3.0));

        let lines = lines(&features);
        assert_eq!(features.len(), 1);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line.first(), line.last());
        // Around the 14 m square
        assert!((45.0..65.0).contains(&length(line)), "{}", length(line));
        for point in line {
            let distance = (point.x - 20.0).abs().max((point.y - 20.0).abs());
            assert!((6.0..8.0).contains(&distance), "{point:?}");
        }
    }

    #[test]
    fn draws_a_wide_steep_band_as_a_gigantic_boulder_area() {
        // 70° from y = 10 to y = 20
        let features = vectorize(&TILE, |_, y| ((y - 10.0) * 2.75).clamp(0.0, 27.5));

        assert!(!features.is_empty());
        for feature in &features {
            let MapFeature::Area(AreaSymbol::GiganticBoulderOrRockPillar206, polygon) = feature
            else {
                panic!("expected a 206 area");
            };
            // Grown by the radius of the dots of cliffs.png
            let bbox = polygon.bbox();
            assert!(bbox.min.y > 8.0 && bbox.min.y < 9.0, "{bbox:?}");
            assert!(bbox.max.y > 21.0 && bbox.max.y < 22.0, "{bbox:?}");
            assert_eq!((bbox.min.x, bbox.max.x), (TILE.min_x, TILE.max_x));
        }
    }

    #[test]
    fn draws_a_cliff_leaving_a_wide_steep_band_as_a_line_from_its_edge() {
        // The band of the previous test, and a 3 m step along x = 20 south of
        // it, which rises gently from y = -20 to y = -10
        let features = vectorize(&TILE, |x, y| {
            let band = ((y - 10.0) * 2.75).clamp(0.0, 27.5);
            let step = if x > 20.0 {
                3.0 * ((y + 20.0) / 10.0).clamp(0.0, 1.0)
            } else {
                0.0
            };
            band + step
        });

        assert!(features.iter().any(|feature| matches!(
            feature,
            MapFeature::Area(AreaSymbol::GiganticBoulderOrRockPillar206, _)
        )));
        let lines = lines(&features);
        // The step also continues north of the band
        let step = lines
            .iter()
            .find(|line| {
                line.iter()
                    .all(|point| (point.x - 20.0).abs() < 1.0 && point.y < 10.0)
            })
            .expect("expected the step south of the band as a line");
        let (min_y, max_y) = step.iter().fold((f64::MAX, f64::MIN), |(min, max), point| {
            (min.min(point.y), max.max(point.y))
        });
        // From where it gets steep to the edge of the area
        assert!(
            min_y < -10.0 && (7.0..9.0).contains(&max_y),
            "{min_y} {max_y}"
        );
    }

    #[test]
    fn neighbouring_tiles_draw_a_cliff_crossing_their_edge_alike() {
        // A winding step from x = 15 to x = 65, whose ends fade out
        let height = |x: f64, y: f64| {
            let step = (1.0 - ((x - 40.0).abs() - 25.0).max(0.0) / 4.0).max(0.0);
            if y > 20.0 + 3.0 * (x / 7.0).sin() {
                3.0 * step
            } else {
                0.0
            }
        };
        let east = extent(40.0, 0.0, 80.0, 40.0);

        let crossings = |tile: &Extent| -> Vec<(f64, f64)> {
            let mut crossings: Vec<(f64, f64)> = lines(&vectorize(tile, height))
                .iter()
                .flat_map(|line| clip_line_part(line, tile))
                .flat_map(|part| [part[0], part[part.len() - 1]])
                .filter(|point| (point.x - 40.0).abs() < 1e-9)
                .map(|point| (point.x, point.y))
                .collect();
            crossings.sort_by(|a, b| a.1.total_cmp(&b.1));
            crossings
        };

        let west_crossings = crossings(&TILE);
        let east_crossings = crossings(&east);
        assert_eq!(west_crossings.len(), 1);
        assert_eq!(west_crossings.len(), east_crossings.len());
        for (west, east) in west_crossings.iter().zip(&east_crossings) {
            assert!((west.1 - east.1).abs() < 1e-6, "{west:?} {east:?}");
        }
    }

    #[test]
    fn joins_the_straightest_branches_at_junctions() {
        // A horizontal line crossed by a vertical one, and a branch off the
        // vertical line at a right angle
        let size = 41;
        let mut cells = vec![false; size * size];
        for i in 5..36 {
            cells[20 * size + i] = true;
            cells[i * size + 20] = true;
        }
        for x in 21..30 {
            cells[10 * size + x] = true;
        }
        let mut skeleton = Skeleton {
            width: size,
            height: size,
            cells,
        };

        let chains = skeleton.prune_spurs();
        assert_eq!(chains.len(), 6);
        let mut joined = skeleton.join_at_junctions(chains);

        joined.sort_by_key(|chain| chain.cells.len());
        let lengths: Vec<usize> = joined.iter().map(|chain| chain.cells.len()).collect();
        assert_eq!(lengths, vec![10, 31, 31]);
        assert!(joined.iter().all(|chain| !chain.closed));
        let (row, column) = (&joined[1].cells, &joined[2].cells);
        let is_row = |cells: &[usize]| cells.iter().all(|cell| cell / size == 20);
        let is_column = |cells: &[usize]| cells.iter().all(|cell| cell % size == 20);
        assert!(is_row(row) && is_column(column) || is_row(column) && is_column(row));
    }

    #[test]
    fn simplifies_lines_keeping_their_ends() {
        let line = [[0.0, 0.0], [1.0, 0.1], [2.0, -0.1], [3.0, 2.0], [4.0, 0.0]];

        assert_eq!(
            simplify_line(&line, 0.5),
            vec![[0.0, 0.0], [2.0, -0.1], [3.0, 2.0], [4.0, 0.0]]
        );
    }

    #[test]
    fn smooths_lines_keeping_their_ends() {
        let line: Vec<[f64; 2]> = (0..7).map(|x| [x as f64, (x % 2) as f64]).collect();

        let smoothed = smooth(&line, false);

        assert_eq!(smoothed[0], line[0]);
        assert_eq!(smoothed[6], line[6]);
        assert_eq!(smoothed[3], [3.0, 0.6]);
    }
}
