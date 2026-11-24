//! Vectorizes the vegetation density fields into ISOM area features for the
//! OCAD export.
//!
//! Each symbol is the iso-line of a continuous 1 m field: the blurred green
//! density for 406, 408 and 410, the distance to high vegetation for 403 and
//! the blurred low vegetation density for 409. ISOM minimum widths and areas
//! are applied to the fields before tracing, which keeps neighbouring symbols
//! consistent. The iso-lines are traced with marching squares and simplified
//! once each, then the symbols are cut out of each other, so that touching
//! areas share exactly the same boundary.

use crate::{
    config::Config,
    constants::{BUFFER, VEGETATION_BLOCK_SIZE},
    error::{Result, ResultContext},
    map_features::{AreaSymbol, Extent, MapFeature},
    tile::Tile,
    vegetation::{
        vegetation_interior, NoDataHandling, RasterInterior, UndergrowthMode, VegetationRasters,
        LOW_DENSITY_BLUR_RADIUS, MEDIUM_DENSITY_BLUR_RADIUS,
    },
};
use contour::ContourBuilder;
use i_overlay::{
    core::{fill_rule::FillRule, overlay_rule::OverlayRule},
    float::single::SingleFloatOverlay,
};
use image::GrayImage;
use imageproc::distance_transform::euclidean_squared_distance_transform;
use log::info;
use shapefile::{Point, Polygon, PolygonRing};
use std::time::Instant;

// Areas are counted in cells and lengths in cells below
const _: () = assert!(VEGETATION_BLOCK_SIZE == 1);

/// Cells vectorized around the tile, so that areas reach the tile edge and
/// areas crossing it are measured beyond it.
const MARGIN: usize = 50;
// The blurs and the 3 x 3 minimum of the margin read the buffer around it
const _: () = assert!(
    MARGIN + MEDIUM_DENSITY_BLUR_RADIUS <= BUFFER
        && MARGIN + LOW_DENSITY_BLUR_RADIUS <= BUFFER
        && MARGIN < BUFFER
);
/// Radius of the white discs the PNG renderer draws around high vegetation.
const OPEN_LAND_DISTANCE: f32 = 2.0;
/// Caps the distance to high vegetation, which is infinite without any.
const MAX_OPEN_LAND_FIELD: f32 = 100.0;
/// Field value of the cells without LiDAR data, outside every area.
const NO_DATA_FIELD: f32 = -1.0;
/// Field value given to the cells generalisation moves into (positive) or
/// out of (negative) an area. Tracing also rounds values closer to zero to
/// it, so that the iso-line agrees with the strict thresholds.
const NUDGE: f32 = 1e-3;
/// Polygons and holes left smaller than this, in m², by simplification and
/// boolean operations are slivers.
const SLIVER_AREA: f64 = 1.0;
/// Generalisation leaves 403 and 409 areas of their minimum area at least,
/// but their iso-lines do not follow the green ones, so cutting out the green
/// areas leaves slivers along them. Pieces smaller than this fraction of the
/// minimum area are dropped, unless they are clipped by the tile edge.
const CUT_SLIVER_FRACTION: f64 = 0.5;

/// ISOM 2017-2 minimum sizes, in ground metres at 1:15 000. The enlarged
/// 1:10 000 symbol set keeps the same sizes on the ground.
struct AreaRules {
    /// Minimum area in m².
    min_area: usize,
    min_width: f32,
    /// Minimum area in m² of a hole, which shows the symbol below.
    min_hole_area: usize,
}

const ROUGH_OPEN_LAND_403: AreaRules = AreaRules {
    min_area: 225,
    min_width: 6.0,
    // Forest
    min_hole_area: 110,
};

const VEGETATION_SLOW_RUNNING_406: AreaRules = AreaRules {
    min_area: 225,
    min_width: 6.0,
    // Forest
    min_hole_area: 110,
};

const VEGETATION_WALK_408: AreaRules = AreaRules {
    min_area: 110,
    min_width: 4.5,
    // 406
    min_hole_area: 225,
};

const VEGETATION_FIGHT_410: AreaRules = AreaRules {
    min_area: 64,
    min_width: 3.75,
    // 408
    min_hole_area: 110,
};

const VEGETATION_WALK_GOOD_VISIBILITY_409: AreaRules = AreaRules {
    min_area: 225,
    min_width: 6.0,
    // Forest
    min_hole_area: 110,
};

/// Returns the vegetation areas of the tile, clipped to its extent, with
/// their boundaries simplified with `simplify_tolerance` metres (0 to keep
/// every traced vertex).
pub fn get_vegetation_features(
    tile: &Tile,
    rasters: &VegetationRasters,
    config: &Config,
    undergrowth_mode: &UndergrowthMode,
    simplify_tolerance: f64,
) -> Result<Vec<MapFeature>> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vectorizing vegetation",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let mut fields = VegetationFields::compute(tile, rasters, config, undergrowth_mode)?;
    fields.generalise();

    let tile_extent = Extent {
        min_x: tile.min_x as f64,
        min_y: tile.min_y as f64,
        max_x: tile.max_x as f64,
        max_y: tile.max_y as f64,
    };
    let window = Extent {
        min_x: tile_extent.min_x - MARGIN as f64,
        min_y: tile_extent.min_y - MARGIN as f64,
        max_x: tile_extent.max_x + MARGIN as f64,
        max_y: tile_extent.max_y + MARGIN as f64,
    };
    let tracer = Tracer {
        builder: ContourBuilder::new(fields.grid.width, fields.grid.height, true)
            .x_origin(window.min_x)
            .y_origin(window.max_y)
            .y_step(-1.0),
        pinned_lines: PinnedLines::around_tile_edges(
            &tile_extent,
            simplify_tolerance.min(MARGIN as f64 / 2.0),
        ),
        window,
        simplify_tolerance,
    };

    let mut green_or_denser = vec![];
    for field in &fields.green {
        green_or_denser.push(tracer.trace(field)?);
    }
    let open_land = tracer.trace(&fields.open_land)?;

    let mut features = FeatureSink {
        features: vec![],
        window,
        tile: tile_extent,
    };
    features.push(
        AreaSymbol::RoughOpenLand403,
        difference(&open_land, &green_or_denser[0]),
        ROUGH_OPEN_LAND_403.min_area as f64 * CUT_SLIVER_FRACTION,
    );
    features.push(
        AreaSymbol::VegetationSlowRunning406,
        difference(&green_or_denser[0], &green_or_denser[1]),
        SLIVER_AREA,
    );
    features.push(
        AreaSymbol::VegetationWalk408,
        difference(&green_or_denser[1], &green_or_denser[2]),
        SLIVER_AREA,
    );
    features.push(
        AreaSymbol::VegetationFight410,
        green_or_denser.pop().unwrap(),
        SLIVER_AREA,
    );

    if let Some(undergrowth) = &fields.undergrowth_409 {
        // ISOM does not combine 409 with the green screens. It may overlap
        // 403, as undergrowth.png overlays the yellow in the PNG rendering
        features.push(
            AreaSymbol::VegetationWalkGoodVisibility409,
            difference(&tracer.trace(undergrowth)?, &green_or_denser[0]),
            VEGETATION_WALK_GOOD_VISIBILITY_409.min_area as f64 * CUT_SLIVER_FRACTION,
        );
    }
    let features = features.features;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. {} vegetation areas vectorized in {:.1?}",
        tile.min_x,
        tile.min_y,
        tile.max_x,
        tile.max_y,
        features.len(),
        duration
    );

    Ok(features)
}

/// A shape is an outer ring followed by its holes, without closing points.
pub type Shapes = Vec<Vec<Vec<[f64; 2]>>>;

fn difference(subject: &Shapes, clip: &Shapes) -> Shapes {
    subject.overlay(clip, OverlayRule::Difference, FillRule::EvenOdd)
}

pub struct FeatureSink {
    pub features: Vec<MapFeature>,
    /// The vectorized window. The shapes reaching its border may continue
    /// beyond it.
    pub window: Extent,
    /// The tile the shapes are clipped to.
    pub tile: Extent,
}

impl FeatureSink {
    /// Pushes one area feature per shape clipped to the tile, dropping the
    /// shapes smaller than `min_area` m² away from the window border, and the
    /// slivers. The shapes are measured whole, before they are clipped, so
    /// that neighbouring tiles keep or drop the shapes crossing their common
    /// edge alike.
    pub fn push(&mut self, symbol: AreaSymbol, shapes: Shapes, min_area: f64) {
        let to_points =
            |ring: Vec<[f64; 2]>| ring.into_iter().map(|[x, y]| Point::new(x, y)).collect();
        let tile_rectangle = rectangle(&self.tile);

        for shape in shapes {
            // The outer ring comes first
            let mut rings = shape.into_iter();
            let Some(outer) = rings.next() else {
                continue;
            };
            let area = ring_area(&outer);
            if area < SLIVER_AREA || (area < min_area && !self.reaches_window_border(&outer)) {
                continue;
            }
            let shape: Vec<Vec<[f64; 2]>> = std::iter::once(outer)
                .chain(rings.filter(|hole| ring_area(hole) >= SLIVER_AREA))
                .collect();

            for clipped in shape.overlay(&tile_rectangle, OverlayRule::Intersect, FillRule::EvenOdd)
            {
                let mut rings = clipped.into_iter();
                let Some(outer) = rings.next() else {
                    continue;
                };
                let polygon = Polygon::with_rings(
                    std::iter::once(PolygonRing::Outer(to_points(outer)))
                        .chain(rings.map(|hole| PolygonRing::Inner(to_points(hole))))
                        .collect(),
                );
                self.features.push(MapFeature::Area(symbol, polygon));
            }
        }
    }

    fn reaches_window_border(&self, ring: &[[f64; 2]]) -> bool {
        // The iso-lines close along the grid border, within a cell of it
        const TOLERANCE: f64 = 1.0;
        ring.iter().any(|[x, y]| {
            x - self.window.min_x < TOLERANCE
                || self.window.max_x - x < TOLERANCE
                || y - self.window.min_y < TOLERANCE
                || self.window.max_y - y < TOLERANCE
        })
    }
}

/// Absolute shoelace area of a ring without closing point.
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let doubled: f64 = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum();
    doubled.abs() / 2.0
}

pub struct Tracer {
    pub builder: ContourBuilder,
    /// The lines simplification keeps the crossings of.
    pub pinned_lines: PinnedLines,
    /// The vectorized window, holding every traced ring.
    pub window: Extent,
    pub simplify_tolerance: f64,
}

impl Tracer {
    /// Returns the area where `field` is positive over the window.
    ///
    /// Vertices are added to the iso-lines where they cross the pinned lines,
    /// and simplification keeps them. A neighbouring tile pins the same lines
    /// along the common edge, so that the areas clipped to either tile cross
    /// that edge at the same points, and batch mode can join them.
    pub fn trace(&self, field: &[f32]) -> Result<Shapes> {
        let values: Vec<f64> = field
            .iter()
            .map(|value| {
                if *value > 0.0 {
                    value.max(NUDGE) as f64
                } else {
                    value.min(-NUDGE) as f64
                }
            })
            .collect();

        let lines = self
            .builder
            .lines(&values, &[0.0])
            .context("could not trace areas")?;

        let rings: Vec<Vec<[f64; 2]>> = lines
            .iter()
            .flat_map(|line| line.geometry().iter())
            .map(|line_string| {
                let mut ring: Vec<[f64; 2]> = line_string
                    .coords()
                    .map(|coord| [coord.x, coord.y])
                    .collect();
                if ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                if self.simplify_tolerance <= 0.0 {
                    return ring;
                }
                simplify_ring(
                    &self.pinned_lines.insert_crossings(&ring),
                    self.simplify_tolerance,
                    |point| self.pinned_lines.contains(point),
                )
            })
            .filter(|ring| ring.len() >= 3)
            .collect();

        // Iso-lines of a threshold never cross, so the even-odd rule turns
        // nested rings into holes and islands in holes. Each ring is
        // simplified on its own, so neighbouring rings may cross by up to the
        // tolerance. The pieces this leaves after the boolean operations are
        // dropped as slivers.
        Ok(rings.overlay(
            &rectangle(&self.window),
            OverlayRule::Intersect,
            FillRule::EvenOdd,
        ))
    }
}

/// The corners of `extent`.
fn rectangle(extent: &Extent) -> Vec<[f64; 2]> {
    vec![
        [extent.min_x, extent.min_y],
        [extent.max_x, extent.min_y],
        [extent.max_x, extent.max_y],
        [extent.min_x, extent.max_y],
    ]
}

/// Axis-aligned lines, as their axis (0 for x = the coordinate, 1 for
/// y = the coordinate) and coordinate.
pub struct PinnedLines(Vec<(usize, f64)>);

impl PinnedLines {
    /// The lines of the edges of `tile`, and lines `band` on either side of
    /// each one. A neighbouring tile has the same lines along their common
    /// edge. Simplification keeps the iso-lines within the cells these lines
    /// form, so both tiles simplify the iso-lines within `band` of that edge
    /// alike, whatever they do beyond.
    pub fn around_tile_edges(tile: &Extent, band: f64) -> Self {
        let mut lines = vec![];
        for (axis, edge) in [
            (0, tile.min_x),
            (0, tile.max_x),
            (1, tile.min_y),
            (1, tile.max_y),
        ] {
            lines.extend([(axis, edge - band), (axis, edge), (axis, edge + band)]);
        }
        Self(lines)
    }

    fn contains(&self, point: [f64; 2]) -> bool {
        // The boolean operations snap the coordinates to an integer grid
        const TOLERANCE: f64 = 1e-3;
        self.0
            .iter()
            .any(|(axis, line)| (point[*axis] - line).abs() < TOLERANCE)
    }

    /// Adds a vertex to `ring`, without closing point, wherever a segment
    /// crosses one of the lines.
    fn insert_crossings(&self, ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut with_crossings = Vec::with_capacity(ring.len());

        for (index, start) in ring.iter().enumerate() {
            with_crossings.push(*start);

            // From the lower endpoint, so that the neighbouring tile computes
            // the same point whichever way its ring runs
            let end = ring[(index + 1) % ring.len()];
            let (a, b) = if *start < end {
                (*start, end)
            } else {
                (end, *start)
            };
            let mut crossings: Vec<(f64, [f64; 2])> = vec![];
            for (axis, line) in &self.0 {
                if (a[*axis] - line) * (b[*axis] - line) < 0.0 {
                    let t = (line - a[*axis]) / (b[*axis] - a[*axis]);
                    let mut point = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                    point[*axis] = *line;
                    // By distance from `start`
                    crossings.push((squared_distance(*start, point), point));
                }
            }

            crossings.sort_by(|first, second| first.0.total_cmp(&second.0));
            with_crossings.extend(crossings.into_iter().map(|(_, point)| point));
        }

        with_crossings
    }
}

/// Douglas–Peucker simplification of a ring without closing point, keeping
/// the vertices for which `is_pinned` holds.
pub fn simplify_ring(
    ring: &[[f64; 2]],
    tolerance: f64,
    is_pinned: impl Fn([f64; 2]) -> bool,
) -> Vec<[f64; 2]> {
    if tolerance <= 0.0 || ring.len() < 4 {
        return ring.to_vec();
    }

    // Douglas–Peucker keeps the ends of a chain, so the ring is split in
    // chains at its pinned vertices. It starts at the first one, and is also
    // split at the vertex farthest from it when there are fewer than two.
    // Without any, it starts at its lowest vertex, so that a neighbouring tile
    // tracing the same ring from another vertex simplifies it alike.
    let first = (0..ring.len())
        .find(|index| is_pinned(ring[*index]))
        .unwrap_or_else(|| {
            (0..ring.len())
                .min_by(|a, b| ring[*a].partial_cmp(&ring[*b]).unwrap())
                .unwrap()
        });
    let closed: Vec<[f64; 2]> = ring[first..]
        .iter()
        .chain(&ring[..=first])
        .copied()
        .collect();
    let len = ring.len();

    let mut anchors: Vec<usize> = (0..len).filter(|index| is_pinned(closed[*index])).collect();
    if anchors.len() < 2 {
        let farthest = (1..len)
            .max_by(|a, b| {
                squared_distance(closed[0], closed[*a])
                    .total_cmp(&squared_distance(closed[0], closed[*b]))
            })
            .unwrap();
        anchors = vec![0, farthest];
    }

    let mut kept = vec![false; closed.len()];
    for anchor in &anchors {
        kept[*anchor] = true;
    }

    let squared_tolerance = tolerance * tolerance;
    let mut chains: Vec<(usize, usize)> = anchors
        .iter()
        .zip(anchors.iter().skip(1).chain([&len]))
        .map(|(first, last)| (*first, *last))
        .collect();
    while let Some((first, last)) = chains.pop() {
        let farthest_from_chord = (first + 1..last)
            .map(|index| {
                (
                    index,
                    squared_segment_distance(closed[index], closed[first], closed[last]),
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

    // The closing point duplicates the first one
    closed[..len]
        .iter()
        .zip(&kept)
        .filter(|(_, kept)| **kept)
        .map(|(point, _)| *point)
        .collect()
}

fn squared_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

pub fn squared_segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let squared_length = dx * dx + dy * dy;
    if squared_length == 0.0 {
        return squared_distance(point, start);
    }

    let t = (((point[0] - start[0]) * dx + (point[1] - start[1]) * dy) / squared_length)
        .clamp(0.0, 1.0);
    squared_distance(point, [start[0] + t * dx, start[1] + t * dy])
}

/// The vectorized window: the tile and a margin of `MARGIN` cells, row-major
/// from the north-west corner.
#[derive(Clone, Copy)]
struct Grid {
    width: usize,
    height: usize,
}

impl Grid {
    fn len(self) -> usize {
        self.width * self.height
    }

    fn is_border(self, index: usize) -> bool {
        let (x, y) = (index % self.width, index / self.width);
        x == 0 || y == 0 || x == self.width - 1 || y == self.height - 1
    }

    fn is_in_margin(self, index: usize) -> bool {
        let (x, y) = (index % self.width, index / self.width);
        x < MARGIN || y < MARGIN || x >= self.width - MARGIN || y >= self.height - MARGIN
    }

    fn neighbors(self, index: usize) -> impl Iterator<Item = usize> {
        let (x, y) = (index % self.width, index / self.width);
        [
            (x > 0).then(|| index - 1),
            (x + 1 < self.width).then(|| index + 1),
            (y > 0).then(|| index - self.width),
            (y + 1 < self.height).then(|| index + self.width),
        ]
        .into_iter()
        .flatten()
    }

    fn image(self, pixels: impl Iterator<Item = bool>) -> GrayImage {
        GrayImage::from_raw(
            self.width as u32,
            self.height as u32,
            pixels.map(|pixel| if pixel { 255 } else { 0 }).collect(),
        )
        .unwrap()
    }

    /// Squared distance from each cell to the nearest cell of `mask`.
    fn squared_distances(self, mask: impl Iterator<Item = bool>) -> Vec<f64> {
        euclidean_squared_distance_transform(&self.image(mask)).into_raw()
    }

    /// Calls `visit` with the cells of each 4-connected component of `mask`
    /// and whether it touches the grid border.
    fn for_each_component(self, mask: &[bool], mut visit: impl FnMut(&[usize], bool)) {
        let mut visited = vec![false; self.len()];
        let mut stack = vec![];
        let mut cells = vec![];

        for start in 0..self.len() {
            if !mask[start] || visited[start] {
                continue;
            }

            visited[start] = true;
            stack.push(start);
            cells.clear();
            let mut touches_border = false;

            while let Some(cell) = stack.pop() {
                cells.push(cell);
                touches_border |= self.is_border(cell);
                for neighbor in self.neighbors(cell) {
                    if mask[neighbor] && !visited[neighbor] {
                        visited[neighbor] = true;
                        stack.push(neighbor);
                    }
                }
            }

            visit(&cells, touches_border);
        }
    }

    /// The cell each cell takes its value from: itself when it has data,
    /// else, in the margin, the nearest cell of the tile when that one has
    /// data. That extends the areas crossing the tile edge beyond it when a
    /// neighbouring tile is missing, so that they are generalised as crossing
    /// it, and traced beyond it instead of closed just inside it.
    fn extrusion_sources(self, valid: &[bool]) -> Vec<Option<usize>> {
        (0..self.len())
            .map(|cell| {
                if valid[cell] {
                    return Some(cell);
                }
                if !self.is_in_margin(cell) {
                    return None;
                }

                let (x, y) = (cell % self.width, cell / self.width);
                let source = y.clamp(MARGIN, self.height - MARGIN - 1) * self.width
                    + x.clamp(MARGIN, self.width - MARGIN - 1);
                valid[source].then_some(source)
            })
            .collect()
    }
}

/// Fields over the grid, positive inside their symbol's area.
struct VegetationFields {
    grid: Grid,
    /// Whether each cell has a value: it has LiDAR data, or it is in the
    /// margin and extended from a tile cell with data.
    valid: Vec<bool>,
    /// The green density minus each green threshold: positive in 406 or
    /// denser, in 408 or denser, and in 410.
    green: [Vec<f32>; 3],
    /// The distance to high vegetation minus the radius it clears.
    open_land: Vec<f32>,
    /// The low vegetation density minus its threshold, in 409 mode.
    undergrowth_409: Option<Vec<f32>>,
}

impl VegetationFields {
    /// Computes the fields as the PNG renderer classifies the vegetation,
    /// except that cells without data are left out of the blurs instead of
    /// counting as dense vegetation.
    fn compute(
        tile: &Tile,
        rasters: &VegetationRasters,
        config: &Config,
        undergrowth_mode: &UndergrowthMode,
    ) -> Result<Self> {
        let interior = vegetation_interior(tile, rasters)?;
        let window = RasterInterior {
            x: interior.x - MARGIN,
            y: interior.y - MARGIN,
            width: interior.width + 2 * MARGIN,
            height: interior.height + 2 * MARGIN,
        };
        let grid = Grid {
            width: window.width,
            height: window.height,
        };

        let densities =
            rasters.densities(window, config, undergrowth_mode, NoDataHandling::Excluded);
        let squared_distances_to_high = grid.squared_distances(densities.high.iter().copied());

        let sources = grid.extrusion_sources(&densities.valid);
        let field = |value: &dyn Fn(usize) -> f32| -> Vec<f32> {
            sources
                .iter()
                .map(|source| source.map_or(NO_DATA_FIELD, value))
                .collect()
        };
        let green_field = |threshold: f32| field(&|cell| densities.green[cell] - threshold);

        let undergrowth = densities
            .low
            .as_ref()
            .map(|low| field(&|cell| low[cell] - config.low_vegetation_density_threshold));
        let mut light_green = green_field(config.green_threshold_1);
        if let (UndergrowthMode::Symbol406, Some(undergrowth)) = (undergrowth_mode, &undergrowth) {
            // Undergrowth is drawn with the 406 screen
            for (green, undergrowth) in light_green.iter_mut().zip(undergrowth) {
                *green = green.max(*undergrowth);
            }
        }

        Ok(Self {
            grid,
            valid: sources.iter().map(Option::is_some).collect(),
            green: [
                light_green,
                green_field(config.green_threshold_2),
                green_field(config.green_threshold_3),
            ],
            open_land: field(&|cell| {
                (squared_distances_to_high[cell].sqrt() as f32 - OPEN_LAND_DISTANCE)
                    .min(MAX_OPEN_LAND_FIELD)
            }),
            undergrowth_409: undergrowth
                .filter(|_| matches!(undergrowth_mode, UndergrowthMode::Symbol409)),
        })
    }

    /// Applies the ISOM minimum widths and areas.
    fn generalise(&mut self) {
        let grid = self.grid;
        let green_rules = [
            VEGETATION_SLOW_RUNNING_406,
            VEGETATION_WALK_408,
            VEGETATION_FIGHT_410,
        ];

        // The green areas are nested. Islands are removed from the densest
        // down, keeping those that hold a denser area that survived. Holes
        // are filled from the lightest up, keeping those that reach below
        // the lighter area. That keeps the areas nested.
        let mut denser: Option<Vec<bool>> = None;
        for (field, rules) in self.green.iter_mut().zip(&green_rules).rev() {
            remove_thin_parts(field, None, denser.as_deref(), rules.min_width, grid);
            remove_small_islands(field, None, denser.as_deref(), rules.min_area, grid);
            denser = Some(get_mask(field, None));
        }

        let mut lighter = self.valid.clone();
        for (field, rules) in self.green.iter_mut().zip(&green_rules) {
            fill_small_holes(field, None, &lighter, rules.min_hole_area, grid);
            lighter = get_mask(field, None);
        }

        // 403 and 409 are measured where they are not covered by green
        let not_green: Vec<bool> = get_mask(&self.green[0], None)
            .into_iter()
            .map(|green| !green)
            .collect();
        generalise_area(
            &mut self.open_land,
            &not_green,
            &self.valid,
            &ROUGH_OPEN_LAND_403,
            grid,
        );
        if let Some(undergrowth) = &mut self.undergrowth_409 {
            generalise_area(
                undergrowth,
                &not_green,
                &self.valid,
                &VEGETATION_WALK_GOOD_VISIBILITY_409,
                grid,
            );
        }
    }
}

fn generalise_area(
    field: &mut [f32],
    allowed: &[bool],
    valid: &[bool],
    rules: &AreaRules,
    grid: Grid,
) {
    remove_thin_parts(field, Some(allowed), None, rules.min_width, grid);
    remove_small_islands(field, Some(allowed), None, rules.min_area, grid);
    fill_small_holes(field, Some(allowed), valid, rules.min_hole_area, grid);
}

/// The cells where `field` is positive, within `allowed`.
fn get_mask(field: &[f32], allowed: Option<&[bool]>) -> Vec<bool> {
    field
        .iter()
        .enumerate()
        .map(|(cell, value)| *value > 0.0 && allowed.is_none_or(|allowed| allowed[cell]))
        .collect()
}

/// Removes the parts of the area narrower than `min_width`, with a
/// morphological opening, except the cells of `keep`.
fn remove_thin_parts(
    field: &mut [f32],
    allowed: Option<&[bool]>,
    keep: Option<&[bool]>,
    min_width: f32,
    grid: Grid,
) {
    let mask = get_mask(field, allowed);
    let radius = (min_width / 2.0) as f64;

    let squared_distances_to_outside = grid.squared_distances(mask.iter().map(|inside| !inside));
    let eroded = mask
        .iter()
        .zip(&squared_distances_to_outside)
        .map(|(inside, distance)| *inside && *distance > radius * radius);
    // Half a cell more restores the cells the erosion took from wide parts
    let squared_distances_to_eroded = grid.squared_distances(eroded);
    let squared_reach = (radius + 0.5) * (radius + 0.5);

    for cell in 0..grid.len() {
        if mask[cell]
            && squared_distances_to_eroded[cell] > squared_reach
            && !keep.is_some_and(|keep| keep[cell])
        {
            field[cell] = -NUDGE;
        }
    }
}

/// Removes the islands smaller than `min_area` cells, except those touching
/// the grid border, which may continue beyond it, and those holding a cell
/// of `keep`.
fn remove_small_islands(
    field: &mut [f32],
    allowed: Option<&[bool]>,
    keep: Option<&[bool]>,
    min_area: usize,
    grid: Grid,
) {
    let mask = get_mask(field, allowed);

    grid.for_each_component(&mask, |cells, touches_border| {
        if touches_border
            || cells.len() >= min_area
            || keep.is_some_and(|keep| cells.iter().any(|cell| keep[*cell]))
        {
            return;
        }

        for cell in cells {
            field[*cell] = -NUDGE;
        }
    });
}

/// Fills the holes smaller than `min_area` cells, except those touching the
/// grid border and those holding a cell outside `fillable`.
fn fill_small_holes(
    field: &mut [f32],
    allowed: Option<&[bool]>,
    fillable: &[bool],
    min_area: usize,
    grid: Grid,
) {
    let outside: Vec<bool> = get_mask(field, allowed)
        .into_iter()
        .map(|inside| !inside)
        .collect();

    grid.for_each_component(&outside, |cells, touches_border| {
        if touches_border || cells.len() >= min_area || !cells.iter().all(|cell| fillable[*cell]) {
            return;
        }

        for cell in cells {
            // Cells outside `allowed` may already be positive
            if field[*cell] <= 0.0 {
                field[*cell] = NUDGE;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: Grid = Grid {
        width: 20,
        height: 20,
    };

    /// A field over `GRID`, positive in the cells where `inside` holds.
    fn field(inside: impl Fn(usize, usize) -> bool) -> Vec<f32> {
        (0..GRID.len())
            .map(|cell| {
                if inside(cell % GRID.width, cell / GRID.width) {
                    1.0
                } else {
                    -1.0
                }
            })
            .collect()
    }

    fn count_inside(field: &[f32]) -> usize {
        field.iter().filter(|value| **value > 0.0).count()
    }

    fn square(x: usize, y: usize, size: usize) -> impl Fn(usize, usize) -> bool {
        move |cell_x, cell_y| (x..x + size).contains(&cell_x) && (y..y + size).contains(&cell_y)
    }

    #[test]
    fn removes_small_islands_away_from_the_border() {
        let mut values = field(|x, y| square(2, 2, 3)(x, y) || square(10, 10, 5)(x, y));

        remove_small_islands(&mut values, None, None, 10, GRID);

        assert_eq!(count_inside(&values), 25);
    }

    #[test]
    fn keeps_islands_touching_the_border_or_holding_a_kept_cell() {
        let mut values = field(|x, y| square(0, 2, 3)(x, y) || square(10, 10, 3)(x, y));
        let keep = get_mask(&field(|x, y| x == 11 && y == 11), None);

        remove_small_islands(&mut values, None, Some(&keep), 10, GRID);

        assert_eq!(count_inside(&values), 18);
    }

    #[test]
    fn fills_small_fillable_holes() {
        let ring = |x: usize, y: usize| square(2, 2, 16)(x, y) && !square(5, 5, 2)(x, y);
        let mut values = field(ring);
        let all = vec![true; GRID.len()];

        fill_small_holes(&mut values, None, &all, 5, GRID);
        assert_eq!(count_inside(&values), 256);

        let mut values = field(ring);
        let not_hole = get_mask(&field(|x, y| !(x == 5 && y == 5)), None);

        fill_small_holes(&mut values, None, &not_hole, 5, GRID);
        assert_eq!(count_inside(&values), 252);
    }

    #[test]
    fn removes_thin_parts_and_keeps_wide_ones() {
        // A 10 x 10 square with a 2 cells wide, 6 cells long spur
        let mut values = field(|x, y| square(2, 2, 10)(x, y) || (x >= 12 && (6..8).contains(&y)));

        remove_thin_parts(&mut values, None, None, 4.0, GRID);

        let mask = get_mask(&values, None);
        assert!(mask[6 * GRID.width + 6], "the square's center was removed");
        assert!(mask[2 * GRID.width + 6], "the square's edge was removed");
        assert!(!mask[6 * GRID.width + 16], "the spur was kept");
    }

    #[test]
    fn keeps_thin_parts_of_kept_cells() {
        let line = |x: usize, y: usize| y == 10 && (2..18).contains(&x);
        let mut values = field(line);
        let keep = get_mask(&field(line), None);

        remove_thin_parts(&mut values, None, Some(&keep), 4.0, GRID);

        assert_eq!(count_inside(&values), 16);
    }

    #[test]
    fn components_are_four_connected() {
        let mask = get_mask(&field(|x, y| (x, y) == (3, 3) || (x, y) == (4, 4)), None);
        let mut sizes = vec![];

        GRID.for_each_component(&mask, |cells, touches_border| {
            assert!(!touches_border);
            sizes.push(cells.len());
        });

        assert_eq!(sizes, vec![1, 1]);
    }

    #[test]
    fn simplifies_rings_within_tolerance() {
        let mut ring: Vec<[f64; 2]> = (0..=10).map(|x| [x as f64, 0.0]).collect();
        ring.push([10.0, 10.0]);
        ring.push([5.0, 10.2]);
        ring.push([0.0, 10.0]);

        let simplified = simplify_ring(&ring, 0.5, |_| false);

        assert_eq!(
            simplified,
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]
        );
        assert_eq!(simplify_ring(&ring, 0.0, |_| false), ring);
        assert_eq!(simplify_ring(&ring, 0.1, |_| false).len(), 5);
    }

    #[test]
    fn keeps_pinned_vertices_when_simplifying() {
        let mut ring: Vec<[f64; 2]> = (0..=10).map(|x| [x as f64, 0.0]).collect();
        ring.push([10.0, 10.0]);
        ring.push([0.0, 10.0]);

        let simplified = simplify_ring(&ring, 0.5, |point| point == [3.0, 0.0]);
        assert_eq!(
            simplified,
            vec![
                [3.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0]
            ]
        );

        let simplified = simplify_ring(&ring, 0.5, |point| point[1] == 0.0);
        assert_eq!(simplified.len(), ring.len());
    }

    #[test]
    fn keeps_the_points_where_iso_lines_cross_the_tile_edge() {
        // A wavy band crossing the tile edge at x = 10, traced in two tiles
        let grid = Grid {
            width: 20,
            height: 20,
        };
        let values: Vec<f32> = (0..grid.len())
            .map(|cell| {
                let (x, y) = ((cell % grid.width) as f32, (cell / grid.width) as f32);
                3.0 - (y - 10.0 - 2.0 * (x * 0.7).sin()).abs()
            })
            .collect();
        let tracer = |min_x: f64, max_x: f64| Tracer {
            builder: ContourBuilder::new(grid.width, grid.height, true),
            pinned_lines: PinnedLines::around_tile_edges(&extent(min_x, 0.0, max_x, 20.0), 1.0),
            window: extent(0.0, 0.0, 20.0, 20.0),
            simplify_tolerance: 1.0,
        };

        let edge_points = |shapes: Shapes| {
            let mut points: Vec<[i64; 2]> = shapes
                .iter()
                .flatten()
                .flatten()
                .filter(|point| (point[0] - 10.0).abs() < 1e-3)
                .map(|point| {
                    [
                        (point[0] * 1e4).round() as i64,
                        (point[1] * 1e4).round() as i64,
                    ]
                })
                .collect();
            points.sort();
            points
        };

        let west = edge_points(tracer(0.0, 10.0).trace(&values).unwrap());
        let east = edge_points(tracer(10.0, 19.0).trace(&values).unwrap());
        assert_eq!(west.len(), 2);
        assert_eq!(west, east);
    }

    #[test]
    fn neighbouring_tiles_meet_along_their_common_edge() {
        // Tiles cut from the same vegetation, as their buffered rasters are
        const SIZE: i64 = 100;
        let raster_size = SIZE as usize + 2 * BUFFER;
        let tile = |min_x: i64, min_y: i64| Tile {
            lidar_dir_path: Default::default(),
            render_dir_path: Default::default(),
            min_x,
            min_y,
            max_x: min_x + SIZE,
            max_y: min_y + SIZE,
        };
        let rasters = |tile: &Tile| {
            let cells = (0..raster_size * raster_size).map(|cell| {
                let x = (tile.min_x - BUFFER as i64 + (cell % raster_size) as i64) as f64;
                let y = (tile.max_y + BUFFER as i64 - 1 - (cell / raster_size) as i64) as f64;
                (x, y)
            });
            let medium = cells
                .clone()
                .map(|(x, y)| (5.0 + 5.0 * (x / 13.0).sin() * (y / 17.0).cos()).round() as u8)
                .collect();
            let high = cells
                .clone()
                .map(|(x, y)| {
                    if (x / 23.0 + y / 29.0).sin() > 0.3 {
                        5
                    } else {
                        0
                    }
                })
                .collect();
            let low = cells
                .map(|(x, y)| (2.0 + 2.0 * (x / 11.0 + y / 7.0).cos()).round() as u8)
                .collect();
            VegetationRasters::from_pixels(
                raster_size as u32,
                raster_size as u32,
                high,
                medium,
                low,
            )
        };

        // The vertices of each symbol's areas on the line `axis` = `SIZE`, to
        // the centimetre, far finer than an OCAD paper unit
        let edge_points = |tile: Tile, axis: usize, mode: &UndergrowthMode, tolerance: f64| {
            let features = get_vegetation_features(
                &tile,
                &rasters(&tile),
                &Config::default(),
                mode,
                tolerance,
            )
            .unwrap();
            let mut points: Vec<(String, i64)> = features
                .iter()
                .flat_map(|feature| match feature {
                    MapFeature::Area(symbol, polygon) => polygon
                        .rings()
                        .iter()
                        .flat_map(|ring| ring.points())
                        .map(|point| [point.x, point.y])
                        .filter(|point| (point[axis] - SIZE as f64).abs() < 1e-3)
                        .map(|point| {
                            (
                                format!("{symbol:?}"),
                                (point[1 - axis] * 100.0).round() as i64,
                            )
                        })
                        .collect::<Vec<_>>(),
                    MapFeature::Point(..) | MapFeature::Line(..) => vec![],
                })
                .collect();
            points.sort();
            points.dedup();
            points
        };

        for mode in [
            UndergrowthMode::Merge,
            UndergrowthMode::Symbol406,
            UndergrowthMode::Symbol409,
        ] {
            for tolerance in [0.0, 1.0] {
                let origin = edge_points(tile(0, 0), 0, &mode, tolerance);
                let east = edge_points(tile(SIZE, 0), 0, &mode, tolerance);
                assert!(origin.len() > 4);
                assert_eq!(origin, east, "{mode:?}, tolerance {tolerance}");

                let origin = edge_points(tile(0, 0), 1, &mode, tolerance);
                let north = edge_points(tile(0, SIZE), 1, &mode, tolerance);
                assert!(origin.len() > 4);
                assert_eq!(origin, north, "{mode:?}, tolerance {tolerance}");
            }
        }
    }

    fn extent(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Extent {
        Extent {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    #[test]
    fn traces_nested_areas_with_holes() {
        // A ring of 10 x 10 cells around a 4 x 4 hole, holding a 2 x 2 island
        let grid = Grid {
            width: 20,
            height: 20,
        };
        let values = field(|x, y| {
            (square(2, 2, 10)(x, y) && !square(5, 5, 4)(x, y)) || square(6, 6, 2)(x, y)
        });
        let tracer = Tracer {
            builder: ContourBuilder::new(grid.width, grid.height, true),
            pinned_lines: PinnedLines(vec![]),
            window: extent(0.0, 0.0, 20.0, 20.0),
            simplify_tolerance: 0.0,
        };

        let shapes = tracer.trace(&values).unwrap();

        assert_eq!(shapes.len(), 2);
        let areas: Vec<Vec<f64>> = shapes
            .iter()
            .map(|shape| shape.iter().map(|ring| ring_area(ring)).collect())
            .collect();
        // The outer ring passes half way between the cell centers, its
        // corners cut, and so on
        assert!(areas.iter().any(|shape| shape.len() == 2
            && (shape[0] - 99.5).abs() < 1e-6
            && (shape[1] - 15.5).abs() < 1e-6));
        assert!(areas
            .iter()
            .any(|shape| shape.len() == 1 && (shape[0] - 3.5).abs() < 1e-6));

        // Clipped to the left half of the ring
        let mut sink = FeatureSink {
            features: vec![],
            window: extent(0.0, 0.0, 20.0, 20.0),
            tile: extent(0.0, 0.0, 7.0, 20.0),
        };
        sink.push(AreaSymbol::VegetationSlowRunning406, shapes, SLIVER_AREA);
        assert_eq!(sink.features.len(), 2);
        assert!(sink.features.iter().all(|feature| match feature {
            MapFeature::Area(_, polygon) => polygon.bbox().max.x <= 7.0,
            MapFeature::Point(..) | MapFeature::Line(..) => false,
        }));
    }

    fn square_shape(min: f64, max: f64) -> Vec<Vec<[f64; 2]>> {
        vec![vec![[min, min], [max, min], [max, max], [min, max]]]
    }

    fn sink() -> FeatureSink {
        FeatureSink {
            features: vec![],
            window: extent(0.0, 0.0, 100.0, 100.0),
            tile: extent(0.0, 0.0, 100.0, 100.0),
        }
    }

    #[test]
    fn cuts_denser_areas_out_of_lighter_ones() {
        let mut sink = sink();

        sink.push(
            AreaSymbol::VegetationSlowRunning406,
            difference(
                &vec![square_shape(10.0, 20.0)],
                &vec![square_shape(12.0, 14.0)],
            ),
            SLIVER_AREA,
        );

        let features = sink.features;
        assert_eq!(features.len(), 1);
        let MapFeature::Area(AreaSymbol::VegetationSlowRunning406, polygon) = &features[0] else {
            panic!("unexpected feature");
        };
        assert_eq!(polygon.rings().len(), 2);
        assert!(matches!(polygon.rings()[0], PolygonRing::Outer(_)));
        assert!(matches!(polygon.rings()[1], PolygonRing::Inner(_)));
    }

    #[test]
    fn measures_shapes_before_clipping_them_to_the_tile() {
        let mut sink = FeatureSink {
            features: vec![],
            window: extent(-50.0, -50.0, 100.0, 100.0),
            tile: extent(0.0, 0.0, 50.0, 50.0),
        };

        sink.push(
            AreaSymbol::RoughOpenLand403,
            vec![
                // Large, but mostly beyond the tile edge
                square_shape(40.0, 60.0),
                // Small, crossing the tile edge
                square_shape(45.0, 52.0),
                // A sliver within the tile of a large shape
                square_shape(49.9, 70.0),
            ],
            100.0,
        );

        let areas: Vec<f64> = sink
            .features
            .iter()
            .map(|feature| match feature {
                MapFeature::Area(_, polygon) => {
                    let bbox = polygon.bbox();
                    (bbox.max.x - bbox.min.x) * (bbox.max.y - bbox.min.y)
                }
                MapFeature::Point(..) | MapFeature::Line(..) => unreachable!(),
            })
            .collect();
        assert_eq!(areas.len(), 2);
        assert!((areas[0] - 100.0).abs() < 1e-6);
        assert!((areas[1] - 0.01).abs() < 1e-6);
    }

    #[test]
    fn drops_small_shapes_away_from_the_window_border() {
        let mut sink = sink();

        sink.push(
            AreaSymbol::RoughOpenLand403,
            vec![
                // Too small
                square_shape(10.0, 15.0),
                // Reaching the window border
                square_shape(0.0, 5.0),
                square_shape(50.0, 70.0),
                // A sliver along the window border
                square_shape(0.0, 0.5),
            ],
            100.0,
        );

        let widths: Vec<f64> = sink
            .features
            .iter()
            .map(|feature| match feature {
                MapFeature::Area(_, polygon) => polygon.bbox().max.x - polygon.bbox().min.x,
                MapFeature::Point(..) | MapFeature::Line(..) => unreachable!(),
            })
            .collect();
        assert_eq!(widths, vec![5.0, 20.0]);
    }

    /// A grid of a 4 x 4 tile and its margin, without data in the margin.
    fn grid_without_neighbors() -> (Grid, Vec<bool>) {
        let grid = Grid {
            width: 2 * MARGIN + 4,
            height: 2 * MARGIN + 4,
        };
        let valid = (0..grid.len())
            .map(|cell| !grid.is_in_margin(cell))
            .collect();
        (grid, valid)
    }

    #[test]
    fn extends_the_tile_into_the_margin_without_data() {
        let (grid, mut valid) = grid_without_neighbors();
        let tile_cell = |x: usize, y: usize| (MARGIN + y) * grid.width + MARGIN + x;
        // A tile edge cell without data
        valid[tile_cell(3, 1)] = false;

        let sources = grid.extrusion_sources(&valid);

        assert_eq!(sources[tile_cell(1, 1)], Some(tile_cell(1, 1)));
        assert_eq!(sources[tile_cell(3, 1)], None);
        // West of the tile, north-west of it, and beyond the edge cell
        // without data
        assert_eq!(sources[(MARGIN + 2) * grid.width], Some(tile_cell(0, 2)));
        assert_eq!(sources[0], Some(tile_cell(0, 0)));
        assert_eq!(sources[(MARGIN + 1) * grid.width + grid.width - 1], None);
    }

    #[test]
    fn keeps_small_islands_crossing_the_tile_edge_without_neighbor() {
        let (grid, valid) = grid_without_neighbors();
        let sources = grid.extrusion_sources(&valid);
        // A 2 x 2 island on the tile's west edge
        let inside = |cell: usize| {
            let (x, y) = (cell % grid.width, cell / grid.width);
            (MARGIN..MARGIN + 2).contains(&x) && (MARGIN..MARGIN + 2).contains(&y)
        };
        let mut values: Vec<f32> = sources
            .iter()
            .map(|source| match source {
                Some(source) if inside(*source) => 1.0,
                Some(_) => -1.0,
                None => NO_DATA_FIELD,
            })
            .collect();

        remove_small_islands(&mut values, None, None, 10, grid);

        assert!(values[MARGIN * grid.width + MARGIN] > 0.0);
    }
}
