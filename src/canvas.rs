use skia_safe::{
    surfaces, Color, Data, EncodedImageFormat, Image, Paint, PaintCap, PaintStyle, Path,
    PathEffect, PathFillType, Surface,
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
    pub fn save(&mut self) {
        self.canvas().save();
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
        self.save();
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
        self.save();
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
        self.save();
    }

    #[inline]
    pub fn data(&mut self) -> Result<Data> {
        let image = self.surface.image_snapshot();
        let mut context = self.surface.direct_context();
        image
            .encode(context.as_mut(), EncodedImageFormat::PNG, None)
            .ok_or_else(|| CassiniError::InvalidInput {
                message: "could not encode canvas as PNG".to_owned(),
            })
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
    pub fn save_as(&mut self, filename: &FsPath) -> Result<()> {
        let d = self.data()?;
        let mut file =
            File::create(filename).context(format!("could not create `{}`", filename.display()))?;
        let bytes = d.as_bytes();
        file.write_all(bytes)
            .context(format!("could not write `{}`", filename.display()))?;
        Ok(())
    }

    #[inline]
    pub fn load_from(filename: &FsPath) -> Result<Canvas> {
        let data = Data::from_filename(filename).ok_or_else(|| CassiniError::InvalidArtifact {
            path: filename.to_path_buf(),
            message: "could not read image data".to_owned(),
        })?;
        let image = Image::from_encoded(data).ok_or_else(|| CassiniError::InvalidArtifact {
            path: filename.to_path_buf(),
            message: "could not decode image".to_owned(),
        })?;
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
