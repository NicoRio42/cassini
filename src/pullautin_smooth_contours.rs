use core::f64;
use log::info;
use shapefile::dbase::{FieldValue, Record};
use shapefile::record::polyline::GenericPolyline;
use shapefile::{Point, Polyline, Reader};
use std::fs::{create_dir_all, File};
use std::io::BufReader;
use std::time::Instant;
use tiff::decoder::{Decoder, DecodingResult};

use crate::constants::BUFFER;
use crate::tile::Tile;

pub fn pullautin_smooth_contours(tile: &Tile, avg_alt: &Vec<Vec<f64>>) -> Vec<(Vec<f64>, Vec<f64>, f64)> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Smoothing contours",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let smoothing: f64 = 0.7;
    let curviness: f64 = 1.1;

    let size: f64 = 2.0;
    let xstart: f64 = (tile.min_x - BUFFER as i64) as f64;
    let ystart: f64 = (tile.min_y - BUFFER as i64) as f64;
    let xmax: u64 = ((tile.max_x + BUFFER as i64 - xstart as i64) as f64 / 2.).ceil() as u64;
    let ymax: u64 = ((tile.max_y + BUFFER as i64 - ystart as i64) as f64 / 2.).ceil() as u64;

    let mut steepness = vec![vec![f64::NAN; (ymax + 2) as usize]; (xmax + 2) as usize];
    let mut smoothed_contours: Vec<(Vec<f64>, Vec<f64>, f64)> = vec![];

    // Computing a basic steepness matrix
    for i in 1..xmax {
        for j in 1..ymax {
            let mut low: f64 = f64::MAX;
            let mut high: f64 = f64::MIN;
            for ii in i - 1..i + 2 {
                for jj in j - 1..j + 2 {
                    let tmp = avg_alt[ii as usize][jj as usize];

                    if tmp < low {
                        low = tmp;
                    }
                    if tmp > high {
                        high = tmp;
                    }
                }
            }

            steepness[i as usize][j as usize] = high - low;
        }
    }

    let contours_polylines_path = tile.render_dir_path.join("contours-raw").join("contours-raw.shp");

    let mut contours_polylines_reader: shapefile::Reader<BufReader<File>, BufReader<File>> =
        Reader::from_path(&contours_polylines_path).unwrap();

    let contours_polylines_reader_for_table_info: shapefile::Reader<BufReader<File>, BufReader<File>> =
        Reader::from_path(&contours_polylines_path).unwrap();

    let table_info = contours_polylines_reader_for_table_info.into_table_info();

    let contours_dir = tile.render_dir_path.join("contours");
    create_dir_all(&contours_dir).expect("Could not create contours dir");

    let mut writer =
        shapefile::Writer::from_path_with_info(contours_dir.join("contours.shp"), table_info).unwrap();

    for shape_record in contours_polylines_reader.iter_shapes_and_records_as::<Polyline, Record>() {
        let (line, record) = shape_record.unwrap();
        let mut x_array = Vec::<f64>::new();
        let mut y_array = Vec::<f64>::new();

        // Assuming line.parts().len() == 1
        for part in line.parts() {
            for point in part {
                x_array.push(point.x);
                y_array.push(point.y);
            }
        }

        let elevation = match record.get("elev") {
            Some(FieldValue::Numeric(Some(x))) => x,
            Some(_) => &f64::NAN,
            None => panic!("Field 'elev' is not within polygon-dataset"),
        };

        let mut el_x_len = x_array.len();

        if el_x_len < 15 {
            continue;
        }

        let height = *elevation;

        if el_x_len > 101 {
            let mut newx: Vec<f64> = vec![];
            let mut newy: Vec<f64> = vec![];
            let mut xpre = x_array[0];
            let mut ypre = y_array[0];

            newx.push(x_array[0]);
            newy.push(y_array[0]);

            for k in 1..(el_x_len - 1) {
                let xx = ((x_array[k] - xstart) / size + 0.5).floor() as usize;
                let yy = ((y_array[k] - ystart) / size + 0.5).floor() as usize;

                let ss = steepness[xx][yy];
                if ss.is_nan() || ss < 0.5 {
                    if ((xpre - x_array[k]).powi(2) + (ypre - y_array[k]).powi(2)).sqrt() >= 4.0 {
                        newx.push(x_array[k]);
                        newy.push(y_array[k]);
                        xpre = x_array[k];
                        ypre = y_array[k];
                    }
                } else {
                    newx.push(x_array[k]);
                    newy.push(y_array[k]);
                    xpre = x_array[k];
                    ypre = y_array[k];
                }
            }
            newx.push(x_array[el_x_len - 1]);
            newy.push(y_array[el_x_len - 1]);

            x_array.clear();
            x_array.append(&mut newx);
            y_array.clear();
            y_array.append(&mut newy);
            el_x_len = x_array.len();
        }
        // Smoothing
        let mut dx: Vec<f64> = vec![f64::NAN; el_x_len];
        let mut dy: Vec<f64> = vec![f64::NAN; el_x_len];

        for k in 2..(el_x_len - 3) {
            dx[k] = (x_array[k - 2]
                + x_array[k - 1]
                + x_array[k]
                + x_array[k + 1]
                + x_array[k + 2]
                + x_array[k + 3])
                / 6.0;
            dy[k] = (y_array[k - 2]
                + y_array[k - 1]
                + y_array[k]
                + y_array[k + 1]
                + y_array[k + 2]
                + y_array[k + 3])
                / 6.0;
        }

        let mut xa: Vec<f64> = vec![f64::NAN; el_x_len];
        let mut ya: Vec<f64> = vec![f64::NAN; el_x_len];
        for k in 1..(el_x_len - 1) {
            xa[k] = (x_array[k - 1] + x_array[k] / (0.01 + smoothing) + x_array[k + 1])
                / (2.0 + 1.0 / (0.01 + smoothing));
            ya[k] = (y_array[k - 1] + y_array[k] / (0.01 + smoothing) + y_array[k + 1])
                / (2.0 + 1.0 / (0.01 + smoothing));
        }

        if x_array.first() == x_array.last() && y_array.first() == y_array.last() {
            let vx = (x_array[1] + x_array[0] / (0.01 + smoothing) + x_array[el_x_len - 2])
                / (2.0 + 1.0 / (0.01 + smoothing));
            let vy = (y_array[1] + y_array[0] / (0.01 + smoothing) + y_array[el_x_len - 2])
                / (2.0 + 1.0 / (0.01 + smoothing));
            xa[0] = vx;
            ya[0] = vy;
            xa[el_x_len - 1] = vx;
            ya[el_x_len - 1] = vy;
        } else {
            xa[0] = x_array[0];
            ya[0] = y_array[0];
            xa[el_x_len - 1] = x_array[el_x_len - 1];
            ya[el_x_len - 1] = y_array[el_x_len - 1];
        }
        for k in 1..(el_x_len - 1) {
            x_array[k] =
                (xa[k - 1] + xa[k] / (0.01 + smoothing) + xa[k + 1]) / (2.0 + 1.0 / (0.01 + smoothing));
            y_array[k] =
                (ya[k - 1] + ya[k] / (0.01 + smoothing) + ya[k + 1]) / (2.0 + 1.0 / (0.01 + smoothing));
        }
        if xa.first() == xa.last() && ya.first() == ya.last() {
            let vx =
                (xa[1] + xa[0] / (0.01 + smoothing) + xa[el_x_len - 2]) / (2.0 + 1.0 / (0.01 + smoothing));
            let vy =
                (ya[1] + ya[0] / (0.01 + smoothing) + ya[el_x_len - 2]) / (2.0 + 1.0 / (0.01 + smoothing));
            x_array[0] = vx;
            y_array[0] = vy;
            x_array[el_x_len - 1] = vx;
            y_array[el_x_len - 1] = vy;
        } else {
            x_array[0] = xa[0];
            y_array[0] = ya[0];
            x_array[el_x_len - 1] = xa[el_x_len - 1];
            y_array[el_x_len - 1] = ya[el_x_len - 1];
        }

        for k in 1..(el_x_len - 1) {
            xa[k] = (x_array[k - 1] + x_array[k] / (0.01 + smoothing) + x_array[k + 1])
                / (2.0 + 1.0 / (0.01 + smoothing));
            ya[k] = (y_array[k - 1] + y_array[k] / (0.01 + smoothing) + y_array[k + 1])
                / (2.0 + 1.0 / (0.01 + smoothing));
        }

        if x_array.first() == x_array.last() && y_array.first() == y_array.last() {
            let vx = (x_array[1] + x_array[0] / (0.01 + smoothing) + x_array[el_x_len - 2])
                / (2.0 + 1.0 / (0.01 + smoothing));
            let vy = (y_array[1] + y_array[0] / (0.01 + smoothing) + y_array[el_x_len - 2])
                / (2.0 + 1.0 / (0.01 + smoothing));
            xa[0] = vx;
            ya[0] = vy;
            xa[el_x_len - 1] = vx;
            ya[el_x_len - 1] = vy;
        } else {
            xa[0] = x_array[0];
            ya[0] = y_array[0];
            xa[el_x_len - 1] = x_array[el_x_len - 1];
            ya[el_x_len - 1] = y_array[el_x_len - 1];
        }
        for k in 0..el_x_len {
            x_array[k] = xa[k];
            y_array[k] = ya[k];
        }

        let mut dx2: Vec<f64> = vec![f64::NAN; el_x_len];
        let mut dy2: Vec<f64> = vec![f64::NAN; el_x_len];
        for k in 2..(el_x_len - 3) {
            dx2[k] = (x_array[k - 2]
                + x_array[k - 1]
                + x_array[k]
                + x_array[k + 1]
                + x_array[k + 2]
                + x_array[k + 3])
                / 6.0;
            dy2[k] = (y_array[k - 2]
                + y_array[k - 1]
                + y_array[k]
                + y_array[k + 1]
                + y_array[k + 2]
                + y_array[k + 3])
                / 6.0;
        }

        for k in 3..(el_x_len - 3) {
            let vx = x_array[k] + (dx[k] - dx2[k]) * curviness;
            let vy = y_array[k] + (dy[k] - dy2[k]) * curviness;
            x_array[k] = vx;
            y_array[k] = vy;
        }

        let mut points: Vec<Point> = vec![];

        for k in 0..el_x_len {
            points.push(Point {
                x: x_array[k],
                y: y_array[k],
            });
        }

        let smoothed_polyline = GenericPolyline::new(points);
        let _ = writer.write_shape_and_record(&smoothed_polyline, &record);
        smoothed_contours.push((x_array, y_array, height));
    }

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Contours smoothed in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    return smoothed_contours;
}

pub fn get_elevation_matrix_from_dem(tile: &Tile) -> Vec<Vec<f64>> {
    let dem_path = tile.render_dir_path.join("dem-low-resolution-with-buffer.tif");
    let dem_tif_file = File::open(dem_path).expect("Cannot find dem tif image!");

    let mut dem_img_decoder = Decoder::new(dem_tif_file).expect("Cannot create decoder");
    dem_img_decoder = dem_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (dem_width, dem_height) = dem_img_decoder.dimensions().unwrap();

    let width: usize = dem_width as usize;
    let height: usize = dem_height as usize;
    let mut avg_alt = vec![vec![f64::NAN; height + 2]; width + 2];

    let DecodingResult::F64(image_data) = dem_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    // Building avg_alt matrix and defining hmin and hmax
    for index in 0..image_data.len() {
        let x = index % width;
        let y = height - index / width;
        let elevation = image_data[index] as f64;
        avg_alt[x][y] = elevation;
    }

    return avg_alt;
}
