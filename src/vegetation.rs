use crate::{
    buffer::create_tif_with_buffer,
    config::Config,
    constants::{
        BUFFER, GREEN_1, GREEN_2, GREEN_3, INCH, TRANSPARENT, VEGETATION_BLOCK_SIZE, WHITE, YELLOW,
    },
    error::{CassiniError, Result, ResultContext},
    tile::Tile,
};
use image::{imageops, RgbaImage};
use imageproc::{
    drawing::{draw_filled_ellipse_mut, draw_filled_rect_mut},
    rect::Rect,
};
use log::info;
use std::{
    f32::consts::E,
    fs::File,
    path::{Path, PathBuf},
    time::Instant,
};
use tiff::decoder::{Decoder, DecodingResult};

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum UndergrowthMode {
    None,
    Merge,
    #[value(name = "406")]
    Symbol406,
    #[value(name = "409")]
    Symbol409,
}

pub fn render_vegetation(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
    image_width: u32,
    image_height: u32,
    config: &Config,
    undergrowth_mode: &UndergrowthMode,
) -> Result<()> {
    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering vegetation",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let vegetation_block_size_pixel = VEGETATION_BLOCK_SIZE as f32 * config.dpi_resolution / INCH;
    let casted_base_vegetation_block_size_pixel = (vegetation_block_size_pixel * 2.).ceil() as i32;
    let casted_green_block_size_pixel = (vegetation_block_size_pixel).ceil() as u32;

    create_tif_with_buffer(
        tile,
        neighbor_tiles,
        BUFFER as i64,
        "high_vegetation",
        1.0,
        None,
    )?;
    create_tif_with_buffer(
        tile,
        neighbor_tiles,
        BUFFER as i64,
        "medium_vegetation",
        1.0,
        None,
    )?;
    create_tif_with_buffer(
        tile,
        neighbor_tiles,
        BUFFER as i64,
        "low_vegetation",
        1.0,
        None,
    )?;

    let high_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("high_vegetation_with_buffer.tif"))?;

    let medium_vegetation = get_image_data_from_tif(
        &tile
            .render_dir_path
            .join("medium_vegetation_with_buffer.tif"),
    )?;

    let low_vegetation =
        get_image_data_from_tif(&tile.render_dir_path.join("low_vegetation_with_buffer.tif"))?;

    let mut base_vegetation_img = RgbaImage::from_pixel(image_width, image_height, YELLOW);
    let mut green_vegetation_img = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);
    let mut undergrowth_vegetation_img =
        RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);

    let interior = vegetation_interior(
        tile,
        [&high_vegetation, &medium_vegetation, &low_vegetation],
    )?;
    let classification = classify_vegetation(
        &high_vegetation,
        &medium_vegetation,
        &low_vegetation,
        interior,
        config,
        undergrowth_mode,
    );

    // High vegetation and undergrowth use a single color per layer, so they can
    // be rasterized in cache-friendly row-major order without changing overlap
    // precedence.
    for y in 0..classification.height {
        let y_pixel = (y as f32 * vegetation_block_size_pixel) as i32;
        for x in 0..classification.width {
            let index = y * classification.width + x;
            let x_pixel = (x as f32 * vegetation_block_size_pixel) as i32;

            if classification.high[index] != 0 {
                draw_filled_ellipse_mut(
                    &mut base_vegetation_img,
                    (x_pixel, y_pixel),
                    casted_base_vegetation_block_size_pixel,
                    casted_base_vegetation_block_size_pixel,
                    WHITE,
                );
            }

            if classification.undergrowth[index] != 0 {
                let color = match undergrowth_mode {
                    UndergrowthMode::Symbol406 => GREEN_1,
                    UndergrowthMode::Symbol409 => GREEN_3,
                    UndergrowthMode::None | UndergrowthMode::Merge => unreachable!(),
                };
                draw_filled_rect_mut(
                    &mut undergrowth_vegetation_img,
                    Rect::at(x_pixel, y_pixel)
                        .of_size(casted_green_block_size_pixel, casted_green_block_size_pixel),
                    color,
                );
            }
        }
    }

    // Green rectangles may overlap after DPI scaling and have different colors.
    // Keep the original x-major drawing order so existing color precedence is
    // unchanged even though classification now traverses source pixels by row.
    for x in 0..classification.width {
        let x_pixel = (x as f32 * vegetation_block_size_pixel) as i32;
        for y in 0..classification.height {
            let index = y * classification.width + x;
            let color = match classification.green[index] {
                1 => GREEN_1,
                2 => GREEN_2,
                3 => GREEN_3,
                _ => continue,
            };
            let y_pixel = (y as f32 * vegetation_block_size_pixel) as i32;
            draw_filled_rect_mut(
                &mut green_vegetation_img,
                Rect::at(x_pixel, y_pixel)
                    .of_size(casted_green_block_size_pixel, casted_green_block_size_pixel),
                color,
            );
        }
    }

    match undergrowth_mode {
        UndergrowthMode::Symbol406 => {
            imageops::overlay(&mut base_vegetation_img, &undergrowth_vegetation_img, 0, 0);
        }
        UndergrowthMode::Symbol409 => {
            let undergrowth_output_path = tile.render_dir_path.join("undergrowth.png");

            undergrowth_vegetation_img
                .save(&undergrowth_output_path)
                .context(format!(
                    "could not save `{}`",
                    undergrowth_output_path.display()
                ))?;
        }
        UndergrowthMode::None | UndergrowthMode::Merge => {}
    }

    imageops::overlay(&mut base_vegetation_img, &green_vegetation_img, 0, 0);
    let vegetation_output_path = tile.render_dir_path.join("vegetation.png");

    base_vegetation_img
        .save(&vegetation_output_path)
        .context(format!(
            "could not save `{}`",
            vegetation_output_path.display()
        ))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Vegetation rendered in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    Ok(())
}

#[derive(Clone, Copy)]
struct RasterInterior {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

struct VegetationClassification {
    width: usize,
    height: usize,
    high: Vec<u8>,
    green: Vec<u8>,
    undergrowth: Vec<u8>,
}

fn vegetation_interior(tile: &Tile, images: [&TifImage; 3]) -> Result<RasterInterior> {
    let width = tile
        .max_x
        .checked_sub(tile.min_x)
        .and_then(|width| usize::try_from(width).ok())
        .ok_or_else(|| CassiniError::InvalidArtifact {
            path: tile.render_dir_path.clone(),
            message: "tile width must be non-negative and addressable".to_owned(),
        })?;
    let height = tile
        .max_y
        .checked_sub(tile.min_y)
        .and_then(|height| usize::try_from(height).ok())
        .ok_or_else(|| CassiniError::InvalidArtifact {
            path: tile.render_dir_path.clone(),
            message: "tile height must be non-negative and addressable".to_owned(),
        })?;
    let required_width = BUFFER
        .checked_mul(2)
        .and_then(|buffer| width.checked_add(buffer))
        .ok_or_else(|| CassiniError::InvalidArtifact {
            path: tile.render_dir_path.clone(),
            message: "buffered vegetation width overflows addressable memory".to_owned(),
        })?;
    let required_height = BUFFER
        .checked_mul(2)
        .and_then(|buffer| height.checked_add(buffer))
        .ok_or_else(|| CassiniError::InvalidArtifact {
            path: tile.render_dir_path.clone(),
            message: "buffered vegetation height overflows addressable memory".to_owned(),
        })?;

    let dimensions = (images[0].width, images[0].height);
    if images
        .iter()
        .any(|image| (image.width, image.height) != dimensions)
    {
        return Err(CassiniError::InvalidArtifact {
            path: tile.render_dir_path.clone(),
            message: "buffered vegetation rasters must have identical dimensions".to_owned(),
        });
    }
    if (dimensions.0 as usize) < required_width || (dimensions.1 as usize) < required_height {
        return Err(CassiniError::InvalidArtifact {
            path: tile.render_dir_path.clone(),
            message: format!(
                "buffered vegetation rasters are {}x{} but at least {required_width}x{required_height} pixels are required",
                dimensions.0, dimensions.1
            ),
        });
    }

    Ok(RasterInterior {
        x: BUFFER,
        y: BUFFER,
        width,
        height,
    })
}

fn classify_vegetation(
    high: &TifImage,
    medium: &TifImage,
    low: &TifImage,
    interior: RasterInterior,
    config: &Config,
    undergrowth_mode: &UndergrowthMode,
) -> VegetationClassification {
    let medium_kernel = gaussian_kernel(2);
    let merge_low = matches!(undergrowth_mode, UndergrowthMode::Merge).then_some(&low.pixels[..]);
    let medium_density = separable_gaussian_filter(
        &medium.pixels,
        merge_low,
        medium.width as usize,
        interior,
        &medium_kernel,
    );
    let low_density = matches!(
        undergrowth_mode,
        UndergrowthMode::Symbol406 | UndergrowthMode::Symbol409
    )
    .then(|| {
        separable_gaussian_filter(
            &low.pixels,
            None,
            low.width as usize,
            interior,
            &gaussian_kernel(4),
        )
    });

    let output_len = interior.width * interior.height;
    let mut classification = VegetationClassification {
        width: interior.width,
        height: interior.height,
        high: vec![0; output_len],
        green: vec![0; output_len],
        undergrowth: vec![0; output_len],
    };
    let input_width = high.width as usize;

    for output_y in 0..interior.height {
        let source_y = interior.y + output_y;
        for output_x in 0..interior.width {
            let source_x = interior.x + output_x;
            let output_index = output_y * interior.width + output_x;
            classification.high[output_index] = u8::from(
                minimum_3_by_3(&high.pixels, input_width, source_x, source_y)
                    > config.yellow_threshold as u8,
            );

            let density = medium_density[output_index];
            classification.green[output_index] = if density > config.green_threshold_3 {
                3
            } else if density > config.green_threshold_2 {
                2
            } else if density > config.green_threshold_1 {
                1
            } else {
                0
            };

            if let Some(low_density) = &low_density {
                classification.undergrowth[output_index] =
                    u8::from(low_density[output_index] > config.low_vegetation_density_threshold);
            }
        }
    }

    classification
}

#[inline]
fn minimum_3_by_3(pixels: &[u8], width: usize, x: usize, y: usize) -> u8 {
    let mut minimum = u8::MAX;
    for row in (y - 1)..=(y + 1) {
        for &pixel in &pixels[(row * width + x - 1)..=(row * width + x + 1)] {
            minimum = minimum.min(pixel);
        }
    }
    minimum
}

fn gaussian_kernel(radius: usize) -> Vec<f32> {
    let size = 2 * radius + 1;
    let sigma = radius as f32 / 2.0_f32; // avoid sigma = 0
    let two_sigma_sq = 2.0 * sigma * sigma;
    let mut kernel = Vec::with_capacity(size);
    let mut sum = 0.0;
    let center = radius as isize;

    for x in 0..size {
        let dx = x as isize - center;
        let value = E.powf(-((dx * dx) as f32) / two_sigma_sq);
        kernel.push(value);
        sum += value;
    }
    for value in &mut kernel {
        *value /= sum;
    }

    kernel
}

fn separable_gaussian_filter(
    primary: &[u8],
    secondary: Option<&[u8]>,
    input_width: usize,
    interior: RasterInterior,
    kernel: &[f32],
) -> Vec<f32> {
    debug_assert!(kernel.len() > 1 && kernel.len() % 2 == 1);
    debug_assert!(secondary.is_none_or(|pixels| pixels.len() == primary.len()));
    let radius = kernel.len() / 2;
    let intermediate_height = interior.height + 2 * radius;
    let mut horizontal = vec![0.0f32; interior.width * intermediate_height];

    for intermediate_y in 0..intermediate_height {
        let source_y = interior.y + intermediate_y - radius;
        let source_row = source_y * input_width;
        let target_row = intermediate_y * interior.width;
        for output_x in 0..interior.width {
            let source_x = interior.x + output_x;
            let mut sum = 0.0f32;
            for (kernel_x, &weight) in kernel.iter().enumerate() {
                let source_index = source_row + source_x + kernel_x - radius;
                let value = match secondary {
                    Some(secondary) => {
                        primary[source_index] as u16 + secondary[source_index] as u16
                    }
                    None => primary[source_index] as u16,
                };
                sum += value as f32 * weight;
            }
            horizontal[target_row + output_x] = sum;
        }
    }

    let mut output = vec![0.0f32; interior.width * interior.height];
    for output_y in 0..interior.height {
        let output_row = output_y * interior.width;
        for output_x in 0..interior.width {
            let mut sum = 0.0f32;
            for (kernel_y, &weight) in kernel.iter().enumerate() {
                sum += horizontal[(output_y + kernel_y) * interior.width + output_x] * weight;
            }
            output[output_row + output_x] = sum;
        }
    }
    output
}

struct TifImage {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

fn get_image_data_from_tif(path: &Path) -> Result<TifImage> {
    let tif_file = File::open(path).context(format!("could not open `{}`", path.display()))?;
    let mut img_decoder = Decoder::new(tif_file)
        .context(format!("could not create decoder for `{}`", path.display()))?;
    img_decoder = img_decoder.with_limits(tiff::decoder::Limits::unlimited());
    let (width, height) = img_decoder.dimensions().context(format!(
        "could not read dimensions from `{}`",
        path.display()
    ))?;

    let decoded = img_decoder
        .read_image()
        .context(format!("could not decode `{}`", path.display()))?;
    let DecodingResult::U8(image_data) = decoded else {
        return Err(CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message: "expected an 8-bit TIFF band".to_owned(),
        });
    };

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message: "TIFF dimensions overflow addressable memory".to_owned(),
        })?;
    if image_data.len() != expected_len {
        return Err(CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message: format!(
                "TIFF contains {} pixels but its dimensions require {expected_len}",
                image_data.len()
            ),
        });
    }

    Ok(TifImage {
        pixels: image_data,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint::black_box, time::Instant};

    fn synthetic_image(width: usize, height: usize, seed: u32) -> TifImage {
        let mut state = seed;
        let pixels = (0..width * height)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        TifImage {
            pixels,
            width: width as u32,
            height: height as u32,
        }
    }

    fn legacy_kernel(radius: usize) -> Vec<Vec<f32>> {
        let size = 2 * radius + 1;
        let sigma = radius as f32 / 2.0;
        let two_sigma_sq = 2.0 * sigma * sigma;
        let center = radius as isize;
        let mut kernel = vec![vec![0.0; size]; size];
        let mut sum = 0.0;
        for (y, row) in kernel.iter_mut().enumerate() {
            for (x, value) in row.iter_mut().enumerate() {
                let dx = x as isize - center;
                let dy = y as isize - center;
                *value = E.powf(-((dx * dx + dy * dy) as f32) / two_sigma_sq);
                sum += *value;
            }
        }
        for row in &mut kernel {
            for value in row {
                *value /= sum;
            }
        }
        kernel
    }

    fn legacy_filter(image: &TifImage, interior: RasterInterior, radius: usize) -> Vec<f32> {
        let kernel = legacy_kernel(radius);
        let input_width = image.width as usize;
        let input_height = image.height as usize;
        let mut output = vec![0.0; interior.width * interior.height];
        for output_x in 0..interior.width {
            for output_y in 0..interior.height {
                let source_x = interior.x + output_x;
                let source_y = interior.y + output_y;
                let mut weighted_sum = 0.0;
                let mut weight_total = 0.0;
                for (kernel_y, row) in kernel.iter().enumerate() {
                    for (kernel_x, &weight) in row.iter().enumerate() {
                        let x = source_x as isize + kernel_x as isize - radius as isize;
                        let y = source_y as isize + kernel_y as isize - radius as isize;
                        if x < 0 || y < 0 || x >= input_width as isize || y >= input_height as isize
                        {
                            continue;
                        }
                        weighted_sum +=
                            image.pixels[y as usize * input_width + x as usize] as f32 * weight;
                        weight_total += weight;
                    }
                }
                output[output_y * interior.width + output_x] = weighted_sum / weight_total;
            }
        }
        output
    }

    fn legacy_minimum(image: &TifImage, x: usize, y: usize) -> u8 {
        let mut minimum = u8::MAX;
        let width = image.width as usize;
        let height = image.height as usize;
        for matrix_y in 0..5 {
            for matrix_x in 0..5 {
                if x + matrix_x < 2
                    || y + matrix_y < 2
                    || matrix_y == 0
                    || matrix_x == 0
                    || matrix_y == 4
                    || matrix_x == 4
                {
                    continue;
                }
                let sample_x = x + matrix_x - 2;
                let sample_y = y + matrix_y - 2;
                if sample_x < width && sample_y < height {
                    minimum = minimum.min(image.pixels[sample_y * width + sample_x]);
                }
            }
        }
        minimum
    }

    fn legacy_classify(
        high: &TifImage,
        medium: &TifImage,
        low: &TifImage,
        interior: RasterInterior,
        config: &Config,
        undergrowth_mode: &UndergrowthMode,
    ) -> VegetationClassification {
        let mut medium_density = legacy_filter(medium, interior, 2);
        if matches!(undergrowth_mode, UndergrowthMode::Merge) {
            let low_medium_density = legacy_filter(low, interior, 2);
            for (medium, low) in medium_density.iter_mut().zip(low_medium_density) {
                *medium += low;
            }
        }
        let low_density = matches!(
            undergrowth_mode,
            UndergrowthMode::Symbol406 | UndergrowthMode::Symbol409
        )
        .then(|| legacy_filter(low, interior, 4));
        let output_len = interior.width * interior.height;
        let mut result = VegetationClassification {
            width: interior.width,
            height: interior.height,
            high: vec![0; output_len],
            green: vec![0; output_len],
            undergrowth: vec![0; output_len],
        };
        for output_x in 0..interior.width {
            for output_y in 0..interior.height {
                let source_x = interior.x + output_x;
                let source_y = interior.y + output_y;
                let output_index = output_y * interior.width + output_x;
                result.high[output_index] = u8::from(
                    legacy_minimum(high, source_x, source_y) > config.yellow_threshold as u8,
                );
                let density = medium_density[output_index];
                result.green[output_index] = if density > config.green_threshold_3 {
                    3
                } else if density > config.green_threshold_2 {
                    2
                } else if density > config.green_threshold_1 {
                    1
                } else {
                    0
                };
                if let Some(low_density) = &low_density {
                    result.undergrowth[output_index] = u8::from(
                        low_density[output_index] > config.low_vegetation_density_threshold,
                    );
                }
            }
        }
        result
    }

    fn assert_same_classification(
        actual: &VegetationClassification,
        expected: &VegetationClassification,
    ) {
        assert_eq!(actual.high, expected.high);
        assert_eq!(actual.green, expected.green);
        assert_eq!(actual.undergrowth, expected.undergrowth);
    }

    #[test]
    fn separable_filter_stays_within_float_tolerance_of_legacy_filter() {
        let width = 40;
        let height = 38;
        let primary = synthetic_image(width, height, 1);
        let secondary = synthetic_image(width, height, 2);
        let interior = RasterInterior {
            x: 6,
            y: 6,
            width: 27,
            height: 25,
        };

        for (radius, second) in [(2, None), (2, Some(&secondary)), (4, None)] {
            let optimized = separable_gaussian_filter(
                &primary.pixels,
                second.map(|image| &image.pixels[..]),
                width,
                interior,
                &gaussian_kernel(radius),
            );
            let mut legacy = legacy_filter(&primary, interior, radius);
            if let Some(second) = second {
                let legacy_second = legacy_filter(second, interior, radius);
                for (primary, secondary) in legacy.iter_mut().zip(legacy_second) {
                    *primary += secondary;
                }
            }
            let max_difference = optimized
                .iter()
                .zip(legacy.iter())
                .map(|(optimized, legacy)| (optimized - legacy).abs())
                .fold(0.0f32, f32::max);
            assert!(
                max_difference < 0.0002,
                "maximum filter difference was {max_difference}"
            );
        }
    }

    #[test]
    fn classification_has_strict_threshold_boundaries() {
        let width = 15;
        let height = 15;
        let interior = RasterInterior {
            x: 5,
            y: 5,
            width: 5,
            height: 5,
        };
        let config = Config::default();
        for value in 0..=5 {
            let image = TifImage {
                pixels: vec![value; width * height],
                width: width as u32,
                height: height as u32,
            };
            for mode in [
                UndergrowthMode::None,
                UndergrowthMode::Merge,
                UndergrowthMode::Symbol406,
                UndergrowthMode::Symbol409,
            ] {
                let optimized =
                    classify_vegetation(&image, &image, &image, interior, &config, &mode);
                let expected_high = u8::from(value > 1);
                let density = if matches!(mode, UndergrowthMode::Merge) {
                    value * 2
                } else {
                    value
                };
                let expected_green = if density > 3 {
                    3
                } else if density > 2 {
                    2
                } else if density > 1 {
                    1
                } else {
                    0
                };
                let expected_undergrowth = u8::from(
                    matches!(
                        mode,
                        UndergrowthMode::Symbol406 | UndergrowthMode::Symbol409
                    ) && value > 1,
                );
                assert!(optimized.high.iter().all(|&class| class == expected_high));
                assert!(
                    optimized.green.iter().all(|&class| class == expected_green),
                    "unexpected green class for value={value}, mode={mode:?}"
                );
                assert!(
                    optimized
                        .undergrowth
                        .iter()
                        .all(|&class| class == expected_undergrowth),
                    "unexpected undergrowth class for value={value}, mode={mode:?}"
                );
            }
        }
    }

    fn benchmark_strategy(
        optimized: bool,
        high: &TifImage,
        medium: &TifImage,
        low: &TifImage,
        interior: RasterInterior,
        config: &Config,
        mode: &UndergrowthMode,
    ) -> (f64, VegetationClassification) {
        let started = Instant::now();
        let result = if optimized {
            classify_vegetation(high, medium, low, interior, config, mode)
        } else {
            legacy_classify(high, medium, low, interior, config, mode)
        };
        let elapsed = started.elapsed().as_secs_f64();
        black_box(&result);
        (elapsed, result)
    }

    #[test]
    #[ignore = "performance benchmark; run explicitly with --ignored"]
    fn benchmark_vegetation_classification() {
        let size = std::env::var("CASSINI_VEGETATION_BENCHMARK_SIZE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1_000);
        let repetitions = std::env::var("CASSINI_BENCHMARK_REPETITIONS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5);
        let input_size = size + 2 * BUFFER;
        let high = synthetic_image(input_size, input_size, 11);
        let medium = synthetic_image(input_size, input_size, 22);
        let low = synthetic_image(input_size, input_size, 33);
        let interior = RasterInterior {
            x: BUFFER,
            y: BUFFER,
            width: size,
            height: size,
        };
        let config = Config::default();

        for mode in [UndergrowthMode::Merge, UndergrowthMode::Symbol409] {
            let mut legacy_timings = Vec::with_capacity(repetitions);
            let mut optimized_timings = Vec::with_capacity(repetitions);
            for repetition in 0..repetitions {
                let mut expected = None;
                for optimized in [repetition % 2 != 0, repetition % 2 == 0] {
                    let (elapsed, result) = benchmark_strategy(
                        optimized, &high, &medium, &low, interior, &config, &mode,
                    );
                    if let Some(expected) = &expected {
                        assert_same_classification(&result, expected);
                    } else {
                        expected = Some(result);
                    }
                    if optimized {
                        optimized_timings.push(elapsed);
                    } else {
                        legacy_timings.push(elapsed);
                    }
                }
            }

            let legacy_mean = legacy_timings.iter().sum::<f64>() / repetitions as f64;
            let optimized_mean = optimized_timings.iter().sum::<f64>() / repetitions as f64;
            let improvement = 100.0 * (legacy_mean - optimized_mean) / legacy_mean;
            println!(
                "vegetation benchmark: mode={mode:?}, cells={}, repetitions={repetitions}, legacy_mean={legacy_mean:.6}s, optimized_mean={optimized_mean:.6}s, improvement={improvement:.2}%",
                size * size
            );
        }
    }
}
