use image::{ImageBuffer, Rgb};
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

use crate::config::get_config;
use crate::helpers::remove_dir_content;

pub fn generate_dem_and_vegetation_density_tiff_images_from_laz_file(
    laz_path: &PathBuf,
    output_dir_path: &PathBuf,
) {
    let start = Instant::now();

    let mut file = File::open(&laz_path).unwrap();

    let header = Header::read_from(&mut file).unwrap();
    let config = get_config();

    let min_x = header.min_x.round() as i64;
    let min_y = header.min_y.round() as i64;
    let max_x = header.max_x.round() as i64;
    let max_y = header.max_y.round() as i64;

    file.seek(std::io::SeekFrom::Start(0)).unwrap();

    let mut reader =
        Reader::new(BufReader::with_capacity(1024 * 1024, file)).expect("Could not create reader");

    let mut elevation_matrix: Vec<f64> = vec![0.0; 1000 * 1000];
    let mut count_height_matrix: Vec<i32> = vec![0; 1000 * 1000];

    let mut vegetation_set: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut low_vegetation_matrix: Vec<u8> = vec![0; 1000 * 1000];
    let mut medium_vegetation_matrix: Vec<u8> = vec![0; 1000 * 1000];
    let mut high_vegetation_matrix: Vec<u8> = vec![0; 1000 * 1000];

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
                elevation_matrix[idx_y * 1000 + idx_x] += pt.z;
                count_height_matrix[idx_y * 1000 + idx_x] += 1;
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
        let divider = count_height_matrix[index];

        if divider == 0 {
            continue;
        }

        elevation_matrix[index] = elevation_matrix[index] / count_height_matrix[index] as f64;
    }

    for (x, y, z) in vegetation_set {
        let idx_x = (x / 2) as usize;
        let idx_y = (y / 2) as usize;
        let elevation = z as f64 / 2.;

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

    let mut terrain_rgb_img = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(1000, 1000);

    for (index, elevation) in elevation_matrix.iter().enumerate() {
        let y = 1000 - index / 1000 - 1;
        let x = index % 1000;

        let rgb = encode_elevation_to_rgb(*elevation as f32);
        terrain_rgb_img.put_pixel(x as u32, y as u32, Rgb(rgb));
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

    let vegetation_image_path = output_dir_path.join("vegetation.png");
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

fn encode_elevation_to_rgb(elevation: f32) -> [u8; 3] {
    // Mapbox terrain-rgb expects elevation in meters.
    // Elevations below -10000 are clamped to -10000.
    let e = (elevation.max(-10000.0) + 10000.0).round() as u32;
    let r = ((e >> 16) & 0xFF) as u8;
    let g = ((e >> 8) & 0xFF) as u8;
    let b = (e & 0xFF) as u8;
    [r, g, b]
}
