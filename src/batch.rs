use crate::{
    lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file,
    merge::merge_maps,
    render::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file,
    tile::{Tile, TileWithNeighbors},
};
use las::raw::Header;
use log::info;
use std::{
    collections::HashMap,
    fs::{read_dir, File},
    path::{Path, PathBuf},
    sync::Arc,
    thread::{sleep, spawn, JoinHandle},
    time::Duration,
};

pub fn batch(
    input_dir: &str,
    output_dir: &str,
    number_of_threads: usize,
    skip_lidar: bool,
    skip_vector: bool,
    skip_520: bool,
) {
    let tiles = get_tiles_with_neighbors(input_dir, output_dir);
    let tiles_arc = Arc::new(tiles.clone());
    let chunk_size = (tiles.len() + number_of_threads - 1) / number_of_threads;

    if !skip_lidar {
        let tiles_chunks: Vec<Vec<TileWithNeighbors>> =
            tiles_arc.chunks(chunk_size).map(|chunk| chunk.to_vec()).collect();

        let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(number_of_threads);

        for chunk in tiles_chunks {
            let chunk = Arc::new(chunk);

            let spawned_thread = spawn(move || {
                for tile in chunk.iter() {
                    info!(
                        "Tile min_x={} min_y={} max_x={} max_y={}. Generating raw rasters",
                        tile.tile.min_x, tile.tile.min_y, tile.tile.max_x, tile.tile.max_y
                    );

                    generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                        &tile.laz_path,
                        &tile.tile.lidar_dir_path,
                    );
                }

                sleep(Duration::from_millis(1));
            });

            handles.push(spawned_thread);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    let tiles_chunks: Vec<Vec<TileWithNeighbors>> =
        tiles_arc.chunks(chunk_size).map(|chunk| chunk.to_vec()).collect();

    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(number_of_threads);

    for chunk in tiles_chunks {
        let chunk = Arc::new(chunk);

        let spawned_thread = spawn(move || {
            for tile in chunk.iter() {
                info!(
                    "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map",
                    tile.tile.min_x, tile.tile.min_y, tile.tile.max_x, tile.tile.max_y
                );

                generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
                    tile.tile.clone(),
                    tile.neighbors.clone(),
                    skip_vector,
                    skip_520,
                );
            }

            sleep(Duration::from_millis(1));
        });

        handles.push(spawned_thread);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    merge_maps(output_dir, tiles);
}

pub fn get_tiles_with_neighbors(input_dir: &str, output_dir: &str) -> Vec<TileWithNeighbors> {
    let paths = read_dir(input_dir).expect(&format!("There is no {} directory.", input_dir));
    let mut tiles: Vec<TileWithNeighbors> = vec![];
    let mut tiles_map = HashMap::<(i64, i64, i64, i64), PathBuf>::new();

    for dir_entry in paths {
        let path = dir_entry.expect("Problem reading directory entry").path();

        match path.extension() {
            Some(extension) => {
                if !path.is_file() || extension != "laz" {
                    continue;
                }

                let mut file = File::open(&path).expect("Cound not open laz file");
                let header = Header::read_from(&mut file).unwrap();

                tiles_map.insert(
                    (
                        header.min_x.round() as i64,
                        header.min_y.round() as i64,
                        header.max_x.round() as i64,
                        header.max_y.round() as i64,
                    ),
                    path,
                );
            }
            None => {}
        }
    }

    for ((min_x, min_y, max_x, max_y), laz_path) in tiles_map.clone().into_iter() {
        let width = max_x - min_x;
        let height = max_y - min_y;

        let dir_path = Path::new(output_dir).join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y));

        let tile = Tile {
            lidar_dir_path: dir_path.to_path_buf(),
            render_dir_path: dir_path,
            min_x,
            min_y,
            max_x,
            max_y,
        };

        let neighbors: Vec<PathBuf> = vec![
            get_neighbor_tile_from_hash_map(&tiles_map, output_dir, min_x, max_y, max_x, max_y + height),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                max_x,
                max_y,
                max_x + width,
                max_y + height,
            ),
            get_neighbor_tile_from_hash_map(&tiles_map, output_dir, max_x, min_y, max_x + width, max_y),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                max_x,
                min_y - height,
                max_x + width,
                min_y,
            ),
            get_neighbor_tile_from_hash_map(&tiles_map, output_dir, min_x, min_y - height, max_x, min_y),
            get_neighbor_tile_from_hash_map(
                &tiles_map,
                output_dir,
                min_x - width,
                min_y - height,
                min_x,
                min_y,
            ),
            get_neighbor_tile_from_hash_map(&tiles_map, output_dir, min_x - width, min_y, min_x, max_y),
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

    return tiles;
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
        Some(_) => Some(Path::new(output_dir).join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y))),
        None => None,
    };
}
