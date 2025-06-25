use image::{ImageBuffer, Rgb, Rgba};
use las::{
    point::Classification::{Ground, HighVegetation, LowVegetation, MediumVegetation},
    raw::Header,
    Reader,
};
use log::info;
use std::io::Write;
use std::io::{BufReader, Seek};
use std::path::PathBuf;
use std::time::Instant;
use std::{
    collections::HashSet,
    fs::{create_dir_all, File},
};

use crate::{constants::DEM_BLOCK_SIZE, helpers::remove_dir_content, terrain_rgba::encode_elevation_to_rgba};

pub fn generate_dem_and_vegetation_density_rasters_from_laz_file(
    laz_path: &PathBuf,
    output_dir_path: &PathBuf,
) {
    let start = Instant::now();

    let mut file = File::open(&laz_path).unwrap();

    let header = Header::read_from(&mut file).unwrap();

    let min_x = header.min_x.round() as usize;
    let min_y = header.min_y.round() as usize;
    let max_x = header.max_x.round() as usize;
    let max_y = header.max_y.round() as usize;
    let width = (max_x - min_x) * DEM_BLOCK_SIZE as usize;
    let height = (max_y - min_y) * DEM_BLOCK_SIZE as usize;

    file.seek(std::io::SeekFrom::Start(0)).unwrap();

    let mut reader =
        Reader::new(BufReader::with_capacity(1024 * 1024, file)).expect("Could not create reader");

    let mut elevation_matrix: Vec<f32> = vec![f32::NAN; width * height];
    let mut elevation_count_matrix: Vec<i32> = vec![0; width * height];

    let mut vegetation_set: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut low_vegetation_matrix: Vec<u8> = vec![0; width * height];
    let mut medium_vegetation_matrix: Vec<u8> = vec![0; width * height];
    let mut high_vegetation_matrix: Vec<u8> = vec![0; width * height];

    for ptu in reader.points() {
        let pt = ptu.unwrap();

        pt.x;
        pt.y;
        pt.z;
        pt.classification;

        let idx_x = (pt.x - min_x as f64).floor() as usize;
        let idx_y = (pt.y - min_y as f64).floor() as usize;

        match pt.classification {
            Ground => {
                let previous_elevation_value = elevation_matrix[idx_y * 1000 + idx_x];

                if previous_elevation_value.is_nan() {
                    elevation_matrix[idx_y * 1000 + idx_x] = pt.z as f32
                } else {
                    elevation_matrix[idx_y * 1000 + idx_x] += pt.z as f32;
                }

                elevation_count_matrix[idx_y * 1000 + idx_x] += 1;
            }
            LowVegetation | MediumVegetation | HighVegetation => {
                let cell_idx_x = ((pt.x - min_x as f64) * 2.).floor() as i32;
                let cell_idx_y = ((pt.y - min_y as f64) * 2.).floor() as i32;
                let cell_idx_z = (pt.z * 2.).floor() as i32;

                vegetation_set.insert((cell_idx_x, cell_idx_y, cell_idx_z));
            }
            _ => (),
        }
    }

    for index in 0..elevation_matrix.len() {
        let divider = elevation_count_matrix[index];

        if divider == 0 {
            continue;
        }

        elevation_matrix[index] = elevation_matrix[index] / divider as f32;
    }

    for (x, y, z) in vegetation_set {
        let idx_x = (x / 2) as usize;
        let idx_y = (y / 2) as usize;
        let elevation = z as f32 / 2.;

        let ground_elevation = elevation_matrix[idx_y * 1000 + idx_x];
        let height = elevation - ground_elevation;

        if height > 0. && height <= 0.5 {
            low_vegetation_matrix[idx_y * 1000 + idx_x] += 1;
        }

        if height > 0.5 && height <= 4. {
            medium_vegetation_matrix[idx_y * 1000 + idx_x] += 1;
        }

        if height > 4. && height <= 30. {
            high_vegetation_matrix[idx_y * 1000 + idx_x] += 1;
        }
    }

    let mut terrain_rgb_img = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(1000, 1000);

    for (index, elevation) in elevation_matrix.iter().enumerate() {
        let y = 1000 - index / 1000 - 1;
        let x = index % 1000;

        let rgba = encode_elevation_to_rgba(*elevation);
        terrain_rgb_img.put_pixel(x as u32, y as u32, Rgba(rgba));
    }

    // Cleaning up output directory to fix https://github.com/NicoRio42/cassini/issues/7
    if output_dir_path.exists() {
        remove_dir_content(output_dir_path).unwrap();
    } else {
        create_dir_all(&output_dir_path).expect("Could not create out dir");
    }

    let dem_image_path = output_dir_path.join("dem.png");
    terrain_rgb_img.save(dem_image_path).unwrap();

    let mut vegetation_img = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(1000, 1000);

    for (index, low_vegetation) in low_vegetation_matrix.iter().enumerate() {
        let y = 1000 - index / 1000 - 1;
        let x = index % 1000;
        let medium_vegetation = medium_vegetation_matrix[index];
        let high_vegetation = high_vegetation_matrix[index];

        vegetation_img.put_pixel(
            x as u32,
            y as u32,
            Rgb([
                (*low_vegetation).clamp(0, 255) as u8,
                medium_vegetation.clamp(0, 255) as u8,
                high_vegetation.clamp(0, 255) as u8,
            ]),
        );
    }

    let vegetation_image_path = output_dir_path.join("raw_vegetation.png");
    vegetation_img.save(vegetation_image_path).unwrap();

    // The existence of the extent.txt file is used as a proof of right execution of lidar pipeline by mapant-fr-worker

    let mut extent_file =
        File::create(&output_dir_path.join("extent.txt")).expect("Could not create extent.txt file");

    extent_file
        .write_all(format!("{}|{}|{}|{}", min_x, min_y, max_x, max_y).as_bytes())
        .expect("Could not write to the extent.txt file");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. PDAL pipeline executed in {:.1?}",
        min_x, min_y, max_x, max_y, duration
    );
}
