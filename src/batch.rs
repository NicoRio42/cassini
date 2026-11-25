use crate::{
    config::Config,
    dem::{create_dem_with_buffer, DEM_WITH_BUFFER_FILE_NAME},
    download::download_osm_file,
    error::{CassiniError, Result, ResultContext, Stage, TileId},
    grid::GridExtent,
    helpers::remove_if_exists,
    lidar::{
        generate_dem_tiff_image_from_laz_file,
        generate_vegetation_density_tiff_images_from_laz_file, read_tile_id, VegetationLayers,
    },
    merge::merge_maps,
    ocad_export::{
        get_batch_georeferencing, prepare_tile_for_merge, write_merged_ocd_file,
        MERGED_OCD_FILE_NAME,
    },
    ocd::{Georeferencing, OcdTemplate},
    render::{cleanup_render_step_files, render_tile_map, DemWithBuffer, RenderOptions},
    tile::{Tile, TileWithNeighbors},
};
use log::{info, warn};
use std::{
    collections::HashMap,
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

    if skip_lidar {
        check_dems_with_buffer(&tiles)?;
        cleanup_render_step_files(&tiles, output_dir)?;
    } else {
        // In `LidarClass` mode, the vegetation densities are computed in the
        // same pass over the points as the DEM.
        run_stage(&tiles, chunk_size, Stage::Lidar, move |tile| {
            info!(
                "Tile min_x={} min_y={} max_x={} max_y={}. Generating raw rasters",
                tile.tile.min_x, tile.tile.min_y, tile.tile.max_x, tile.tile.max_y
            );

            generate_dem_tiff_image_from_laz_file(
                &tile.laz_path,
                tile.dem_path.as_deref(),
                &tile.tile.lidar_dir_path,
                vegetation_layers,
            )
            .map(|_| ())
        })?;

        // Every DEM is written, so each tile's can be mosaicked with its
        // neighbours'. The render stage reads `dem_with_buffer.tif`, so
        // that the vegetation heights and the contours and cliffs come from
        // the same filled DEM.
        run_stage(&tiles, chunk_size, Stage::Lidar, move |tile| {
            let dem_with_buffer = create_dem_with_buffer(&tile.tile, &tile.neighbors)?;
            if vegetation_layers == VegetationLayers::Height {
                generate_vegetation_density_tiff_images_from_laz_file(
                    &tile.laz_path,
                    &TileId::from(&tile.tile),
                    &dem_with_buffer,
                    &tile.tile.lidar_dir_path,
                )?;
            }
            Ok(())
        })?;
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
    let seam_objects = run_stage(&tiles, chunk_size, Stage::Render, move |tile| {
        info!(
            "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map",
            tile.tile.min_x, tile.tile.min_y, tile.tile.max_x, tile.tile.max_y
        );

        let features = render_tile_map(
            tile.tile.clone(),
            tile.neighbors.clone(),
            DemWithBuffer::Existing,
            &options,
            &render_config,
        )?;

        render_ocad_export
            .as_ref()
            .zip(features)
            .map(|(export, features)| {
                prepare_tile_for_merge(&tile.tile, &export.georeferencing, &features)
            })
            .transpose()
    })?;

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

/// Fails unless every tile has the `dem_with_buffer.tif` that `--skip-lidar`
/// renders from, before anything is deleted.
fn check_dems_with_buffer(tiles: &[TileWithNeighbors]) -> Result<()> {
    let missing: Vec<String> = tiles
        .iter()
        .map(|tile| tile.tile.render_dir_path.join(DEM_WITH_BUFFER_FILE_NAME))
        .filter(|path| !path.exists())
        .map(|path| format!("`{}`", path.display()))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(CassiniError::InvalidInput {
        message: format!(
            "`--skip-lidar` renders from the filled DEMs the LiDAR stage wrote, and {} of the {} \
             tiles have none: {}. Run the batch once without `--skip-lidar`",
            missing.len(),
            tiles.len(),
            missing.join(", ")
        ),
    })
}

/// Runs `process` on each of `tiles`, on worker threads taking `chunk_size`
/// tiles each, and returns the results in tile order, or the failures of
/// every tile that failed.
fn run_stage<T, F>(
    tiles: &[TileWithNeighbors],
    chunk_size: usize,
    stage: Stage,
    process: F,
) -> Result<Vec<T>>
where
    T: Send + 'static,
    F: Fn(&TileWithNeighbors) -> Result<T> + Send + Sync + 'static,
{
    let process = Arc::new(process);
    let handles: Vec<Worker<T>> = tiles
        .chunks(chunk_size)
        .map(|chunk| {
            let chunk = chunk.to_vec();
            let first_tile = chunk.first().map(|tile| TileId::from(&tile.tile));
            let process = process.clone();
            let spawned_thread = spawn(move || chunk.iter().map(|tile| process(tile)).collect());
            (first_tile, spawned_thread)
        })
        .collect();

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
    let mut tiles_map = HashMap::<(i64, i64, i64, i64), PathBuf>::new();

    for dir_entry in paths {
        let path = dir_entry
            .context(format!("could not read an entry in `{input_dir}`"))?
            .path();

        match path.extension() {
            Some(extension) => {
                if !path.is_file() || extension != "laz" {
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
            None => {}
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
        .filter_map(|x| x)
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
        let is_hidden = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.'));
        let is_tiff = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("tif") || extension.eq_ignore_ascii_case("tiff")
            });
        if is_tiff && !is_hidden && path.is_file() {
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
    tiles_map: &HashMap<(i64, i64, i64, i64), PathBuf>,
    output_dir: &str,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> Option<PathBuf> {
    return match tiles_map.get(&(min_x, min_y, max_x, max_y)) {
        Some(_) => {
            Some(Path::new(output_dir).join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)))
        }
        None => None,
    };
}

#[cfg(test)]
mod tests {
    use super::{batch, check_dems_with_buffer, find_dems};
    use crate::{
        config::Config,
        error::{CassiniError, TileId},
        grid::{Compression, Grid},
        lidar::VegetationLayers,
        render::RenderOptions,
        tile::{Tile, TileWithNeighbors},
        vegetation::UndergrowthMode,
    };
    use std::{collections::HashMap, path::Path};

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

    #[test]
    fn skip_lidar_needs_the_dem_with_buffer_of_every_tile() {
        let dir = tempfile::tempdir().unwrap();
        let tiles: Vec<TileWithNeighbors> = ["a", "b"]
            .into_iter()
            .map(|name| {
                let tile_dir = dir.path().join(name);
                std::fs::create_dir(&tile_dir).unwrap();
                TileWithNeighbors {
                    laz_path: tile_dir.join("tile.laz"),
                    dem_path: None,
                    tile: Tile {
                        lidar_dir_path: tile_dir.clone(),
                        render_dir_path: tile_dir,
                        min_x: 0,
                        min_y: 0,
                        max_x: 1000,
                        max_y: 1000,
                    },
                    neighbors: vec![],
                }
            })
            .collect();
        std::fs::write(dir.path().join("a").join("dem_with_buffer.tif"), "").unwrap();

        let message = check_dems_with_buffer(&tiles).unwrap_err().to_string();
        assert!(message.contains("1 of the 2 tiles"), "{message}");
        assert!(
            message.contains(&dir.path().join("b").display().to_string()),
            "{message}"
        );

        std::fs::write(dir.path().join("b").join("dem_with_buffer.tif"), "").unwrap();
        assert!(check_dems_with_buffer(&tiles).is_ok());
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
