use crate::{
    config::Config,
    download::download_osm_file,
    error::{CassiniError, Result, ResultContext, Stage, TileId},
    helpers::remove_if_exists,
    lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file,
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
use las::raw::Header;
use log::info;
use std::{
    collections::HashMap,
    fs::{read_dir, File},
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

    let tiles = get_tiles_with_neighbors(input_dir, output_dir)?;
    if tiles.is_empty() {
        return Err(CassiniError::InvalidInput {
            message: format!("no LAZ files were found in `{input_dir}`"),
        });
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
                            &tile.tile.lidar_dir_path,
                        )
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

                let mut file = File::open(&path)
                    .context(format!("could not open LiDAR file `{}`", path.display()))?;
                let header = Header::read_from(&mut file)
                    .context(format!("could not read LiDAR header `{}`", path.display()))?;

                let extent = (
                    header.min_x.round() as i64,
                    header.min_y.round() as i64,
                    header.max_x.round() as i64,
                    header.max_y.round() as i64,
                );
                if extent.0 >= extent.2 || extent.1 >= extent.3 {
                    return Err(CassiniError::InvalidInput {
                        message: format!("LiDAR file `{}` has an invalid extent", path.display()),
                    });
                }

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
            tile,
            neighbors,
        })
    }

    Ok(tiles)
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
    use super::batch;
    use crate::{
        config::Config, error::CassiniError, render::RenderOptions, vegetation::UndergrowthMode,
    };

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
