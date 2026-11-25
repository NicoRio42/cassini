use crate::{
    config::Config,
    dem::{check_dem_with_buffer, create_dem_with_buffer},
    download::download_osm_file,
    error::{CassiniError, Result, ResultContext, Stage, TileId},
    files::{is_input_file, remove_if_exists, GEOTIFF_EXTENSIONS, LAZ_EXTENSIONS},
    grid::GridExtent,
    lidar::{
        read_tile_id, read_vegetation_layers, run_dem_pass, VegetationByHeightPass,
        VegetationLayers, VEGETATION_LAYERS_FILE_NAME,
    },
    merge::merge_maps,
    ocad_export::{
        get_batch_georeferencing, prepare_tile_for_merge, write_merged_ocd_file,
        MERGED_OCD_FILE_NAME,
    },
    ocd::{Georeferencing, OcdTemplate},
    render::{remove_render_step_files, render_tile_map, RenderOptions},
    tile::{Tile, TileWithNeighbors},
};
use log::{info, warn};
use std::{
    collections::{BTreeMap, HashMap},
    fs::read_dir,
    path::{Path, PathBuf},
    sync::Arc,
    thread::{spawn, JoinHandle},
};

/// A worker thread processing a chunk of tiles, with the first tile of the
/// chunk to report a panic.
type Worker<T> = (Option<TileId>, JoinHandle<Vec<Result<T>>>);

/// The OCAD export of a batch to a single merged file.
#[derive(Clone)]
struct MergedOcadExport {
    template: Arc<OcdTemplate>,
    /// The tiles are exported in the coordinates of the merged file.
    georeferencing: Georeferencing,
}

pub fn batch(
    input_dir: &str,
    output_dir: &str,
    number_of_threads: usize,
    skip_lidar: bool,
    vegetation_layers: VegetationLayers,
    options: RenderOptions,
    config: Config,
) -> Result<()> {
    if number_of_threads == 0 {
        return Err(CassiniError::InvalidInput {
            message: "the number of batch threads must be greater than zero".to_owned(),
        });
    }

    // With `--skip-lidar`, the DEMs are the `dem.tif` files of the previous run.
    let tiles = get_tiles_with_neighbors(input_dir, output_dir, !skip_lidar)?;
    if tiles.is_empty() {
        return Err(CassiniError::InvalidInput {
            message: format!("no LAZ files were found in `{input_dir}`"),
        });
    }
    if skip_lidar {
        let ignored = geotiff_paths(input_dir)?.len();
        if ignored > 0 {
            warn!(
                "Ignoring the {ignored} GeoTIFF files of `{input_dir}` with `--skip-lidar`: the \
                 DEMs are those of the previous run"
            );
        }
    }

    let chunk_size = tiles.len().div_ceil(number_of_threads);

    let ocad_export = options
        .ocad
        .as_ref()
        .map(|ocad| -> Result<MergedOcadExport> {
            Ok(MergedOcadExport {
                template: ocad.template.clone(),
                georeferencing: get_batch_georeferencing(&ocad.template, get_batch_extent(&tiles))?,
            })
        })
        .transpose()?;
    let merged_ocd_path = Path::new(output_dir).join(MERGED_OCD_FILE_NAME);
    remove_if_exists(&merged_ocd_path).context(format!(
        "could not remove stale merged map `{}`",
        merged_ocd_path.display()
    ))?;

    // The LiDAR output that `--skip-lidar` reuses is checked before anything
    // is deleted.
    let reused_layers = if skip_lidar {
        Some(check_previous_lidar_stage(&tiles)?)
    } else {
        None
    };
    cleanup_previous_run(&tiles, output_dir)?;
    match reused_layers {
        Some(layers) => info!(
            "Reusing the LiDAR output of the previous run, with the vegetation layers by {}",
            layers.name()
        ),
        None => run_lidar_stage(&tiles, chunk_size, vegetation_layers)?,
    }

    if !options.skip_vector {
        for tile in tiles.iter() {
            let osm_path = tile.tile.render_dir_path.join(format!(
                "{:0>7}_{:0>7}.osm",
                tile.tile.min_x, tile.tile.max_y
            ));

            if !osm_path.exists() {
                download_osm_file(
                    tile.tile.min_x,
                    tile.tile.min_y,
                    tile.tile.max_x,
                    tile.tile.max_y,
                    &tile.tile.render_dir_path,
                )?;
            }
        }
    }

    let render_config = config.clone();
    let render_ocad_export = ocad_export.clone();
    let seam_objects = run_stage(
        tiles.clone(),
        chunk_size,
        Stage::Render,
        move |tile: TileWithNeighbors| {
            info!("Tile {}. Rendering map", tile.tile);

            let features =
                render_tile_map(tile.tile.clone(), tile.neighbors, &options, &render_config)?;

            render_ocad_export
                .as_ref()
                .zip(features)
                .map(|(export, features)| {
                    prepare_tile_for_merge(&tile.tile, &export.georeferencing, &features)
                })
                .transpose()
        },
    )?;

    if let Some(export) = &ocad_export {
        write_merged_ocd_file(
            Path::new(output_dir),
            &export.template,
            &export.georeferencing,
            tiles.iter().map(|tile| &tile.tile),
            seam_objects.into_iter().flatten().collect(),
        )?;
    }

    merge_maps(output_dir, tiles, &config)?;

    Ok(())
}

/// Writes the LiDAR output of every tile, and the filled DEM of each
/// buffered tile, `dem_with_buffer.tif`, which needs the DEMs of its
/// neighbours: the DEM pass of every tile, then the filled DEMs and, in
/// `Height` mode, the vegetation passes above them.
fn run_lidar_stage(
    tiles: &[TileWithNeighbors],
    chunk_size: usize,
    layers: VegetationLayers,
) -> Result<()> {
    // In `LidarClass` mode, the vegetation densities are computed in the same
    // pass over the points as the DEM.
    let dem_passes = run_stage(
        tiles.to_vec(),
        chunk_size,
        Stage::Lidar,
        move |tile: TileWithNeighbors| {
            info!("Tile {}. Generating raw rasters", tile.tile);

            let dem_pass = run_dem_pass(
                &tile.laz_path,
                tile.dem_path.as_deref(),
                &tile.tile.lidar_dir_path,
                layers,
            )?;
            Ok((tile, dem_pass.vegetation_by_height))
        },
    )?;

    // Every DEM is written, so each tile's can be mosaicked with its
    // neighbours'. The render stage reads `dem_with_buffer.tif`, so that the
    // vegetation heights and the contours and cliffs come from the same
    // filled DEM.
    run_stage(
        dem_passes,
        chunk_size,
        Stage::Lidar,
        |(tile, vegetation_by_height): (TileWithNeighbors, Option<VegetationByHeightPass>)| {
            let dem_with_buffer = create_dem_with_buffer(&tile.tile, &tile.neighbors)?;
            if let Some(vegetation_by_height) = vegetation_by_height {
                vegetation_by_height.run(&dem_with_buffer)?;
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// The files that the versions before 0.19.0, which ran PDAL and `ogr2ogr`,
/// wrote in the tile directories, and that nothing reads any more.
const PREVIOUS_VERSION_FILES: [&str; 6] = [
    "dem-low-resolution.tif",
    "high-vegetation.tif",
    "low-vegetation.tif",
    "medium-vegetation.tif",
    "pipeline.json",
    "shapes",
];

/// Removes the maps of the previous run, so that a run that fails leaves
/// none of them next to its own output: the render output of every tile,
/// which the render step also removes but only once it reaches the tile, and
/// the merged maps. Also removes the files of the versions before 0.19.0:
/// unlike the output directory of `process` and `lidar`, the tile
/// directories only hold what `batch` writes.
fn cleanup_previous_run(tiles: &[TileWithNeighbors], output_dir: &str) -> Result<()> {
    for tile in tiles {
        let dir_path = &tile.tile.render_dir_path;
        remove_render_step_files(dir_path)?;
        for name in PREVIOUS_VERSION_FILES {
            let path = dir_path.join(name);
            remove_if_exists(&path).context(format!(
                "could not remove the output of a previous version `{}`",
                path.display()
            ))?;
        }
    }

    let entries =
        read_dir(output_dir).context(format!("could not read output directory `{output_dir}`"))?;
    for entry in entries {
        let entry = entry.context(format!("could not read an entry in `{output_dir}`"))?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("merged-map")
        {
            std::fs::remove_file(entry.path()).context(format!(
                "could not remove stale merged map `{}`",
                entry.path().display()
            ))?;
        }
    }
    Ok(())
}

/// Fails unless every tile has the LiDAR output that `--skip-lidar` renders
/// from, before anything is deleted: that of a batch of Cassini 0.19.0 or
/// later, with its filled DEM. Returns how its vegetation densities were
/// sorted into layers.
fn check_previous_lidar_stage(tiles: &[TileWithNeighbors]) -> Result<VegetationLayers> {
    let mut problems = vec![];
    let mut failed_tiles = 0;
    let mut tile_layers = vec![];
    for tile in tiles {
        let problem_count = problems.len();
        match read_vegetation_layers(&tile.tile.lidar_dir_path) {
            Ok(Some(layers)) => tile_layers.push(layers),
            Ok(None) => problems.push(format!(
                "`{}` has no `{VEGETATION_LAYERS_FILE_NAME}`, which the LiDAR stage writes since \
                 Cassini 0.19.0",
                tile.tile.lidar_dir_path.display()
            )),
            Err(error) => problems.push(error.to_string()),
        }
        if let Err(error) = check_dem_with_buffer(&tile.tile) {
            problems.push(error.to_string());
        }
        if problems.len() > problem_count {
            failed_tiles += 1;
        }
    }
    if !problems.is_empty() {
        return Err(CassiniError::InvalidInput {
            message: format!(
                "`--skip-lidar` renders from the LiDAR output of the previous run, and {failed_tiles} \
                 of the {} tiles have none to reuse: {}. Run the batch once without `--skip-lidar`",
                tiles.len(),
                problems.join("; ")
            ),
        });
    }

    let layers = tile_layers[0];
    let by_height = tile_layers
        .iter()
        .filter(|&&layers| layers == VegetationLayers::Height)
        .count();
    if tile_layers.iter().any(|&other| other != layers) {
        return Err(CassiniError::InvalidInput {
            message: format!(
                "the previous run sorted the vegetation of {by_height} tiles by height and {} by \
                 LiDAR class. Run the batch once without `--skip-lidar`",
                tiles.len() - by_height
            ),
        });
    }
    Ok(layers)
}

/// An item processed by a batch stage, for a tile.
trait StageItem: Send + 'static {
    /// The tile, whose failures and panics are reported with its extent.
    fn tile(&self) -> &Tile;
}

impl StageItem for TileWithNeighbors {
    fn tile(&self) -> &Tile {
        &self.tile
    }
}

impl<T: Send + 'static> StageItem for (TileWithNeighbors, T) {
    fn tile(&self) -> &Tile {
        &self.0.tile
    }
}

/// Runs `process` on each of `items`, on worker threads taking `chunk_size`
/// items each, and returns the results in item order, or the failures of
/// every item that failed.
fn run_stage<I, T, F>(items: Vec<I>, chunk_size: usize, stage: Stage, process: F) -> Result<Vec<T>>
where
    I: StageItem,
    T: Send + 'static,
    F: Fn(I) -> Result<T> + Send + Sync + 'static,
{
    let process = Arc::new(process);
    let mut items = items.into_iter().peekable();
    let mut handles: Vec<Worker<T>> = vec![];
    while items.peek().is_some() {
        let chunk: Vec<I> = items.by_ref().take(chunk_size).collect();
        let first_tile = chunk.first().map(|item| TileId::from(item.tile()));
        let process = process.clone();
        let spawned_thread = spawn(move || chunk.into_iter().map(|item| process(item)).collect());
        handles.push((first_tile, spawned_thread));
    }

    let (values, failures) = collect_worker_results(handles, stage);
    if !failures.is_empty() {
        return Err(CassiniError::Batch {
            failure_count: failures.len(),
            failures,
        });
    }
    Ok(values)
}

/// The extent covering every tile, as min x, min y, max x, max y.
fn get_batch_extent(tiles: &[TileWithNeighbors]) -> (i64, i64, i64, i64) {
    tiles.iter().fold(
        (i64::MAX, i64::MAX, i64::MIN, i64::MIN),
        |(min_x, min_y, max_x, max_y), tile| {
            (
                min_x.min(tile.tile.min_x),
                min_y.min(tile.tile.min_y),
                max_x.max(tile.tile.max_x),
                max_y.max(tile.tile.max_y),
            )
        },
    )
}

/// Returns the results of the workers' tiles, and the failures of the tiles
/// and of the workers that panicked.
fn collect_worker_results<T>(handles: Vec<Worker<T>>, stage: Stage) -> (Vec<T>, Vec<CassiniError>) {
    let mut values = Vec::new();
    let mut failures = Vec::new();
    for (tile, handle) in handles {
        match handle.join() {
            Ok(results) => {
                for result in results {
                    match result {
                        Ok(value) => values.push(value),
                        Err(error) => failures.push(error),
                    }
                }
            }
            Err(payload) => failures.push(CassiniError::WorkerPanicked {
                stage,
                tile_suffix: CassiniError::tile_suffix(tile),
                message: panic_message(payload),
            }),
        }
    }
    (values, failures)
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}

/// The tiles of the LAZ files of `input_dir`, with their DEM files from
/// `input_dir` if `use_dem_files` (see `find_dems`).
pub fn get_tiles_with_neighbors(
    input_dir: &str,
    output_dir: &str,
    use_dem_files: bool,
) -> Result<Vec<TileWithNeighbors>> {
    let paths =
        read_dir(input_dir).context(format!("could not read input directory `{input_dir}`"))?;
    let mut tiles: Vec<TileWithNeighbors> = vec![];
    // Ordered, for the same merged map, OCAD file and logs on every run.
    let mut tiles_map = BTreeMap::<(i64, i64, i64, i64), PathBuf>::new();

    for dir_entry in paths {
        let path = dir_entry
            .context(format!("could not read an entry in `{input_dir}`"))?
            .path();

        if !is_input_file(&path, &LAZ_EXTENSIONS) {
            continue;
        }

        let tile = read_tile_id(&path)?;
        let extent = (tile.min_x, tile.min_y, tile.max_x, tile.max_y);

        if let Some(previous) = tiles_map.insert(extent, path.clone()) {
            return Err(CassiniError::InvalidInput {
                message: format!(
                    "LiDAR files `{}` and `{}` have the same rounded extent",
                    previous.display(),
                    path.display()
                ),
            });
        }
    }

    let mut dems = if use_dem_files {
        let tile_ids: Vec<TileId> = tiles_map
            .keys()
            .map(|&(min_x, min_y, max_x, max_y)| TileId {
                min_x,
                min_y,
                max_x,
                max_y,
            })
            .collect();
        find_dems(input_dir, &tile_ids)?
    } else {
        HashMap::new()
    };

    for ((min_x, min_y, max_x, max_y), laz_path) in tiles_map.clone().into_iter() {
        let width = max_x - min_x;
        let height = max_y - min_y;

        let dir_path =
            Path::new(output_dir).join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y));

        let tile = Tile {
            lidar_dir_path: dir_path.to_path_buf(),
            render_dir_path: dir_path,
            min_x,
            min_y,
            max_x,
            max_y,
        };

        let neighbors: Vec<PathBuf> = vec![
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                min_x,
                max_y,
                max_x,
                max_y + height,
            ),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                max_x,
                max_y,
                max_x + width,
                max_y + height,
            ),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                max_x,
                min_y,
                max_x + width,
                max_y,
            ),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                max_x,
                min_y - height,
                max_x + width,
                min_y,
            ),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                min_x,
                min_y - height,
                max_x,
                min_y,
            ),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                min_x - width,
                min_y - height,
                min_x,
                min_y,
            ),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                min_x - width,
                min_y,
                min_x,
                max_y,
            ),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                min_x - width,
                max_y,
                min_x,
                max_y + height,
            ),
        ]
        .into_iter()
        .flatten()
        .collect();

        tiles.push(TileWithNeighbors {
            laz_path,
            dem_path: dems.remove(&TileId::from(&tile)),
            tile,
            neighbors,
        })
    }

    Ok(tiles)
}

/// The GeoTIFF files (`.tif` or `.tiff`, in any case) of `input_dir`, sorted,
/// without hidden files such as the `._*` files macOS writes on some drives.
fn geotiff_paths(input_dir: &str) -> Result<Vec<PathBuf>> {
    let mut paths = vec![];
    for dir_entry in
        read_dir(input_dir).context(format!("could not read input directory `{input_dir}`"))?
    {
        let path = dir_entry
            .context(format!("could not read an entry in `{input_dir}`"))?
            .path();
        if is_input_file(&path, &GEOTIFF_EXTENSIONS) {
            paths.push(path);
        }
    }
    // For the same errors and warnings whatever the directory order.
    paths.sort();
    Ok(paths)
}

/// The DEM of each of `tiles` that has one: the GeoTIFF file of `input_dir`
/// whose extent's centre is in the tile. Matching by extent rather than by
/// file name lets renamed files work, and IGN's DEM tiles, offset by a
/// quarter metre, match their LiDAR tiles.
///
/// A GeoTIFF file that is not a DEM in Lambert-93, such as an orthophoto, is
/// ignored with a warning, as is a DEM centred in no tile. Two DEMs centred
/// in the same tile are an error.
fn find_dems(input_dir: &str, tiles: &[TileId]) -> Result<HashMap<TileId, PathBuf>> {
    let mut dems = HashMap::<TileId, PathBuf>::new();
    for dem_path in geotiff_paths(input_dir)? {
        let extent = match GridExtent::read_geotiff(&dem_path) {
            Ok(extent) => extent,
            Err(error) => {
                warn!(
                    "`{}` is not a DEM Cassini can read, ignoring it: {error}",
                    dem_path.display()
                );
                continue;
            }
        };
        let centre_x = (extent.min_x + extent.max_x()) / 2.0;
        let centre_y = (extent.min_y() + extent.max_y) / 2.0;
        let Some(&tile) = tiles.iter().find(|tile| {
            (tile.min_x as f64..tile.max_x as f64).contains(&centre_x)
                && (tile.min_y as f64..tile.max_y as f64).contains(&centre_y)
        }) else {
            warn!(
                "DEM `{}` is centred on ({centre_x}, {centre_y}), in no LiDAR tile, ignoring it",
                dem_path.display()
            );
            continue;
        };
        if let Some(previous) = dems.get(&tile) {
            return Err(CassiniError::InvalidInput {
                message: format!(
                    "DEMs `{}` and `{}` are both centred in tile {tile}",
                    previous.display(),
                    dem_path.display(),
                ),
            });
        }
        dems.insert(tile, dem_path);
    }

    if !dems.is_empty() {
        info!(
            "Using the DEMs of `{input_dir}` for {} of the {} tiles",
            dems.len(),
            tiles.len()
        );
        let mut without_dem: Vec<&TileId> = tiles
            .iter()
            .filter(|tile| !dems.contains_key(tile))
            .collect();
        if !without_dem.is_empty() {
            without_dem.sort_by_key(|tile| (tile.min_x, tile.min_y));
            warn!(
                "No DEM in `{input_dir}` for {} of the {} tiles, whose DEM is computed from the \
                 ground points instead: {}. The contours may show seams between the two kinds \
                 of DEM",
                without_dem.len(),
                tiles.len(),
                without_dem
                    .iter()
                    .map(|tile| tile.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    Ok(dems)
}

fn get_neighbor_tile_from_hash_map(
    tiles_map: &BTreeMap<(i64, i64, i64, i64), PathBuf>,
    output_dir: &str,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> Option<PathBuf> {
    tiles_map
        .get(&(min_x, min_y, max_x, max_y))
        .map(|_| Path::new(output_dir).join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)))
}

#[cfg(test)]
mod tests {
    use super::{
        batch, check_previous_lidar_stage, find_dems, get_tiles_with_neighbors, run_lidar_stage,
    };
    use crate::{
        config::Config,
        constants::{WHITE, YELLOW},
        dem::{read_dem_with_buffer, DEM_WITH_BUFFER_FILE_NAME},
        error::{CassiniError, TileId},
        grid::{Compression, Grid},
        lidar::{
            test_tiles::{
                counts_in, vegetation_counts, write_tile, write_tile_at, CELLS, MIN_X, MIN_Y,
                POINTS, SIDE,
            },
            VegetationLayers, VEGETATION_LAYERS_FILE_NAME,
        },
        render::RenderOptions,
        render_transform::RenderTransform,
        tile::{Tile, TileWithNeighbors},
        vegetation::UndergrowthMode,
    };
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
    };

    /// The 1 km tile with its south-west corner at (`min_x`, `min_y`).
    fn tile(min_x: i64, min_y: i64) -> TileId {
        TileId {
            min_x,
            min_y,
            max_x: min_x + 1000,
            max_y: min_y + 1000,
        }
    }

    /// Writes a 1 km DEM of 0.5 m cells to `path`, offset by a quarter metre
    /// from the tile with its south-west corner at (`min_x`, `min_y`), as
    /// IGN's are.
    fn write_dem(path: &Path, min_x: f64, min_y: f64) {
        Grid::new(
            min_x - 0.25,
            min_y + 1000.25,
            0.5,
            2000,
            2000,
            Some(-9999.0),
            0.0f32,
        )
        .write_geotiff(path, Compression::None)
        .unwrap();
    }

    #[test]
    fn finds_the_dems_of_the_tiles_containing_their_centre() {
        let dir = tempfile::tempdir().unwrap();
        let input_dir = dir.path().to_str().unwrap();
        let tiles = [tile(0, 0), tile(1000, 0), tile(0, 1000)];
        // Names unrelated to the tiles, and upper case extensions.
        write_dem(&dir.path().join("b.TIF"), 0.0, 0.0);
        write_dem(&dir.path().join("a.tiff"), 1000.0, 0.0);
        // A DEM matching no tile, files that are not DEMs, and a hidden copy
        // of a DEM, as macOS writes them.
        write_dem(&dir.path().join("far.tif"), 5000.0, 0.0);
        std::fs::write(dir.path().join("notes.txt"), "not a DEM").unwrap();
        std::fs::create_dir(dir.path().join("folder.tif")).unwrap();
        std::fs::write(dir.path().join("broken.tif"), "not a GeoTIFF").unwrap();
        write_dem(&dir.path().join(".hidden.tif"), 0.0, 1000.0);

        assert_eq!(
            find_dems(input_dir, &tiles).unwrap(),
            HashMap::from([
                (tile(0, 0), dir.path().join("b.TIF")),
                (tile(1000, 0), dir.path().join("a.tiff")),
            ])
        );

        // Two DEMs for one tile.
        write_dem(&dir.path().join("c.tif"), 0.0, 0.0);
        let error = find_dems(input_dir, &[tile(0, 0)]).unwrap_err();
        assert!(matches!(error, CassiniError::InvalidInput { .. }));
        let message = error.to_string();
        assert!(
            message.contains("b.TIF") && message.contains("c.tif"),
            "{message}"
        );
    }

    /// Two adjacent 10 m tiles in `dir/in`, the first with its ground at
    /// 100 m and the vegetation points of `test_tiles::POINTS`, the second
    /// with its ground at 110 m only, west to east, with their output in
    /// `dir/out`.
    fn two_tiles(dir: &Path) -> Vec<TileWithNeighbors> {
        let input = dir.join("in");
        std::fs::create_dir(&input).unwrap();
        write_tile(&input.join("a.laz"), 0.0, 100.0, &POINTS);
        write_tile(&input.join("b.laz"), SIDE, 110.0, &[]);
        let output = dir.join("out");
        get_tiles_with_neighbors(input.to_str().unwrap(), output.to_str().unwrap(), true).unwrap()
    }

    #[test]
    fn reads_laz_files_in_any_case_and_skips_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in");
        std::fs::create_dir(&input).unwrap();
        write_tile(&input.join("a.LAZ"), 0.0, 100.0, &[]);
        // The AppleDouble file macOS writes next to `b.laz` on some drives.
        std::fs::write(input.join("._b.laz"), "not a LAZ file").unwrap();
        std::fs::create_dir(input.join("folder.laz")).unwrap();
        let output = dir.path().join("out");

        let tiles =
            get_tiles_with_neighbors(input.to_str().unwrap(), output.to_str().unwrap(), false)
                .unwrap();

        let laz_paths: Vec<_> = tiles.iter().map(|tile| &tile.laz_path).collect();
        assert_eq!(laz_paths, [&input.join("a.LAZ")]);
    }

    #[test]
    fn lists_the_tiles_west_to_east_then_south_to_north_whatever_the_file_names() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in");
        std::fs::create_dir(&input).unwrap();
        for (name, offset) in [
            ("a", (2, 1)),
            ("b", (0, 1)),
            ("c", (1, 0)),
            ("d", (0, 0)),
            ("e", (2, 0)),
            ("f", (1, 1)),
        ] {
            write_tile_at(
                &input.join(format!("{name}.laz")),
                (offset.0 as f64 * SIDE, offset.1 as f64 * SIDE),
                100.0,
                &[],
            );
        }
        let output = dir.path().join("out");

        let tiles =
            get_tiles_with_neighbors(input.to_str().unwrap(), output.to_str().unwrap(), false)
                .unwrap();

        let corners: Vec<(i64, i64)> = tiles
            .iter()
            .map(|tile| (tile.tile.min_x, tile.tile.min_y))
            .collect();
        let (x, y) = (MIN_X as i64, MIN_Y as i64);
        let side = SIDE as i64;
        assert_eq!(
            corners,
            [
                (x, y),
                (x, y + side),
                (x + side, y),
                (x + side, y + side),
                (x + 2 * side, y),
                (x + 2 * side, y + side),
            ]
        );
    }

    /// Runs a batch without vectors on the tiles of `dir/in`, to `dir/out`.
    fn render_batch(dir: &Path, config: Config) -> PathBuf {
        let output = dir.join("out");
        std::fs::create_dir(&output).unwrap();
        batch(
            dir.join("in").to_str().unwrap(),
            output.to_str().unwrap(),
            2,
            false,
            VegetationLayers::Height,
            RenderOptions {
                skip_vector: true,
                skip_520: false,
                undergrowth_mode: UndergrowthMode::None,
                shapefiles_dir: None,
                ocad: None,
            },
            config,
        )
        .unwrap();
        output
    }

    /// Four points 20 m above the ground in each cell of `columns`, in
    /// distinct 0.5 m voxels.
    fn high_vegetation(columns: std::ops::Range<usize>) -> Vec<(f64, f64, f64, u8)> {
        columns
            .flat_map(|col| (0..SIDE as usize).map(move |row| (col as f64, row as f64)))
            .flat_map(|(col, row)| {
                [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                    .map(|(dx, dy)| (col + dx, row + dy, 120.0, 1))
            })
            .collect()
    }

    #[test]
    fn the_open_land_around_high_vegetation_reaches_across_tile_edges() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in");
        std::fs::create_dir(&input).unwrap();
        // High vegetation up to the east edge of the west tile, whose cells
        // next to the edge see the column of the east tile.
        write_tile(&input.join("a.laz"), 0.0, 101.0, &high_vegetation(6..10));
        write_tile(&input.join("b.laz"), SIDE, 101.0, &high_vegetation(0..1));
        let config = Config::default();

        let output = render_batch(dir.path(), config.clone());

        let merged = image::open(output.join("merged-map.png"))
            .unwrap()
            .into_rgba8();
        let tile_at = |min_x: f64| Tile {
            lidar_dir_path: PathBuf::new(),
            render_dir_path: PathBuf::new(),
            min_x: min_x as i64,
            min_y: MIN_Y as i64,
            max_x: (min_x + SIDE) as i64,
            max_y: (MIN_Y + SIDE) as i64,
        };
        let seam = RenderTransform::for_tile(&tile_at(MIN_X + SIDE), &config)
            .pixel_rect()
            .left
            - RenderTransform::for_tile(&tile_at(MIN_X), &config)
                .pixel_rect()
                .left;
        // The clearing of the high vegetation 0.5 m west of the edge reaches
        // about 1.6 m, or 3.8 pixels, east of it.
        let row = merged.height() / 2;
        for col in seam as u32..seam as u32 + 3 {
            assert_eq!(*merged.get_pixel(col, row), WHITE, "column {col}");
        }
    }

    #[test]
    fn the_merged_map_has_no_seam_between_the_tiles_at_any_dpi_resolution() {
        // Dense medium vegetation in every 1 m cell, on flat ground at 101 m,
        // away from any contour.
        let points: Vec<(f64, f64, f64, u8)> = (0..SIDE as usize)
            .flat_map(|col| (0..SIDE as usize).map(move |row| (col as f64, row as f64)))
            .flat_map(|(col, row)| {
                (0..36).map(move |i| {
                    let (dx, dy) = ((i % 6) as f64, (i / 6) as f64);
                    (col + 0.05 + 0.15 * dx, row + 0.05 + 0.15 * dy, 103.0, 1)
                })
            })
            .collect();
        // At 210 and 940 DPI, vegetation cells of fixed pixel size left the
        // last column or row of some of these tiles uncovered.
        for dpi in [210.0, 300.0, 600.0, 940.0] {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("in");
            std::fs::create_dir(&input).unwrap();
            for (col, row) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                write_tile_at(
                    &input.join(format!("{col}_{row}.laz")),
                    (col as f64 * SIDE, row as f64 * SIDE),
                    101.0,
                    &points,
                );
            }
            let output = render_batch(
                dir.path(),
                Config {
                    dpi_resolution: dpi,
                    ..Config::default()
                },
            );

            let merged = image::open(output.join("merged-map.png"))
                .unwrap()
                .into_rgba8();
            let uncovered = merged
                .enumerate_pixels()
                .filter(|(_, _, pixel)| pixel[3] != 255 || **pixel == YELLOW)
                .map(|(x, y, _)| (x, y))
                .collect::<Vec<_>>();
            assert!(
                uncovered.is_empty(),
                "{dpi} DPI, {}x{} px: {} pixels without vegetation, from {:?}",
                merged.width(),
                merged.height(),
                uncovered.len(),
                uncovered.first()
            );
        }
    }

    #[test]
    fn the_lidar_stage_takes_the_heights_above_the_dems_filled_with_the_neighbours() {
        let dir = tempfile::tempdir().unwrap();
        let tiles = two_tiles(dir.path());
        let [low, medium, high] = CELLS;
        for (layers, expected) in [
            (
                VegetationLayers::Height,
                counts_in([Some(low), Some(medium), Some(high)]),
            ),
            // The unclassified point is not low vegetation by class.
            (
                VegetationLayers::LidarClass,
                counts_in([None, Some(high), Some(medium)]),
            ),
        ] {
            run_lidar_stage(&tiles, 1, layers).unwrap();
            assert_eq!(check_previous_lidar_stage(&tiles).unwrap(), layers);
            assert_eq!(vegetation_counts(&tiles[0].tile.lidar_dir_path), expected);
            assert_eq!(
                vegetation_counts(&tiles[1].tile.lidar_dir_path),
                counts_in([None; 3])
            );

            // Each filled DEM holds the ground of both tiles.
            for tile in &tiles {
                let dem = read_dem_with_buffer(&tile.tile).unwrap();
                let elevation = |x: f64, y: f64| {
                    dem.get(
                        ((x - dem.min_x) / dem.resolution) as usize,
                        ((dem.max_y - y) / dem.resolution) as usize,
                    )
                };
                assert_eq!(elevation(MIN_X + 5.0, MIN_Y + 5.0), 100.0);
                assert_eq!(elevation(MIN_X + SIDE + 5.0, MIN_Y + 5.0), 110.0);
            }
        }
    }

    #[test]
    fn skip_lidar_needs_the_lidar_output_of_a_batch_since_0_19_for_every_tile() {
        let dir = tempfile::tempdir().unwrap();
        let tiles = two_tiles(dir.path());
        run_lidar_stage(&tiles, 2, VegetationLayers::Height).unwrap();
        let (a, b) = (&tiles[0].tile, &tiles[1].tile);
        let marker = |tile: &Tile| tile.lidar_dir_path.join(VEGETATION_LAYERS_FILE_NAME);
        let filled_dem = |tile: &Tile| tile.render_dir_path.join(DEM_WITH_BUFFER_FILE_NAME);
        let check = |expected: &[&str]| {
            let message = check_previous_lidar_stage(&tiles).unwrap_err().to_string();
            for expected in expected {
                assert!(message.contains(expected), "{message}");
            }
        };
        let b_dir = b.lidar_dir_path.display().to_string();

        // The LiDAR output of a batch before 0.19.0, whose render stage
        // wrote the filled DEM.
        std::fs::remove_file(marker(b)).unwrap();
        check(&["1 of the 2 tiles", &b_dir, "0.19.0"]);

        // The filled DEM of another tile, or none.
        std::fs::write(marker(b), "height").unwrap();
        std::fs::copy(filled_dem(a), filled_dem(b)).unwrap();
        check(&["1 of the 2 tiles", &b_dir, "is not the filled DEM of tile"]);
        std::fs::remove_file(filled_dem(b)).unwrap();
        check(&["1 of the 2 tiles", &b_dir, "does not exist"]);

        // Vegetation layers sorted differently.
        run_lidar_stage(&tiles, 2, VegetationLayers::Height).unwrap();
        std::fs::write(marker(a), "lidar-class").unwrap();
        check(&["1 tiles by height and 1 by LiDAR class"]);

        std::fs::write(marker(a), "height").unwrap();
        assert_eq!(
            check_previous_lidar_stage(&tiles).unwrap(),
            VegetationLayers::Height
        );
    }

    #[test]
    fn rejects_zero_threads_before_reading_input() {
        let error = batch(
            "this-directory-does-not-need-to-exist",
            "unused",
            0,
            false,
            VegetationLayers::Height,
            RenderOptions {
                skip_vector: false,
                skip_520: false,
                undergrowth_mode: UndergrowthMode::None,
                shapefiles_dir: None,
                ocad: None,
            },
            Config::default(),
        )
        .unwrap_err();

        assert!(matches!(error, CassiniError::InvalidInput { .. }));
    }
}
