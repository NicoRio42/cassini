use image::{codecs::webp::WebPEncoder, ExtendedColorType};
use skia_safe::{
    images, surfaces, AlphaType, Color, ColorType, Data, Image, ImageInfo, Paint, PaintCap,
    PaintStyle, Path, PathEffect, PathFillType, Surface,
};
use std::fs::File;
use std::io::Write;
use std::mem;
use std::path::Path as FsPath;

use crate::error::{CassiniError, Result, ResultContext};

pub struct Canvas {
    surface: Surface,
    path: Path,
    paint: Paint,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Result<Canvas> {
        if width <= 0 || height <= 0 {
            return Err(CassiniError::InvalidInput {
                message: format!("canvas dimensions must be positive, got {width}x{height}"),
            });
        }

        let mut surface = surfaces::raster_n32_premul((width, height)).ok_or_else(|| {
            CassiniError::InvalidInput {
                message: format!("could not allocate a {width}x{height} canvas"),
            }
        })?;
        let path = Path::new();
        let mut paint = Paint::default();
        paint.set_color(Color::BLACK);
        paint.set_anti_alias(true);
        paint.set_stroke_width(1.0);
        surface.canvas().clear(0x00000000);
        Ok(Canvas {
            surface,
            path,
            paint,
        })
    }

    #[inline]
    pub fn set_line_width(&mut self, width: f32) {
        self.paint.set_stroke_width(width);
    }

    #[inline]
    pub fn set_color(&mut self, rgb: (u8, u8, u8)) {
        self.paint.set_blend_mode(skia_safe::BlendMode::SrcOver);
        self.paint.set_color(Color::from_rgb(rgb.0, rgb.1, rgb.2));
    }

    #[inline]
    pub fn set_color_with_alpha(&mut self, rgb: (u8, u8, u8), alpha: u8) {
        self.paint.set_blend_mode(skia_safe::BlendMode::SrcOver);
        self.paint
            .set_color(Color::from_argb(alpha, rgb.0, rgb.1, rgb.2));
    }

    #[inline]
    pub fn set_transparent_color(&mut self) {
        self.paint.set_blend_mode(skia_safe::BlendMode::SrcIn);
        self.paint.set_color(Color::TRANSPARENT);
    }

    #[inline]
    pub fn _set_stroke_cap_round(&mut self) {
        self.paint.set_stroke_cap(PaintCap::Round);
    }

    #[inline]
    pub fn _unset_stroke_cap(&mut self) {
        self.paint.set_stroke_cap(PaintCap::Butt);
    }

    #[inline]
    pub fn set_dash(&mut self, interval_on: f32, interval_off: f32) {
        self.paint
            .set_path_effect(PathEffect::dash(&[interval_on, interval_off], 0.0));
    }

    #[inline]
    pub fn unset_dash(&mut self) {
        self.paint.set_path_effect(None);
    }

    #[inline]
    pub fn draw_polyline(&mut self, pts: &[(f32, f32)]) {
        let new_path = Path::new();
        let _ = mem::replace(&mut self.path, new_path);
        self.paint.set_style(PaintStyle::Stroke);
        self.path.move_to((pts[0].0, pts[0].1));
        for pt in pts.iter() {
            self.path.line_to((pt.0, pt.1));
        }
        self.surface.canvas().draw_path(&self.path, &self.paint);
    }

    #[inline]
    pub fn draw_filled_polygon(&mut self, pts: &[(f32, f32)]) {
        let new_path = Path::new();
        let _ = mem::replace(&mut self.path, new_path);
        self.paint.set_style(PaintStyle::StrokeAndFill);
        self.path.move_to((pts[0].0, pts[0].1));

        for pt in pts.iter() {
            self.path.line_to((pt.0, pt.1));
        }

        self.surface.canvas().draw_path(&self.path, &self.paint);
    }

    #[inline]
    pub fn draw_filled_polygon_with_holes(
        &mut self,
        outer_geometry: &[(f32, f32)],
        holes: &Vec<Vec<(f32, f32)>>,
    ) {
        let new_path = Path::new();
        let _ = mem::replace(&mut self.path, new_path);
        self.paint.set_style(PaintStyle::StrokeAndFill);
        self.path
            .move_to((outer_geometry[0].0, outer_geometry[0].1));

        for pt in outer_geometry.iter() {
            self.path.line_to((pt.0, pt.1));
        }

        self.path.close();

        for hole in holes {
            self.path.move_to((hole[0].0, hole[0].1));

            for pt in hole.iter() {
                self.path.line_to((pt.0, pt.1));
            }

            self.path.close();
        }

        self.path.set_fill_type(PathFillType::EvenOdd);

        self.surface.canvas().draw_path(&self.path, &self.paint);
    }

    #[inline]
    pub fn lossless_webp_data(&mut self) -> Result<Vec<u8>> {
        let image = self.surface.image_snapshot();
        let width = image.width();
        let height = image.height();
        let image_info = ImageInfo::new(
            (width, height),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let row_bytes = image_info.min_row_bytes();
        let mut pixels = vec![0; image_info.compute_byte_size(row_bytes)];

        if !self
            .canvas()
            .read_pixels(&image_info, &mut pixels, row_bytes, (0, 0))
        {
            return Err(CassiniError::InvalidInput {
                message: "could not read canvas pixels for lossless WebP encoding".to_owned(),
            });
        }

        let mut encoded = Vec::new();
        WebPEncoder::new_lossless(&mut encoded)
            .encode(
                &pixels,
                width as u32,
                height as u32,
                ExtendedColorType::Rgba8,
            )
            .context("could not encode canvas as lossless WebP")?;
        Ok(encoded)
    }

    #[inline]
    pub fn image(&mut self) -> skia_safe::image::Image {
        self.surface.image_snapshot()
    }

    #[inline]
    fn canvas(&mut self) -> &skia_safe::Canvas {
        self.surface.canvas()
    }

    #[inline]
    pub fn save_as_lossless_webp(&mut self, filename: &FsPath) -> Result<()> {
        let d = self.lossless_webp_data()?;
        let mut file =
            File::create(filename).context(format!("could not create `{}`", filename.display()))?;
        file.write_all(&d)
            .context(format!("could not write `{}`", filename.display()))?;
        Ok(())
    }

    #[inline]
    pub fn load_from(filename: &FsPath) -> Result<Canvas> {
        let is_webp = filename
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("webp"));

        let image = if is_webp {
            let rgba = image::open(filename)
                .context(format!("could not decode `{}`", filename.display()))?
                .into_rgba8();
            let (width, height) = rgba.dimensions();
            let image_info = ImageInfo::new(
                (width as i32, height as i32),
                ColorType::RGBA8888,
                AlphaType::Unpremul,
                None,
            );
            images::raster_from_data(
                &image_info,
                Data::new_copy(rgba.as_raw()),
                image_info.min_row_bytes(),
            )
            .ok_or_else(|| CassiniError::InvalidArtifact {
                path: filename.to_path_buf(),
                message: "could not create a canvas image from decoded WebP pixels".to_owned(),
            })?
        } else {
            let data =
                Data::from_filename(filename).ok_or_else(|| CassiniError::InvalidArtifact {
                    path: filename.to_path_buf(),
                    message: "could not read image data".to_owned(),
                })?;
            Image::from_encoded(data).ok_or_else(|| CassiniError::InvalidArtifact {
                path: filename.to_path_buf(),
                message: "could not decode image".to_owned(),
            })?
        };
        let mut c = Canvas::new(image.width(), image.height())?;
        c.draw_image(image);
        Ok(c)
    }

    #[inline]
    pub fn draw_image(&mut self, image: Image) {
        self.surface.canvas().draw_image(image, (0, 0), None);
    }

    #[inline]
    pub fn overlay(&mut self, other_canvas: &mut Canvas, x: f32, y: f32) {
        self.surface
            .canvas()
            .draw_image(other_canvas.image(), (x, y), None);
    }
}

#[cfg(test)]
mod tests {
    use super::Canvas;
    use std::time::Instant;

    fn draw_vector_dense_scene(
        geometry_count: usize,
        emulate_unbalanced_saves: bool,
    ) -> (f64, usize, Vec<u8>) {
        let mut canvas = Canvas::new(64, 64).unwrap();
        let polyline = [(1.0, 1.0), (62.0, 8.0), (8.0, 32.0), (62.0, 62.0)];
        let polygon = [(4.0, 4.0), (60.0, 4.0), (32.0, 60.0), (4.0, 4.0)];
        let outer = [(2.0, 2.0), (62.0, 2.0), (62.0, 62.0), (2.0, 62.0)];
        let holes = vec![vec![(20.0, 20.0), (44.0, 20.0), (44.0, 44.0), (20.0, 44.0)]];

        canvas.set_color((12, 34, 56));
        let started = Instant::now();
        for geometry_index in 0..geometry_count {
            match geometry_index % 3 {
                0 => canvas.draw_polyline(&polyline),
                1 => canvas.draw_filled_polygon(&polygon),
                _ => canvas.draw_filled_polygon_with_holes(&outer, &holes),
            }
            if emulate_unbalanced_saves {
                canvas.canvas().save();
            }
        }
        let draw_elapsed = started.elapsed();
        let save_count = canvas.canvas().save_count();
        let pixels = canvas.lossless_webp_data().unwrap();
        let cleanup_started = Instant::now();
        drop(canvas);
        let elapsed = draw_elapsed + cleanup_started.elapsed();
        (elapsed.as_secs_f64(), save_count, pixels)
    }

    #[test]
    fn lossless_webp_round_trips_canvas_pixels() {
        let mut canvas = Canvas::new(2, 2).unwrap();
        canvas.set_color((12, 34, 56));
        canvas.draw_filled_polygon(&[
            (-1.0, -1.0),
            (3.0, -1.0),
            (3.0, 3.0),
            (-1.0, 3.0),
            (-1.0, -1.0),
        ]);

        let encoded = canvas.lossless_webp_data().unwrap();
        assert_eq!(&encoded[0..4], b"RIFF");
        assert_eq!(&encoded[8..12], b"WEBP");

        let decoded = image::load_from_memory_with_format(&encoded, image::ImageFormat::WebP)
            .unwrap()
            .into_rgba8();
        assert!(decoded.pixels().all(|pixel| pixel.0 == [12, 34, 56, 255]));
    }

    #[test]
    fn drawing_does_not_grow_the_canvas_save_stack() {
        let mut canvas = Canvas::new(64, 64).unwrap();
        let initial_save_count = canvas.canvas().save_count();
        let polyline = [(1.0, 1.0), (62.0, 62.0)];
        let polygon = [(4.0, 4.0), (60.0, 4.0), (32.0, 60.0), (4.0, 4.0)];
        let outer = [(2.0, 2.0), (62.0, 2.0), (62.0, 62.0), (2.0, 62.0)];
        let holes = vec![vec![(20.0, 20.0), (44.0, 20.0), (44.0, 44.0), (20.0, 44.0)]];

        canvas.draw_polyline(&polyline);
        canvas.draw_filled_polygon(&polygon);
        canvas.draw_filled_polygon_with_holes(&outer, &holes);

        assert_eq!(canvas.canvas().save_count(), initial_save_count);
    }

    #[test]
    #[ignore = "performance benchmark; run explicitly with --ignored"]
    fn benchmark_vector_dense_canvas_drawing() {
        let geometry_count = std::env::var("CASSINI_CANVAS_BENCHMARK_GEOMETRIES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(100_000);
        let repetitions = std::env::var("CASSINI_BENCHMARK_REPETITIONS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5);

        let mut legacy_timings = Vec::with_capacity(repetitions);
        let mut optimized_timings = Vec::with_capacity(repetitions);
        let mut expected_output = None;
        let mut legacy_save_count = 0;
        let mut optimized_save_count = 0;
        for repetition in 0..repetitions {
            for emulate_unbalanced_saves in [repetition % 2 == 0, repetition % 2 != 0] {
                let (elapsed, save_count, output) =
                    draw_vector_dense_scene(geometry_count, emulate_unbalanced_saves);
                if let Some(expected) = &expected_output {
                    assert_eq!(
                        &output, expected,
                        "benchmark strategies produced different output"
                    );
                } else {
                    expected_output = Some(output);
                }
                if emulate_unbalanced_saves {
                    legacy_timings.push(elapsed);
                    legacy_save_count = save_count;
                } else {
                    optimized_timings.push(elapsed);
                    optimized_save_count = save_count;
                }
            }
        }

        legacy_timings.sort_by(f64::total_cmp);
        optimized_timings.sort_by(f64::total_cmp);
        let legacy_mean = legacy_timings.iter().sum::<f64>() / repetitions as f64;
        let optimized_mean = optimized_timings.iter().sum::<f64>() / repetitions as f64;
        let improvement = 100.0 * (legacy_mean - optimized_mean) / legacy_mean;
        println!(
            "canvas benchmark: {geometry_count} geometries, {repetitions} repetitions, legacy_mean={legacy_mean:.6}s, optimized_mean={optimized_mean:.6}s, improvement={improvement:.2}%, legacy_save_count={legacy_save_count}, optimized_save_count={optimized_save_count}, webp_bytes={}",
            expected_output.unwrap().len()
        );
    }
}
