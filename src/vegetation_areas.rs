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
    constants::VEGETATION_BLOCK_SIZE,
    error::{Result, ResultContext},
    map_features::{AreaSymbol, Extent, MapFeature},
    tile::Tile,
    vegetation::{
        gaussian_kernel, minimum_3_by_3, separable_gaussian_filter, vegetation_interior,
        RasterInterior, UndergrowthMode, VegetationRasters, LOW_DENSITY_BLUR_RADIUS,
        MEDIUM_DENSITY_BLUR_RADIUS,
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

/// Value of the vegetation raster cells without LiDAR data.
const NO_DATA: u8 = 255;
/// Cells vectorized around the tile, so that areas reach the tile edge and
/// areas crossing it are measured beyond it. Must not exceed `BUFFER` minus
/// the blur radius.
const MARGIN: usize = 50;
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

    let tracer = Tracer {
        builder: ContourBuilder::new(fields.grid.width, fields.grid.height, true)
            .x_origin((tile.min_x - MARGIN as i64) as f64)
            .y_origin((tile.max_y + MARGIN as i64) as f64)
            .y_step(-1.0),
        tile_rectangle: vec![
            [tile.min_x as f64, tile.min_y as f64],
            [tile.max_x as f64, tile.min_y as f64],
            [tile.max_x as f64, tile.max_y as f64],
            [tile.min_x as f64, tile.max_y as f64],
        ],
        simplify_tolerance,
    };

    let mut green_or_denser = vec![];
    for field in &mut fields.green {
        fields.grid.extend_into_margin(field, &fields.valid);
        green_or_denser.push(tracer.trace(field)?);
    }
    fields
        .grid
        .extend_into_margin(&mut fields.open_land, &fields.valid);
    let open_land = tracer.trace(&fields.open_land)?;

    let mut features = FeatureSink {
        features: vec![],
        extent: Extent {
            min_x: tile.min_x as f64,
            min_y: tile.min_y as f64,
            max_x: tile.max_x as f64,
            max_y: tile.max_y as f64,
        },
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

    if let Some(undergrowth) = &mut fields.undergrowth_409 {
        fields.grid.extend_into_margin(undergrowth, &fields.valid);
        // ISOM does not combine 409 with the green screens
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
type Shapes = Vec<Vec<Vec<[f64; 2]>>>;

fn difference(subject: &Shapes, clip: &Shapes) -> Shapes {
    subject.overlay(clip, OverlayRule::Difference, FillRule::EvenOdd)
}

struct FeatureSink {
    features: Vec<MapFeature>,
    extent: Extent,
}

impl FeatureSink {
    /// Pushes one area feature per shape, dropping the shapes smaller than
    /// `min_area` m² away from the tile edge, and the slivers.
    fn push(&mut self, symbol: AreaSymbol, shapes: Shapes, min_area: f64) {
        let to_points =
            |ring: Vec<[f64; 2]>| ring.into_iter().map(|[x, y]| Point::new(x, y)).collect();

        for shape in shapes {
            // The outer ring comes first
            let mut rings = shape.into_iter();
            let Some(outer) = rings.next() else {
                continue;
            };
            let area = ring_area(&outer);
            if area < SLIVER_AREA || (area < min_area && !self.touches_edge(&outer)) {
                continue;
            }

            let polygon = Polygon::with_rings(
                std::iter::once(PolygonRing::Outer(to_points(outer)))
                    .chain(
                        rings
                            .filter(|hole| ring_area(hole) >= SLIVER_AREA)
                            .map(|hole| PolygonRing::Inner(to_points(hole))),
                    )
                    .collect(),
            );
            self.features.push(MapFeature::Area(symbol, polygon));
        }
    }

    fn touches_edge(&self, ring: &[[f64; 2]]) -> bool {
        const TOLERANCE: f64 = 1e-6;
        ring.iter().any(|[x, y]| {
            (x - self.extent.min_x).abs() < TOLERANCE
                || (x - self.extent.max_x).abs() < TOLERANCE
                || (y - self.extent.min_y).abs() < TOLERANCE
                || (y - self.extent.max_y).abs() < TOLERANCE
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

struct Tracer {
    builder: ContourBuilder,
    tile_rectangle: Vec<[f64; 2]>,
    simplify_tolerance: f64,
}

impl Tracer {
    /// Returns the area where `field` is positive, clipped to the tile.
    fn trace(&self, field: &[f32]) -> Result<Shapes> {
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
            .context("could not trace vegetation areas")?;

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
                simplify_ring(&ring, self.simplify_tolerance)
            })
            .filter(|ring| ring.len() >= 3)
            .collect();

        // Iso-lines of a threshold never cross, so the even-odd rule turns
        // nested rings into holes and islands in holes.
        Ok(rings.overlay(
            &self.tile_rectangle,
            OverlayRule::Intersect,
            FillRule::EvenOdd,
        ))
    }
}

/// Douglas–Peucker simplification of a ring without closing point.
fn simplify_ring(ring: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    if tolerance <= 0.0 || ring.len() < 4 {
        return ring.to_vec();
    }

    // Douglas–Peucker keeps the ends of a chain, so the ring is split in two
    // chains at the vertex farthest from its first one
    let farthest = (1..ring.len())
        .max_by(|a, b| {
            squared_distance(ring[0], ring[*a]).total_cmp(&squared_distance(ring[0], ring[*b]))
        })
        .unwrap();

    let closed: Vec<[f64; 2]> = ring.iter().copied().chain([ring[0]]).collect();
    let mut kept = vec![false; closed.len()];
    kept[0] = true;
    kept[farthest] = true;

    let squared_tolerance = tolerance * tolerance;
    let mut chains = vec![(0, farthest), (farthest, ring.len())];
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
    ring.iter()
        .zip(&kept)
        .filter(|(_, kept)| **kept)
        .map(|(point, _)| *point)
        .collect()
}

fn squared_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn squared_segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
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

    /// Gives the cells without data in the margin the mean of their
    /// neighbours, so that areas are traced beyond the tile edge and not
    /// closed just inside it when a neighbouring tile is missing.
    fn extend_into_margin(self, field: &mut [f32], valid: &[bool]) {
        let mut known = valid.to_vec();

        // Two rings are enough, the margin is clipped off
        for _ in 0..2 {
            let extended: Vec<(usize, f32)> = (0..self.len())
                .filter(|cell| !known[*cell] && self.is_in_margin(*cell))
                .filter_map(|cell| {
                    let (sum, count) = self
                        .neighbors(cell)
                        .filter(|neighbor| known[*neighbor])
                        .fold((0.0, 0), |(sum, count), neighbor| {
                            (sum + field[neighbor], count + 1)
                        });
                    (count > 0).then(|| (cell, sum / count as f32))
                })
                .collect();

            for (cell, value) in extended {
                field[cell] = value;
                known[cell] = true;
            }
        }
    }
}

/// Fields over the grid, positive inside their symbol's area.
struct VegetationFields {
    grid: Grid,
    /// Whether each cell has LiDAR data.
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
        let interior = vegetation_interior(tile, [&rasters.high, &rasters.medium, &rasters.low])?;
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
        let raster_width = rasters.medium.width as usize;

        let raster_valid: Vec<u8> = rasters
            .high
            .pixels
            .iter()
            .zip(&rasters.medium.pixels)
            .zip(&rasters.low.pixels)
            .map(|((high, medium), low)| {
                u8::from(*high != NO_DATA && *medium != NO_DATA && *low != NO_DATA)
            })
            .collect();
        let without_no_data = |pixels: &[u8]| -> Vec<u8> {
            pixels
                .iter()
                .zip(&raster_valid)
                .map(|(pixel, valid)| pixel * valid)
                .collect()
        };
        let medium = without_no_data(&rasters.medium.pixels);
        let low = without_no_data(&rasters.low.pixels);

        // Normalized convolution: the blur of the values over the blur of
        // the data mask
        let blur = |pixels: &[u8], secondary: Option<&[u8]>, radius: usize| -> Vec<f32> {
            let kernel = gaussian_kernel(radius);
            let sums = separable_gaussian_filter(pixels, secondary, raster_width, window, &kernel);
            let weights =
                separable_gaussian_filter(&raster_valid, None, raster_width, window, &kernel);
            sums.iter()
                .zip(weights)
                .map(|(sum, weight)| if weight > 0.0 { sum / weight } else { 0.0 })
                .collect()
        };

        let merge_low = matches!(undergrowth_mode, UndergrowthMode::Merge).then_some(&low[..]);
        let green_density = blur(&medium, merge_low, MEDIUM_DENSITY_BLUR_RADIUS);
        let low_density = matches!(
            undergrowth_mode,
            UndergrowthMode::Symbol406 | UndergrowthMode::Symbol409
        )
        .then(|| blur(&low, None, LOW_DENSITY_BLUR_RADIUS));

        let source_index = |cell: usize| {
            (window.y + cell / grid.width) * raster_width + window.x + cell % grid.width
        };
        let valid: Vec<bool> = (0..grid.len())
            .map(|cell| raster_valid[source_index(cell)] != 0)
            .collect();

        // No data is the largest value, so the minimum ignores it
        let is_high = |cell: usize| {
            let source = source_index(cell);
            valid[cell]
                && minimum_3_by_3(
                    &rasters.high.pixels,
                    raster_width,
                    source % raster_width,
                    source / raster_width,
                ) > config.yellow_threshold as u8
        };
        let squared_distances_to_high = grid.squared_distances((0..grid.len()).map(is_high));

        let field = |value: &dyn Fn(usize) -> f32| -> Vec<f32> {
            (0..grid.len())
                .map(|cell| {
                    if valid[cell] {
                        value(cell)
                    } else {
                        NO_DATA_FIELD
                    }
                })
                .collect()
        };

        let undergrowth = |cell: usize| {
            low_density.as_ref().unwrap()[cell] - config.low_vegetation_density_threshold
        };
        let light_green = |cell: usize| {
            let green = green_density[cell] - config.green_threshold_1;
            match undergrowth_mode {
                // Undergrowth is drawn with the 406 screen
                UndergrowthMode::Symbol406 => green.max(undergrowth(cell)),
                _ => green,
            }
        };

        Ok(Self {
            grid,
            green: [
                field(&light_green),
                field(&|cell| green_density[cell] - config.green_threshold_2),
                field(&|cell| green_density[cell] - config.green_threshold_3),
            ],
            open_land: field(&|cell| {
                (squared_distances_to_high[cell].sqrt() as f32 - OPEN_LAND_DISTANCE)
                    .min(MAX_OPEN_LAND_FIELD)
            }),
            undergrowth_409: matches!(undergrowth_mode, UndergrowthMode::Symbol409)
                .then(|| field(&undergrowth)),
            valid,
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
        let mut values = field(|x, y| square(2, 2, 10)(x, y) || (x >= 12 && y >= 6 && y < 8));

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

        let simplified = simplify_ring(&ring, 0.5);

        assert_eq!(
            simplified,
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]
        );
        assert_eq!(simplify_ring(&ring, 0.0), ring);
        assert_eq!(simplify_ring(&ring, 0.1).len(), 5);
    }

    #[test]
    fn traces_nested_areas_with_holes_and_clips_them() {
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
            tile_rectangle: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
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
        let clipped = Tracer {
            tile_rectangle: vec![[0.0, 0.0], [7.0, 0.0], [7.0, 20.0], [0.0, 20.0]],
            ..tracer
        }
        .trace(&values)
        .unwrap();
        assert!(clipped
            .iter()
            .flatten()
            .flatten()
            .all(|point| point[0] <= 7.0));
    }

    fn square_shape(min: f64, max: f64) -> Vec<Vec<[f64; 2]>> {
        vec![vec![[min, min], [max, min], [max, max], [min, max]]]
    }

    fn sink() -> FeatureSink {
        FeatureSink {
            features: vec![],
            extent: Extent {
                min_x: 0.0,
                min_y: 0.0,
                max_x: 100.0,
                max_y: 100.0,
            },
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
    fn drops_small_shapes_away_from_the_tile_edge() {
        let mut sink = sink();

        sink.push(
            AreaSymbol::RoughOpenLand403,
            vec![
                // Too small
                square_shape(10.0, 15.0),
                // Clipped by the tile edge
                square_shape(0.0, 5.0),
                square_shape(50.0, 70.0),
                // A sliver along the tile edge
                square_shape(0.0, 0.5),
            ],
            100.0,
        );

        let widths: Vec<f64> = sink
            .features
            .iter()
            .map(|feature| match feature {
                MapFeature::Area(_, polygon) => polygon.bbox().max.x - polygon.bbox().min.x,
                MapFeature::Line(..) => unreachable!(),
            })
            .collect();
        assert_eq!(widths, vec![5.0, 20.0]);
    }

    #[test]
    fn extends_fields_into_the_margin_without_data() {
        let grid = Grid {
            width: 2 * MARGIN + 2,
            height: 2 * MARGIN + 2,
        };
        let valid: Vec<bool> = (0..grid.len())
            .map(|cell| !grid.is_in_margin(cell))
            .collect();
        let mut values: Vec<f32> = valid
            .iter()
            .map(|valid| if *valid { 2.0 } else { NO_DATA_FIELD })
            .collect();

        grid.extend_into_margin(&mut values, &valid);

        let row = MARGIN * grid.width;
        assert_eq!(values[row + MARGIN - 1], 2.0);
        assert_eq!(values[row + MARGIN - 2], 2.0);
        assert_eq!(values[row + MARGIN - 3], NO_DATA_FIELD);
    }
}
