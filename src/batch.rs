use crate::{
    config::Config,
    download::download_osm_file,
    error::{CassiniError, Result, ResultContext, Stage, TileId},
    grid::GridExtent,
    helpers::remove_if_exists,
    lidar::{generate_dem_and_vegetation_density_tiff_images_from_laz_file, read_tile_id},
    merge::merge_maps,
    ocad_export::{
        get_batch_georeferencing, prepare_tile_for_merge, write_merged_ocd_file,
        MERGED_OCD_FILE_NAME,
    },
    ocad_merge::TileObjects,
    ocd::{Georeferencing, OcdTemplate},
    render::{cleanup_render_step_files, render_tile_map, RenderOptions},
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
    options: RenderOptions,
    config: Config,
) -> Result<()> {
    if number_of_threads == 0 {
        return Err(CassiniError::InvalidInput {
            message: "the number of batch threads must be greater than zero".to_owned(),
        });
    }

    let mut tiles = get_tiles_with_neighbors(input_dir, output_dir)?;
    if tiles.is_empty() {
        return Err(CassiniError::InvalidInput {
            message: format!("no LAZ files were found in `{input_dir}`"),
        });
    }
    // With `--skip-lidar`, the DEMs are the `dem.tif` files of the previous run.
    if !skip_lidar {
        assign_dems(input_dir, &mut tiles)?;
    }

    let tiles_arc = Arc::new(tiles.clone());
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
        cleanup_render_step_files(&tiles, output_dir)?;
    } else {
        let tiles_chunks: Vec<Vec<TileWithNeighbors>> = tiles_arc
            .chunks(chunk_size)
            .map(|chunk| chunk.to_vec())
            .collect();

        let mut handles: Vec<Worker<()>> = Vec::with_capacity(number_of_threads);

        for chunk in tiles_chunks {
            let first_tile = chunk.first().map(|tile| TileId::from(&tile.tile));
            let chunk = Arc::new(chunk);

            let spawned_thread = spawn(move || {
                chunk
                    .iter()
                    .map(|tile| {
                        info!(
                            "Tile min_x={} min_y={} max_x={} max_y={}. Generating raw rasters",
                            tile.tile.min_x, tile.tile.min_y, tile.tile.max_x, tile.tile.max_y
                        );

                        generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                            &tile.laz_path,
                            tile.dem_path.as_deref(),
                            &tile.tile.lidar_dir_path,
                        )
                        .map(|_| ())
                    })
                    .collect()
            });

            handles.push((first_tile, spawned_thread));
        }

        let (_, failures) = collect_worker_results(handles, Stage::Lidar);
        if !failures.is_empty() {
            return Err(CassiniError::Batch {
                failure_count: failures.len(),
                failures,
            });
        }
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

    let tiles_chunks: Vec<Vec<TileWithNeighbors>> = tiles_arc
        .chunks(chunk_size)
        .map(|chunk| chunk.to_vec())
        .collect();

    let mut handles: Vec<Worker<Option<TileObjects>>> = Vec::with_capacity(number_of_threads);

    for chunk in tiles_chunks {
        let first_tile = chunk.first().map(|tile| TileId::from(&tile.tile));
        let chunk = Arc::new(chunk);

        let cloned_options = options.clone();
        let cloned_config = config.clone();
        let cloned_ocad_export = ocad_export.clone();

        let spawned_thread = spawn(move || {
            chunk
                .iter()
                .map(|tile| {
                    info!(
                        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map",
                        tile.tile.min_x, tile.tile.min_y, tile.tile.max_x, tile.tile.max_y
                    );

                    let features = render_tile_map(
                        tile.tile.clone(),
                        tile.neighbors.clone(),
                        &cloned_options,
                        &cloned_config,
                    )?;

                    cloned_ocad_export
                        .as_ref()
                        .zip(features)
                        .map(|(export, features)| {
                            prepare_tile_for_merge(&tile.tile, &export.georeferencing, &features)
                        })
                        .transpose()
                })
                .collect()
        });

        handles.push((first_tile, spawned_thread));
    }

    let (seam_objects, failures) = collect_worker_results(handles, Stage::Render);
    if !failures.is_empty() {
        return Err(CassiniError::Batch {
            failure_count: failures.len(),
            failures,
        });
    }

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

pub fn get_tiles_with_neighbors(
    input_dir: &str,
    output_dir: &str,
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
            dem_path: None,
            tile,
            neighbors,
        })
    }

    Ok(tiles)
}

/// Sets the DEM of each tile to the GeoTIFF file of `input_dir` whose extent's
/// centre is in the tile. Matching by extent rather than by file name lets
/// renamed files work, and IGN's DEM tiles, offset by a quarter metre, match
/// their LiDAR tiles.
fn assign_dems(input_dir: &str, tiles: &mut [TileWithNeighbors]) -> Result<()> {
    let mut dem_paths = vec![];
    for dir_entry in
        read_dir(input_dir).context(format!("could not read input directory `{input_dir}`"))?
    {
        let path = dir_entry
            .context(format!("could not read an entry in `{input_dir}`"))?
            .path();
        let is_tiff = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("tif") || extension.eq_ignore_ascii_case("tiff")
            });
        if is_tiff && path.is_file() {
            dem_paths.push(path);
        }
    }
    // For the same errors and warnings whatever the directory order.
    dem_paths.sort();

    for dem_path in dem_paths {
        let extent = GridExtent::read_geotiff(&dem_path)?;
        let centre_x = (extent.min_x + extent.max_x()) / 2.0;
        let centre_y = (extent.min_y() + extent.max_y) / 2.0;
        let Some(tile) = tiles.iter_mut().find(|tile| {
            (tile.tile.min_x as f64..tile.tile.max_x as f64).contains(&centre_x)
                && (tile.tile.min_y as f64..tile.tile.max_y as f64).contains(&centre_y)
        }) else {
            warn!(
                "DEM `{}` is centred on ({centre_x}, {centre_y}), in no LiDAR tile, ignoring it",
                dem_path.display()
            );
            continue;
        };
        if let Some(previous) = &tile.dem_path {
            return Err(CassiniError::InvalidInput {
                message: format!(
                    "DEMs `{}` and `{}` are both centred in tile {}",
                    previous.display(),
                    dem_path.display(),
                    TileId::from(&tile.tile)
                ),
            });
        }
        tile.dem_path = Some(dem_path);
    }

    let mut without_dem: Vec<TileId> = tiles
        .iter()
        .filter(|tile| tile.dem_path.is_none())
        .map(|tile| TileId::from(&tile.tile))
        .collect();
    if without_dem.len() < tiles.len() {
        info!(
            "Using the DEMs of `{input_dir}` for {} of the {} tiles",
            tiles.len() - without_dem.len(),
            tiles.len()
        );
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
                    .map(TileId::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    Ok(())
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
    use super::{assign_dems, batch};
    use crate::{
        config::Config,
        error::CassiniError,
        grid::{Compression, Grid},
        render::RenderOptions,
        tile::{Tile, TileWithNeighbors},
        vegetation::UndergrowthMode,
    };
    use std::path::{Path, PathBuf};

    /// The 1 km tile with its south-west corner at (`min_x`, `min_y`).
    fn tile(min_x: i64, min_y: i64) -> TileWithNeighbors {
        TileWithNeighbors {
            laz_path: PathBuf::from(format!("{min_x}_{min_y}.laz")),
            dem_path: None,
            tile: Tile {
                lidar_dir_path: PathBuf::new(),
                render_dir_path: PathBuf::new(),
                min_x,
                min_y,
                max_x: min_x + 1000,
                max_y: min_y + 1000,
            },
            neighbors: vec![],
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
    fn assigns_dems_to_the_tiles_containing_their_centre() {
        let dir = tempfile::tempdir().unwrap();
        let input_dir = dir.path().to_str().unwrap();
        let mut tiles = vec![tile(0, 0), tile(1000, 0), tile(0, 1000)];
        // Names unrelated to the tiles, and upper case extensions.
        write_dem(&dir.path().join("b.TIF"), 0.0, 0.0);
        write_dem(&dir.path().join("a.tiff"), 1000.0, 0.0);
        // A DEM matching no tile, and files that are not DEMs.
        write_dem(&dir.path().join("far.tif"), 5000.0, 0.0);
        std::fs::write(dir.path().join("notes.txt"), "not a DEM").unwrap();
        std::fs::create_dir(dir.path().join("folder.tif")).unwrap();

        assign_dems(input_dir, &mut tiles).unwrap();
        let dem_paths: Vec<_> = tiles.iter().map(|tile| tile.dem_path.clone()).collect();
        assert_eq!(
            dem_paths,
            [
                Some(dir.path().join("b.TIF")),
                Some(dir.path().join("a.tiff")),
                None
            ]
        );

        // Two DEMs for one tile.
        write_dem(&dir.path().join("c.tif"), 0.0, 0.0);
        let error = assign_dems(input_dir, &mut [tile(0, 0)]).unwrap_err();
        assert!(matches!(error, CassiniError::InvalidInput { .. }));
        let message = error.to_string();
        assert!(
            message.contains("b.TIF") && message.contains("c.tif"),
            "{message}"
        );
        std::fs::remove_file(dir.path().join("c.tif")).unwrap();

        // A file that is not a GeoTIFF, or not in Lambert-93.
        std::fs::write(dir.path().join("broken.tif"), "not a GeoTIFF").unwrap();
        let error = assign_dems(input_dir, &mut [tile(0, 0)]).unwrap_err();
        assert!(error.to_string().contains("broken.tif"), "{error}");
    }

    #[test]
    fn rejects_zero_threads_before_reading_input() {
        let error = batch(
            "this-directory-does-not-need-to-exist",
            "unused",
            0,
            false,
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
