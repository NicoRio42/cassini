use image::RgbaImage;
use imageproc::drawing::draw_line_segment_mut;
use log::info;
use shapefile::dbase::{FieldIOError, FieldWriter, WritableRecord};
use shapefile::record::polyline::GenericPolyline;
use shapefile::{Point, Reader, Writer};
use std::fs::{create_dir_all, File};
use std::io::{BufReader, Write};
use std::time::Instant;

struct FormLineRecord {
    id: i32,
    elev: f64,
}

impl WritableRecord for FormLineRecord {
    fn write_using<'a, W>(&self, field_writer: &mut FieldWriter<'a, W>) -> Result<(), FieldIOError>
    where
        W: Write,
    {
        field_writer.write_next_field_value(&self.id)?;
        field_writer.write_next_field_value(&self.elev)?;
        Ok(())
    }
}

use crate::config::Config;
use crate::constants::{BUFFER, INCH, PURPLE};
use crate::{
    constants::{BROWN, TRANSPARENT},
    tile::Tile,
};

pub fn pullautin_cull_formlines_render_contours(
    tile: &Tile,
    image_width: u32,
    image_height: u32,
    config: &Config,
    avg_alt: &Vec<Vec<f64>>,
    smoothed_contours: Vec<(Vec<f64>, Vec<f64>, f64)>,
) {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Culling formlines and rendering contours",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let scalefactor = 1.0;
    let formlineaddition: f64 = 17.;
    let minimumgap: u32 = 30;
    let dashlength: f64 = 60.;
    let gaplength: f64 = 12.;
    let mut img = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);
    let formlinesteepness: f64 = 0.37;
    let buffer_in_pixels = BUFFER as f32 * (config.dpi_resolution / INCH) as f32;
    let indexcontours: f64 = 25.;
    let contour_interval: f64 = 5.0;
    let halfinterval = contour_interval / 2.0 * scalefactor;

    let dem_cell_size: f64 = 2.0;
    let xstart: f64 = (tile.min_x - BUFFER as i64) as f64;
    let ystart: f64 = (tile.min_y - BUFFER as i64) as f64;
    let x0 = xstart as f64;
    let y0 = ystart as f64;

    let sxmax: usize = ((tile.max_x + BUFFER as i64 - xstart as i64) as f64 / 2.).ceil() as usize;
    let symax: usize = ((tile.max_y + BUFFER as i64 - ystart as i64) as f64 / 2.).ceil() as usize;
    let mut steepness = vec![vec![0.0f64; symax + 2]; sxmax + 2];

    // Building a more complex steepness matrix
    for i in 6..(sxmax - 7) {
        for j in 6..(symax - 7) {
            let mut det: f64 = 0.0;
            let mut high: f64 = f64::MIN;

            let xyz_i_m4_j = avg_alt[i - 4][j];
            let xyz_i_j = avg_alt[i][j];
            let xyz_i_4_j = avg_alt[i + 4][j];
            let xyz_i_j_m4 = avg_alt[i][j - 4];
            let xyz_i_j_4 = avg_alt[i][j + 4];
            let xyz_i_m4_j_m4 = avg_alt[i - 4][j - 4];
            let xyz_i_4_j_4 = avg_alt[i + 4][j + 4];
            let xyz_i_m4_j_4 = avg_alt[i - 4][j + 4];
            let xyz_i_4_j_m4 = avg_alt[i + 4][j - 4];

            let mut temp = (xyz_i_m4_j - xyz_i_j).abs() / 4.0;
            let temp2 = (xyz_i_j - xyz_i_4_j).abs() / 4.0;
            let det2 =
                (xyz_i_j - 0.5 * (xyz_i_m4_j + xyz_i_4_j)).abs() - 0.05 * (xyz_i_m4_j - xyz_i_4_j).abs();
            let mut porr = (((avg_alt[i - 6][j] - avg_alt[i + 6][j]) / 12.0).abs()
                - ((avg_alt[i - 3][j] - avg_alt[i + 3][j]) / 6.0).abs())
            .abs();

            if det2 > det {
                det = det2;
            }
            if temp2 < temp {
                temp = temp2;
            }
            if temp > high {
                high = temp;
            }

            let mut temp = (xyz_i_j_m4 - xyz_i_j).abs() / 4.0;
            let temp2 = (xyz_i_j - xyz_i_j_m4).abs() / 4.0;
            let det2 =
                (xyz_i_j - 0.5 * (xyz_i_j_m4 + xyz_i_j_4)).abs() - 0.05 * (xyz_i_j_m4 - xyz_i_j_4).abs();
            let porr2 = (((avg_alt[i][j - 6] - avg_alt[i][j + 6]) / 12.0).abs()
                - ((avg_alt[i][j - 3] - avg_alt[i][j + 3]) / 6.0).abs())
            .abs();

            if porr2 > porr {
                porr = porr2;
            }
            if det2 > det {
                det = det2;
            }
            if temp2 < temp {
                temp = temp2;
            }
            if temp > high {
                high = temp;
            }

            let mut temp = (xyz_i_m4_j_m4 - xyz_i_j).abs() / 5.6;
            let temp2 = (xyz_i_j - xyz_i_4_j_4).abs() / 5.6;
            let det2 = (xyz_i_j - 0.5 * (xyz_i_m4_j_m4 + xyz_i_4_j_4)).abs()
                - 0.05 * (xyz_i_m4_j_m4 - xyz_i_4_j_4).abs();
            let porr2 = (((avg_alt[i - 6][j - 6] - avg_alt[i + 6][j + 6]) / 17.0).abs()
                - ((avg_alt[i - 3][j - 3] - avg_alt[i + 3][j + 3]) / 8.5).abs())
            .abs();

            if porr2 > porr {
                porr = porr2;
            }
            if det2 > det {
                det = det2;
            }
            if temp2 < temp {
                temp = temp2;
            }
            if temp > high {
                high = temp;
            }

            let mut temp = (xyz_i_m4_j_4 - xyz_i_j).abs() / 5.6;
            let temp2 = (xyz_i_j - xyz_i_4_j_m4).abs() / 5.6;
            let det2 = (xyz_i_j - 0.5 * (xyz_i_4_j_m4 + xyz_i_m4_j_4)).abs()
                - 0.05 * (xyz_i_4_j_m4 - xyz_i_m4_j_4).abs();
            let porr2 = (((avg_alt[i + 6][j - 6] - avg_alt[i - 6][j + 6]) / 17.0).abs()
                - ((avg_alt[i + 3][j - 3] - avg_alt[i - 3][j + 3]) / 8.5).abs())
            .abs();

            if porr2 > porr {
                porr = porr2;
            }
            if det2 > det {
                det = det2;
            }
            if temp2 < temp {
                temp = temp2;
            }
            if temp > high {
                high = temp;
            }

            let mut val = 12.0 * high / (1.0 + 8.0 * det);
            if porr > 0.25 * 0.67 / (0.3 + formlinesteepness) {
                val = 0.01;
            }
            if high > val {
                val = high;
            }

            steepness[i][j] = val;
        }
    }

    let mut id: i32 = 0;
    let contours_polylines_path = tile.render_dir_path.join("contours-raw").join("contours-raw.shp");

    let contours_polylines_reader: shapefile::Reader<BufReader<File>, BufReader<File>> =
        Reader::from_path(&contours_polylines_path).unwrap();

    let table_info = contours_polylines_reader.into_table_info();

    let formlines_dir = tile.render_dir_path.join("formlines");
    create_dir_all(&formlines_dir).expect("Could not create formlines dir");

    let mut writer = Writer::from_path_with_info(formlines_dir.join("formlines.shp"), table_info).unwrap();

    for smoothed_contour in smoothed_contours {
        let color = if is_contour_depression(&smoothed_contour, xstart, ystart, avg_alt, dem_cell_size) {
            PURPLE
        } else {
            BROWN
        };

        let (x_array, y_array, elevation) = smoothed_contour;
        let mut x = Vec::<f64>::new();
        let mut y = Vec::<f64>::new();

        for i in 0..x_array.len() {
            x.push((x_array[i] - x0) * 600.0 / 254.0 / scalefactor);
            y.push((y0 - y_array[i]) * 600.0 / 254.0 / scalefactor);
        }

        let mut curvew = 2.0;

        if elevation.round() as isize % indexcontours as isize == 0 {
            curvew = 3.5;
        } else if elevation.round() as isize % contour_interval as isize != 0
            && (elevation * 2.).round() as isize % (halfinterval * 2.).round() as isize == 0
        {
            curvew = 1.5;
        }

        let mut smallringtest = false;
        let mut help = vec![false; x.len()];
        let mut help2 = vec![false; x.len()];

        if curvew == 1.5 {
            for i in 0..x.len() {
                help[i] = false;
                help2[i] = true;
                let xx = (((x[i] / 600.0 * 254.0 * scalefactor + x0) - xstart) / dem_cell_size).floor();
                let yy = (((-y[i] / 600.0 * 254.0 * scalefactor + y0) - ystart) / dem_cell_size).floor();

                if curvew != 1.5
                    || &steepness[xx as usize][yy as usize] < &formlinesteepness
                    || &steepness[xx as usize][yy as usize + 1] < &formlinesteepness
                    || &steepness[xx as usize + 1][yy as usize] < &formlinesteepness
                    || &steepness[xx as usize + 1][yy as usize + 1] < &formlinesteepness
                {
                    help[i] = true;
                }
            }
            for i in 5..(x.len() - 6) {
                let mut apu = 0;
                for j in (i - 5)..(i + 4) {
                    if help[j] {
                        apu += 1;
                    }
                }
                if apu < 5 {
                    help2[i] = false;
                }
            }
            for i in 0..6 {
                help2[i] = help2[6]
            }
            for i in (x.len() - 6)..x.len() {
                help2[i] = help2[x.len() - 7]
            }
            let mut on = 0.0;
            for i in 0..x.len() {
                if help2[i] {
                    on = formlineaddition
                }
                if on > 0.0 {
                    help2[i] = true;
                    on -= 1.0;
                }
            }
            if x.first() == x.last() && y.first() == y.last() && on > 0.0 {
                let mut i = 0;
                while i < x.len() && on > 0.0 {
                    help2[i] = true;
                    on -= 1.0;
                    i += 1;
                }
            }
            let mut on = 0.0;
            for i in 0..x.len() {
                let ii = x.len() - i - 1;
                if help2[ii] {
                    on = formlineaddition
                }
                if on > 0.0 {
                    help2[ii] = true;
                    on -= 1.0;
                }
            }
            if x.first() == x.last() && y.first() == y.last() && on > 0.0 {
                let mut i = (x.len() - 1) as i32;
                while i > -1 && on > 0.0 {
                    help2[i as usize] = true;
                    on -= 1.0;
                    i -= 1;
                }
            }
            // Let's not break small form line rings
            smallringtest = false;
            if x.first() == x.last() && y.first() == y.last() && x.len() < 122 {
                for i in 1..x.len() {
                    if help2[i] {
                        smallringtest = true
                    }
                }
            }
            // Let's draw short gaps together
            if !smallringtest {
                let mut tester = 1;
                for i in 1..x.len() {
                    if help2[i] {
                        if tester < i && ((i - tester) as u32) < minimumgap {
                            for j in tester..(i + 1) {
                                help2[j] = true;
                            }
                        }
                        tester = i;
                    }
                }
                // Ring handling
                if x.first() == x.last() && y.first() == y.last() && x.len() < 2 {
                    let mut i = 1;
                    while i < x.len() && !help2[i] {
                        i += 1
                    }
                    let mut j = x.len() - 1;
                    while j > 1 && !help2[i] {
                        j -= 1
                    }
                    if ((x.len() - j + i - 1) as u32) < minimumgap && j > i {
                        for k in 0..(i + 1) {
                            help2[k] = true
                        }
                        for k in j..x.len() {
                            help2[k] = true
                        }
                    }
                }
            }
        }

        let mut linedist = 0.0;
        let mut onegapdone = false;
        let mut gap = 0.0;
        let mut formlinestart = false;

        let mut current_formline: Vec<(f64, f64)> = vec![];

        for i in 1..x.len() {
            if curvew != 1.5 || help2[i] || smallringtest {
                if curvew == 1.5 {
                    if !formlinestart {
                        current_formline = vec![];
                        formlinestart = true;
                    }

                    current_formline.push((
                        x[i] / 600.0 * 254.0 * scalefactor + x0,
                        -y[i] / 600.0 * 254.0 * scalefactor + y0,
                    ));
                }

                if curvew == 1.5 {
                    let step = ((x[i - 1] - x[i]).powi(2) + (y[i - 1] - y[i]).powi(2)).sqrt();
                    if i < 4 {
                        linedist = 0.0
                    }
                    linedist += step;
                    if linedist > dashlength && i > 10 && i < x.len() - 11 {
                        let mut sum = 0.0;
                        for k in (i - 4)..(i + 6) {
                            sum += ((x[k - 1] - x[k]).powi(2) + (y[k - 1] - y[k]).powi(2)).sqrt()
                        }
                        let mut toonearend = false;
                        for k in (i - 10)..(i + 10) {
                            if !help2[k] {
                                toonearend = true;
                                break;
                            }
                        }
                        if !toonearend
                            && ((x[i - 5] - x[i + 5]).powi(2) + (y[i - 5] - y[i + 5]).powi(2)).sqrt() * 1.138
                                > sum
                        {
                            linedist = 0.0;
                            gap = gaplength;
                            onegapdone = true;
                        }
                    }
                    if !onegapdone && (i < x.len() - 9) && i > 6 {
                        gap = gaplength * 0.82;
                        onegapdone = true;
                        linedist = 0.0
                    }
                    if gap > 0.0 {
                        gap -= step;
                        if gap < 0.0 && onegapdone && step > 0.0 {
                            let mut n = -curvew - 0.5;
                            while n < curvew + 0.5 {
                                let mut m = -curvew - 0.5;
                                while m < curvew + 0.5 {
                                    let start = (
                                        ((-x[i - 1] * gap + (step + gap) * x[i]) / step + n) as f32
                                            - buffer_in_pixels,
                                        image_height as f32
                                            + buffer_in_pixels
                                            + ((-y[i - 1] * gap + (step + gap) * y[i]) / step + m) as f32,
                                    );

                                    let end = (
                                        (x[i] + n) as f32 - buffer_in_pixels,
                                        image_height as f32 + buffer_in_pixels + (y[i] + m) as f32,
                                    );

                                    draw_line_segment_mut(&mut img, start, end, color);
                                    m += 1.0;
                                }
                                n += 1.0;
                            }
                            gap = 0.0;
                        }
                    } else {
                        let mut n = -curvew - 0.5;
                        while n < curvew + 0.5 {
                            let mut m = -curvew - 0.5;
                            while m < curvew + 0.5 {
                                let start = (
                                    (x[i - 1] + n) as f32 - buffer_in_pixels,
                                    image_height as f32 + buffer_in_pixels + (y[i - 1] + m) as f32,
                                );

                                let end = (
                                    (x[i] + n) as f32 - buffer_in_pixels,
                                    image_height as f32 + buffer_in_pixels + (y[i] + m) as f32,
                                );

                                draw_line_segment_mut(&mut img, start, end, color);
                                m += 1.0;
                            }
                            n += 1.0;
                        }
                    }
                } else {
                    let mut n = -curvew;
                    while n < curvew {
                        let mut m = -curvew;
                        while m < curvew {
                            let start = (
                                (x[i - 1] + n) as f32 - buffer_in_pixels,
                                image_height as f32 + buffer_in_pixels + (y[i - 1] + m) as f32,
                            );

                            let end = (
                                (x[i] + n) as f32 - buffer_in_pixels,
                                image_height as f32 + buffer_in_pixels + (y[i] + m) as f32,
                            );

                            draw_line_segment_mut(&mut img, start, end, color);
                            m += 1.0;
                        }
                        n += 1.0;
                    }
                }
            } else if formlinestart {
                write_formline_shape_to_shapefile(&current_formline, id, elevation, &mut writer);
                id += 1;
                formlinestart = false;
            }
        }

        if formlinestart {
            write_formline_shape_to_shapefile(&current_formline, id, elevation, &mut writer);
            id += 1;
        }
    }

    // TODO: img.save takes 8 seconds, maybe mutualize with other images saving
    img.save(tile.render_dir_path.join("contours.png"))
        .expect("could not save output png");

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Formlines and contours generated in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );
}

fn write_formline_shape_to_shapefile(
    current_formline: &Vec<(f64, f64)>,
    id: i32,
    elevation: f64,
    writer: &mut shapefile::Writer<std::io::BufWriter<File>>,
) {
    if current_formline.len() < 2 {
        return;
    }

    let mut points: Vec<Point> = vec![];

    for (x, y) in current_formline {
        points.push(Point { x: *x, y: *y });
    }

    let record = FormLineRecord { id, elev: elevation };

    let smoothed_polyline = GenericPolyline::new(points);
    let _ = writer.write_shape_and_record(&smoothed_polyline, &record);
}

fn is_contour_depression(
    (x_array, y_array, elevation): &(Vec<f64>, Vec<f64>, f64),
    xstart: f64,
    ystart: f64,
    avg_alt: &Vec<Vec<f64>>,
    dem_cell_size: f64,
) -> bool {
    let is_closed = x_array.first() == x_array.last() && y_array.first() == y_array.last();

    if !is_closed {
        return false;
    }

    if x_array.len() < 4 || y_array.len() < 4 {
        return false;
    }

    let mut contour_min_x = f64::MAX;
    let mut contour_max_x = f64::MIN;
    let mut contour_min_y = f64::MAX;
    let mut contour_max_y = f64::MIN;

    for i in 0..x_array.len() {
        let x = x_array[i];
        let y = y_array[i];

        if x < contour_min_x {
            contour_min_x = x;
        }
        if x > contour_max_x {
            contour_max_x = x;
        }
        if y < contour_min_y {
            contour_min_y = y;
        }
        if y > contour_max_y {
            contour_max_y = y;
        }
    }

    // Sample a point inside the closed contour using a grid search that maximizes
    // distance to edges (approximation of the largest inscribed circle).
    let mut best_point: Option<(f64, f64)> = None;
    let mut best_dist = -1.0_f64;

    let mut x = contour_min_x;
    while x <= contour_max_x {
        let mut y = contour_min_y;

        while y <= contour_max_y {
            if point_in_polygon(x, y, x_array, y_array) {
                let dist = min_distance_to_edges(x, y, x_array, y_array);

                if dist > best_dist {
                    best_dist = dist;
                    best_point = Some((x, y));
                }
            }

            y += dem_cell_size;
        }
        x += dem_cell_size;
    }

    let Some((px, py)) = best_point else {
        return false;
    };

    let inside_elevation =
        get_point_elevation_from_dem_bilinear_interpolation(px, py, xstart, ystart, dem_cell_size, avg_alt);

    if inside_elevation.is_nan() {
        return false;
    }

    inside_elevation < *elevation
}

fn point_in_polygon(x: f64, y: f64, x_array: &Vec<f64>, y_array: &Vec<f64>) -> bool {
    let mut inside = false;
    let mut j = x_array.len() - 1;

    for i in 0..x_array.len() {
        let xi = x_array[i];
        let yi = y_array[i];
        let xj = x_array[j];
        let yj = y_array[j];

        let intersect = ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi + f64::EPSILON) + xi);

        if intersect {
            inside = !inside;
        }

        j = i;
    }
    inside
}

fn min_distance_to_edges(x: f64, y: f64, x_array: &Vec<f64>, y_array: &Vec<f64>) -> f64 {
    let mut min_dist = f64::MAX;

    for i in 1..x_array.len() {
        let x1 = x_array[i - 1];
        let y1 = y_array[i - 1];
        let x2 = x_array[i];
        let y2 = y_array[i];

        let dist = distance_point_to_segment(x, y, x1, y1, x2, y2);

        if dist < min_dist {
            min_dist = dist;
        }
    }

    min_dist
}

fn distance_point_to_segment(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;

    if dx == 0.0 && dy == 0.0 {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }

    let t = ((px - x1) * dx + (py - y1) * dy) / (dx * dx + dy * dy);
    let t = t.clamp(0.0, 1.0);
    let proj_x = x1 + t * dx;
    let proj_y = y1 + t * dy;
    ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
}

fn get_point_elevation_from_dem_bilinear_interpolation(
    x: f64,
    y: f64,
    xstart: f64,
    ystart: f64,
    dem_cell_size: f64,
    avg_alt: &Vec<Vec<f64>>,
) -> f64 {
    let poin_local_x = (x - xstart) / dem_cell_size;
    let poin_local_y = (y - ystart) / dem_cell_size;

    if poin_local_x.is_nan() || poin_local_y.is_nan() || poin_local_x < 0.0 || poin_local_y < 0.0 {
        return f64::NAN;
    }

    let x0 = poin_local_x.floor() as usize;
    let y0 = poin_local_y.floor() as usize;
    let x1 = x0 + 1;
    let y1 = y0 + 1;

    if x1 >= avg_alt.len() || y1 >= avg_alt[0].len() {
        return f64::NAN;
    }

    let fx = poin_local_x - x0 as f64;
    let fy = poin_local_y - y0 as f64;

    let v00 = avg_alt[x0][y0];
    let v10 = avg_alt[x1][y0];
    let v01 = avg_alt[x0][y1];
    let v11 = avg_alt[x1][y1];

    if v00.is_nan() || v10.is_nan() || v01.is_nan() || v11.is_nan() {
        return f64::NAN;
    }

    let v0 = v00 * (1.0 - fx) + v10 * fx;
    let v1 = v01 * (1.0 - fx) + v11 * fx;
    v0 * (1.0 - fy) + v1 * fy
}
