use std::{fs::File, path::PathBuf};
use tiff::{
    decoder::{Decoder, DecodingResult},
    encoder::{colortype, TiffEncoder},
    tags::Tag,
};

const MAX_PIXEL_DISTANCE: usize = 10;

/// Fill holes in a tiff raster
/// Work as a replacment for gdal_fillnodata because it is a python gdal utility not working properly on all environments
pub fn fill_nodata_in_raster(input_raster_path: &PathBuf, output_raster_path: &PathBuf) {
    let tif_file = File::open(input_raster_path).expect("Cannot find dem tif image!");

    let mut img_decoder = Decoder::new(tif_file).expect("Cannot create decoder");
    img_decoder = img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (width, height) = img_decoder.dimensions().unwrap();

    let matrix_width: usize = width as usize;
    let matrix_height: usize = height as usize;
    let mut matrix = vec![vec![f64::NAN; matrix_height]; matrix_width];

    let DecodingResult::F64(mut image_data) = img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    for index in 0..image_data.len() {
        let x = index % matrix_width;
        let y = index / matrix_width;

        // Sometime there is more pixel data than it should be given the header's dimensions
        if y > height as usize {
            break;
        }

        let value = image_data[index] as f64;
        matrix[x][y] = if value == -9999. { f64::NAN } else { value }; // TODO: Generalize to all nodata value possibilities
    }

    for x in 0..(matrix_width) {
        for y in 0..(matrix_height) {
            let value = matrix[x][y];

            if !value.is_nan() {
                continue;
            }

            let mut top = f64::NAN;
            let mut top_weight = f64::NAN;
            let mut top_right = f64::NAN;
            let mut top_right_weight = f64::NAN;
            let mut right = f64::NAN;
            let mut right_weight = f64::NAN;
            let mut bottom_right = f64::NAN;
            let mut bottom_right_weight = f64::NAN;
            let mut bottom = f64::NAN;
            let mut bottom_weight = f64::NAN;
            let mut bottom_left = f64::NAN;
            let mut bottom_left_weight = f64::NAN;
            let mut left = f64::NAN;
            let mut left_weight = f64::NAN;
            let mut top_left = f64::NAN;
            let mut top_left_weight = f64::NAN;

            let mut i = 1;

            while i <= MAX_PIXEL_DISTANCE
                && (top.is_nan()
                    || top_right.is_nan()
                    || right.is_nan()
                    || bottom_right.is_nan()
                    || bottom.is_nan()
                    || bottom_left.is_nan()
                    || left.is_nan()
                    || top_left.is_nan())
            {
                if top.is_nan() && (y + i) < matrix_height {
                    let target_value = matrix[x][y + i];

                    if !target_value.is_nan() {
                        top_weight = 1. / i as f64;
                        top = target_value * top_weight;
                    }
                }

                if top_right.is_nan() && (y + i) < matrix_height && (x + i) < matrix_width {
                    let target_value = matrix[x + i][y + i];

                    if !target_value.is_nan() {
                        top_right_weight = 1. / i as f64 / f64::sqrt(2.);
                        top_right = target_value * top_right_weight;
                    }
                }

                if right.is_nan() && (x + i) < matrix_width {
                    let target_value = matrix[x + i][y];

                    if !target_value.is_nan() {
                        right_weight = 1. / i as f64;
                        right = target_value * right_weight;
                    }
                }

                if bottom_right.is_nan() && (y as isize - i as isize) >= 0 && (x + i) < matrix_width
                {
                    let target_value = matrix[x + i][y - i];

                    if !target_value.is_nan() {
                        bottom_right_weight = 1. / i as f64 / f64::sqrt(2.);
                        bottom_right = target_value * bottom_right_weight;
                    }
                }

                if bottom.is_nan() && (y as isize - i as isize) >= 0 {
                    let target_value = matrix[x][y - i];

                    if !target_value.is_nan() {
                        bottom_weight = 1. / i as f64;
                        bottom = target_value * bottom_weight;
                    }
                }

                if bottom_left.is_nan()
                    && (y as isize - i as isize) >= 0
                    && (x as isize - i as isize) >= 0
                {
                    let target_value = matrix[x - i][y - i];

                    if !target_value.is_nan() {
                        bottom_left_weight = 1. / i as f64 / f64::sqrt(2.);
                        bottom_left = target_value * bottom_left_weight;
                    }
                }

                if left.is_nan() && (x as isize - i as isize) >= 0 {
                    let target_value = matrix[x - i][y];

                    if !target_value.is_nan() {
                        left_weight = 1. / i as f64;
                        left = target_value * left_weight;
                    }
                }

                if top_left.is_nan() && (y + i) < matrix_height && (x as isize - i as isize) >= 0 {
                    let target_value = matrix[x - i][y + i];

                    if !target_value.is_nan() {
                        top_left_weight = 1. / i as f64 / f64::sqrt(2.);
                        top_left = target_value * top_left_weight;
                    }
                }

                i += 1;
            }

            if top.is_nan()
                && top_right.is_nan()
                && right.is_nan()
                && bottom_right.is_nan()
                && bottom.is_nan()
                && bottom_left.is_nan()
                && left.is_nan()
                && top_left.is_nan()
            {
                continue;
            }

            let mut value_sum = 0.;
            let mut weight_sum = 0.;

            if !top.is_nan() {
                value_sum += top;
                weight_sum += top_weight;
            }

            if !top_right.is_nan() {
                value_sum += top_right;
                weight_sum += top_right_weight;
            }

            if !right.is_nan() {
                value_sum += right;
                weight_sum += right_weight;
            }

            if !bottom_right.is_nan() {
                value_sum += bottom_right;
                weight_sum += bottom_right_weight;
            }

            if !bottom.is_nan() {
                value_sum += bottom;
                weight_sum += bottom_weight;
            }

            if !bottom_left.is_nan() {
                value_sum += bottom_left;
                weight_sum += bottom_left_weight;
            }

            if !left.is_nan() {
                value_sum += left;
                weight_sum += left_weight;
            }

            if !top_left.is_nan() {
                value_sum += top_left;
                weight_sum += top_left_weight;
            }

            let index = y * matrix_width + x;
            image_data[index] = value_sum / weight_sum;
        }
    }

    let mut file = File::create(output_raster_path).unwrap();
    let mut tiff = TiffEncoder::new(&mut file).unwrap();

    let mut image = tiff
        .new_image::<colortype::Gray64Float>(width, height)
        .unwrap();

    let encoder = image.encoder();

    match img_decoder.find_tag(Tag::XResolution).unwrap_or(None) {
        Some(v) => {
            encoder
                .write_tag(Tag::XResolution, &v.into_f64_vec().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    match img_decoder.find_tag(Tag::YResolution).unwrap_or(None) {
        Some(v) => {
            encoder
                .write_tag(Tag::YResolution, &v.into_f64_vec().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    match img_decoder
        .find_tag(Tag::ModelPixelScaleTag)
        .unwrap_or(None)
    {
        Some(v) => {
            encoder
                .write_tag(Tag::ModelPixelScaleTag, &v.into_f64_vec().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    match img_decoder
        .find_tag(Tag::ModelTransformationTag)
        .unwrap_or(None)
    {
        Some(v) => {
            encoder
                .write_tag(Tag::ModelTransformationTag, &v.into_f64_vec().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    match img_decoder.find_tag(Tag::ModelTiepointTag).unwrap_or(None) {
        Some(v) => {
            encoder
                .write_tag(Tag::ModelTiepointTag, &v.into_f64_vec().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    match img_decoder
        .find_tag(Tag::GeoKeyDirectoryTag)
        .unwrap_or(None)
    {
        Some(v) => {
            encoder
                .write_tag(Tag::GeoKeyDirectoryTag, &v.into_u64_vec().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    match img_decoder
        .find_tag(Tag::GeoDoubleParamsTag)
        .unwrap_or(None)
    {
        Some(v) => {
            encoder
                .write_tag(Tag::GeoDoubleParamsTag, &v.into_f64_vec().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    match img_decoder.find_tag(Tag::GeoAsciiParamsTag).unwrap_or(None) {
        Some(v) => {
            encoder
                .write_tag(Tag::GeoAsciiParamsTag, &v.into_string().unwrap()[..])
                .unwrap();
        }
        None => {}
    }

    encoder.write_tag(Tag::GdalNodata, -9999.).unwrap();
    image.write_data(&image_data).unwrap();
}
