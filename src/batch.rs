use crate::{
    lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file,
    merge::merge_maps,
    png::generate_png_from_dem_vegetation_density_tiff_images_and_vector_file,
    tile::{NeighborTiles, Tile, TileWithNeighbors},
};
use las::raw::Header;
use std::{
    collections::HashMap,
    fs::{read_dir, File},
    path::{Path, PathBuf},
    sync::Arc,
    thread::{sleep, spawn, JoinHandle},
    time::Duration,
};

pub fn batch(number_of_threads: usize, skip_lidar: bool) {
    println!("Batch mode");
    println!("Generating raw rasters for every tiles");

    let tiles = get_tiles_with_neighbors();
    let tiles_arc = Arc::new(tiles.clone()); // Wrap the tiles in an Arc

    if !skip_lidar {
        let tiles_chunks: Vec<Vec<TileWithNeighbors>> = tiles_arc
            .chunks(number_of_threads)
            .map(|chunk| chunk.to_vec())
            .collect();

        let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(number_of_threads);

        for chunk in tiles_chunks {
            let chunk = Arc::new(chunk);

            let spawned_thread = spawn(move || {
                for tile in chunk.iter() {
                    println!("{:?}", tile.tile.dir_path);

                    generate_dem_and_vegetation_density_tiff_images_from_laz_file(
                        &tile.tile.laz_path,
                        &tile.tile.dir_path,
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

    let tiles_chunks: Vec<Vec<TileWithNeighbors>> = tiles_arc
        .chunks(number_of_threads)
        .map(|chunk| chunk.to_vec())
        .collect();

    let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(number_of_threads);

    for chunk in tiles_chunks {
        let chunk = Arc::new(chunk);

        let spawned_thread = spawn(move || {
            for tile in chunk.iter() {
                println!("{:?}", tile.tile.dir_path);

                generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
                    tile.tile.clone(),
                    tile.neighbors.clone(),
                );
            }

            sleep(Duration::from_millis(1));
        });

        handles.push(spawned_thread);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    merge_maps(tiles);
}

pub fn get_tiles_with_neighbors() -> Vec<TileWithNeighbors> {
    let paths = read_dir("in").unwrap();
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
                        header.min_x as i64,
                        header.min_y as i64,
                        header.max_x as i64,
                        header.max_y as i64,
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
        let dir_path = Path::new("out").join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y));

        let tile = Tile {
            laz_path,
            dir_path,
            min_x,
            min_y,
            max_x,
            max_y,
        };

        tiles.push(TileWithNeighbors {
            tile,
            neighbors: NeighborTiles {
                top: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    min_x,
                    max_y,
                    max_x,
                    max_y + height,
                ),
                top_right: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    max_x,
                    max_y,
                    max_x + width,
                    max_y + height,
                ),
                right: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    max_x,
                    min_y,
                    max_x + width,
                    max_y,
                ),
                bottom_right: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    max_x,
                    min_y - height,
                    max_x + width,
                    min_y,
                ),
                bottom: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    min_x,
                    min_y - height,
                    max_x,
                    min_y,
                ),
                bottom_left: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    min_x - width,
                    min_y - height,
                    min_x,
                    min_y,
                ),
                left: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    min_x - width,
                    min_y,
                    min_x,
                    max_y,
                ),
                top_left: get_neighbor_tile_from_hash_map(
                    &tiles_map,
                    min_x - width,
                    max_y,
                    min_x,
                    max_y + height,
                ),
            },
        })
    }

    return tiles;
}

fn get_neighbor_tile_from_hash_map(
    tiles_map: &HashMap<(i64, i64, i64, i64), PathBuf>,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> Option<Tile> {
    return match tiles_map.get(&(min_x, min_y, max_x, max_y)) {
        Some(neighbor_path) => Some(Tile {
            laz_path: neighbor_path.clone(),
            dir_path: Path::new("out").join(format!("{}_{}_{}_{}", min_x, min_y, max_x, max_y)),
            min_x,
            min_y,
            max_x,
            max_y,
        }),
        None => None,
    };
}
