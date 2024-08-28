use std::{fs::File, path::PathBuf};
use tiff::{
    decoder::{ifd::Value, Decoder, DecodingResult},
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

    let all_tags = [
        Tag::Artist,
        Tag::BitsPerSample,
        Tag::CellLength,
        Tag::CellWidth,
        Tag::ColorMap,
        Tag::Compression,
        Tag::Copyright,
        Tag::DateTime,
        Tag::ExtraSamples,
        Tag::FillOrder,
        Tag::FreeByteCounts,
        Tag::FreeOffsets,
        Tag::GrayResponseCurve,
        Tag::GrayResponseUnit,
        Tag::HostComputer,
        Tag::ImageDescription,
        Tag::ImageLength,
        Tag::ImageWidth,
        Tag::Make,
        Tag::MaxSampleValue,
        Tag::MinSampleValue,
        Tag::Model,
        Tag::NewSubfileType,
        Tag::Orientation,
        Tag::PhotometricInterpretation,
        Tag::PlanarConfiguration,
        Tag::ResolutionUnit,
        Tag::RowsPerStrip,
        Tag::SamplesPerPixel,
        Tag::Software,
        Tag::StripByteCounts,
        Tag::StripOffsets,
        Tag::SubfileType,
        Tag::Threshholding,
        Tag::XResolution,
        Tag::YResolution,
        Tag::Predictor,
        Tag::TileWidth,
        Tag::TileLength,
        Tag::TileOffsets,
        Tag::TileByteCounts,
        Tag::SampleFormat,
        Tag::SMinSampleValue,
        Tag::SMaxSampleValue,
        Tag::JPEGTables,
        Tag::ModelPixelScaleTag,
        Tag::ModelTransformationTag,
        Tag::ModelTiepointTag,
        Tag::GeoKeyDirectoryTag,
        Tag::GeoDoubleParamsTag,
        Tag::GeoAsciiParamsTag,
        Tag::GdalNodata,
    ];

    // Copying all tags from input raster to output raster
    // TODO: DRY this monster
    for tag in all_tags {
        match img_decoder.find_tag(tag).unwrap_or(None) {
            Some(v) => match v {
                Value::Byte(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::Short(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::Signed(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::SignedBig(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::Unsigned(v) | Value::Ifd(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::UnsignedBig(v) | Value::IfdBig(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::Float(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::Double(v) => {
                    let _ = encoder.write_tag(tag, v);
                }
                Value::List(list_value) => {
                    let first_value_option = list_value.first();

                    match first_value_option {
                        None => {
                            let empty_slice: &[i32] = &[];
                            let _ = encoder.write_tag(tag, empty_slice);
                        }
                        Some(first_value) => match first_value {
                            Value::Byte(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_u8().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            Value::Short(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_u16().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            Value::Signed(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_i32().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            Value::SignedBig(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_i64().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            Value::Unsigned(v) | Value::Ifd(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_u32().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            Value::UnsignedBig(v) | Value::IfdBig(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_u64().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            Value::Float(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_f32().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            Value::Double(v) => {
                                let mut output_vec = vec![v.clone()];
                                for value in &list_value {
                                    let parsed_value = value.clone().into_f64().unwrap();
                                    output_vec.push(parsed_value)
                                }
                                let _ = encoder.write_tag(tag, &output_vec[..]);
                            }
                            // Value::Rational(v1, v2) => {
                            //     let _ = encoder.write_tag(tag, &[v1, v2][..]);
                            // }
                            // Value::RationalBig(v1, v2) => {
                            //     let _ = encoder.write_tag(tag, &[v1, v2][..]);
                            // }
                            // Value::SRational(v1, v2) => {
                            //     let _ = encoder.write_tag(tag, &[v1, v2][..]);
                            // }
                            // Value::SRationalBig(v1, v2) => {
                            //     let _ = encoder.write_tag(tag, &[v1, v2][..]);
                            // }
                            // Value::Ascii(v) => {
                            //     let mut output_vec = vec![&v.clone()[..]];
                            //     for value in &list_value {
                            //         let parsed_value = value.clone().into_string().unwrap();
                            //         output_vec.push(&parsed_value[..])
                            //     }
                            //     let _ = encoder.write_tag(tag, &output_vec[..]);
                            // }
                            _ => {}
                        },
                    }

                    // for value in v {
                    //     value.into();
                    // }
                }
                Value::Rational(v1, v2) => {
                    let _ = encoder.write_tag(tag, &[v1, v2][..]);
                }
                Value::RationalBig(v1, v2) => {
                    let _ = encoder.write_tag(tag, &[v1, v2][..]);
                }
                Value::SRational(v1, v2) => {
                    let _ = encoder.write_tag(tag, &[v1, v2][..]);
                }
                Value::SRationalBig(v1, v2) => {
                    let _ = encoder.write_tag(tag, &[v1, v2][..]);
                }
                Value::Ascii(v) => {
                    let _ = encoder.write_tag(tag, &v[..]);
                }
                _ => {}
            },
            None => {}
        }
    }

    image.write_data(&image_data).unwrap();
}
